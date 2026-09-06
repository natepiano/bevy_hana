//! Installed monitor topology and raw lifetime events.

use std::collections::HashMap;
#[cfg(any(test, feature = "test"))]
use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Added;
use bevy::prelude::Commands;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::IVec2;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ReflectResource;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
#[cfg(any(test, feature = "test"))]
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
use bevy::prelude::World;
use bevy::prelude::debug;
use bevy::window::Monitor;
use bevy::window::MonitorSelection;
use bevy::winit::WinitMonitors;
#[cfg(feature = "monitor-probe")]
use bevy_diagnostic::FrameCount;
use hana_kana::ToI32;
use hana_kana::ToU32;
use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::PlatformDeviceHandle;
#[cfg(target_os = "macos")]
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::ReportedSerial;
use winit::monitor::MonitorHandle;
#[cfg(target_os = "macos")]
use winit::platform::macos::MonitorHandleExtMacOS;

use super::DisplayProductName;
use super::MonitorDiscoveryRequestCoverage;
use super::current_monitor;
use super::display_product_name;
use super::identity;
use super::identity::DisplayIdentity;
use super::identity::DisplayIdentityEvidence;
use super::identity::MonitorConfiguration;
use super::identity::MonitorConfigurationState;
#[cfg(any(test, feature = "test"))]
use super::identity::MonitorIdentificationError;
#[cfg(any(test, feature = "test"))]
use super::identity::QualifiedEvidence;
#[cfg(feature = "monitor-probe")]
use super::monitor_probe;
#[cfg(feature = "monitor-probe")]
use super::monitor_probe::TopologyChangeKind;
#[cfg(feature = "monitor-probe")]
use super::monitor_probe::TopologyProbeRecord;
#[cfg(feature = "monitor-probe")]
use super::monitor_probe::TopologyProducerSchedule;
use crate::Platform;

/// Position of one monitor in the current operating-system enumeration.
///
/// `CurrentMonitorIndex` is an adapter value for APIs such as [`MonitorSelection::Index`]. It has
/// no serde implementation, and converting it to an integer requires an explicit adapter-boundary
/// call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
#[type_path = "hana_clerestory::monitors"]
pub struct CurrentMonitorIndex(usize);

impl CurrentMonitorIndex {
    /// Mark a position as belonging to the current operating-system monitor enumeration.
    #[must_use]
    pub const fn from_current_enumeration(index: usize) -> Self { Self(index) }

    /// Select this monitor through Bevy's current monitor-enumeration adapter.
    #[must_use]
    pub const fn selection(self) -> MonitorSelection { MonitorSelection::Index(self.0) }

    /// Read the entry at this position from a collection ordered like the current monitor
    /// enumeration.
    #[must_use]
    pub fn select<T>(self, values: &[T]) -> Option<&T> { values.get(self.0) }

    /// Return the integer required by an adapter protocol that uses the same current enumeration.
    #[must_use]
    pub const fn adapter_value(self) -> usize { self.0 }
}

impl Display for CurrentMonitorIndex {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// Geometry and window-system adapter data for one live monitor.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[type_path = "hana_clerestory::monitors"]
pub struct MonitorDescriptor {
    /// Position in the current operating-system monitor enumeration.
    pub index:             CurrentMonitorIndex,
    /// Scale factor supplied by the current Bevy monitor entity.
    pub scale:             f64,
    /// Top-left corner supplied by the current Bevy monitor entity.
    pub physical_position: IVec2,
    /// Monitor dimensions in pixels supplied by the current Bevy monitor entity.
    pub physical_size:     UVec2,
}

impl MonitorDescriptor {
    /// Build geometry for one entry in the current operating-system monitor enumeration.
    #[must_use]
    pub const fn for_current_enumeration(
        index: usize,
        scale: f64,
        physical_position: IVec2,
        physical_size: UVec2,
    ) -> Self {
        Self {
            index: CurrentMonitorIndex::from_current_enumeration(index),
            scale,
            physical_position,
            physical_size,
        }
    }

    /// Logical desktop coordinate of this monitor's top-left corner, as the desktop would
    /// number it at `scale`.
    ///
    /// The scale is a parameter and never read from `self` on purpose. A monitor's logical
    /// origin is a function of the scale in force when the coordinate was written, so reading
    /// `self.scale` here would silently reinterpret a stored coordinate under the live scale —
    /// the exact defect the offset format exists to remove.
    #[must_use]
    pub(crate) fn logical_origin_at_scale(&self, scale: f64) -> IVec2 {
        IVec2::new(
            (f64::from(self.physical_position.x) / scale)
                .round()
                .to_i32(),
            (f64::from(self.physical_position.y) / scale)
                .round()
                .to_i32(),
        )
    }

    /// Physical desktop position of a window sitting `logical_offset` from this monitor's
    /// top-left corner, at this monitor's live scale.
    #[must_use]
    pub(crate) fn physical_from_logical_offset(&self, logical_offset: IVec2) -> IVec2 {
        self.physical_position
            + IVec2::new(
                (f64::from(logical_offset.x) * self.scale).round().to_i32(),
                (f64::from(logical_offset.y) * self.scale).round().to_i32(),
            )
    }

    /// Logical desktop position of a window sitting `logical_offset` from this monitor's
    /// top-left corner, as the desktop numbers it at this monitor's live scale.
    #[must_use]
    pub(crate) fn logical_from_logical_offset(&self, logical_offset: IVec2) -> IVec2 {
        self.logical_origin_at_scale(self.scale) + logical_offset
    }

    /// Monitor dimensions as the desktop numbers them at this monitor's live scale.
    ///
    /// Sizes and offsets saved for a window are logical, so fitting one onto a monitor compares
    /// against this rather than against `physical_size`.
    #[must_use]
    pub(crate) fn logical_size(&self) -> UVec2 {
        UVec2::new(
            (f64::from(self.physical_size.x) / self.scale)
                .round()
                .to_u32(),
            (f64::from(self.physical_size.y) / self.scale)
                .round()
                .to_u32(),
        )
    }
}

/// One current monitor entity and its entity-free metadata.
#[derive(Clone, Copy, Debug)]
pub struct LiveMonitor<'a> {
    /// Bevy monitor entity for the current monitor lifetime.
    pub entity:       Entity,
    /// Geometry and adapter data copied for the current monitor lifetime.
    pub descriptor:   &'a MonitorDescriptor,
    /// Product name the operating system shows the user for this display.
    pub product_name: &'a DisplayProductName,
}

/// Monotonic version of the installed monitor entity, geometry, and reporter-evidence topology.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Resource, Reflect)]
#[reflect(Resource)]
#[type_path = "hana_clerestory::monitors"]
pub struct MonitorTopologyRevision(u64);

impl MonitorTopologyRevision {
    /// Return the installed revision number.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }

    const fn next(self) -> Self { Self(self.0 + 1) }

    #[cfg(test)]
    pub(super) const fn from_test_raw(revision: u64) -> Self { Self(revision) }
}

/// Last installed monitor topology in Bevy's cached `WinitMonitors` order.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
#[type_path = "hana_clerestory::monitors"]
pub struct Monitors {
    #[reflect(ignore)]
    pub(super) live: Vec<InstalledMonitor>,
}

/// A monitor entity lifetime started independently of kernel device arrival.
///
/// The `entity` field is valid only while the monitor remains present in [`Monitors`]. Durable
/// device identity is read from `hana_rigging::Devices`, never from this lifetime event.
#[derive(Event, Debug, Clone, Reflect)]
#[reflect(Event)]
#[type_path = "hana_clerestory::monitors"]
pub(crate) struct MonitorConnected {
    /// Newly-created Bevy monitor entity.
    pub entity: Entity,
}

/// A monitor entity lifetime ended.
///
/// `former_entity` identifies the ended lifetime and must not be retained as a
/// current monitor target.
#[derive(Event, Debug, Clone, Reflect)]
#[reflect(Event)]
#[type_path = "hana_clerestory::monitors"]
pub(crate) struct MonitorDisconnected {
    /// Bevy monitor entity from the ended lifetime.
    pub former_entity: Entity,
}

impl Monitors {
    /// Iterate over all current monitor entities and their metadata.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = LiveMonitor<'_>> + '_ {
        self.live.iter().map(|monitor| LiveMonitor {
            entity:       monitor.entity,
            descriptor:   &monitor.descriptor,
            product_name: &monitor.product_name,
        })
    }

    /// Find the monitor containing position `(physical_x, physical_y)`.
    ///
    /// Coordinates are physical pixels in winit's monitor coordinate space.
    #[must_use]
    pub fn at(&self, physical_x: i32, physical_y: i32) -> Option<&MonitorDescriptor> {
        self.live
            .iter()
            .map(|monitor| &monitor.descriptor)
            .find(|descriptor| {
                physical_x >= descriptor.physical_position.x
                    && physical_x
                        < descriptor.physical_position.x + descriptor.physical_size.x.to_i32()
                    && physical_y >= descriptor.physical_position.y
                    && physical_y
                        < descriptor.physical_position.y + descriptor.physical_size.y.to_i32()
            })
    }

    /// Return whether no monitors are available.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.live.is_empty() }

    /// Get the first monitor in current enumeration order.
    ///
    /// # Panics
    ///
    /// Panics if no monitors exist.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "fail fast - no monitors means unrecoverable state"
    )]
    pub fn first(&self) -> &MonitorDescriptor {
        &self
            .live
            .first()
            .expect("Monitors::first() requires at least one monitor")
            .descriptor
    }

    /// Find the monitor a window is on, using the window center.
    ///
    /// All inputs are physical pixels in winit's monitor coordinate space.
    #[must_use]
    pub fn monitor_for_window(
        &self,
        physical_position: IVec2,
        physical_width: u32,
        physical_height: u32,
    ) -> &MonitorDescriptor {
        let physical_center_x = physical_position.x + (physical_width / 2).to_i32();
        let physical_center_y = physical_position.y + (physical_height / 2).to_i32();
        self.closest_to(physical_center_x, physical_center_y)
    }

    /// Find the monitor at a position, or the closest monitor to that position.
    ///
    /// # Panics
    ///
    /// Panics if no monitors exist.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "fail fast - no monitors means unrecoverable state"
    )]
    pub fn closest_to(&self, physical_x: i32, physical_y: i32) -> &MonitorDescriptor {
        if let Some(descriptor) = self.at(physical_x, physical_y) {
            return descriptor;
        }

        self.live
            .iter()
            .map(|monitor| &monitor.descriptor)
            .min_by_key(|descriptor| {
                let physical_right =
                    descriptor.physical_position.x + descriptor.physical_size.x.to_i32();
                let physical_bottom =
                    descriptor.physical_position.y + descriptor.physical_size.y.to_i32();

                let dx = if physical_x < descriptor.physical_position.x {
                    descriptor.physical_position.x - physical_x
                } else if physical_x >= physical_right {
                    physical_x - physical_right + 1
                } else {
                    0
                };

                let dy = if physical_y < descriptor.physical_position.y {
                    descriptor.physical_position.y - physical_y
                } else if physical_y >= physical_bottom {
                    physical_y - physical_bottom + 1
                } else {
                    0
                };

                dx * dx + dy * dy
            })
            .expect("Monitors::closest_to() requires at least one monitor")
    }

    /// Builds an installed monitor topology for downstream regression tests.
    #[cfg(any(test, feature = "test"))]
    #[must_use]
    pub fn from_test_monitors(
        monitors: impl IntoIterator<Item = (Entity, MonitorDescriptor)>,
    ) -> Self {
        let live = monitors
            .into_iter()
            .map(|(entity, descriptor)| InstalledMonitor {
                entity,
                descriptor,
                product_name: DisplayProductName::PlatformHasNoConcept,
                device_evidence: DisplayDeviceEvidence::unavailable(),
                legacy_identity: DisplayIdentity::Anonymous,
            })
            .collect();
        Self { live }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct InstalledMonitor {
    pub(super) entity:          Entity,
    pub(super) descriptor:      MonitorDescriptor,
    pub(super) product_name:    DisplayProductName,
    pub(super) device_evidence: DisplayDeviceEvidence,
    pub(super) legacy_identity: DisplayIdentity,
}

/// Kernel-facing evidence retained for one display without its Clerestory descriptor.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DisplayDeviceEvidence {
    pub(crate) identity_evidence:      DisplayIdentityEvidence,
    pub(crate) platform_device_handle: PlatformDeviceHandle,
    pub(crate) attachment:             AttachmentPath,
}

impl DisplayDeviceEvidence {
    #[cfg(any(test, feature = "test"))]
    #[allow(
        clippy::missing_const_for_fn,
        reason = "`platform_device_handle` is a `const fn` only on non-macOS targets"
    )]
    fn unavailable() -> Self {
        Self {
            identity_evidence:      DisplayIdentityEvidence::Unavailable {
                serial: ReportedSerial::PlatformCannotReport,
            },
            platform_device_handle: platform_device_handle(None),
            attachment:             AttachmentPath::PlatformHasNoConcept,
        }
    }
}

/// Evidence cached for one exact entity in Clerestory's current topology bookkeeping.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EnumeratedDisplayEvidence {
    pub(crate) entity:          Entity,
    pub(crate) descriptor:      MonitorDescriptor,
    pub(crate) device_evidence: DisplayDeviceEvidence,
    pub(crate) legacy_identity: DisplayIdentity,
}

/// Whether Clerestory has installed an observed display topology for the device reporter.
#[derive(Default, Resource)]
pub(crate) enum DisplayTopologyObservation {
    /// `init_monitors` has not yet installed its first observed topology.
    #[default]
    AwaitingInitialTopology,
    /// Kernel-facing evidence from a completed Clerestory topology observation.
    Observed(Vec<EnumeratedDisplayEvidence>),
}

struct ScannedMonitor {
    entity:            Entity,
    cached_index:      Option<usize>,
    handle:            Option<MonitorHandle>,
    index:             usize,
    winit_name:        Option<String>,
    scale:             f64,
    physical_position: IVec2,
    physical_size:     UVec2,
}

pub(super) struct MonitorChanges {
    pub(super) connected:        Vec<InstalledMonitor>,
    pub(super) disconnected:     Vec<InstalledMonitor>,
    #[cfg(feature = "monitor-probe")]
    pub(super) evidence_changed: Vec<InstalledMonitor>,
}

#[cfg(any(test, feature = "test"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TopologyProducerActivity {
    pub topology_scans:       usize,
    pub component_reads:      usize,
    pub cached_order_lookups: usize,
    pub identity_requests:    usize,
    pub handle_lookups:       usize,
    pub evidence_loads:       usize,
}

#[cfg(any(test, feature = "test"))]
#[derive(Default, Resource)]
pub(crate) struct InjectedMonitorEvidence {
    evidence:        HashMap<Entity, Result<Vec<u8>, MonitorIdentificationError>>,
    attachments:     HashMap<Entity, AttachmentPath>,
    /// The capture address the scripted platform reports for each display.
    ///
    /// A scripted display has no `MonitorHandle` to read one off, and the handle is not identity
    /// material: it is the per-process address a screen reporter joins its own records to the
    /// display reporter's on. Deriving it from the absent handle reports
    /// `PlatformDeviceHandle::PlatformReportedNothing` for every scripted display, which keys
    /// correctly and joins to nothing.
    device_handles:  HashMap<Entity, PlatformDeviceHandle>,
    product_names:   HashMap<Entity, DisplayProductName>,
    missing_handles: HashSet<Entity>,
    activity:        TopologyProducerActivity,
}

#[cfg(any(test, feature = "test"))]
impl InjectedMonitorEvidence {
    #[cfg(test)]
    pub(crate) fn identified(entity: Entity, evidence: &'static [u8]) -> Self {
        Self {
            evidence: HashMap::from([(entity, Ok(evidence.to_vec()))]),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn identified_pair(
        first: (Entity, &'static [u8]),
        second: (Entity, &'static [u8]),
    ) -> Self {
        Self {
            evidence: HashMap::from([
                (first.0, Ok(first.1.to_vec())),
                (second.0, Ok(second.1.to_vec())),
            ]),
            ..Self::default()
        }
    }

    /// Stand in for the platform behind a set of scripted displays.
    ///
    /// `bytes` is the identity material the display publishes, or `None` for a display whose scan
    /// produces none; the second case joins `missing_handles`, which is how the scan reports a
    /// display it can name no durable key for.
    pub(crate) fn for_scripted_displays(
        displays: impl IntoIterator<
            Item = (
                Entity,
                Option<Vec<u8>>,
                AttachmentPath,
                PlatformDeviceHandle,
            ),
        >,
    ) -> Self {
        let mut injected = Self::default();
        for (entity, bytes, attachment, device_handle) in displays {
            injected.attachments.insert(entity, attachment);
            injected.device_handles.insert(entity, device_handle);
            match bytes {
                Some(bytes) => {
                    injected.evidence.insert(entity, Ok(bytes));
                },
                None => {
                    injected.missing_handles.insert(entity);
                },
            }
        }
        injected
    }

    #[cfg(test)]
    pub(crate) fn report_product_name(&mut self, entity: Entity, product_name: impl Into<String>) {
        self.product_names
            .insert(entity, DisplayProductName::Reported(product_name.into()));
    }
}

#[cfg(any(test, feature = "test"))]
#[derive(Default, Resource)]
pub(crate) struct InjectedWinitMonitorOrder {
    entities: Vec<Entity>,
}

#[cfg(any(test, feature = "test"))]
impl InjectedWinitMonitorOrder {
    #[cfg(test)]
    pub(crate) fn single(entity: Entity) -> Self {
        Self {
            entities: vec![entity],
        }
    }

    /// Stand in for the cached winit enumeration order behind a set of scripted displays.
    pub(crate) const fn for_entities(entities: Vec<Entity>) -> Self { Self { entities } }

    #[cfg(test)]
    pub(crate) fn pair(first: Entity, second: Entity) -> Self {
        Self {
            entities: vec![first, second],
        }
    }
}

fn monitor_changes(previous: &Monitors, rebuilt: &Monitors) -> MonitorChanges {
    let connected = rebuilt
        .live
        .iter()
        .filter(|monitor| {
            !previous
                .live
                .iter()
                .any(|previous| previous.entity == monitor.entity)
        })
        .cloned()
        .collect();
    let disconnected = previous
        .live
        .iter()
        .filter(|monitor| {
            !rebuilt
                .live
                .iter()
                .any(|rebuilt| rebuilt.entity == monitor.entity)
        })
        .cloned()
        .collect();

    #[cfg(feature = "monitor-probe")]
    let evidence_changed = rebuilt
        .live
        .iter()
        .filter(|monitor| {
            previous.live.iter().any(|previous| {
                previous.entity == monitor.entity
                    && (previous.device_evidence != monitor.device_evidence
                        || previous.legacy_identity != monitor.legacy_identity)
            })
        })
        .cloned()
        .collect();

    MonitorChanges {
        connected,
        disconnected,
        #[cfg(feature = "monitor-probe")]
        evidence_changed,
    }
}

fn installed_topology_changed(previous: &Monitors, rebuilt: &Monitors) -> bool {
    previous.live.len() != rebuilt.live.len()
        || previous.live.iter().any(|previous_monitor| {
            rebuilt
                .live
                .iter()
                .find(|rebuilt_monitor| rebuilt_monitor.entity == previous_monitor.entity)
                .is_none_or(|rebuilt_monitor| {
                    rebuilt_monitor.entity != previous_monitor.entity
                        || rebuilt_monitor.descriptor != previous_monitor.descriptor
                        || rebuilt_monitor.product_name != previous_monitor.product_name
                        || rebuilt_monitor.device_evidence != previous_monitor.device_evidence
                        || rebuilt_monitor.legacy_identity != previous_monitor.legacy_identity
                })
        })
}

fn assign_cached_monitor_order(
    scanned: &mut [ScannedMonitor],
    winit_monitors: &WinitMonitors,
    #[cfg(any(test, feature = "test"))] mut injected_evidence: Option<&mut InjectedMonitorEvidence>,
    #[cfg(any(test, feature = "test"))] injected_order: Option<&InjectedWinitMonitorOrder>,
) {
    #[cfg(any(test, feature = "test"))]
    if let Some(injected_order) = injected_order {
        for monitor in &mut *scanned {
            if let Some(injected_evidence) = injected_evidence.as_deref_mut() {
                injected_evidence.activity.cached_order_lookups += 1;
            }
            monitor.cached_index = injected_order
                .entities
                .iter()
                .position(|entity| *entity == monitor.entity);
        }
        assign_unassociated_indices(scanned, injected_order.entities.len());
        return;
    }

    let cached_handles: Vec<_> = (0..).map_while(|index| winit_monitors.nth(index)).collect();
    for monitor in &mut *scanned {
        #[cfg(any(test, feature = "test"))]
        if let Some(injected_evidence) = injected_evidence.as_deref_mut() {
            injected_evidence.activity.cached_order_lookups += 1;
        }
        monitor.handle = winit_monitors.find_entity(monitor.entity);
        monitor.cached_index = monitor.handle.as_ref().and_then(|handle| {
            cached_handles
                .iter()
                .position(|cached_handle| cached_handle == handle)
        });
    }
    assign_unassociated_indices(scanned, cached_handles.len());
}

fn assign_unassociated_indices(scanned: &mut [ScannedMonitor], cached_monitor_count: usize) {
    scanned.sort_by_key(|monitor| {
        (
            monitor.cached_index.is_none(),
            monitor.cached_index.unwrap_or_default(),
            monitor.entity.to_bits(),
        )
    });

    let mut unassociated_index = cached_monitor_count;
    for monitor in scanned {
        monitor.index = monitor.cached_index.unwrap_or_else(|| {
            let index = unassociated_index;
            unassociated_index += 1;
            index
        });
    }
}

fn display_evidence(
    monitor: &ScannedMonitor,
    configuration: MonitorConfigurationState,
    platform: Platform,
    #[cfg(any(test, feature = "test"))] injected_evidence: Option<&mut InjectedMonitorEvidence>,
) -> (DisplayDeviceEvidence, DisplayIdentity) {
    let unavailable = |attachment| {
        (
            DisplayDeviceEvidence {
                identity_evidence: DisplayIdentityEvidence::Unavailable {
                    serial: ReportedSerial::PlatformCannotReport,
                },
                platform_device_handle: platform_device_handle(monitor.handle.as_ref()),
                attachment,
            },
            DisplayIdentity::Anonymous,
        )
    };

    #[cfg(any(test, feature = "test"))]
    if let Some(injected_evidence) = injected_evidence {
        injected_evidence.activity.identity_requests += 1;
        // A scripted display publishes its capture address through the injection rather than
        // through a `MonitorHandle` it does not have, and every branch below reports the same one.
        let scripted_device_handle = injected_evidence
            .device_handles
            .get(&monitor.entity)
            .cloned()
            .unwrap_or_else(|| platform_device_handle(monitor.handle.as_ref()));
        let scripted_unavailable = |attachment| {
            (
                DisplayDeviceEvidence {
                    identity_evidence: DisplayIdentityEvidence::Unavailable {
                        serial: ReportedSerial::PlatformCannotReport,
                    },
                    platform_device_handle: scripted_device_handle.clone(),
                    attachment,
                },
                DisplayIdentity::Anonymous,
            )
        };
        if monitor.cached_index.is_none() {
            injected_evidence.activity.handle_lookups += 1;
            return scripted_unavailable(identity::attachment_path_when_unreported(platform));
        }
        if injected_evidence.missing_handles.contains(&monitor.entity) {
            injected_evidence.activity.handle_lookups += 1;
            return scripted_unavailable(identity::attachment_path_when_unreported(platform));
        }
        let attachment = injected_evidence
            .attachments
            .get(&monitor.entity)
            .cloned()
            .unwrap_or_else(|| identity::attachment_path_when_unreported(platform));
        let evidence = injected_evidence
            .evidence
            .get(&monitor.entity)
            .cloned()
            .unwrap_or(Err(
                MonitorIdentificationError::StablePhysicalIdentityUnavailable,
            ));
        injected_evidence.activity.handle_lookups += 1;
        injected_evidence.activity.evidence_loads += 1;
        return evidence.map_or_else(
            |_| scripted_unavailable(attachment.clone()),
            |bytes| {
                let qualified = QualifiedEvidence::Synthetic(bytes);
                let (identity_evidence, legacy_identity) =
                    identity::classify_display_evidence(&qualified);
                (
                    DisplayDeviceEvidence {
                        identity_evidence,
                        platform_device_handle: scripted_device_handle.clone(),
                        attachment: attachment.clone(),
                    },
                    legacy_identity,
                )
            },
        );
    }

    if monitor.cached_index.is_none()
        || matches!(configuration, MonitorConfigurationState::Unavailable(_))
        || platform.is_wayland()
    {
        return unavailable(identity::attachment_path_when_unreported(platform));
    }

    let Some(handle) = monitor.handle.as_ref() else {
        return unavailable(identity::attachment_path_when_unreported(platform));
    };
    let observation = identity::monitor_evidence(handle, platform);
    let attachment = observation.attachment.clone();
    observation.identity.map_or_else(
        |_| unavailable(attachment.clone()),
        |evidence| {
            let (identity_evidence, legacy_identity) =
                identity::classify_display_evidence(&evidence);
            (
                DisplayDeviceEvidence {
                    identity_evidence,
                    platform_device_handle: platform_device_handle(monitor.handle.as_ref()),
                    attachment: attachment.clone(),
                },
                legacy_identity,
            )
        },
    )
}

/// Retain the exact window-server handle attached to this topology entry.
#[cfg(target_os = "macos")]
fn platform_device_handle(monitor_handle: Option<&MonitorHandle>) -> PlatformDeviceHandle {
    let Some(monitor_handle) = monitor_handle else {
        return PlatformDeviceHandle::PlatformReportedNothing;
    };
    ReportedId::new(monitor_handle.native_id().to_string()).map_or(
        PlatformDeviceHandle::PlatformReportedNothing,
        PlatformDeviceHandle::Reported,
    )
}

/// Report that this platform supplies no handle shared with another display reporter.
#[cfg(not(target_os = "macos"))]
const fn platform_device_handle(_: Option<&MonitorHandle>) -> PlatformDeviceHandle {
    PlatformDeviceHandle::PlatformHasNoConcept
}

fn display_product_name(
    monitor: &ScannedMonitor,
    platform: Platform,
    #[cfg(any(test, feature = "test"))] injected_evidence: Option<&InjectedMonitorEvidence>,
) -> DisplayProductName {
    #[cfg(any(test, feature = "test"))]
    if let Some(injected_evidence) = injected_evidence {
        return injected_evidence
            .product_names
            .get(&monitor.entity)
            .cloned()
            .unwrap_or(DisplayProductName::PlatformHasNoConcept);
    }

    display_product_name::from_platform(
        monitor.handle.as_ref(),
        monitor.winit_name.as_deref(),
        platform,
    )
}

fn build_monitors(
    monitors: &Query<(Entity, &Monitor)>,
    winit_monitors: &WinitMonitors,
    configuration: MonitorConfigurationState,
    platform: Platform,
    #[cfg(any(test, feature = "test"))] mut injected_evidence: Option<&mut InjectedMonitorEvidence>,
    #[cfg(any(test, feature = "test"))] injected_order: Option<&InjectedWinitMonitorOrder>,
) -> Monitors {
    #[cfg(any(test, feature = "test"))]
    if let Some(injected_evidence) = injected_evidence.as_deref_mut() {
        injected_evidence.activity.topology_scans += 1;
    }

    let mut scanned = Vec::new();
    for (entity, monitor) in monitors.iter() {
        #[cfg(any(test, feature = "test"))]
        if let Some(injected_evidence) = injected_evidence.as_deref_mut() {
            injected_evidence.activity.component_reads += 1;
        }
        scanned.push(ScannedMonitor {
            entity,
            cached_index: None,
            handle: None,
            index: usize::MAX,
            winit_name: monitor.name.clone(),
            scale: monitor.scale_factor,
            physical_position: monitor.physical_position,
            physical_size: monitor.physical_size(),
        });
    }
    assign_cached_monitor_order(
        &mut scanned,
        winit_monitors,
        #[cfg(any(test, feature = "test"))]
        injected_evidence.as_deref_mut(),
        #[cfg(any(test, feature = "test"))]
        injected_order,
    );

    let evidence: HashMap<_, _> = scanned
        .iter()
        .map(|monitor| {
            let evidence = display_evidence(
                monitor,
                configuration,
                platform,
                #[cfg(any(test, feature = "test"))]
                injected_evidence.as_deref_mut(),
            );
            (monitor.entity, evidence)
        })
        .collect();

    let live = scanned
        .into_iter()
        .map(|monitor| {
            let (device_evidence, legacy_identity) =
                evidence.get(&monitor.entity).cloned().unwrap_or_else(|| {
                    display_evidence(
                        &monitor,
                        configuration,
                        platform,
                        #[cfg(any(test, feature = "test"))]
                        None,
                    )
                });
            let product_name = display_product_name(
                &monitor,
                platform,
                #[cfg(any(test, feature = "test"))]
                injected_evidence.as_deref(),
            );
            InstalledMonitor {
                entity: monitor.entity,
                descriptor: MonitorDescriptor::for_current_enumeration(
                    monitor.index,
                    monitor.scale,
                    monitor.physical_position,
                    monitor.physical_size,
                ),
                product_name,
                device_evidence,
                legacy_identity,
            }
        })
        .collect();
    Monitors { live }
}

fn queue_topology_install(
    commands: &mut Commands,
    rebuilt: Monitors,
    revision: MonitorTopologyRevision,
    changes: MonitorChanges,
    #[cfg(feature = "monitor-probe")] probe_records: Vec<TopologyProbeRecord>,
) {
    let topology_is_empty = rebuilt.is_empty();
    let display_topology_observation = DisplayTopologyObservation::Observed(
        rebuilt
            .live
            .iter()
            .map(|monitor| EnumeratedDisplayEvidence {
                entity:          monitor.entity,
                descriptor:      monitor.descriptor,
                device_evidence: monitor.device_evidence.clone(),
                legacy_identity: monitor.legacy_identity,
            })
            .collect(),
    );
    commands.queue(move |world: &mut World| {
        world.insert_resource(rebuilt);
        world.insert_resource(display_topology_observation);
        world.insert_resource(revision);
        if topology_is_empty {
            current_monitor::remove_current_monitors_for_empty_topology(world);
        }
        #[cfg(feature = "monitor-probe")]
        for record in probe_records {
            #[cfg(test)]
            monitor_probe::capture_record(world, &record);
            record.emit();
        }
        for monitor in changes.connected {
            world.trigger(MonitorConnected {
                entity: monitor.entity,
            });
        }
        for monitor in changes.disconnected {
            world.trigger(MonitorDisconnected {
                former_entity: monitor.entity,
            });
        }
    });
}

/// Initialize the [`Monitors`] resource at startup.
pub(super) fn init_monitors(
    mut commands: Commands,
    monitors: Query<(Entity, &Monitor)>,
    winit_monitors: Res<WinitMonitors>,
    configuration: Res<MonitorConfiguration>,
    platform: Res<Platform>,
    #[cfg(feature = "monitor-probe")] frame_count: Res<FrameCount>,
    #[cfg(any(test, feature = "test"))] mut injected_evidence: Option<
        ResMut<InjectedMonitorEvidence>,
    >,
    #[cfg(any(test, feature = "test"))] injected_order: Option<Res<InjectedWinitMonitorOrder>>,
    _: NonSendMarker,
) {
    let configuration = configuration.state();
    let rebuilt = build_monitors(
        &monitors,
        &winit_monitors,
        configuration,
        *platform,
        #[cfg(any(test, feature = "test"))]
        injected_evidence.as_deref_mut(),
        #[cfg(any(test, feature = "test"))]
        injected_order.as_deref(),
    );
    debug!("[init_monitors] Found {} monitors", rebuilt.iter().len());
    let revision = MonitorTopologyRevision::default();
    commands.insert_resource(MonitorDiscoveryRequestCoverage::from_initial_topology(
        revision,
    ));
    let changes = monitor_changes(&Monitors { live: Vec::new() }, &rebuilt);
    #[cfg(feature = "monitor-probe")]
    let probe_records = monitor_probe::changed_probe_records(
        &changes,
        frame_count.0,
        TopologyProducerSchedule::PreStartup,
        configuration,
        revision,
    );
    queue_topology_install(
        &mut commands,
        rebuilt,
        revision,
        changes,
        #[cfg(feature = "monitor-probe")]
        probe_records,
    );
}

/// Revalidate and install monitor entity-lifetime topology changes.
pub(super) fn update_monitors(
    mut commands: Commands,
    monitors: Query<(Entity, &Monitor)>,
    winit_monitors: Res<WinitMonitors>,
    added_monitors: Query<(), Added<Monitor>>,
    mut removed_monitors: RemovedComponents<Monitor>,
    previous: Res<Monitors>,
    revision: Res<MonitorTopologyRevision>,
    configuration: Res<MonitorConfiguration>,
    platform: Res<Platform>,
    #[cfg(feature = "monitor-probe")] frame_count: Res<FrameCount>,
    #[cfg(any(test, feature = "test"))] mut injected_evidence: Option<
        ResMut<InjectedMonitorEvidence>,
    >,
    #[cfg(any(test, feature = "test"))] injected_order: Option<Res<InjectedWinitMonitorOrder>>,
    _: NonSendMarker,
) {
    let configuration_changed = configuration.is_changed();
    let configuration = configuration.state();
    let monitor_added = !added_monitors.is_empty();
    let monitor_removed = removed_monitors.read().count() != 0;
    if !monitor_added && !monitor_removed && !configuration_changed {
        return;
    }

    let rebuilt = build_monitors(
        &monitors,
        &winit_monitors,
        configuration,
        *platform,
        #[cfg(any(test, feature = "test"))]
        injected_evidence.as_deref_mut(),
        #[cfg(any(test, feature = "test"))]
        injected_order.as_deref(),
    );
    if !installed_topology_changed(&previous, &rebuilt) {
        if configuration_changed {
            #[cfg(feature = "monitor-probe")]
            for record in monitor_probe::current_probe_records(
                &previous,
                frame_count.0,
                TopologyProducerSchedule::Update,
                configuration,
                *revision,
                TopologyChangeKind::RevalidatedUnchanged,
            ) {
                record.emit();
            }
        }
        return;
    }

    let revision = revision.next();
    let changes = monitor_changes(&previous, &rebuilt);
    debug!(
        "[update_monitors] installed revision={} with {} monitors",
        revision.get(),
        rebuilt.iter().len(),
    );
    #[cfg(feature = "monitor-probe")]
    let probe_records = monitor_probe::changed_probe_records(
        &changes,
        frame_count.0,
        TopologyProducerSchedule::Update,
        configuration,
        revision,
    );
    queue_topology_install(
        &mut commands,
        rebuilt,
        revision,
        changes,
        #[cfg(feature = "monitor-probe")]
        probe_records,
    );
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::SystemParamValidationError;

    use super::*;

    fn descriptor(index: usize) -> MonitorDescriptor {
        MonitorDescriptor::for_current_enumeration(
            index,
            2.0,
            IVec2::new(index.to_i32() * 1920, 0),
            UVec2::new(1920, 1080),
        )
    }

    fn retained(
        entity: Entity,
        index: usize,
        legacy_identity: DisplayIdentity,
    ) -> InstalledMonitor {
        InstalledMonitor {
            entity,
            descriptor: descriptor(index),
            product_name: DisplayProductName::PlatformHasNoConcept,
            device_evidence: DisplayDeviceEvidence::unavailable(),
            legacy_identity,
        }
    }

    #[test]
    fn monitor_lifetime_changes_follow_entities_not_geometry() {
        let first = Entity::from_bits(1);
        let replacement = Entity::from_bits(2);
        let previous = Monitors {
            live: vec![retained(first, 0, DisplayIdentity::Anonymous)],
        };
        let rebuilt = Monitors {
            live: vec![retained(replacement, 0, DisplayIdentity::Anonymous)],
        };

        let changes = monitor_changes(&previous, &rebuilt);

        assert_eq!(changes.connected.len(), 1);
        assert_eq!(changes.connected[0].entity, replacement);
        assert_eq!(changes.disconnected.len(), 1);
        assert_eq!(changes.disconnected[0].entity, first);
    }

    fn monitor_component() -> Monitor {
        Monitor {
            name:                    None,
            physical_height:         1_080,
            physical_width:          1_920,
            physical_position:       IVec2::ZERO,
            refresh_rate_millihertz: None,
            scale_factor:            1.0,
            video_modes:             Vec::new(),
        }
    }

    fn topology_with_injected_product_name(
        product_name: Option<&str>,
    ) -> Result<Monitors, SystemParamValidationError> {
        let mut world = World::new();
        let entity = world.spawn(monitor_component()).id();
        let mut injected_evidence = InjectedMonitorEvidence::default();
        if let Some(product_name) = product_name {
            injected_evidence.report_product_name(entity, product_name);
        }
        let winit_monitors = WinitMonitors::default();
        let injected_order = InjectedWinitMonitorOrder::single(entity);
        let mut monitor_query =
            bevy::ecs::system::SystemState::<Query<(Entity, &Monitor)>>::new(&mut world);
        let monitors = monitor_query.get(&world)?;

        Ok(build_monitors(
            &monitors,
            &winit_monitors,
            MonitorConfigurationState::Unavailable(
                MonitorIdentificationError::StablePhysicalIdentityUnavailable,
            ),
            Platform::X11,
            Some(&mut injected_evidence),
            Some(&injected_order),
        ))
    }

    #[test]
    fn build_monitors_exposes_an_injected_product_name() -> Result<(), SystemParamValidationError> {
        let monitors = topology_with_injected_product_name(Some("DELL S3425DW"))?;

        assert_eq!(
            monitors
                .iter()
                .map(|monitor| monitor.product_name.as_reported())
                .collect::<Vec<_>>(),
            vec![Some("DELL S3425DW")],
        );
        Ok(())
    }

    #[test]
    fn build_monitors_classifies_an_absent_injected_product_name()
    -> Result<(), SystemParamValidationError> {
        let monitors = topology_with_injected_product_name(None)?;

        assert_eq!(monitors.iter().len(), 1);
        assert!(monitors.iter().all(|monitor| matches!(
            monitor.product_name,
            DisplayProductName::PlatformHasNoConcept
        )));
        Ok(())
    }
}
