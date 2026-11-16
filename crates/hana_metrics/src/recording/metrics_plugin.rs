//! Service installation, registration and timestamp-preserving ingestion.

use bevy_app::App;
use bevy_app::First;
use bevy_app::Last;
use bevy_app::Plugin;
use bevy_diagnostic::Diagnostic;
use bevy_diagnostic::DiagnosticMeasurement;
use bevy_diagnostic::DiagnosticsPlugin;
use bevy_diagnostic::DiagnosticsStore;
use bevy_ecs::prelude::IntoScheduleConfigs;
use bevy_ecs::prelude::Mut;
use bevy_ecs::prelude::Res;
use bevy_ecs::prelude::ResMut;
use bevy_ecs::prelude::SystemSet;
use hana_kana::ToF64;

#[cfg(feature = "test")]
use super::ClockSource;
use super::EvictionHistory;
use super::MeasurementFailure;
use super::MetricRecorder;
use super::MetricRegistry;
use super::MetricSource;
use super::ObservationClock;
use super::RegisteredMetric;
use super::lock;
#[cfg(feature = "test")]
use crate::ManualRunClock;
use crate::MetricDefinition;
use crate::MetricKind;
use crate::MetricNumber;
use crate::MetricRegistrationError;
use crate::NativeDiagnosticDescription;
use crate::definition;
use crate::demand;
use crate::demand::DemandLedger;

/// Installs the shared diagnostic namespace, clock, demand and append ingest.
pub struct MetricsPlugin;

/// Ordering boundaries for consumer feeds and ordinary producers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum MetricsSystems {
    /// Applies queued subscription changes before producers run.
    Reconcile,
    /// Drains accepted observations with their original timestamps.
    Ingest,
}

impl Plugin for MetricsPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<DiagnosticsPlugin>() {
            app.add_plugins(DiagnosticsPlugin);
        }
        let clock = ObservationClock::monotonic();
        #[cfg(feature = "test")]
        let clock = app.world().get_resource::<ManualRunClock>().map_or_else(
            || clock.clone(),
            |manual| ObservationClock {
                origin: clock.origin,
                source: ClockSource::Manual(manual.clone()),
            },
        );
        app.insert_resource(clock)
            .init_resource::<MetricRegistry>()
            .init_resource::<DemandLedger>()
            .init_resource::<crate::MetricsSettings>()
            .configure_sets(
                First,
                (MetricsSystems::Reconcile, MetricsSystems::Ingest).chain(),
            )
            .add_systems(First, demand::reconcile.in_set(MetricsSystems::Reconcile))
            .add_systems(First, ingest.in_set(MetricsSystems::Ingest))
            .add_systems(Last, ingest.in_set(MetricsSystems::Ingest));
    }
    fn finish(&self, app: &mut App) {
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<MetricRegistry>| {
                discover_native(&mut registry, world.resource::<DiagnosticsStore>());
            });
    }
}

/// Registers a dormant typed producer after validating all App-free metadata.
///
/// # Errors
/// Rejects invalid metadata, duplicate paths (including native paths), or a missing service.
pub fn register_metric<K: MetricKind>(
    app: &mut App,
    definition: MetricDefinition<K>,
) -> Result<MetricRecorder<K>, MetricRegistrationError> {
    let descriptor = definition.describe()?;
    let world = app.world();
    let registry = world
        .get_resource::<MetricRegistry>()
        .ok_or(MetricRegistrationError::ServiceNotInstalled)?;
    let store = world
        .get_resource::<DiagnosticsStore>()
        .ok_or(MetricRegistrationError::ServiceNotInstalled)?;
    if registry.entries.contains_key(descriptor.path()) || store.get(descriptor.path()).is_some() {
        return Err(MetricRegistrationError::DuplicatePath);
    }
    let clock = world.resource::<ObservationClock>().clone();
    let (entry, shared) = RegisteredMetric::owned(descriptor, clock, definition.recording());
    let recorder = MetricRecorder::new(shared);
    let mut diagnostic = Diagnostic::new(entry.descriptor.path().clone());
    diagnostic.is_enabled = false;
    app.world_mut()
        .resource_mut::<DiagnosticsStore>()
        .add(diagnostic);
    app.world_mut()
        .resource_mut::<MetricRegistry>()
        .entries
        .insert(entry.descriptor.path().clone(), entry);
    Ok(recorder)
}

/// Explicitly adopts native metadata without replacing the existing diagnostic.
///
/// # Errors
/// Rejects invalid metadata, duplicate descriptions, missing service or missing native diagnostic.
pub fn describe_diagnostic(
    app: &mut App,
    description: NativeDiagnosticDescription,
) -> Result<(), MetricRegistrationError> {
    let descriptor = description.describe()?;
    let world = app.world();
    let registry = world
        .get_resource::<MetricRegistry>()
        .ok_or(MetricRegistrationError::ServiceNotInstalled)?;
    if registry
        .entries
        .get(descriptor.path())
        .is_some_and(|entry| !matches!(entry.source, MetricSource::DiscoveredNative))
    {
        return Err(MetricRegistrationError::DuplicatePath);
    }
    if world
        .resource::<DiagnosticsStore>()
        .get(descriptor.path())
        .is_none()
    {
        return Err(MetricRegistrationError::NativeDiagnosticMissing);
    }
    if let Some(entry) = app
        .world_mut()
        .resource_mut::<MetricRegistry>()
        .entries
        .get_mut(descriptor.path())
    {
        entry.descriptor = descriptor;
        entry.source = MetricSource::AdoptedNative;
        return Ok(());
    }
    let entry = RegisteredMetric::adopted(descriptor);
    app.world_mut()
        .resource_mut::<MetricRegistry>()
        .entries
        .insert(entry.descriptor.path().clone(), entry);
    Ok(())
}

fn ingest(
    mut registry: ResMut<MetricRegistry>,
    mut store: ResMut<DiagnosticsStore>,
    clock: Res<ObservationClock>,
) {
    if registry.active.is_empty() {
        return;
    }
    let MetricRegistry { entries, active } = &mut *registry;
    for path in active.iter() {
        let Some(entry) = entries.get_mut(path) else {
            continue;
        };
        drain_metric(entry, &mut store, &clock);
    }
}

pub(crate) fn drain_metric(
    entry: &mut RegisteredMetric,
    store: &mut DiagnosticsStore,
    clock: &ObservationClock,
) {
    let MetricSource::Owned(owned) = &mut entry.source else {
        return;
    };
    let recording_epoch = {
        let mut pending = lock(&owned.recorder.pending);
        std::mem::swap(&mut pending.samples, &mut owned.spare);
        pending.epoch
    };
    let mut samples = std::mem::take(&mut owned.spare);
    if samples.is_empty() {
        owned.spare = samples;
        return;
    }
    // Read after swapping: concurrent producers may append until the swap.
    let now = clock.now();
    if !samples.is_sorted_by_key(|sample| sample.extent.end()) {
        samples.sort_by_key(|sample| sample.extent.end());
    }
    for &sample in &samples {
        let accepted = entry.accept(sample, now, recording_epoch);
        let MetricSource::Owned(owned) = &mut entry.source else {
            continue;
        };
        match accepted {
            Ok(sample) => {
                let value = match sample.number {
                    MetricNumber::Scalar(value) => value,
                    MetricNumber::Count(total) => total.to_f64(),
                };
                let at = sample.extent.end();
                let Some(time) = clock.origin.checked_add(at.duration()) else {
                    owned.quality.reject(MeasurementFailure::ClockDomain);
                    continue;
                };
                owned
                    .smoothing
                    .observe(value, at, entry.descriptor.smoothing().method());
                if let Some(diagnostic) = store.get_mut(entry.descriptor.path()) {
                    diagnostic.add_measurement(DiagnosticMeasurement { time, value });
                }
                if owned.retained.len() == owned.retention_capacity.get()
                    && let Some(discarded) = owned.retained.pop_front()
                {
                    owned.evicted = EvictionHistory::Through {
                        at:       discarded.extent.end(),
                        sequence: discarded.sequence,
                    };
                }
                owned.retained.push_back(sample);
            },
            Err(failure) => owned.quality.reject(failure),
        }
    }
    samples.clear();
    if let MetricSource::Owned(owned) = &mut entry.source {
        owned.spare = samples;
    }
}

fn discover_native(registry: &mut MetricRegistry, store: &DiagnosticsStore) {
    for diagnostic in store.iter() {
        let path = diagnostic.path();
        if definition::validate_native_path(path.as_str()).is_err() {
            continue;
        }
        registry.entries.entry(path.clone()).or_insert_with(|| {
            RegisteredMetric::discovered(crate::MetricDescriptor::discovered(path.clone()))
        });
    }
}
