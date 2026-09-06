use bevy::diagnostic::FrameCount;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::World;
use bevy::prelude::error;
use bevy::window::OnMonitor;
use bevy::window::WindowPosition;
use hana_clerestory::CurrentMonitor;
use hana_clerestory::LiveDisplayContradictionCount;
use hana_clerestory::LiveDisplayEndpoint;
use hana_clerestory::LiveDisplayEndpointLookup;
use hana_clerestory::LiveDisplayMatchError;
use hana_clerestory::MonitorDescriptor;
use hana_clerestory::Monitors;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::RoleKeyError;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;
use serde::Serialize;
use serde::Serializer;

use super::ProbeSession;
use crate::ProbeMonitorSelection;
use crate::ProbeStartupMode;
use crate::constants::FIELD_SELECTED_MONITOR_INDEX;
use crate::constants::FIELD_WINDOW_KEY;
use crate::constants::KIND_RECOVERY_AVAILABLE;
use crate::constants::KIND_RECOVERY_CANCELLATION_REQUESTED;
use crate::constants::KIND_RECOVERY_MISMATCH;
use crate::constants::KIND_RECOVERY_PENDING;
use crate::constants::KIND_RECOVERY_READY;
use crate::constants::KIND_RECOVERY_RESTORED;
use crate::constants::KIND_WINDOW_CREATED;
use crate::constants::PROBE_SCHEMA_VERSION;
use crate::constants::PROBE_WINDOW_COUNT;
use crate::constants::PRODUCER_RECOVERY_READY;
use crate::control::CommandReceipt;
use crate::control::CommandReceipts;
use crate::setup::ProbeWindowRegistrationComplete;
use crate::setup::ProbeWindowRole;
use crate::setup::ProbeWindowScenario;
use crate::trace::ProbeTrace;
use crate::trace::TraceRecord;

enum MonitorName {
    Reported(String),
    Unavailable,
}

impl From<Option<String>> for MonitorName {
    fn from(name: Option<String>) -> Self { name.map_or(Self::Unavailable, Self::Reported) }
}

impl Serialize for MonitorName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Reported(name) => serializer.serialize_some(name),
            Self::Unavailable => serializer.serialize_none(),
        }
    }
}

enum MonitorIdentityEvidence {
    Verified(String),
    Unverified,
}

impl Serialize for MonitorIdentityEvidence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Verified(identity) => serializer.serialize_some(identity),
            Self::Unverified => serializer.serialize_none(),
        }
    }
}

#[derive(Serialize)]
struct MonitorSnapshot {
    entity:            u64,
    name:              MonitorName,
    identity:          String,
    verified_id:       MonitorIdentityEvidence,
    index:             usize,
    scale:             f64,
    physical_position: [i32; 2],
    physical_size:     [u32; 2],
}

struct ProjectedDisplayKeys(Vec<(DeviceKey, MonitorDescriptor)>);

impl ProjectedDisplayKeys {
    fn from_world(world: &mut World) -> Self {
        let mut live_displays = world.query::<(&DeviceKey, &LiveDisplayEndpoint)>();
        Self(
            live_displays
                .iter(world)
                .map(|(device_key, endpoint)| (device_key.clone(), endpoint.descriptor))
                .collect(),
        )
    }

    fn key_for_descriptor(
        &self,
        descriptor: MonitorDescriptor,
    ) -> Result<DeviceKey, LiveDisplayMatchError> {
        let mut matching = self
            .0
            .iter()
            .filter(|(_, candidate)| *candidate == descriptor)
            .map(|(device_key, _)| device_key.clone());
        let Some(first) = matching.next() else {
            return Err(LiveDisplayMatchError::NoLiveDisplayMatches);
        };
        let Some(_) = matching.next() else {
            return Ok(first);
        };
        let count = LiveDisplayContradictionCount::from_second_and_remaining(matching.count());
        Err(LiveDisplayMatchError::SeveralLiveDisplaysMatch { count })
    }
}

impl MonitorSnapshot {
    fn from_descriptor(
        entity: Entity,
        name: MonitorName,
        descriptor: MonitorDescriptor,
        device_key: Result<DeviceKey, LiveDisplayMatchError>,
    ) -> Self {
        let (identity, verified_id) = match device_key {
            Ok(device_key) => (
                format!("Verified({device_key:?})"),
                MonitorIdentityEvidence::Verified(format!("{device_key:?}")),
            ),
            Err(_) => (
                String::from("Unverified"),
                MonitorIdentityEvidence::Unverified,
            ),
        };
        Self {
            entity: entity.to_bits(),
            name,
            identity,
            verified_id,
            index: descriptor.index.adapter_value(),
            scale: descriptor.scale,
            physical_position: [
                descriptor.physical_position.x,
                descriptor.physical_position.y,
            ],
            physical_size: [descriptor.physical_size.x, descriptor.physical_size.y],
        }
    }
}

#[derive(Default, Serialize)]
struct RecoveryCounts {
    pending:      usize,
    available:    usize,
    restored:     usize,
    mismatch:     usize,
    cancellation: usize,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Readiness {
    Ready,
    Pending,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Resource)]
pub(crate) enum ProbeReadiness {
    Ready,
    #[default]
    Pending,
}

impl From<ProbeReadiness> for Readiness {
    fn from(readiness: ProbeReadiness) -> Self {
        match readiness {
            ProbeReadiness::Ready => Self::Ready,
            ProbeReadiness::Pending => Self::Pending,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Presence {
    Present,
    Absent,
}

impl From<bool> for Presence {
    fn from(present: bool) -> Self { if present { Self::Present } else { Self::Absent } }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Focus {
    Focused,
    Unfocused,
}

impl From<bool> for Focus {
    fn from(focused: bool) -> Self {
        if focused {
            Self::Focused
        } else {
            Self::Unfocused
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum MonitorCoverage {
    Full,
    Partial,
}

impl From<bool> for MonitorCoverage {
    fn from(covers: bool) -> Self { if covers { Self::Full } else { Self::Partial } }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum NativeFullscreen {
    Fullscreen,
    Windowed,
    Unavailable,
}

impl From<Option<bool>> for NativeFullscreen {
    fn from(state: Option<bool>) -> Self {
        match state {
            Some(true) => Self::Fullscreen,
            Some(false) => Self::Windowed,
            None => Self::Unavailable,
        }
    }
}

#[derive(Serialize)]
struct WindowSnapshot {
    key:                 String,
    entity:              u64,
    recovery_policy:     RecoveryPolicyObservation,
    current_monitor:     ProbeMonitorObservation,
    requested_mode:      String,
    effective_mode:      EffectiveWindowModeObservation,
    position:            String,
    physical_size:       [u32; 2],
    #[serde(rename = "decorated")]
    decoration_presence: Presence,
    #[serde(rename = "focused")]
    focus:               Focus,
    native_fullscreen:   NativeFullscreen,
    #[serde(rename = "covers_current_monitor")]
    monitor_coverage:    MonitorCoverage,
    replacement_count:   usize,
    recovery_counts:     RecoveryCounts,
}

enum RecoveryPolicyObservation {
    Authored(String),
    Unavailable,
}

impl Serialize for RecoveryPolicyObservation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Authored(policy) => serializer.serialize_some(policy),
            Self::Unavailable => serializer.serialize_none(),
        }
    }
}

enum ProbeMonitorObservation {
    Available(MonitorSnapshot),
    Unavailable,
}

impl Serialize for ProbeMonitorObservation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Available(monitor) => serializer.serialize_some(monitor),
            Self::Unavailable => serializer.serialize_none(),
        }
    }
}

enum EffectiveWindowModeObservation {
    Available(String),
    Unavailable,
}

impl Serialize for EffectiveWindowModeObservation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Available(mode) => serializer.serialize_some(mode),
            Self::Unavailable => serializer.serialize_none(),
        }
    }
}

#[derive(Clone, Copy)]
enum CurrentMonitorObservation {
    Available(CurrentMonitor),
    Unavailable,
}

impl From<Option<CurrentMonitor>> for CurrentMonitorObservation {
    fn from(current_monitor: Option<CurrentMonitor>) -> Self {
        current_monitor.map_or(Self::Unavailable, Self::Available)
    }
}

#[derive(Clone, Copy)]
enum OnMonitorObservation {
    Available(Entity),
    Unavailable,
}

#[derive(Clone, Copy)]
enum MonitorDescriptorObservation {
    Available(MonitorDescriptor),
    Unavailable,
}

impl From<Option<Entity>> for OnMonitorObservation {
    fn from(on_monitor: Option<Entity>) -> Self {
        on_monitor.map_or(Self::Unavailable, Self::Available)
    }
}

#[derive(Serialize)]
pub(super) struct ProbeSnapshot {
    schema_version:         u32,
    run_id:                 String,
    boot_nonce:             String,
    #[serde(rename = "ready")]
    readiness:              Readiness,
    startup_mode:           &'static str,
    selected_monitor_index: usize,
    topology_revision:      u64,
    record_cursor:          u64,
    monitors:               Vec<MonitorSnapshot>,
    windows:                Vec<WindowSnapshot>,
    terminal_failure:       Presence,
    command_receipts:       Vec<CommandReceipt>,
}

fn record_count(records: &[TraceRecord], kind: &str, window_key: &str) -> usize {
    records
        .iter()
        .filter(|record| {
            record.kind == kind
                && record
                    .fields
                    .iter()
                    .any(|(name, value)| name == FIELD_WINDOW_KEY && value == window_key)
        })
        .count()
}

fn recovery_counts(records: &[TraceRecord], window_key: &str) -> RecoveryCounts {
    RecoveryCounts {
        pending:      record_count(records, KIND_RECOVERY_PENDING, window_key),
        available:    record_count(records, KIND_RECOVERY_AVAILABLE, window_key),
        restored:     record_count(records, KIND_RECOVERY_RESTORED, window_key),
        mismatch:     record_count(records, KIND_RECOVERY_MISMATCH, window_key),
        cancellation: record_count(records, KIND_RECOVERY_CANCELLATION_REQUESTED, window_key),
    }
}

fn binding_ready(
    bindings: &Bindings,
    role_statuses: &Query<&RoleStatus>,
    scenario: ProbeWindowScenario,
    expected_policy: RecoveryPolicy,
) -> Result<bool, RoleKeyError> {
    let ProbeWindowRole::KernelRole(role) = scenario.role()? else {
        return Ok(false);
    };
    Ok(bindings.binding(&role).is_ok_and(|binding| {
        binding.recovery == expected_policy
            && bindings.role_entity(&role).is_ok_and(|entity| {
                role_statuses
                    .get(entity)
                    .is_ok_and(|status| matches!(status.view(), RoleStatusView::Established { .. }))
            })
    }))
}

fn common_roles_ready(
    bindings: &Bindings,
    role_statuses: &Query<&RoleStatus>,
) -> Result<bool, RoleKeyError> {
    Ok(binding_ready(
        bindings,
        role_statuses,
        ProbeWindowScenario::PrimaryAutomaticReturn,
        RecoveryPolicy::ReapplyOnReturn,
    )? && binding_ready(
        bindings,
        role_statuses,
        ProbeWindowScenario::ApplicationRequestedReturn,
        RecoveryPolicy::ReapplyOnRequest,
    )? && binding_ready(
        bindings,
        role_statuses,
        ProbeWindowScenario::RestoreOnly,
        RecoveryPolicy::Forget,
    )?)
}

fn automatic_role_ready(
    startup_mode: ProbeStartupMode,
    bindings: &Bindings,
    role_statuses: &Query<&RoleStatus>,
) -> Result<bool, RoleKeyError> {
    match startup_mode {
        ProbeStartupMode::Exclusive => match ProbeWindowScenario::ManagedAutomaticReturn.role()? {
            ProbeWindowRole::KernelRole(role) => Ok(bindings.binding(&role).is_err()),
            ProbeWindowRole::UnmanagedControl => Ok(false),
        },
        ProbeStartupMode::Windowed | ProbeStartupMode::Borderless => binding_ready(
            bindings,
            role_statuses,
            ProbeWindowScenario::ManagedAutomaticReturn,
            RecoveryPolicy::ReapplyOnReturn,
        ),
    }
}

/// Sets [`ProbeReadiness`] to `Ready` once every probe window is registered and sitting on the
/// selected monitor, that monitor resolves to a device key, and every probe role's binding is in
/// place. A run already at `Ready` returns without recording again.
pub(crate) fn record_probe_readiness(
    startup_mode: Res<ProbeStartupMode>,
    monitor_selection: Res<ProbeMonitorSelection>,
    monitors: Res<Monitors>,
    live_display_endpoints: LiveDisplayEndpointLookup,
    bindings: Res<Bindings>,
    role_statuses: Query<&RoleStatus>,
    windows: Query<(
        &ProbeWindowScenario,
        &OnMonitor,
        Has<ProbeWindowRegistrationComplete>,
    )>,
    mut readiness: ResMut<ProbeReadiness>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    if *readiness == ProbeReadiness::Ready {
        return;
    }
    let Some(target) = monitors.iter().find(|monitor| {
        monitor.descriptor.index.adapter_value() == monitor_selection.selected_monitor_index()
    }) else {
        return;
    };
    if live_display_endpoints
        .key_for_descriptor(*target.descriptor)
        .is_err()
    {
        return;
    }
    let all_windows_placed = windows
        .iter()
        .all(|(_, on_monitor, registered)| registered && on_monitor.0 == target.entity);
    if windows.iter().count() != PROBE_WINDOW_COUNT || !all_windows_placed {
        return;
    }
    let common_roles_ready = match common_roles_ready(&bindings, &role_statuses) {
        Ok(common_roles_ready) => common_roles_ready,
        Err(error) => {
            error!("[record_probe_readiness] common probe role invariant failed: {error}");
            return;
        },
    };
    if !common_roles_ready {
        return;
    }
    let automatic_ready = match automatic_role_ready(*startup_mode, &bindings, &role_statuses) {
        Ok(automatic_ready) => automatic_ready,
        Err(error) => {
            error!("[record_probe_readiness] automatic probe role invariant failed: {error}");
            return;
        },
    };
    if !automatic_ready {
        return;
    }
    trace.record(
        frame_count.0,
        PRODUCER_RECOVERY_READY,
        KIND_RECOVERY_READY,
        vec![(
            FIELD_SELECTED_MONITOR_INDEX.into(),
            monitor_selection.selected_monitor_index().to_string(),
        )],
    );
    *readiness = ProbeReadiness::Ready;
}

pub(super) fn snapshot(world: &mut World) -> Result<ProbeSnapshot, RoleKeyError> {
    let window_values = {
        let mut windows_query = world.query::<(
            Entity,
            &Window,
            &ProbeWindowScenario,
            Option<&CurrentMonitor>,
            Option<&OnMonitor>,
        )>();
        windows_query
            .iter(world)
            .filter(|(_, _, scenario, ..)| **scenario != ProbeWindowScenario::UnmanagedControl)
            .map(|(entity, window, scenario, current_monitor, on_monitor)| {
                (
                    entity,
                    window.clone(),
                    *scenario,
                    CurrentMonitorObservation::from(current_monitor.copied()),
                    OnMonitorObservation::from(on_monitor.map(|on_monitor| on_monitor.0)),
                )
            })
            .collect::<Vec<_>>()
    };
    let projected_display_keys = ProjectedDisplayKeys::from_world(world);
    let session = world.resource::<ProbeSession>();
    let run_id = session.run_id.clone();
    let boot_nonce = session.boot_nonce.clone();
    let startup_mode = world.resource::<ProbeStartupMode>().selector();
    let selected_monitor_index = world
        .resource::<ProbeMonitorSelection>()
        .selected_monitor_index();
    let topology_revision = world
        .resource::<hana_clerestory::MonitorTopologyRevision>()
        .get();
    let readiness = *world.resource::<ProbeReadiness>();
    let trace = world.resource::<ProbeTrace>().clone();
    let records = trace.records();
    let record_cursor = records.last().map_or(0, |record| record.sequence);
    let terminal_failure: Presence = records
        .iter()
        .any(|record| record.kind == KIND_RECOVERY_MISMATCH)
        .into();
    let command_receipts = world
        .resource::<CommandReceipts>()
        .0
        .values()
        .cloned()
        .collect();
    let monitors_resource = world.resource::<Monitors>();
    let monitors = monitors_resource
        .iter()
        .map(|monitor| {
            let monitor_name = MonitorName::from(
                world
                    .get::<bevy::window::Monitor>(monitor.entity)
                    .and_then(|monitor| monitor.name.clone()),
            );
            MonitorSnapshot::from_descriptor(
                monitor.entity,
                monitor_name,
                *monitor.descriptor,
                projected_display_keys.key_for_descriptor(*monitor.descriptor),
            )
        })
        .collect();
    let bindings = world.resource::<Bindings>();
    let windows = window_values
        .iter()
        .map(|(entity, window, scenario, current_monitor, on_monitor)| {
            window_snapshot(
                *entity,
                window,
                *scenario,
                *current_monitor,
                *on_monitor,
                monitors_resource,
                &projected_display_keys,
                bindings,
                &records,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ProbeSnapshot {
        schema_version: PROBE_SCHEMA_VERSION,
        run_id,
        boot_nonce,
        readiness: readiness.into(),
        startup_mode,
        selected_monitor_index,
        topology_revision,
        record_cursor,
        monitors,
        windows,
        terminal_failure,
        command_receipts,
    })
}

fn current_monitor_snapshot(
    current_monitor: CurrentMonitorObservation,
    on_monitor: OnMonitorObservation,
    monitors: &Monitors,
    projected_display_keys: &ProjectedDisplayKeys,
) -> ProbeMonitorObservation {
    let MonitorDescriptorObservation::Available(descriptor) =
        monitor_descriptor_observation(current_monitor, on_monitor, monitors)
    else {
        return ProbeMonitorObservation::Unavailable;
    };
    let entity = match on_monitor {
        OnMonitorObservation::Available(entity) => entity,
        OnMonitorObservation::Unavailable => {
            let Some(entity) = monitors
                .iter()
                .find(|monitor| *monitor.descriptor == descriptor)
                .map(|monitor| monitor.entity)
            else {
                return ProbeMonitorObservation::Unavailable;
            };
            entity
        },
    };
    ProbeMonitorObservation::Available(MonitorSnapshot::from_descriptor(
        entity,
        MonitorName::Unavailable,
        descriptor,
        projected_display_keys.key_for_descriptor(descriptor),
    ))
}

fn monitor_descriptor_observation(
    current_monitor: CurrentMonitorObservation,
    on_monitor: OnMonitorObservation,
    monitors: &Monitors,
) -> MonitorDescriptorObservation {
    match current_monitor {
        CurrentMonitorObservation::Available(current_monitor) => {
            MonitorDescriptorObservation::Available(current_monitor.descriptor)
        },
        CurrentMonitorObservation::Unavailable => match on_monitor {
            OnMonitorObservation::Available(on_monitor) => monitors
                .iter()
                .find(|monitor| monitor.entity == on_monitor)
                .map_or(MonitorDescriptorObservation::Unavailable, |monitor| {
                    MonitorDescriptorObservation::Available(*monitor.descriptor)
                }),
            OnMonitorObservation::Unavailable => MonitorDescriptorObservation::Unavailable,
        },
    }
}

fn window_snapshot(
    entity: Entity,
    window: &Window,
    scenario: ProbeWindowScenario,
    current_monitor: CurrentMonitorObservation,
    on_monitor: OnMonitorObservation,
    monitors: &Monitors,
    projected_display_keys: &ProjectedDisplayKeys,
    bindings: &Bindings,
    records: &[TraceRecord],
) -> Result<WindowSnapshot, RoleKeyError> {
    let recovery_policy = match scenario.role()? {
        ProbeWindowRole::KernelRole(role) => bindings
            .binding(&role)
            .map_or(RecoveryPolicyObservation::Unavailable, |binding| {
                RecoveryPolicyObservation::Authored(format!("{:?}", binding.recovery))
            }),
        ProbeWindowRole::UnmanagedControl => RecoveryPolicyObservation::Unavailable,
    };
    let descriptor_observation =
        monitor_descriptor_observation(current_monitor, on_monitor, monitors);
    let monitor_coverage: MonitorCoverage = matches!(
        descriptor_observation,
        MonitorDescriptorObservation::Available(descriptor)
            if {
            window.resolution.physical_size() == descriptor.physical_size
                && match window.position {
                    WindowPosition::At(position) => position == descriptor.physical_position,
                    WindowPosition::Automatic | WindowPosition::Centered(_) => false,
                }
            }
    )
    .into();
    let created = record_count(records, KIND_WINDOW_CREATED, scenario.key());
    Ok(WindowSnapshot {
        key: scenario.key().into(),
        entity: entity.to_bits(),
        recovery_policy,
        current_monitor: current_monitor_snapshot(
            current_monitor,
            on_monitor,
            monitors,
            projected_display_keys,
        ),
        requested_mode: format!("{:?}", window.mode),
        effective_mode: match current_monitor {
            CurrentMonitorObservation::Available(current_monitor) => {
                EffectiveWindowModeObservation::Available(format!(
                    "{:?}",
                    current_monitor.effective_window_mode
                ))
            },
            CurrentMonitorObservation::Unavailable => EffectiveWindowModeObservation::Unavailable,
        },
        position: format!("{:?}", window.position),
        physical_size: [
            window.resolution.physical_width(),
            window.resolution.physical_height(),
        ],
        decoration_presence: window.decorations.into(),
        focus: window.focused.into(),
        native_fullscreen: native_fullscreen(entity),
        monitor_coverage,
        replacement_count: created.saturating_sub(1),
        recovery_counts: recovery_counts(records, scenario.key()),
    })
}

#[cfg(target_os = "macos")]
fn native_fullscreen(entity: Entity) -> NativeFullscreen {
    use bevy::winit::WINIT_WINDOWS;
    use objc2_app_kit::NSView;
    use objc2_app_kit::NSWindowStyleMask;
    use raw_window_handle::HasWindowHandle;
    use raw_window_handle::RawWindowHandle;

    WINIT_WINDOWS
        .with_borrow(|winit_windows| {
            let winit_window = winit_windows.get_window(entity)?;
            let handle = winit_window.window_handle().ok()?;
            let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() else {
                return None;
            };
            // SAFETY: `ns_view` comes from the live winit window handle above.
            let ns_view: &NSView = unsafe { appkit_handle.ns_view.cast().as_ref() };
            let window = ns_view.window()?;
            Some(window.styleMask().contains(NSWindowStyleMask::FullScreen))
        })
        .into()
}

#[cfg(not(target_os = "macos"))]
const fn native_fullscreen(_: Entity) -> NativeFullscreen { NativeFullscreen::Unavailable }

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::constants::PROBE_WINDOW_HEIGHT;
    use crate::constants::PROBE_WINDOW_WIDTH;
    use crate::constants::RESTORE_ONLY_WINDOW_KEY;

    #[test]
    fn snapshot_wire_keeps_readiness_and_nested_monitor_window_fields()
    -> Result<(), serde_json::Error> {
        let snapshot = ProbeSnapshot {
            schema_version:         PROBE_SCHEMA_VERSION,
            run_id:                 "run".into(),
            boot_nonce:             "boot".into(),
            readiness:              Readiness::Ready,
            startup_mode:           "windowed",
            selected_monitor_index: 1,
            topology_revision:      2,
            record_cursor:          3,
            monitors:               vec![MonitorSnapshot {
                entity:            4,
                name:              MonitorName::Reported("display".into()),
                identity:          "Verified(display)".into(),
                verified_id:       MonitorIdentityEvidence::Verified("display".into()),
                index:             1,
                scale:             2.0,
                physical_position: [0, 0],
                physical_size:     [1_920, 1_080],
            }],
            windows:                vec![WindowSnapshot {
                key:                 "primary".into(),
                entity:              5,
                recovery_policy:     RecoveryPolicyObservation::Authored("ReapplyOnReturn".into()),
                current_monitor:     ProbeMonitorObservation::Available(MonitorSnapshot {
                    entity:            4,
                    name:              MonitorName::Unavailable,
                    identity:          "Verified(display)".into(),
                    verified_id:       MonitorIdentityEvidence::Verified("display".into()),
                    index:             1,
                    scale:             2.0,
                    physical_position: [0, 0],
                    physical_size:     [1_920, 1_080],
                }),
                requested_mode:      "Windowed".into(),
                effective_mode:      EffectiveWindowModeObservation::Available("Windowed".into()),
                position:            "At(IVec2(0, 0))".into(),
                physical_size:       [800, 540],
                decoration_presence: Presence::Present,
                focus:               Focus::Focused,
                native_fullscreen:   NativeFullscreen::Unavailable,
                monitor_coverage:    MonitorCoverage::Partial,
                replacement_count:   0,
                recovery_counts:     RecoveryCounts::default(),
            }],
            terminal_failure:       Presence::Absent,
            command_receipts:       Vec::new(),
        };

        let value = serde_json::to_value(snapshot)?;
        assert_eq!(value.get("ready"), Some(&Value::String("ready".into())));
        assert!(value.get("selected_monitor_index").is_some());
        assert!(value["monitors"][0].get("verified_id").is_some());
        assert!(value["monitors"][0].get("physical_position").is_some());
        assert!(value["windows"][0].get("key").is_some());
        assert!(
            value["windows"][0]["current_monitor"]
                .get("verified_id")
                .is_some()
        );
        assert!(value["windows"][0].get("recovery_counts").is_some());
        assert!(value.get("terminal_failure").is_some());
        Ok(())
    }

    #[test]
    fn snapshot_wire_keeps_semantic_absence_as_null() -> Result<(), serde_json::Error> {
        let monitor_snapshot = MonitorSnapshot {
            entity:            1,
            name:              MonitorName::Unavailable,
            identity:          "Unverified".into(),
            verified_id:       MonitorIdentityEvidence::Unverified,
            index:             0,
            scale:             1.0,
            physical_position: [0, 0],
            physical_size:     [1_920, 1_080],
        };
        let window_snapshot = WindowSnapshot {
            key:                 RESTORE_ONLY_WINDOW_KEY.into(),
            entity:              2,
            recovery_policy:     RecoveryPolicyObservation::Unavailable,
            current_monitor:     ProbeMonitorObservation::Unavailable,
            requested_mode:      "Windowed".into(),
            effective_mode:      EffectiveWindowModeObservation::Unavailable,
            position:            "Automatic".into(),
            physical_size:       [PROBE_WINDOW_WIDTH, PROBE_WINDOW_HEIGHT],
            decoration_presence: Presence::Present,
            focus:               Focus::Unfocused,
            native_fullscreen:   NativeFullscreen::Unavailable,
            monitor_coverage:    MonitorCoverage::Partial,
            replacement_count:   0,
            recovery_counts:     RecoveryCounts::default(),
        };

        let monitor_value = serde_json::to_value(monitor_snapshot)?;
        let window_value = serde_json::to_value(window_snapshot)?;
        assert!(monitor_value["name"].is_null());
        assert!(monitor_value["verified_id"].is_null());
        assert!(window_value["recovery_policy"].is_null());
        assert!(window_value["current_monitor"].is_null());
        assert!(window_value["effective_mode"].is_null());
        Ok(())
    }
}
