//! Projection of kernel-owned window configurations into the RON adapter.

use std::collections::HashMap;
use std::env::current_exe;
use std::fs::create_dir_all;
use std::fs::rename;
use std::fs::write;
use std::path::Path;

use bevy::prelude::DetectChanges;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::warn;
use hana_rigging::prelude::Binding;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::ConfiguredDeviceMode;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::LastKnownGoodConfiguration;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleState;

use super::EstablishedWindowPlacement;
use super::PersistedWindowPlacements;
use super::PersistedWindowState;
use super::constants::STATE_TEMPORARY_EXTENSION;
use super::format;
use crate::ManagedWindowPersistence;
use crate::managed::ManagedWindowRegistry;
use crate::restore_window_config::RestoreWindowConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StateFileWrite {
    Written,
    Failed,
}

/// Outcome of projecting one role-owned kernel binding into a writable v5 record.
///
/// Each refusal names why this role cannot write now. The batch writer can then retain its prior
/// record under `RememberAll` or omit it under `ActiveOnly` without confusing an offline display
/// with a missing capture or a driver that owns another erased configuration type.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum EstablishedWindowPersistenceProjection {
    /// No kernel binding is registered for this persistence role.
    AbsentBinding,
    /// The kernel has not made this role ready for authoritative persistence.
    NonReadyRole,
    /// The endpoint's configured display is currently marked offline.
    OfflineInventory,
    /// The role has not yet captured a last-known-good driver configuration.
    MissingLastKnownGoodConfiguration,
    /// The role's erased configuration belongs to a driver other than Clerestory's window driver.
    WrongErasedConfiguration,
    /// The ready binding supplied a serializable v5 record.
    WritableSerializedValue(PersistedWindowState),
}

pub(super) fn save_all_states(
    path: &Path,
    states: &HashMap<RoleKey, PersistedWindowState>,
) -> StateFileWrite {
    if let Some(parent) = path.parent()
        && let Err(error) = create_dir_all(parent)
    {
        warn!("[save_all_states] Failed to create directory {parent:?}: {error}");
        return StateFileWrite::Failed;
    }
    let wire_states: HashMap<_, _> = states
        .iter()
        .filter_map(|(role, state)| {
            format::PersistedWindowRole::try_from(role)
                .ok()
                .map(|persisted_role| (persisted_role, state.clone()))
        })
        .collect();
    let contents = match format::encode(&wire_states) {
        Ok(contents) => contents,
        Err(error) => {
            warn!("[save_all_states] Failed to serialize state: {error}");
            return StateFileWrite::Failed;
        },
    };
    let temporary_path = path.with_extension(STATE_TEMPORARY_EXTENSION);
    if let Err(error) = write(&temporary_path, &contents) {
        warn!("[save_all_states] Failed to write state file {temporary_path:?}: {error}");
        return StateFileWrite::Failed;
    }
    if let Err(error) = rename(&temporary_path, path) {
        warn!("[save_all_states] Failed to replace state file {path:?}: {error}");
        return StateFileWrite::Failed;
    }
    StateFileWrite::Written
}

fn project_established_binding(
    binding: &Binding,
    inventory: &HardwareInventory,
    app_name: &str,
) -> EstablishedWindowPersistenceProjection {
    if binding.state != RoleState::Ready {
        return EstablishedWindowPersistenceProjection::NonReadyRole;
    }
    if inventory
        .configured_device(&binding.endpoint.device)
        .is_ok_and(|configured| configured.mode == ConfiguredDeviceMode::Offline)
    {
        return EstablishedWindowPersistenceProjection::OfflineInventory;
    }
    let LastKnownGoodConfiguration::Known(configuration) = &binding.last_known_good else {
        return EstablishedWindowPersistenceProjection::MissingLastKnownGoodConfiguration;
    };
    let Some(placement) = configuration
        .as_any()
        .downcast_ref::<EstablishedWindowPlacement>()
    else {
        return EstablishedWindowPersistenceProjection::WrongErasedConfiguration;
    };
    EstablishedWindowPersistenceProjection::WritableSerializedValue(
        placement.project(binding.endpoint.device.clone(), app_name),
    )
}

fn project_established(
    role: &RoleKey,
    bindings: &Bindings,
    inventory: &HardwareInventory,
    app_name: &str,
) -> EstablishedWindowPersistenceProjection {
    let Ok(binding) = bindings.binding(role) else {
        return EstablishedWindowPersistenceProjection::AbsentBinding;
    };
    project_established_binding(binding, inventory, app_name)
}

pub(super) fn write_established_window_configurations(
    config: Res<RestoreWindowConfig>,
    bindings: Res<Bindings>,
    inventory: Res<HardwareInventory>,
    managed_windows: Res<ManagedWindowRegistry>,
    retention: Res<ManagedWindowPersistence>,
    mut persisted: ResMut<PersistedWindowPlacements>,
) {
    if !bindings.is_changed()
        && !inventory.is_changed()
        && !managed_windows.is_changed()
        && !retention.is_changed()
        && !persisted.requires_save()
    {
        return;
    }

    let mut projected = initial_serialized_states(retention.clone(), &persisted);
    let app_name = application_name();
    let primary_role = super::primary_window_role();
    if let Ok(primary_role) = primary_role {
        apply_projection(
            &mut projected,
            primary_role.clone(),
            project_established(&primary_role, &bindings, &inventory, &app_name),
        );
    }
    for role in managed_windows.roles() {
        apply_projection(
            &mut projected,
            role.clone(),
            project_established(role, &bindings, &inventory, &app_name),
        );
    }

    if !write_is_required_after_projection(&persisted, &projected) {
        return;
    }
    if save_all_states(&config.path, &projected) == StateFileWrite::Written {
        persisted.replace_serialized(projected);
        persisted.mark_saved();
    }
}

/// Decide whether a normal save must write the projected records.
///
/// `PersistedWindowPlacements` records a required write when live reporter evidence upgrades a
/// retained v4 target. That upgrade already changes the authoritative cache, so comparing the
/// projection with the cache alone would incorrectly suppress the v5 rewrite.
fn write_is_required_after_projection(
    persisted: &PersistedWindowPlacements,
    projected: &HashMap<RoleKey, PersistedWindowState>,
) -> bool {
    persisted.requires_save()
        || persisted.iter().count() != projected.len()
        || persisted
            .iter()
            .any(|(role, state)| projected.get(role) != Some(state))
}

fn initial_serialized_states(
    retention: ManagedWindowPersistence,
    persisted: &PersistedWindowPlacements,
) -> HashMap<RoleKey, PersistedWindowState> {
    match retention {
        ManagedWindowPersistence::RememberAll => persisted
            .iter()
            .map(|(role, state)| (role.clone(), state.clone()))
            .collect(),
        ManagedWindowPersistence::ActiveOnly => HashMap::new(),
    }
}

fn apply_projection(
    projected: &mut HashMap<RoleKey, PersistedWindowState>,
    role: RoleKey,
    projection: EstablishedWindowPersistenceProjection,
) {
    if let EstablishedWindowPersistenceProjection::WritableSerializedValue(state) = projection {
        projected.insert(role, state);
    }
}

pub(super) fn application_name() -> String {
    current_exe()
        .ok()
        .and_then(|executable_path| {
            executable_path
                .file_stem()
                .and_then(|file_stem| file_stem.to_str())
                .map(String::from)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use bevy::MinimalPlugins;
    use bevy::prelude::App;
    use bevy::prelude::IVec2;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::UVec2;
    use bevy::prelude::Update;
    use bevy::prelude::World;
    use hana_rigging::prelude::ApplyDeadline;
    use hana_rigging::prelude::ApplyPermit;
    use hana_rigging::prelude::AttachmentPath;
    use hana_rigging::prelude::AttemptId;
    use hana_rigging::prelude::AttemptOutcome;
    use hana_rigging::prelude::AttemptProgress;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::Capabilities;
    use hana_rigging::prelude::CaptureOutcome;
    use hana_rigging::prelude::Claim;
    use hana_rigging::prelude::ConfiguredDevice;
    use hana_rigging::prelude::DeviceDescriptor;
    use hana_rigging::prelude::DeviceEndpoint;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::DeviceRecord;
    use hana_rigging::prelude::DeviceReporter;
    use hana_rigging::prelude::DeviceScan;
    use hana_rigging::prelude::DiscoveryCadence;
    use hana_rigging::prelude::DiscoveryWork;
    use hana_rigging::prelude::DriverId;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EndpointId;
    use hana_rigging::prelude::MainThreadDiscoveryJob;
    use hana_rigging::prelude::OnAbort;
    use hana_rigging::prelude::OnSessionLoss;
    use hana_rigging::prelude::PlatformDeviceHandle;
    use hana_rigging::prelude::Presence;
    use hana_rigging::prelude::RecoveryPolicy;
    use hana_rigging::prelude::ReportedAs;
    use hana_rigging::prelude::ReportedId;
    use hana_rigging::prelude::ReportedParent;
    use hana_rigging::prelude::ReportedSerial;
    use hana_rigging::prelude::ReporterCoverage;
    use hana_rigging::prelude::ReporterRegistration;
    use hana_rigging::prelude::RequestedConfiguration;
    use hana_rigging::prelude::RetryOn;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RiggingSystems;
    use hana_rigging::prelude::SchemeName;
    use tempfile::NamedTempFile;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::ManagedWindow;
    use crate::managed;
    use crate::persistence::EstablishedWindowPosition;
    use crate::persistence::PersistedPanelIdentityV4;
    use crate::persistence::PersistedPosition;
    use crate::persistence::PersistedWindowStateDecodeOutcome;
    use crate::persistence::PersistedWindowTargetV5;
    use crate::persistence::SavedWindowMode;

    struct TestWindowDriver;

    impl EndpointDriver for TestWindowDriver {
        type Configuration = EstablishedWindowPlacement;

        fn capture(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            CaptureOutcome::Read(placement())
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
            _: &Self::Configuration,
            _: AttemptId,
            _: ApplyPermit,
        ) {
        }

        fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
            AttemptProgress::Finished(AttemptOutcome::Succeeded)
        }
    }

    struct FixedDisplayReporter(Vec<DeviceKey>);

    impl DeviceReporter for FixedDisplayReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let device_keys = self.0.clone();
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| {
                DeviceScan::Complete(device_keys.iter().cloned().map(display_record).collect())
            }))
        }
    }

    enum ManagedMembership {
        Absent,
        Present,
    }

    struct PersistenceHarness {
        app:            App,
        directory:      TempDir,
        path:           PathBuf,
        primary_role:   RoleKey,
        managed_role:   RoleKey,
        primary_device: DeviceKey,
    }

    impl PersistenceHarness {
        fn new(managed_membership: ManagedMembership) -> Result<Self, String> {
            let directory = tempdir()
                .map_err(|error| format!("failed to create persistence directory: {error}"))?;
            let path = directory.path().join("windows.ron");
            let primary_role = super::super::primary_window_role()
                .map_err(|error| format!("failed to create primary role: {error}"))?;
            let managed_role = super::super::managed_window_role("inspector")
                .map_err(|error| format!("failed to create managed role: {error}"))?;
            let primary_device = reported_display_key("primary-display")?;
            let managed_device = reported_display_key("inspector-display")?;

            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_plugins(RiggingPlugin)
                .register_device_scheme(display_scheme()?)
                .insert_resource(RestoreWindowConfig { path: path.clone() })
                .insert_resource(ManagedWindowPersistence::ActiveOnly)
                .init_resource::<ManagedWindowRegistry>()
                .init_resource::<PersistedWindowPlacements>()
                .add_observer(managed::on_managed_window_added)
                .add_systems(
                    Update,
                    write_established_window_configurations.after(RiggingSystems::Apply),
                );
            let driver = app.add_endpoint_driver(TestWindowDriver);
            app.add_device_reporter(
                FixedDisplayReporter(vec![primary_device.clone(), managed_device.clone()]),
                ReporterRegistration::required(
                    DiscoveryCadence::Periodic {
                        interval: Duration::ZERO,
                    },
                    ReporterCoverage::MatchingEvidenceOnly,
                ),
            );
            {
                let mut bindings = app.world_mut().resource_mut::<Bindings>();
                bindings
                    .register(binding_for(
                        primary_role.clone(),
                        driver,
                        primary_device.clone(),
                    ))
                    .map_err(|error| {
                        format!("failed to register primary persistence binding: {error}")
                    })?;
                bindings
                    .register(binding_for(managed_role.clone(), driver, managed_device))
                    .map_err(|error| {
                        format!("failed to register managed persistence binding: {error}")
                    })?;
            }
            if matches!(managed_membership, ManagedMembership::Present) {
                app.world_mut().spawn(ManagedWindow {
                    name: "inspector".into(),
                });
            }
            for _ in 0..8 {
                app.update();
            }
            {
                let bindings = app.world().resource::<Bindings>();
                let primary_state = bindings
                    .binding(&primary_role)
                    .map_err(|error| format!("primary persistence binding disappeared: {error}"))?
                    .state;
                let managed_state = bindings
                    .binding(&managed_role)
                    .map_err(|error| format!("managed persistence binding disappeared: {error}"))?
                    .state;
                if primary_state != RoleState::Ready || managed_state != RoleState::Ready {
                    return Err(format!(
                        "persistence roles did not become ready: primary={primary_state:?}, managed={managed_state:?}"
                    ));
                }
            }

            Ok(Self {
                app,
                directory,
                path,
                primary_role,
                managed_role,
                primary_device,
            })
        }

        fn persisted_roles(&self) -> Result<HashMap<RoleKey, PersistedWindowState>, String> {
            let contents = fs::read_to_string(&self.path)
                .map_err(|error| format!("failed to read persisted roles: {error}"))?;
            let mut registered_schemes = hana_rigging::prelude::RegisteredSchemes::default();
            registered_schemes.register(display_scheme()?);
            match super::super::load::decode_roles(
                &contents,
                &crate::monitors::MonitorDeviceAssociation::default(),
                &registered_schemes,
            ) {
                PersistedWindowStateDecodeOutcome::Decoded(states) => Ok(states),
                PersistedWindowStateDecodeOutcome::WholeFileRejected => {
                    Err(String::from("failed to decode persisted roles"))
                },
            }
        }
    }

    fn placement() -> EstablishedWindowPlacement {
        EstablishedWindowPlacement {
            position:          EstablishedWindowPosition::Restorable {
                logical_offset: IVec2::new(10, 20),
            },
            logical_size:      UVec2::new(800, 600),
            saved_window_mode: SavedWindowMode::Windowed,
        }
    }

    fn display_key() -> Result<DeviceKey, String> {
        let value = AuthoredId::new("configured-display")
            .map_err(|error| format!("failed to create authored display ID: {error}"))?;
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored { value },
        })
    }

    fn display_scheme() -> Result<SchemeName, String> {
        SchemeName::new("clerestory-test")
            .map_err(|error| format!("failed to create display scheme: {error}"))
    }

    fn reported_display_key(value: &str) -> Result<DeviceKey, String> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: display_scheme()?,
                value:  ReportedId::new(value)
                    .map_err(|error| format!("failed to create reported display ID: {error}"))?,
            },
        })
    }

    fn display_record(device_key: DeviceKey) -> DeviceRecord {
        DeviceRecord {
            reported_as:            ReportedAs::Keyed(device_key),
            parent:                 ReportedParent::Root,
            presence:               Presence::Present,
            claim:                  Claim::NotApplicable,
            capabilities:           Capabilities::new(),
            serial:                 ReportedSerial::NotExposedByUnit,
            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
            attachment:             AttachmentPath::PlatformHasNoConcept,
            descriptor:             DeviceDescriptor::PlatformReportedNothing,
        }
    }

    fn binding(driver: DriverId, key: DeviceKey) -> Result<Binding, String> {
        let role = RoleKey::new("window:primary")
            .map_err(|error| format!("failed to create binding role: {error}"))?;
        Ok(binding_for(role, driver, key))
    }

    fn binding_for(role: RoleKey, driver: DriverId, key: DeviceKey) -> Binding {
        Binding {
            role,
            endpoint: DeviceEndpoint {
                device: key,
                id:     EndpointId::Whole,
            },
            driver,
            recovery: RecoveryPolicy::ReapplyOnReturn,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::Ready,
            requested: RequestedConfiguration::new(placement()),
            last_known_good: LastKnownGoodConfiguration::known(placement()),
            apply_deadline: ApplyDeadline::ProcessDefault,
        }
    }

    #[test]
    fn role_key_roundtrip_preserves_primary_and_managed_namespaces() -> Result<(), String> {
        let primary = super::super::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let managed = super::super::managed_window_role("primary")
            .map_err(|error| format!("failed to create managed role: {error}"))?;
        let state = PersistedWindowState {
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedPanelIdentityV4::Anonymous,
            ),
            position:          PersistedPosition::Unpositioned,
            logical_width:     800,
            logical_height:    600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          "test".to_string(),
        };
        let states = HashMap::from([(primary.clone(), state.clone()), (managed.clone(), state)]);
        let file = NamedTempFile::new()
            .map_err(|error| format!("failed to create state file: {error}"))?;

        assert_eq!(
            save_all_states(file.path(), &states),
            StateFileWrite::Written
        );
        let loaded = super::super::load::load_all_states(
            file.path(),
            &crate::monitors::MonitorDeviceAssociation::default(),
            &hana_rigging::prelude::RegisteredSchemes::default(),
        )
        .ok_or_else(|| String::from("failed to load saved role namespaces"))?;
        let PersistedWindowStateDecodeOutcome::Decoded(loaded) = loaded else {
            return Err(String::from(
                "saved role namespaces failed whole-file decode",
            ));
        };
        assert!(loaded.contains_key(&primary));
        assert!(loaded.contains_key(&managed));
        Ok(())
    }

    #[test]
    fn established_configuration_projects_only_from_a_ready_kernel_binding() -> Result<(), String> {
        let key = display_key()?;
        let mut app = App::new();
        let driver = app.add_endpoint_driver(TestWindowDriver);
        let mut binding = binding(driver, key)?;
        let inventory = HardwareInventory::default();

        let projected = project_established_binding(&binding, &inventory, "clerestory");
        assert!(matches!(
            projected,
            EstablishedWindowPersistenceProjection::WritableSerializedValue(PersistedWindowState {
                position: PersistedPosition::MonitorOffset(IVec2 { x: 10, y: 20 }),
                ..
            })
        ));

        binding.state = RoleState::Waiting;
        assert_eq!(
            project_established_binding(&binding, &inventory, "clerestory"),
            EstablishedWindowPersistenceProjection::NonReadyRole
        );
        Ok(())
    }

    #[test]
    fn offline_inventory_blocks_writes_without_changing_captured_configuration()
    -> Result<(), String> {
        let key = display_key()?;
        let mut app = App::new();
        let driver = app.add_endpoint_driver(TestWindowDriver);
        let binding = binding(driver, key.clone())?;
        let mut inventory = HardwareInventory::default();
        inventory.configure(ConfiguredDevice {
            key,
            mode: ConfiguredDeviceMode::Offline,
        });

        assert_eq!(
            project_established_binding(&binding, &inventory, "clerestory"),
            EstablishedWindowPersistenceProjection::OfflineInventory
        );
        assert!(matches!(
            binding.last_known_good,
            LastKnownGoodConfiguration::Known(_)
        ));
        Ok(())
    }

    #[test]
    fn not_established_does_not_make_a_ready_binding_writable() -> Result<(), String> {
        let key = display_key()?;
        let mut app = App::new();
        let driver = app.add_endpoint_driver(TestWindowDriver);
        let mut binding = binding(driver, key)?;
        binding.last_known_good = LastKnownGoodConfiguration::NotEstablished;

        assert_eq!(
            project_established_binding(&binding, &HardwareInventory::default(), "clerestory"),
            EstablishedWindowPersistenceProjection::MissingLastKnownGoodConfiguration
        );
        assert_eq!(binding.state, RoleState::Ready);
        Ok(())
    }

    #[test]
    fn every_projection_refusal_obeys_both_retention_modes() -> Result<(), String> {
        let role = super::super::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let retained = PersistedWindowState {
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedPanelIdentityV4::Anonymous,
            ),
            position:          PersistedPosition::Unpositioned,
            logical_width:     800,
            logical_height:    600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          String::from("retained"),
        };
        let refusals = [
            EstablishedWindowPersistenceProjection::AbsentBinding,
            EstablishedWindowPersistenceProjection::NonReadyRole,
            EstablishedWindowPersistenceProjection::OfflineInventory,
            EstablishedWindowPersistenceProjection::MissingLastKnownGoodConfiguration,
            EstablishedWindowPersistenceProjection::WrongErasedConfiguration,
        ];
        for retention in [
            ManagedWindowPersistence::RememberAll,
            ManagedWindowPersistence::ActiveOnly,
        ] {
            for refusal in &refusals {
                let mut persisted = PersistedWindowPlacements::default();
                persisted.seed(HashMap::from([(role.clone(), retained.clone())]));
                let mut projected = initial_serialized_states(retention.clone(), &persisted);
                apply_projection(&mut projected, role.clone(), refusal.clone());
                let expected = matches!(retention, ManagedWindowPersistence::RememberAll);
                assert_eq!(
                    projected.contains_key(&role),
                    expected,
                    "{refusal:?} did not follow {retention:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn resolved_pending_target_forces_its_next_normal_save() -> Result<(), String> {
        let role = super::super::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let state = PersistedWindowState {
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedPanelIdentityV4::Anonymous,
            ),
            position:          PersistedPosition::Unpositioned,
            logical_width:     800,
            logical_height:    600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          String::from("retained"),
        };
        let mut persisted = PersistedWindowPlacements::default();
        persisted.seed(HashMap::from([(role, state)]));
        let projected =
            initial_serialized_states(ManagedWindowPersistence::RememberAll, &persisted);

        assert!(!write_is_required_after_projection(&persisted, &projected));
        persisted.requires_save = true;
        assert!(write_is_required_after_projection(&persisted, &projected));
        Ok(())
    }

    #[test]
    fn inventory_mode_changes_reproject_each_role_independently() -> Result<(), String> {
        let mut harness = PersistenceHarness::new(ManagedMembership::Present)?;
        let initially_persisted = harness.persisted_roles()?;
        assert!(initially_persisted.contains_key(&harness.primary_role));
        assert!(initially_persisted.contains_key(&harness.managed_role));

        harness
            .app
            .world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  harness.primary_device.clone(),
                mode: ConfiguredDeviceMode::Offline,
            });
        harness.app.update();

        let offline_projection = harness.persisted_roles()?;
        assert!(!offline_projection.contains_key(&harness.primary_role));
        assert!(offline_projection.contains_key(&harness.managed_role));

        harness
            .app
            .world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  harness.primary_device.clone(),
                mode: ConfiguredDeviceMode::Managed,
            });
        harness.app.update();

        let managed_projection = harness.persisted_roles()?;
        assert!(managed_projection.contains_key(&harness.primary_role));
        assert!(managed_projection.contains_key(&harness.managed_role));
        Ok(())
    }

    #[test]
    fn managed_membership_change_reprojects_the_role_set() -> Result<(), String> {
        let mut harness = PersistenceHarness::new(ManagedMembership::Absent)?;
        let initial_projection = harness.persisted_roles()?;
        assert!(initial_projection.contains_key(&harness.primary_role));
        assert!(!initial_projection.contains_key(&harness.managed_role));

        harness.app.world_mut().spawn(ManagedWindow {
            name: "inspector".into(),
        });
        harness.app.update();

        let changed_projection = harness.persisted_roles()?;
        assert!(changed_projection.contains_key(&harness.primary_role));
        assert!(changed_projection.contains_key(&harness.managed_role));
        Ok(())
    }

    #[test]
    fn unchanged_update_does_not_attempt_a_file_replacement() -> Result<(), String> {
        let mut harness = PersistenceHarness::new(ManagedMembership::Present)?;
        assert!(harness.path.is_file());
        fs::remove_file(&harness.path)
            .map_err(|error| format!("failed to replace state file with directory: {error}"))?;
        fs::create_dir(&harness.path)
            .map_err(|error| format!("failed to create replacement state directory: {error}"))?;
        let temporary_path = harness.path.with_extension(STATE_TEMPORARY_EXTENSION);
        assert!(!temporary_path.exists());

        harness.app.update();

        assert!(!temporary_path.exists());
        assert!(harness.path.is_dir());
        assert!(harness.directory.path().is_dir());
        Ok(())
    }
}
