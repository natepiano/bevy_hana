//! Feature-gated control adapter that runs Clerestory's display machine without a winit window.
//!
//! Add [`DisplayTestAdapter`] before `WindowManagerPlugin`. The production `MonitorPlugin` then
//! installs a scripted topology provider that supplies both halves of what a platform would
//! publish: the display enumeration `MonitorReporter` reads, and the `DisplayIdentityEvidence` a
//! platform read would have produced (on macOS, `classify_display_evidence` reading
//! `CGDisplayCreateUUIDFromDisplayID`). Everything downstream of that evidence stays production
//! code: `MonitorReporter` turns it into a durable display key, `WindowManagerPlugin` registers the
//! reporter, and the kernel device records are the reporter's own.
//!
//! The script is the adapter's whole surface. Clerestory's identity model —
//! `EnumeratedDisplayEvidence`, `DisplayDeviceEvidence`, `DisplayIdentityEvidence`,
//! `DisplayFingerprint`, and `DisplayTopologyObservation` — stays crate-private, and this module
//! owns the translation from the script into it.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use bevy::ecs::query::With;
use bevy::ecs::system::Query;
use bevy::prelude::App;
use bevy::prelude::ApplyDeferred;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::IVec2;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Local;
use bevy::prelude::Plugin;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
use bevy::prelude::Update;
use bevy::prelude::World;
use bevy::window::Monitor;
use bevy::winit::WinitMonitors;
use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::PlatformDeviceHandle;
use hana_rigging::prelude::ReportedSerial;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::RiggingSystems;

use crate::ClerestoryUpdateSet;
use crate::monitors::DisplayDeviceEvidence;
use crate::monitors::DisplayFingerprint;
use crate::monitors::DisplayIdentityEvidence;
use crate::monitors::DisplayTopologyObservation;
use crate::monitors::InjectedMonitorEvidence;
use crate::monitors::InjectedWinitMonitorOrder;
use crate::monitors::MonitorDescriptor;
use crate::monitors::MonitorReporterId;
use crate::platform;
use crate::reporter;
use crate::reporter::DisplayKeyClassification;
use crate::reporter::InjectedFreshWinitDisplays;

/// One display the adapter presents to the production display reporter.
///
/// `name` addresses this display in [`DisplayTestAdapter::enumerate_displays`] and never reaches
/// the kernel. `display_evidence` is the identity material the display publishes, which the
/// reporter derives its durable display key from, and the platform device handle is the
/// per-process capture address another display reporter joins on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayTestDescriptor {
    name:                   String,
    identity:               DisplayTestIdentity,
    platform_device_handle: PlatformDeviceHandle,
}

/// Whether a scripted display publishes identity material the reporter can key it by.
///
/// A display whose scan yields nothing durable is not a rare defect: a panel behind a switcher or
/// an adapter that strips EDID reaches the reporter this way, and it must never be mistaken for
/// the saved display however closely the rest of its descriptors match.
#[derive(Clone, Debug, PartialEq, Eq)]
enum DisplayTestIdentity {
    /// The display publishes this material, which the reporter derives a durable key from.
    Published(Vec<u8>),
    /// The scan produced no durable identity, so the reporter can name no key for this display.
    Unpublished,
}

impl DisplayTestDescriptor {
    /// Describes one scripted display by the evidence its platform would publish about it.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        display_evidence: impl Into<Vec<u8>>,
        platform_device_handle: PlatformDeviceHandle,
    ) -> Self {
        Self {
            name: name.into(),
            identity: DisplayTestIdentity::Published(display_evidence.into()),
            platform_device_handle,
        }
    }

    /// Describes one scripted display whose scan produced no durable identity at all.
    #[must_use]
    pub fn without_published_identity(
        name: impl Into<String>,
        platform_device_handle: PlatformDeviceHandle,
    ) -> Self {
        Self {
            name: name.into(),
            identity: DisplayTestIdentity::Unpublished,
            platform_device_handle,
        }
    }

    fn device_evidence(&self) -> DisplayDeviceEvidence {
        let identity_evidence = match &self.identity {
            DisplayTestIdentity::Published(display_evidence) => {
                DisplayIdentityEvidence::Synthesized {
                    display_fingerprint: DisplayFingerprint::from_evidence_bytes(display_evidence),
                    serial:              ReportedSerial::PlatformCannotReport,
                }
            },
            DisplayTestIdentity::Unpublished => DisplayIdentityEvidence::Unavailable {
                serial: ReportedSerial::NotExposedByUnit,
            },
        };
        DisplayDeviceEvidence {
            identity_evidence,
            platform_device_handle: self.platform_device_handle.clone(),
            attachment: AttachmentPath::PlatformHasNoConcept,
        }
    }

    /// The identity material this display publishes, or `None` when its scan produces none.
    fn published_identity_bytes(&self) -> Option<Vec<u8>> {
        match &self.identity {
            DisplayTestIdentity::Published(display_evidence) => Some(display_evidence.clone()),
            DisplayTestIdentity::Unpublished => None,
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

/// Durable kernel identity the display reporter assigns to one scripted display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayTestDeviceKey {
    /// The reporter names this display durably, and a binding can address it by this key.
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
/// the display evidence a platform would publish; every other part of the display machine — the
/// evidence-to-key rules, the registration, and the device records — remains the production one.
#[derive(Clone, Default)]
pub struct DisplayTestAdapter {
    topology: Arc<Mutex<ScriptedDisplayTopology>>,
    revision: Arc<AtomicUsize>,
}

/// Whether a [`DisplayTestAdapter`] resource was explicitly installed in one Bevy world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayTestAdapterInstallation {
    /// A caller added [`DisplayTestAdapter`] before the production monitor plugin.
    ExplicitlyInstalled,
    /// The production monitor plugin has no scripted display source.
    NotInstalled,
    /// The injected display resource exists without an explicitly installed adapter.
    InjectedWithoutExplicitAdapter,
    /// The adapter resource exists without its injected display resource.
    ExplicitAdapterWithoutInjectedSource,
}

#[derive(Default)]
struct ScriptedDisplayTopology {
    observed:                  Vec<DisplayTestDescriptor>,
    enumerated:                Vec<String>,
    display_reporter_delivery: DisplayTestReporterDelivery,
}

#[derive(Clone, Copy, Default)]
pub(crate) enum DisplayTestReporterDelivery {
    #[default]
    Enumerate,
    Fail,
}

impl DisplayTestAdapter {
    /// Creates an adapter with no scripted display.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Reports whether a caller explicitly installed this adapter in `world`.
    #[must_use]
    pub fn installation(world: &World) -> DisplayTestAdapterInstallation {
        match (
            world.contains_resource::<DisplayTestAdapterResource>(),
            world.contains_resource::<InjectedFreshWinitDisplays>(),
        ) {
            (true, true) => DisplayTestAdapterInstallation::ExplicitlyInstalled,
            (false, false) => DisplayTestAdapterInstallation::NotInstalled,
            (false, true) => DisplayTestAdapterInstallation::InjectedWithoutExplicitAdapter,
            (true, false) => DisplayTestAdapterInstallation::ExplicitAdapterWithoutInjectedSource,
        }
    }

    /// Installs the display set Clerestory has observed, with every display currently enumerated.
    pub fn observe_displays(&self, displays: Vec<DisplayTestDescriptor>) {
        let enumerated = displays
            .iter()
            .map(|display| display.name.clone())
            .collect();
        let mut topology = self
            .topology
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        topology.observed = displays;
        topology.enumerated = enumerated;
        drop(topology);
        self.revision.fetch_add(1, Ordering::SeqCst);
    }

    /// Makes every following production display-reporter discovery return a transport failure.
    pub fn fail_reporter_enumeration(&self) {
        self.topology
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .display_reporter_delivery = DisplayTestReporterDelivery::Fail;
    }

    /// Sets which of the observed displays the platform enumerates right now.
    ///
    /// This is what drives a departure and a return: the adapter retains its scripted evidence
    /// while the platform observation shrinks and grows, exactly as it does when a monitor is
    /// unplugged and later returns.
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

    /// Returns the durable key the display reporter assigns to one scripted display.
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
        match platform::classify_display_key(&evidence, &edid_serial_scheme) {
            DisplayKeyClassification::Keyed(device_key) => DisplayTestDeviceKey::Keyed(device_key),
            DisplayKeyClassification::MatchEvidenceOnly => unreachable!(
                "every scripted descriptor publishes display evidence, so classification always keys"
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

pub(crate) fn display_test_reporter_delivery(world: &World) -> DisplayTestReporterDelivery {
    world.get_resource::<DisplayTestAdapterResource>().map_or(
        DisplayTestReporterDelivery::Enumerate,
        |adapter| {
            adapter
                .0
                .topology
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .display_reporter_delivery
        },
    )
}

impl Plugin for DisplayTestAdapter {
    fn build(&self, app: &mut App) {
        app.insert_resource(DisplayTestAdapterResource(self.clone()))
            .init_resource::<InjectedFreshWinitDisplays>();
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
        .init_resource::<InjectedMonitorEvidence>()
        .init_resource::<InjectedWinitMonitorOrder>()
        .add_systems(
            Update,
            (publish_scripted_display_topology, ApplyDeferred)
                .chain()
                .before(ClerestoryUpdateSet::MonitorTopology),
        )
        .add_systems(
            Update,
            derive_scripted_current_displays
                .after(ClerestoryUpdateSet::MonitorTopology)
                .before(RiggingSystems::Collect),
        );
}

/// The topology this system last published, and the script revision it published it at.
///
/// The revision is what makes a re-publish detectable. The world itself is no longer the record:
/// `publish_scripted_display_topology` spawns and despawns `Monitor` entities and leaves the
/// production monitor scan to derive `DisplayTopologyObservation` from them, so the only thing
/// that says whether the published set is still the current script is the script's own revision.
#[derive(Default)]
enum PublishedDisplayTopology {
    #[default]
    NothingPublished,
    Published {
        revision: usize,
        entities: Vec<PublishedMonitor>,
    },
}

/// One published monitor entity and the display it stands for.
///
/// The name is what lets a republication keep the entity a still-attached display already has.
/// `bevy_winit` spawns a [`Monitor`] entity only for a monitor that arrived and despawns only one
/// that left, and `HasWindows` is a `linked_spawn` relationship target: despawning a monitor
/// despawns every window sitting on it. A test double that replaced the whole set on every change
/// destroyed windows the platform it doubles would have left alone.
struct PublishedMonitor {
    display: String,
    entity:  Entity,
}

impl PublishedDisplayTopology {
    /// Whether the world still carries what this system published for `revision`.
    const fn still_published(&self, revision: usize) -> bool {
        matches!(
            self,
            Self::Published {
                revision: published_revision,
                ..
            } if *published_revision == revision
        )
    }

    /// Monitors the previous publication left in the world, keyed by the display each stands for.
    fn monitors(&self) -> &[PublishedMonitor] {
        match self {
            Self::NothingPublished => &[],
            Self::Published { entities, .. } => entities,
        }
    }
}

const fn scripted_monitor_descriptor(index: usize) -> MonitorDescriptor {
    MonitorDescriptor::for_current_enumeration(index, 1.0, IVec2::ZERO, UVec2::ONE)
}

/// Publish the current script as the topology and winit enumeration the display reporter reads.
///
/// Both resources carry the currently attached displays. The adapter retains every scripted
/// descriptor privately so an exact display can return later, while the production observation
/// must omit a detached display or the reporter sees stale evidence paired with no live handle.
/// Their entity values join the reporter evidence to ordinary Bevy monitor topology. Publishing a
/// real [`Monitor`] component makes the production topology, panel spawning, and destination
/// repair paths observe the same declaration, omission, and re-declaration as the reporter.
/// Hand the production monitor scan the material the platform would publish about these displays.
///
/// The scan is what classifies a display, so feeding it the scripted evidence is what makes
/// `build_monitors` name the same durable key `DisplayTestDeviceKey` does. `display_evidence`
/// reports a display unavailable when its cached index is missing, so the enumeration order has to
/// be scripted alongside the evidence.
///
/// The descriptor's platform device handle is carried along for the same reason. It is not identity
/// material and never reaches the key, but it is the per-process capture address a screen reporter
/// joins its own records to the display reporter's on, and a scripted display has no
/// `MonitorHandle` for the scan to read one off.
fn inject_scripted_monitor_scan(
    enumerated_displays: &[&DisplayTestDescriptor],
    entities: &[PublishedMonitor],
    injected_evidence: &mut InjectedMonitorEvidence,
    injected_order: &mut InjectedWinitMonitorOrder,
) {
    *injected_evidence = InjectedMonitorEvidence::for_scripted_displays(
        enumerated_displays
            .iter()
            .zip(entities.iter())
            .map(|(display, published_monitor)| {
                (
                    published_monitor.entity,
                    display.published_identity_bytes(),
                    AttachmentPath::PlatformHasNoConcept,
                    display.platform_device_handle.clone(),
                )
            }),
    );
    *injected_order = InjectedWinitMonitorOrder::for_entities(
        entities
            .iter()
            .map(|published_monitor| published_monitor.entity)
            .collect(),
    );
}

fn publish_scripted_display_topology(
    mut commands: Commands,
    adapter: Res<DisplayTestAdapterResource>,
    mut published: Local<PublishedDisplayTopology>,
    mut injected_evidence: ResMut<InjectedMonitorEvidence>,
    mut injected_order: ResMut<InjectedWinitMonitorOrder>,
) {
    let revision = adapter.0.revision.load(Ordering::SeqCst);
    if published.still_published(revision) {
        return;
    }
    let topology = adapter
        .0
        .topology
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let enumerated_displays = topology
        .observed
        .iter()
        .filter(|display| topology.enumerated.contains(&display.name))
        .collect::<Vec<_>>();
    let entities = {
        let carried_over = enumerated_displays
            .iter()
            .enumerate()
            .map(|(index, display)| {
                let descriptor = scripted_monitor_descriptor(index);
                let monitor = Monitor {
                    name:                    Some(display.name.clone()),
                    physical_height:         descriptor.physical_size.y,
                    physical_width:          descriptor.physical_size.x,
                    physical_position:       descriptor.physical_position,
                    refresh_rate_millihertz: None,
                    scale_factor:            descriptor.scale,
                    video_modes:             Vec::new(),
                };
                let already_published = published
                    .monitors()
                    .iter()
                    .find(|published_monitor| published_monitor.display == display.name)
                    .map(|published_monitor| published_monitor.entity);
                let entity = match already_published {
                    Some(entity) => {
                        commands.entity(entity).insert(monitor);
                        entity
                    },
                    None => commands.spawn(monitor).id(),
                };
                PublishedMonitor {
                    display: display.name.clone(),
                    entity,
                }
            })
            .collect::<Vec<_>>();
        for departed in published.monitors().iter().filter(|published_monitor| {
            !carried_over
                .iter()
                .any(|kept| kept.entity == published_monitor.entity)
        }) {
            commands.entity(departed.entity).despawn();
        }
        carried_over
    };
    inject_scripted_monitor_scan(
        &enumerated_displays,
        &entities,
        &mut injected_evidence,
        &mut injected_order,
    );
    drop(topology);
    *published = PublishedDisplayTopology::Published { revision, entities };
}

/// Derive the scripted platform's current display list from the topology the reporter will read.
///
/// `reporter::fresh_display_evidence` treats the list as a consistency check against
/// `DisplayTopologyObservation`: an empty list beside a non-empty observation is a transport
/// failure, and a listed entity the observation cannot account for is an association failure.
/// Nothing the script says can satisfy both checks on its own, because the observation is no
/// longer written here — the production monitor scan builds it from the `Monitor` entities
/// `publish_scripted_display_topology` spawned, and installs it through `commands.queue`. Reading
/// the installed observation back is what keeps the two in step: this runs after
/// `ClerestoryUpdateSet::MonitorTopology`, whose own `ApplyDeferred` has already installed the
/// observation for this frame, and before the reporter collects.
///
/// The `Monitor` filter holds the invariant the reporter checks: every entity in the fresh list is
/// one the observation accounts for and one that is still an attached display. No despawn is
/// pending by the time this runs — `publish_scripted_display_topology` chains its own
/// `ApplyDeferred`, so a departing monitor is already gone before the scan that builds the
/// observation — so the filter never subtracts on the ordinary path. It is the guard that keeps a
/// stale observation entry, or any later source of one, from listing a display that no longer
/// exists and turning a scripted departure into a reported association failure.
fn derive_scripted_current_displays(
    mut fresh_displays: ResMut<InjectedFreshWinitDisplays>,
    observation: Res<DisplayTopologyObservation>,
    attached_monitors: Query<(), With<Monitor>>,
) {
    fresh_displays.entities = match observation.as_ref() {
        DisplayTopologyObservation::AwaitingInitialTopology => Vec::new(),
        DisplayTopologyObservation::Observed(observed) => observed
            .iter()
            .map(|evidence| evidence.entity)
            .filter(|entity| attached_monitors.contains(*entity))
            .collect(),
    };
}
