//! Feature-gated control adapter that runs Clerestory's display machine without a winit window.
//!
//! Add [`DisplayTestAdapter`] before `WindowManagerPlugin`. The production `MonitorPlugin` then
//! installs a scripted topology provider that supplies both halves of what a platform would
//! publish: the display enumeration `MonitorReporter` reads, and the `PanelIdentityEvidence` a
//! platform read would have produced (on macOS, `classify_panel_evidence` reading
//! `CGDisplayCreateUUIDFromDisplayID`). Everything downstream of that evidence stays production
//! code: `MonitorReporter` turns it into a durable display key, `WindowManagerPlugin` registers the
//! reporter, and the kernel device records are the reporter's own.
//!
//! The script is the adapter's whole surface. Clerestory's identity model —
//! `EnumeratedDisplayEvidence`, `DisplayDeviceEvidence`, `PanelIdentityEvidence`,
//! `PanelFingerprint`, and `DisplayTopologyObservation` — stays crate-private, and this module
//! owns the translation from the script into it.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use bevy::ecs::change_detection::Tick;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::winit::WinitMonitors;
use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::PlatformDeviceHandle;
use hana_rigging::prelude::ReportedSerial;
use hana_rigging::prelude::ReporterId;

use crate::monitors::DisplayDeviceEvidence;
use crate::monitors::DisplayTopologyObservation;
use crate::monitors::EnumeratedDisplayEvidence;
use crate::monitors::MonitorReporterId;
use crate::monitors::PanelFingerprint;
use crate::monitors::PanelIdentityEvidence;
use crate::reporter;
use crate::reporter::DisplayKeyClassification;
use crate::reporter::InjectedFreshWinitDisplays;

/// One display the adapter presents to the production display reporter.
///
/// `name` addresses this display in [`DisplayTestAdapter::enumerate_displays`] and never reaches
/// the kernel. `panel_evidence` is the identity material the panel publishes, which the reporter
/// mints its durable display key from, and the platform device handle is the per-process capture
/// address another display reporter joins on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayTestDescriptor {
    name:                   String,
    panel_evidence:         Vec<u8>,
    platform_device_handle: PlatformDeviceHandle,
}

impl DisplayTestDescriptor {
    /// Describes one scripted panel by the evidence its platform would publish about it.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        panel_evidence: impl Into<Vec<u8>>,
        platform_device_handle: PlatformDeviceHandle,
    ) -> Self {
        Self {
            name: name.into(),
            panel_evidence: panel_evidence.into(),
            platform_device_handle,
        }
    }

    fn device_evidence(&self) -> DisplayDeviceEvidence {
        DisplayDeviceEvidence {
            panel_identity:         PanelIdentityEvidence::Synthesized {
                fingerprint: PanelFingerprint::from_evidence_bytes(&self.panel_evidence),
                serial:      ReportedSerial::PlatformCannotReport,
            },
            platform_device_handle: self.platform_device_handle.clone(),
            attachment:             AttachmentPath::PlatformHasNoConcept,
        }
    }
}

/// Result of pointing the adapter at the displays a platform currently enumerates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayTestEnumeration {
    /// Every named display is scripted and is now what the platform enumerates.
    Enumerated,
    /// This name was never scripted, so the enumerated set was left unchanged.
    UnscriptedDisplay(String),
}

/// Durable kernel identity the display reporter mints for one scripted panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayTestDeviceKey {
    /// The reporter names this panel durably, and a binding can address it by this key.
    Keyed(DeviceKey),
    /// This name was never scripted.
    UnscriptedDisplay(String),
}

/// Result of asking the adapter which reporter the display machine registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayTestReporterLookup {
    /// `WindowManagerPlugin` registered the display reporter under this kernel handle.
    Registered(ReporterId),
    /// `WindowManagerPlugin` has not been added, so no display reporter exists yet.
    NotRegistered,
}

/// Feature-gated control adapter for deterministic display lifecycle tests.
///
/// Add this plugin before `WindowManagerPlugin`. The script supplies the display enumeration and
/// the panel evidence a platform would publish; every other part of the display machine — the
/// evidence-to-key rules, the registration, and the device records — remains the production one.
#[derive(Clone, Default)]
pub struct DisplayTestAdapter {
    topology: Arc<Mutex<ScriptedDisplayTopology>>,
    revision: Arc<AtomicUsize>,
}

#[derive(Default)]
struct ScriptedDisplayTopology {
    observed:   Vec<DisplayTestDescriptor>,
    enumerated: Vec<String>,
}

impl DisplayTestAdapter {
    /// Creates an adapter with no scripted display.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Installs the panel set Clerestory has observed, with every panel currently enumerated.
    pub fn observe_displays(&self, displays: Vec<DisplayTestDescriptor>) {
        let enumerated = displays
            .iter()
            .map(|display| display.name.clone())
            .collect();
        *self
            .topology
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = ScriptedDisplayTopology {
            observed: displays,
            enumerated,
        };
        self.revision.fetch_add(1, Ordering::SeqCst);
    }

    /// Sets which of the observed panels the platform enumerates right now.
    ///
    /// This is what drives a departure and a return: the observed evidence stays cached while the
    /// enumerated set shrinks and grows, exactly as it does when a monitor is unplugged.
    #[must_use]
    pub fn enumerate_displays(&self, names: &[&str]) -> DisplayTestEnumeration {
        let mut topology = self
            .topology
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for name in names {
            if !topology
                .observed
                .iter()
                .any(|display| display.name == *name)
            {
                return DisplayTestEnumeration::UnscriptedDisplay((*name).to_owned());
            }
        }
        topology.enumerated = names.iter().map(|name| (*name).to_owned()).collect();
        drop(topology);
        self.revision.fetch_add(1, Ordering::SeqCst);
        DisplayTestEnumeration::Enumerated
    }

    /// Returns the durable key the display reporter mints for one scripted panel.
    ///
    /// The classification is the reporter's own, so a test never restates Clerestory's identity
    /// rules and never drifts from them.
    ///
    /// # Panics
    ///
    /// Panics if Clerestory's built-in EDID serial scheme is invalid, which is a crate invariant
    /// rather than a per-call condition.
    #[must_use]
    #[expect(
        clippy::expect_used,
        clippy::unreachable,
        reason = "both name crate invariants of the built-in scheme and the scripted evidence"
    )]
    pub fn display_device_key(&self, name: &str) -> DisplayTestDeviceKey {
        let edid_serial_scheme = reporter::edid_serial_scheme()
            .expect("Clerestory's built-in EDID serial scheme must be valid");
        let evidence = self
            .topology
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .observed
            .iter()
            .find(|display| display.name == name)
            .map(DisplayTestDescriptor::device_evidence);
        let Some(evidence) = evidence else {
            return DisplayTestDeviceKey::UnscriptedDisplay(name.to_owned());
        };
        match reporter::classify_display_key(&evidence, &edid_serial_scheme) {
            DisplayKeyClassification::Keyed(device_key) => DisplayTestDeviceKey::Keyed(device_key),
            DisplayKeyClassification::MatchEvidenceOnly => unreachable!(
                "every scripted descriptor publishes panel evidence, so classification always keys"
            ),
        }
    }

    /// Returns the kernel handle Clerestory's display reporter was registered under.
    #[must_use]
    pub fn display_reporter(&self, world: &World) -> DisplayTestReporterLookup {
        world.get_resource::<MonitorReporterId>().map_or(
            DisplayTestReporterLookup::NotRegistered,
            |monitor_reporter_id| DisplayTestReporterLookup::Registered(monitor_reporter_id.get()),
        )
    }
}

impl Plugin for DisplayTestAdapter {
    fn build(&self, app: &mut App) {
        app.insert_resource(DisplayTestAdapterResource(self.clone()));
    }
}

#[derive(Resource, Clone)]
struct DisplayTestAdapterResource(DisplayTestAdapter);

/// Install the scripted topology provider when a test placed the adapter ahead of `MonitorPlugin`.
pub(crate) fn install_scripted_display_topology(app: &mut App) {
    if !app
        .world()
        .contains_resource::<DisplayTestAdapterResource>()
    {
        return;
    }
    app.init_resource::<WinitMonitors>()
        .add_systems(PreUpdate, apply_scripted_display_topology);
}

/// The topology this system last published, and the change tick it published it at.
///
/// The tick is what makes a re-publish detectable: `update_monitors` reinstalls
/// `DisplayTopologyObservation` whenever it rebuilds a different topology, and it runs in `Update`
/// with no ordering constraint against this `PreUpdate` system. Comparing the recorded tick against
/// the resource's current one distinguishes "this system's own write is still in place" from
/// "someone else has written over it".
#[derive(Default)]
enum PublishedDisplayTopology {
    #[default]
    NothingPublished,
    Published {
        revision:         usize,
        observation_tick: Tick,
        entities:         Vec<Entity>,
    },
}

impl PublishedDisplayTopology {
    /// Whether the world still carries what this system published for `revision`.
    fn still_published(&self, revision: usize, observation_tick: Tick) -> bool {
        matches!(
            self,
            Self::Published {
                revision: published_revision,
                observation_tick: published_tick,
                ..
            } if *published_revision == revision && *published_tick == observation_tick
        )
    }

    /// Entities the previous publication spawned, which the next one replaces.
    fn entities(&self) -> &[Entity] {
        match self {
            Self::NothingPublished => &[],
            Self::Published { entities, .. } => entities,
        }
    }
}

/// Publish the current script as the topology and winit enumeration the display reporter reads.
///
/// The two resources carry different sets on purpose: the observation holds every panel Clerestory
/// has cached evidence for, and the fresh winit set holds only the panels attached right now. Their
/// entity values exist solely to join one to the other, so the spawned entities carry no component.
/// The command queue flushes at the end of `PreUpdate` and the reporter reads both resources in
/// `Update`, so nothing resolves a deferred spawn in between.
fn apply_scripted_display_topology(
    mut commands: Commands,
    adapter: Res<DisplayTestAdapterResource>,
    mut published: Local<PublishedDisplayTopology>,
    mut observation: ResMut<DisplayTopologyObservation>,
    mut fresh_displays: ResMut<InjectedFreshWinitDisplays>,
) {
    let revision = adapter.0.revision.load(Ordering::SeqCst);
    if published.still_published(revision, observation.last_changed()) {
        return;
    }
    for entity in published.entities() {
        commands.entity(*entity).despawn();
    }

    let topology = adapter
        .0
        .topology
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entities: Vec<Entity> = topology
        .observed
        .iter()
        .map(|_| commands.spawn_empty().id())
        .collect();
    *observation = DisplayTopologyObservation::Observed(
        topology
            .observed
            .iter()
            .zip(entities.iter())
            .map(|(display, entity)| EnumeratedDisplayEvidence {
                entity:          *entity,
                device_evidence: display.device_evidence(),
            })
            .collect(),
    );
    fresh_displays.entities = topology
        .observed
        .iter()
        .zip(entities.iter())
        .filter(|(display, _)| topology.enumerated.contains(&display.name))
        .map(|(_, entity)| *entity)
        .collect();
    drop(topology);
    *published = PublishedDisplayTopology::Published {
        revision,
        observation_tick: observation.last_changed(),
        entities,
    };
}
