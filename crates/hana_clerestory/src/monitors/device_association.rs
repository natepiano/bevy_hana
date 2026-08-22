//! Exact bridge from rigging display keys to Clerestory's current monitor entities.

use std::collections::HashMap;
use std::collections::HashSet;

use bevy::prelude::Entity;
use bevy::prelude::Resource;
use bevy::prelude::info;
use bevy::prelude::warn;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceResolution;
use hana_rigging::prelude::Devices;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::PlatformDeviceHandle;
use hana_rigging::prelude::ReportedId;

use super::MonitorDescriptor;
use super::Monitors;
use super::PanelIdentity;
use crate::persistence::PersistedPanelIdentityV4;
use crate::reporter;
use crate::reporter::DisplayKeyClassification;

/// Current exact association from a reporter-issued display key to a panel entity.
///
/// The resource is deliberately Clerestory-owned: the kernel retains durable device state but
/// cannot enumerate monitors or retain window-system geometry. Rebuilding the table from the
/// installed topology makes the reporter scan and a window driver's live monitor lookup share
/// one identity classification without introducing an enumeration-order fallback.
#[derive(Default, Resource)]
pub struct MonitorDeviceAssociation {
    live_by_device: HashMap<DeviceKey, LiveMonitorAssociation>,
}

#[derive(Clone)]
struct LiveMonitorAssociation {
    monitor_entity:         Entity,
    descriptor:             MonitorDescriptor,
    panel_identity:         PanelIdentity,
    platform_device_handle: PlatformDeviceHandle,
}

/// Result of resolving a current Clerestory monitor representation to one reporter-issued key.
///
/// A missing result means no exact, unambiguous reporter key exists. Binding a window to an
/// enumeration position in that case would authorize whichever panel happens to occupy it later.
pub enum MonitorDeviceKeyLookup {
    /// One reporter-issued device key exactly names the current monitor.
    Exact(DeviceKey),
    /// No single reporter-issued key can safely name the requested monitor.
    Unresolved,
}

/// Result of asking Clerestory's current display topology for the process-local handle associated
/// with one durable display key.
///
/// The returned [`ReportedId`] is the handle already observed for the exact live monitor; this
/// lookup never derives one from geometry, enumeration order, or primary-display status. The two
/// unresolved cases remain separate because a caller waiting for a monitor to return has a
/// different diagnostic from a live monitor whose platform report cannot join another provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonitorReportedHandleLookup {
    /// The durable display key names one live monitor carrying a reported platform handle.
    Exact(ReportedId),
    /// The durable key does not currently name one unambiguous live monitor.
    NoCurrentLiveMonitor,
    /// The monitor is live, but its platform report carries no handle another provider can join.
    LiveMonitorWithoutReportedHandle,
}

/// Result of resolving a durable display key to Clerestory's live geometry.
///
/// A named result prevents an absent display that the kernel remembers from being confused with
/// a key no reporter has ever retained. Neither absence admits a substitute monitor: callers
/// must wait, retire their role, or report the unknown configuration explicitly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MonitorDeviceLookup {
    /// The exact key currently names this monitor entity and descriptor.
    Live {
        /// Entity whose lifetime represents the panel currently installed by Bevy.
        monitor_entity: Entity,
        /// Geometry copied at topology installation so it is paired with the same key.
        descriptor:     MonitorDescriptor,
    },
    /// The kernel retains this key, but no current topology entry names a live monitor for it.
    KnownWithoutLiveMonitor,
    /// No accepted reporter has retained this key during the current process.
    UnknownDevice,
}

impl MonitorDeviceAssociation {
    /// Rebuild the bridge from one installed topology using the reporter's key classifier.
    ///
    /// A duplicated durable key is intentionally omitted instead of choosing the first panel.
    /// Two physical panels that offer indistinguishable evidence cannot safely be selected for a
    /// window operation, even though either choice would look plausible on a desktop.
    pub(super) fn refresh(&mut self, monitors: &Monitors) {
        let Ok(edid_serial_scheme) = reporter::edid_serial_scheme() else {
            self.live_by_device.clear();
            return;
        };

        let mut candidates = HashMap::new();
        let mut ambiguous = HashSet::new();
        for monitor in &monitors.live {
            let DisplayKeyClassification::Keyed(device_key) =
                reporter::classify_display_key(&monitor.device_evidence, &edid_serial_scheme)
            else {
                continue;
            };
            if candidates
                .insert(
                    device_key.clone(),
                    LiveMonitorAssociation {
                        monitor_entity:         monitor.entity,
                        descriptor:             monitor.descriptor,
                        panel_identity:         monitor.legacy_panel_identity,
                        platform_device_handle: monitor
                            .device_evidence
                            .platform_device_handle
                            .clone(),
                    },
                )
                .is_some()
            {
                ambiguous.insert(device_key);
            }
        }
        for device_key in &ambiguous {
            candidates.remove(device_key);
        }
        // This runs every frame, so the diagnostics speak only when the resolved key set
        // actually changes; a settled topology stays silent.
        let resolution_changed = candidates.len() != self.live_by_device.len()
            || candidates
                .keys()
                .any(|device_key| !self.live_by_device.contains_key(device_key));
        if resolution_changed {
            // A dropped key is unreachable for every window operation that names a display, so the
            // reason has to be visible: without it a window simply refuses to follow its display
            // and nothing anywhere says why.
            for device_key in &ambiguous {
                warn!(
                    "[monitor_device_association] {device_key:?} is claimed by more than one live display, so no window can be targeted at either; they report indistinguishable identity evidence"
                );
            }
            info!(
                "[monitor_device_association] {} live displays resolved to {} targetable devices",
                monitors.live.len(),
                candidates.len(),
            );
        }
        self.live_by_device = candidates;
    }

    /// Resolve one display key without inventing a geometry-based or primary-monitor fallback.
    #[must_use]
    pub(crate) fn lookup(
        &self,
        devices: &Devices,
        inventory: &HardwareInventory,
        device_key: &DeviceKey,
    ) -> MonitorDeviceLookup {
        if let Some(live) = self.live_by_device.get(device_key) {
            return MonitorDeviceLookup::Live {
                monitor_entity: live.monitor_entity,
                descriptor:     live.descriptor,
            };
        }

        if matches!(devices.resolve(device_key), DeviceResolution::Resolved(_))
            || inventory.configured_device(device_key).is_ok()
        {
            MonitorDeviceLookup::KnownWithoutLiveMonitor
        } else {
            MonitorDeviceLookup::UnknownDevice
        }
    }

    /// Live geometry of the monitor one durable display key currently names.
    ///
    /// `None` is the whole reason a window can be stranded: a role bound to a display that is not
    /// plugged in holds a plan no amount of waiting can land, because nothing will ever resolve
    /// the endpoint it names.
    #[must_use]
    pub(crate) fn live_descriptor(&self, device_key: &DeviceKey) -> Option<MonitorDescriptor> {
        self.live_by_device
            .get(device_key)
            .map(|live| live.descriptor)
    }

    /// Build an association naming exactly these displays, bypassing reporter classification.
    #[cfg(test)]
    pub(crate) fn from_test_live_displays(
        live: impl IntoIterator<Item = (DeviceKey, MonitorDescriptor)>,
    ) -> Self {
        Self {
            live_by_device: live
                .into_iter()
                .map(|(device_key, descriptor)| {
                    (
                        device_key,
                        LiveMonitorAssociation {
                            monitor_entity: Entity::PLACEHOLDER,
                            descriptor,
                            panel_identity: PanelIdentity::Anonymous,
                            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Resolve one current monitor descriptor to its reporter-issued key only when unique.
    #[must_use]
    pub fn device_for_descriptor(&self, descriptor: MonitorDescriptor) -> MonitorDeviceKeyLookup {
        self.device_key_matching(|live| live.descriptor == descriptor)
    }

    /// Resolve one durable display key to the exact process-local handle retained with its live
    /// Clerestory monitor association.
    #[must_use]
    pub fn reported_handle_for_device(
        &self,
        device_key: &DeviceKey,
    ) -> MonitorReportedHandleLookup {
        let Some(live) = self.live_by_device.get(device_key) else {
            return MonitorReportedHandleLookup::NoCurrentLiveMonitor;
        };
        match &live.platform_device_handle {
            PlatformDeviceHandle::Reported(reported_id) => {
                MonitorReportedHandleLookup::Exact(reported_id.clone())
            },
            PlatformDeviceHandle::PlatformHasNoConcept
            | PlatformDeviceHandle::PlatformReportedNothing => {
                MonitorReportedHandleLookup::LiveMonitorWithoutReportedHandle
            },
        }
    }

    /// Resolve one frozen v4 panel record to its reporter-issued key only when unique.
    ///
    /// This is deliberately the only legacy-panel bridge. The result adopts a key already minted
    /// by the fresh reporter scan; it never derives a new key from a persisted fingerprint.
    #[must_use]
    pub(crate) fn device_for_legacy_panel(
        &self,
        panel_identity: PersistedPanelIdentityV4,
    ) -> MonitorDeviceKeyLookup {
        self.device_key_matching(|live| {
            matches!(
                (panel_identity, live.panel_identity),
                (
                    PersistedPanelIdentityV4::Fingerprinted(saved),
                    PanelIdentity::Fingerprinted(live),
                ) if saved.0 == live.get()
            )
        })
    }

    fn device_key_matching(
        &self,
        predicate: impl Fn(&LiveMonitorAssociation) -> bool,
    ) -> MonitorDeviceKeyLookup {
        let mut matching = self
            .live_by_device
            .iter()
            .filter(|(_, live)| predicate(live));
        let Some((device_key, _)) = matching.next() else {
            return MonitorDeviceKeyLookup::Unresolved;
        };
        if matching.next().is_some() {
            return MonitorDeviceKeyLookup::Unresolved;
        }
        MonitorDeviceKeyLookup::Exact(device_key.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::App;
    use bevy::prelude::IVec2;
    use bevy::prelude::UVec2;
    use hana_rigging::prelude::AttachmentPath;
    use hana_rigging::prelude::Capabilities;
    use hana_rigging::prelude::Claim;
    use hana_rigging::prelude::ConfiguredDevice;
    use hana_rigging::prelude::ConfiguredDeviceMode;
    use hana_rigging::prelude::DeviceDescriptor;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::DeviceRecord;
    use hana_rigging::prelude::DeviceReporter;
    use hana_rigging::prelude::DeviceScan;
    use hana_rigging::prelude::DiscoveryCadence;
    use hana_rigging::prelude::DiscoveryWork;
    use hana_rigging::prelude::MainThreadDiscoveryJob;
    use hana_rigging::prelude::PlatformDeviceHandle;
    use hana_rigging::prelude::Presence;
    use hana_rigging::prelude::ReportedAs;
    use hana_rigging::prelude::ReportedParent;
    use hana_rigging::prelude::ReportedSerial;
    use hana_rigging::prelude::ReporterCoverage;
    use hana_rigging::prelude::ReporterRegistration;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;

    use super::*;
    use crate::monitors::CurrentMonitorIndex;
    use crate::monitors::DisplayDeviceEvidence;
    use crate::monitors::DisplayProductName;
    use crate::monitors::PanelFingerprint;
    use crate::monitors::PanelIdentity;
    use crate::monitors::PanelIdentityEvidence;
    use crate::monitors::topology::InstalledMonitor;
    use crate::persistence::PersistedPanelFingerprintV4;
    use crate::persistence::PersistedPanelIdentityV4;

    struct StaticDisplayReporter(DeviceKey);

    impl DeviceReporter for StaticDisplayReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let device_key = self.0.clone();
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| {
                DeviceScan::Complete(vec![DeviceRecord {
                    reported_as:            ReportedAs::Keyed(device_key),
                    parent:                 ReportedParent::Root,
                    presence:               Presence::Present,
                    claim:                  Claim::NotApplicable,
                    capabilities:           Capabilities::new(),
                    serial:                 ReportedSerial::NotExposedByUnit,
                    platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
                    attachment:             AttachmentPath::PlatformReportedNothing,
                    descriptor:             DeviceDescriptor::PlatformReportedNothing,
                }])
            }))
        }
    }

    fn key(fingerprint: PanelFingerprint) -> DeviceKey {
        DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: hana_rigging::prelude::Digest::new(fingerprint.get()),
            },
        }
    }

    fn topology(entity: Entity, fingerprint: PanelFingerprint) -> Monitors {
        topology_with_handle(
            entity,
            fingerprint,
            PlatformDeviceHandle::PlatformHasNoConcept,
        )
    }

    fn topology_with_handle(
        entity: Entity,
        fingerprint: PanelFingerprint,
        platform_device_handle: PlatformDeviceHandle,
    ) -> Monitors {
        Monitors {
            live: vec![InstalledMonitor {
                entity,
                descriptor: MonitorDescriptor {
                    index:             CurrentMonitorIndex::from_current_enumeration(4),
                    scale:             2.0,
                    physical_position: IVec2::new(2_000, 10),
                    physical_size:     UVec2::new(1_920, 1_080),
                },
                product_name: DisplayProductName::PlatformHasNoConcept,
                device_evidence: DisplayDeviceEvidence {
                    panel_identity: PanelIdentityEvidence::Synthesized {
                        fingerprint,
                        serial: ReportedSerial::NotExposedByUnit,
                    },
                    platform_device_handle,
                    attachment: AttachmentPath::PlatformReportedNothing,
                },
                legacy_panel_identity: PanelIdentity::Fingerprinted(fingerprint),
            }],
        }
    }

    #[test]
    fn durable_key_resolves_to_the_exact_live_reported_handle()
    -> Result<(), Box<dyn std::error::Error>> {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"handled-display");
        let reported_id = ReportedId::new("42")?;
        let mut association = MonitorDeviceAssociation::default();
        association.refresh(&topology_with_handle(
            Entity::from_bits(42),
            fingerprint,
            PlatformDeviceHandle::Reported(reported_id.clone()),
        ));

        assert_eq!(
            association.reported_handle_for_device(&key(fingerprint)),
            MonitorReportedHandleLookup::Exact(reported_id)
        );
        Ok(())
    }

    #[test]
    fn reported_handle_lookup_distinguishes_absent_and_unjoinable_live_monitors() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"unhandled-display");
        let device_key = key(fingerprint);
        let mut association = MonitorDeviceAssociation::default();

        assert_eq!(
            association.reported_handle_for_device(&device_key),
            MonitorReportedHandleLookup::NoCurrentLiveMonitor
        );

        association.refresh(&topology(Entity::from_bits(42), fingerprint));
        assert_eq!(
            association.reported_handle_for_device(&device_key),
            MonitorReportedHandleLookup::LiveMonitorWithoutReportedHandle
        );
    }

    #[test]
    fn exact_key_resolves_to_its_installed_descriptor() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"desk-display");
        let monitor_entity = Entity::from_bits(42);
        let mut association = MonitorDeviceAssociation::default();
        association.refresh(&topology(monitor_entity, fingerprint));

        assert_eq!(
            association.lookup(
                &Devices::default(),
                &HardwareInventory::default(),
                &key(fingerprint),
            ),
            MonitorDeviceLookup::Live {
                monitor_entity,
                descriptor: MonitorDescriptor {
                    index:             CurrentMonitorIndex::from_current_enumeration(4),
                    scale:             2.0,
                    physical_position: IVec2::new(2_000, 10),
                    physical_size:     UVec2::new(1_920, 1_080),
                },
            }
        );
    }

    #[test]
    fn frozen_v4_fingerprint_adopts_the_exact_fresh_reporter_key() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"desk-display");
        let device_key = key(fingerprint);
        let mut association = MonitorDeviceAssociation::default();
        association.refresh(&topology(Entity::from_bits(42), fingerprint));

        assert!(matches!(
            association.device_for_legacy_panel(PersistedPanelIdentityV4::Fingerprinted(
                PersistedPanelFingerprintV4(fingerprint.get()),
            )),
            MonitorDeviceKeyLookup::Exact(actual) if actual == device_key
        ));
    }

    #[test]
    fn anonymous_v4_target_cannot_select_a_live_monitor() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"desk-display");
        let mut association = MonitorDeviceAssociation::default();
        association.refresh(&topology(Entity::from_bits(42), fingerprint));

        assert!(matches!(
            association.device_for_legacy_panel(PersistedPanelIdentityV4::Anonymous),
            MonitorDeviceKeyLookup::Unresolved
        ));
    }

    #[test]
    fn retained_absent_display_is_not_an_unknown_key() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"known-but-absent");
        let device_key = key(fingerprint);
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        app.add_device_reporter(
            StaticDisplayReporter(device_key.clone()),
            ReporterRegistration::required(
                DiscoveryCadence::Periodic {
                    interval: Duration::ZERO,
                },
                ReporterCoverage::MatchingEvidenceOnly,
            ),
        );
        app.update();
        app.update();

        assert_eq!(
            MonitorDeviceAssociation::default().lookup(
                app.world().resource::<Devices>(),
                app.world().resource::<HardwareInventory>(),
                &device_key,
            ),
            MonitorDeviceLookup::KnownWithoutLiveMonitor
        );
    }

    #[test]
    fn configured_display_is_known_before_any_reporter_observes_it() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"configured-but-unobserved");
        let device_key = key(fingerprint);
        let mut inventory = HardwareInventory::default();
        inventory.configure(ConfiguredDevice {
            key:  device_key.clone(),
            mode: ConfiguredDeviceMode::Offline,
        });

        assert_eq!(
            MonitorDeviceAssociation::default().lookup(
                &Devices::default(),
                &inventory,
                &device_key,
            ),
            MonitorDeviceLookup::KnownWithoutLiveMonitor
        );
    }

    #[test]
    fn duplicate_live_key_never_selects_the_first_monitor() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"duplicate-display");
        let mut monitors = topology(Entity::from_bits(42), fingerprint);
        monitors
            .live
            .extend(topology(Entity::from_bits(84), fingerprint).live);
        let mut association = MonitorDeviceAssociation::default();
        association.refresh(&monitors);
        let device_key = key(fingerprint);
        let mut inventory = HardwareInventory::default();
        inventory.configure(ConfiguredDevice {
            key:  device_key.clone(),
            mode: ConfiguredDeviceMode::Managed,
        });

        assert_eq!(
            association.lookup(&Devices::default(), &inventory, &device_key,),
            MonitorDeviceLookup::KnownWithoutLiveMonitor
        );
    }

    #[test]
    fn topology_refresh_removes_an_old_live_association() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"departed-display");
        let mut association = MonitorDeviceAssociation::default();
        association.refresh(&topology(Entity::from_bits(42), fingerprint));
        association.refresh(&Monitors { live: Vec::new() });

        assert_eq!(
            association.lookup(
                &Devices::default(),
                &HardwareInventory::default(),
                &key(fingerprint),
            ),
            MonitorDeviceLookup::UnknownDevice
        );
    }

    #[test]
    fn never_retained_display_stays_unknown() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"never-reported");

        assert_eq!(
            MonitorDeviceAssociation::default().lookup(
                &Devices::default(),
                &HardwareInventory::default(),
                &key(fingerprint),
            ),
            MonitorDeviceLookup::UnknownDevice
        );
    }
}
