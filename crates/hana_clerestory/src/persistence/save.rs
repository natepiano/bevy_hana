//! Projection of kernel-owned window configurations into the RON adapter.

use std::collections::HashMap;
use std::collections::HashSet;
use std::env::current_exe;
use std::fs::create_dir_all;
use std::fs::rename;
use std::fs::write;
use std::path::Path;

use bevy::prelude::DetectChanges;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::warn;
use hana_rigging::LastKnownGoodConfigurationAccessError;
use hana_rigging::prelude::Binding;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::ConfiguredDeviceMode;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;

use super::EstablishedWindowPlacement;
use super::PendingWrite;
use super::PersistedWindowPlacement;
use super::PersistedWindowPlacements;
use super::constants::STATE_TEMPORARY_EXTENSION;
use super::format;
use super::window_state::LoadedBindingPolicy;
use crate::ManagedWindowPersistence;
use crate::managed::ManagedWindowRegistry;
use crate::restore_window_config::RestoreWindowConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StateFileWrite {
    Written,
    Failed,
}

/// Outcome of projecting one role-owned kernel binding into a writable v6 record.
///
/// Each refusal names why this role cannot write now. Only `AbsentBinding` means the role itself is
/// gone, so the batch writer drops its record under `ActiveOnly` and keeps it under `RememberAll`.
/// Every other refusal is a condition of this frame — an offline display, a capture that has not
/// happened yet, a driver that owns another erased configuration type — and never costs a role the
/// record it already has.
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
    /// The ready binding supplied a serializable v6 record.
    WritableSerializedValue(Box<PersistedWindowPlacement>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EstablishedRoleReadiness {
    Established,
    NotEstablished,
}

impl From<&RoleStatusView> for EstablishedRoleReadiness {
    fn from(role_status: &RoleStatusView) -> Self {
        if matches!(role_status, RoleStatusView::Established { .. }) {
            Self::Established
        } else {
            Self::NotEstablished
        }
    }
}

pub(super) fn save_all_states(
    path: &Path,
    states: &HashMap<RoleKey, PersistedWindowPlacement>,
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
    role_status: &RoleStatusView,
    inventory: &HardwareInventory,
    app_name: &str,
) -> EstablishedWindowPersistenceProjection {
    match project_established_values(
        &binding.endpoint,
        EstablishedRoleReadiness::from(role_status),
        binding.last_known_good(),
        inventory,
        app_name,
    ) {
        EstablishedWindowPersistenceProjection::WritableSerializedValue(mut placement) => {
            placement.loaded_binding_policy = LoadedBindingPolicy::Saved(binding.policy());
            EstablishedWindowPersistenceProjection::WritableSerializedValue(placement)
        },
        other => other,
    }
}

fn project_established_values(
    endpoint: &DeviceEndpoint,
    readiness: EstablishedRoleReadiness,
    last_known_good: Result<&dyn Reflect, LastKnownGoodConfigurationAccessError>,
    inventory: &HardwareInventory,
    app_name: &str,
) -> EstablishedWindowPersistenceProjection {
    if readiness == EstablishedRoleReadiness::NotEstablished {
        return EstablishedWindowPersistenceProjection::NonReadyRole;
    }
    if inventory
        .configured_device(&endpoint.device)
        .is_ok_and(|configured| configured.mode == ConfiguredDeviceMode::Offline)
    {
        return EstablishedWindowPersistenceProjection::OfflineInventory;
    }
    let Ok(configuration) = last_known_good else {
        return EstablishedWindowPersistenceProjection::MissingLastKnownGoodConfiguration;
    };
    let Some(placement) = configuration
        .as_any()
        .downcast_ref::<EstablishedWindowPlacement>()
    else {
        return EstablishedWindowPersistenceProjection::WrongErasedConfiguration;
    };
    EstablishedWindowPersistenceProjection::WritableSerializedValue(Box::new(
        PersistedWindowPlacement::from(placement.project(endpoint.device.clone(), app_name)),
    ))
}

fn project_established(
    role: &RoleKey,
    bindings: &Bindings,
    role_statuses: &Query<(&RoleKey, &RoleStatus)>,
    inventory: &HardwareInventory,
    app_name: &str,
) -> EstablishedWindowPersistenceProjection {
    let Ok(binding) = bindings.binding(role) else {
        return EstablishedWindowPersistenceProjection::AbsentBinding;
    };
    let Some(role_status) = role_statuses
        .iter()
        .find_map(|(candidate, status)| (candidate == role).then_some(status.view()))
    else {
        return EstablishedWindowPersistenceProjection::NonReadyRole;
    };
    project_established_binding(binding, role_status, inventory, app_name)
}

pub(super) fn write_established_window_configurations(
    config: Res<RestoreWindowConfig>,
    bindings: Res<Bindings>,
    inventory: Res<HardwareInventory>,
    role_statuses: Query<(&RoleKey, &RoleStatus)>,
    managed_windows: Res<ManagedWindowRegistry>,
    retention: Res<ManagedWindowPersistence>,
    mut persisted: ResMut<PersistedWindowPlacements>,
) {
    if !bindings.is_changed()
        && !inventory.is_changed()
        && !managed_windows.is_changed()
        && !retention.is_changed()
        && persisted.pending_write() == PendingWrite::Settled
    {
        return;
    }

    let mut projected = initial_serialized_states(&retention, &persisted, &managed_windows);
    let app_name = application_name();
    let primary_role = super::primary_window_role();
    if let Ok(primary_role) = primary_role {
        apply_projection(
            &mut projected,
            primary_role.clone(),
            project_established(
                &primary_role,
                &bindings,
                &role_statuses,
                &inventory,
                &app_name,
            ),
            &retention,
        );
    }
    for role in managed_windows.roles() {
        apply_projection(
            &mut projected,
            role.clone(),
            project_established(&role, &bindings, &role_statuses, &inventory, &app_name),
            &retention,
        );
    }

    if !write_is_required_after_projection(&persisted, &projected) {
        return;
    }
    if save_all_states(&config.path, &projected) == StateFileWrite::Written {
        persisted.replace_serialized(projected);
        persisted.mark_written();
    }
}

/// Whether a normal save must write the projected records.
///
/// `PersistedWindowPlacements` records a required write when live reporter evidence upgrades a
/// retained v4 target. That upgrade already changes the authoritative cache, so comparing the
/// projection with the cache alone would incorrectly suppress the v5 rewrite.
fn write_is_required_after_projection(
    persisted: &PersistedWindowPlacements,
    projected: &HashMap<RoleKey, PersistedWindowPlacement>,
) -> bool {
    persisted.pending_write() == PendingWrite::Owed
        || persisted.iter().count() != projected.len()
        || persisted
            .iter()
            .any(|(role, state)| projected.get(role) != Some(state))
}

/// Seed the pending write set with the retained records this save may still keep.
///
/// `RememberAll` starts from every retained record, so a role keeps its file entry after its window
/// closes. `ActiveOnly` starts from the retained records of currently managed roles only: a
/// managed window that has closed drops out of the file, while a window that is still open keeps
/// the record it was restored from even when it cannot project a fresh one this frame.
fn initial_serialized_states(
    retention: &ManagedWindowPersistence,
    persisted: &PersistedWindowPlacements,
    managed_windows: &ManagedWindowRegistry,
) -> HashMap<RoleKey, PersistedWindowPlacement> {
    match retention {
        ManagedWindowPersistence::RememberAll => persisted
            .iter()
            .map(|(role, state)| (role.clone(), state.clone()))
            .collect(),
        ManagedWindowPersistence::ActiveOnly => {
            let managed = managed_roles(managed_windows);
            persisted
                .iter()
                .filter(|(role, _)| managed.contains(*role))
                .map(|(role, state)| (role.clone(), state.clone()))
                .collect()
        },
    }
}

/// Roles whose windows are managed right now: the primary window plus every live named window.
///
/// The registry holds only the windows a `ManagedWindowName` gave a role to, so the primary window
/// — whose role is the `window:primary` constant — is added here.
fn managed_roles(managed_windows: &ManagedWindowRegistry) -> HashSet<RoleKey> {
    let mut roles: HashSet<RoleKey> = managed_windows.roles().into_iter().collect();
    if let Ok(primary_role) = super::primary_window_role() {
        roles.insert(primary_role);
    }
    roles
}

/// Fold one role's projection outcome into the pending write set.
///
/// A writable value always replaces whatever record was retained. `AbsentBinding` is the only
/// refusal meaning the role itself is gone, so `ActiveOnly` drops its record and `RememberAll`
/// keeps it. Every other refusal means the role cannot project during this frame, and its retained
/// record survives under both modes rather than being erased by a passing condition.
fn apply_projection(
    projected: &mut HashMap<RoleKey, PersistedWindowPlacement>,
    role: RoleKey,
    projection: EstablishedWindowPersistenceProjection,
    retention: &ManagedWindowPersistence,
) {
    match projection {
        EstablishedWindowPersistenceProjection::WritableSerializedValue(state) => {
            projected.insert(role, *state);
        },
        EstablishedWindowPersistenceProjection::AbsentBinding => {
            if matches!(retention, ManagedWindowPersistence::ActiveOnly) {
                projected.remove(&role);
            }
        },
        EstablishedWindowPersistenceProjection::NonReadyRole
        | EstablishedWindowPersistenceProjection::OfflineInventory
        | EstablishedWindowPersistenceProjection::MissingLastKnownGoodConfiguration
        | EstablishedWindowPersistenceProjection::WrongErasedConfiguration => {},
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
    use hana_rigging::LastKnownGoodConfigurationAccessError;
    use hana_rigging::prelude::Applied;
    use hana_rigging::prelude::ApplyContext;
    use hana_rigging::prelude::ApplyDeadline;
    use hana_rigging::prelude::AttachmentPath;
    use hana_rigging::prelude::AttemptInvalidation;
    use hana_rigging::prelude::AttemptRef;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::BindingAuthoring;
    use hana_rigging::prelude::BindingPolicy;
    use hana_rigging::prelude::Capabilities;
    use hana_rigging::prelude::Claim;
    use hana_rigging::prelude::ConfiguredDevice;
    use hana_rigging::prelude::ConfiguredDeviceName;
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
    use hana_rigging::prelude::DriverCleanupRoleEntity;
    use hana_rigging::prelude::DriverCompletion;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EndpointDriverRegistration;
    use hana_rigging::prelude::EndpointId;
    use hana_rigging::prelude::EstablishedContext;
    use hana_rigging::prelude::LastKnownGoodConfiguration;
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
    use hana_rigging::prelude::RetryOn;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RiggingSystems;
    use hana_rigging::prelude::RoleKey;
    use hana_rigging::prelude::RoleStatus;
    use hana_rigging::prelude::RoleStatusView;
    use hana_rigging::prelude::SchemeName;
    use hana_rigging::prelude::SessionRef;
    use hana_rigging::prelude::SessionReleaseCause;
    use hana_rigging::prelude::TargetResolution;
    use hana_rigging::prelude::TargetResolutionContext;
    #[cfg(test)]
    use hana_rigging::prelude::register_binding;
    use tempfile::NamedTempFile;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::ManagedWindowName;
    use crate::managed;
    use crate::persistence::EstablishedWindowPosition;
    use crate::persistence::PersistedDisplayIdentityV4;
    use crate::persistence::PersistedPosition;
    use crate::persistence::PersistedWindowState;
    use crate::persistence::PersistedWindowStateDecodeOutcome;
    use crate::persistence::PersistedWindowTargetV5;
    use crate::persistence::SavedWindowMode;

    struct TestWindowDriver;

    impl EndpointDriver for TestWindowDriver {
        type Configuration = EstablishedWindowPlacement;
        type Target = ();

        fn resolve_target(
            &mut self,
            _: &mut World,
            _: &TargetResolutionContext<'_>,
            _: &Self::Configuration,
        ) -> TargetResolution<Self::Target> {
            TargetResolution::Reached(())
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            context: ApplyContext<'_, Self::Configuration>,
            _: &Self::Configuration,
            (): Self::Target,
        ) {
            context
                .into_completion()
                .finish(DriverCompletion::Succeeded(Applied::AsDispatched));
        }

        fn established(&mut self, _: &mut World, _: EstablishedContext<'_, Self::Configuration>) {}

        fn cancel_apply(
            &mut self,
            _: &mut World,
            _: &RoleKey,
            _: DriverCleanupRoleEntity,
            _: AttemptRef,
            _: AttemptInvalidation,
        ) {
        }

        fn release_session(
            &mut self,
            _: &mut World,
            _: &RoleKey,
            _: DriverCleanupRoleEntity,
            _: SessionRef,
            _: SessionReleaseCause,
        ) {
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
                    std::time::Duration::from_secs(10),
                ),
            );
            register_binding(
                app.world_mut(),
                binding_for(primary_role.clone(), driver, primary_device.clone()),
            )
            .map_err(|error| format!("failed to register primary persistence binding: {error}"))?;
            register_binding(
                app.world_mut(),
                binding_for(managed_role.clone(), driver, managed_device),
            )
            .map_err(|error| format!("failed to register managed persistence binding: {error}"))?;
            if matches!(managed_membership, ManagedMembership::Present) {
                app.world_mut().spawn(ManagedWindowName("inspector".into()));
            }
            for _ in 0..8 {
                app.update();
            }
            {
                let primary_state = role_status_view(&app, &primary_role);
                let managed_state = role_status_view(&app, &managed_role);
                if !matches!(primary_state, Some(RoleStatusView::Established { .. }))
                    || !matches!(managed_state, Some(RoleStatusView::Established { .. }))
                {
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

        fn persisted_roles(&self) -> Result<HashMap<RoleKey, PersistedWindowPlacement>, String> {
            let contents = fs::read_to_string(&self.path)
                .map_err(|error| format!("failed to read persisted roles: {error}"))?;
            let mut registered_schemes = hana_rigging::prelude::RegisteredSchemes::default();
            registered_schemes.register(display_scheme()?);
            match super::super::load::decode_roles(&contents, &registered_schemes) {
                PersistedWindowStateDecodeOutcome::Decoded(states) => Ok(states),
                PersistedWindowStateDecodeOutcome::WholeFileRejected => {
                    Err(String::from("failed to decode persisted roles"))
                },
            }
        }
    }

    fn role_status_view<'app>(app: &'app App, role: &RoleKey) -> Option<&'app RoleStatusView> {
        let entity = app.world().resource::<Bindings>().role_entity(role).ok()?;
        app.world().get::<RoleStatus>(entity).map(RoleStatus::view)
    }

    fn serialized_state(app_name: &str) -> PersistedWindowState {
        PersistedWindowState {
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedDisplayIdentityV4::Anonymous,
            ),
            position:          PersistedPosition::Unpositioned,
            logical_width:     800,
            logical_height:    600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          app_name.into(),
        }
    }

    fn serialized_placement(app_name: &str) -> PersistedWindowPlacement {
        PersistedWindowPlacement::new(
            serialized_state(app_name),
            LoadedBindingPolicy::Saved(BindingPolicy::new(
                RecoveryPolicy::ReapplyOnReturn,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            )),
        )
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
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
            descriptor:             DeviceDescriptor::PlatformHasNoConcept,
        }
    }

    struct ProjectionBinding {
        endpoint:        DeviceEndpoint,
        readiness:       EstablishedRoleReadiness,
        requested:       EstablishedWindowPlacement,
        last_known_good: LastKnownGoodConfiguration,
    }

    impl ProjectionBinding {
        fn project(
            &self,
            inventory: &HardwareInventory,
            app_name: &str,
        ) -> EstablishedWindowPersistenceProjection {
            project_established_values(
                &self.endpoint,
                self.readiness,
                match &self.last_known_good {
                    LastKnownGoodConfiguration::NotEstablished => {
                        Err(LastKnownGoodConfigurationAccessError::NotEstablished)
                    },
                    LastKnownGoodConfiguration::MatchesRequested => Ok(&self.requested),
                    LastKnownGoodConfiguration::DiffersFromDispatched(configuration) => {
                        Ok(configuration.as_ref())
                    },
                },
                inventory,
                app_name,
            )
        }
    }

    fn binding(key: DeviceKey) -> ProjectionBinding {
        ProjectionBinding {
            endpoint:        DeviceEndpoint {
                device: key,
                id:     EndpointId::Whole,
            },
            readiness:       EstablishedRoleReadiness::Established,
            requested:       placement(),
            last_known_good: LastKnownGoodConfiguration::MatchesRequested,
        }
    }

    fn binding_for(
        role: RoleKey,
        driver: EndpointDriverRegistration<EstablishedWindowPlacement>,
        key: DeviceKey,
    ) -> BindingAuthoring<EstablishedWindowPlacement> {
        BindingAuthoring::new(
            role,
            DeviceEndpoint {
                device: key,
                id:     EndpointId::Whole,
            },
            driver,
            placement(),
            BindingPolicy::new(
                RecoveryPolicy::ReapplyOnReturn,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        )
    }

    #[test]
    fn role_key_roundtrip_preserves_primary_and_managed_namespaces() -> Result<(), String> {
        let primary = super::super::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let managed = super::super::managed_window_role("primary")
            .map_err(|error| format!("failed to create managed role: {error}"))?;
        let state = PersistedWindowState {
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedDisplayIdentityV4::Anonymous,
            ),
            position:          PersistedPosition::Unpositioned,
            logical_width:     800,
            logical_height:    600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          "test".to_string(),
        };
        let states = HashMap::from([
            (
                primary.clone(),
                PersistedWindowPlacement::new(
                    state.clone(),
                    LoadedBindingPolicy::Saved(BindingPolicy::new(
                        RecoveryPolicy::ReapplyOnReturn,
                        RetryOn::NewRevision,
                        OnAbort::default(),
                        OnSessionLoss::default(),
                        ApplyDeadline::ProcessDefault,
                    )),
                ),
            ),
            (
                managed.clone(),
                PersistedWindowPlacement::new(
                    state,
                    LoadedBindingPolicy::Saved(BindingPolicy::new(
                        RecoveryPolicy::ReapplyOnReturn,
                        RetryOn::NewRevision,
                        OnAbort::default(),
                        OnSessionLoss::default(),
                        ApplyDeadline::ProcessDefault,
                    )),
                ),
            ),
        ]);
        let file = NamedTempFile::new()
            .map_err(|error| format!("failed to create state file: {error}"))?;

        assert_eq!(
            save_all_states(file.path(), &states),
            StateFileWrite::Written
        );
        let loaded = super::super::load::load_all_states(
            file.path(),
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
        let mut binding = binding(key);
        let inventory = HardwareInventory::default();

        let projected = binding.project(&inventory, "clerestory");
        assert!(matches!(
            projected,
            EstablishedWindowPersistenceProjection::WritableSerializedValue(placement)
                if matches!(
                    placement.as_ref(),
                    PersistedWindowPlacement {
                        window_state: PersistedWindowState {
                            position: PersistedPosition::MonitorOffset(IVec2 { x: 10, y: 20 }),
                            ..
                        },
                        ..
                    }
                )
        ));

        binding.readiness = EstablishedRoleReadiness::NotEstablished;
        assert_eq!(
            binding.project(&inventory, "clerestory"),
            EstablishedWindowPersistenceProjection::NonReadyRole
        );
        Ok(())
    }

    #[test]
    fn offline_inventory_blocks_writes_without_changing_captured_configuration()
    -> Result<(), String> {
        let key = display_key()?;
        let binding = binding(key.clone());
        let mut inventory = HardwareInventory::default();
        inventory.configure(ConfiguredDevice {
            key,
            mode: ConfiguredDeviceMode::Offline,
            name: ConfiguredDeviceName::NeverDerived,
        });

        assert_eq!(
            binding.project(&inventory, "clerestory"),
            EstablishedWindowPersistenceProjection::OfflineInventory
        );
        assert!(matches!(
            binding.last_known_good,
            LastKnownGoodConfiguration::MatchesRequested
                | LastKnownGoodConfiguration::DiffersFromDispatched(_)
        ));
        Ok(())
    }

    #[test]
    fn not_established_does_not_make_a_ready_binding_writable() -> Result<(), String> {
        let key = display_key()?;
        let mut binding = binding(key);
        binding.last_known_good = LastKnownGoodConfiguration::NotEstablished;

        assert_eq!(
            binding.project(&HardwareInventory::default(), "clerestory"),
            EstablishedWindowPersistenceProjection::MissingLastKnownGoodConfiguration
        );
        assert_eq!(binding.readiness, EstablishedRoleReadiness::Established);
        Ok(())
    }

    #[test]
    fn only_an_absent_binding_costs_a_role_its_retained_record() -> Result<(), String> {
        let role = super::super::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
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
                persisted.seed(HashMap::from([(
                    role.clone(),
                    serialized_placement("retained"),
                )]));
                let mut projected = initial_serialized_states(
                    &retention,
                    &persisted,
                    &ManagedWindowRegistry::default(),
                );
                apply_projection(&mut projected, role.clone(), refusal.clone(), &retention);

                let role_is_gone = matches!(
                    refusal,
                    EstablishedWindowPersistenceProjection::AbsentBinding
                ) && matches!(retention, ManagedWindowPersistence::ActiveOnly);
                assert_eq!(
                    projected.contains_key(&role),
                    !role_is_gone,
                    "{refusal:?} did not follow {retention:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn writable_projection_replaces_the_retained_value() -> Result<(), String> {
        let role = super::super::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let mut projected = HashMap::from([(role.clone(), serialized_placement("retained"))]);

        apply_projection(
            &mut projected,
            role.clone(),
            EstablishedWindowPersistenceProjection::WritableSerializedValue(Box::new(
                serialized_placement("current"),
            )),
            &ManagedWindowPersistence::ActiveOnly,
        );

        assert_eq!(
            projected.get(&role).map(|state| state.app_name.as_str()),
            Some("current")
        );
        Ok(())
    }

    #[test]
    fn managed_role_filter_drops_a_closed_managed_window() -> Result<(), String> {
        let primary = super::super::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let managed = super::super::managed_window_role("closed")
            .map_err(|error| format!("failed to create managed role: {error}"))?;
        let mut persisted = PersistedWindowPlacements::default();
        persisted.seed(HashMap::from([
            (primary.clone(), serialized_placement("primary")),
            (managed.clone(), serialized_placement("managed")),
        ]));

        let projected = initial_serialized_states(
            &ManagedWindowPersistence::ActiveOnly,
            &persisted,
            &ManagedWindowRegistry::default(),
        );

        assert!(projected.contains_key(&primary));
        assert!(!projected.contains_key(&managed));
        Ok(())
    }

    #[test]
    fn resolved_pending_target_forces_its_next_normal_save() -> Result<(), String> {
        let role = super::super::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let state = PersistedWindowState {
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedDisplayIdentityV4::Anonymous,
            ),
            position:          PersistedPosition::Unpositioned,
            logical_width:     800,
            logical_height:    600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          String::from("retained"),
        };
        let mut persisted = PersistedWindowPlacements::default();
        persisted.seed(HashMap::from([(role, state)]));
        let projected = initial_serialized_states(
            &ManagedWindowPersistence::RememberAll,
            &persisted,
            &ManagedWindowRegistry::default(),
        );

        assert!(!write_is_required_after_projection(&persisted, &projected));
        persisted.pending_write = PendingWrite::Owed;
        assert!(write_is_required_after_projection(&persisted, &projected));
        Ok(())
    }

    #[test]
    fn an_offline_display_does_not_cost_a_role_its_saved_record() -> Result<(), String> {
        let mut harness = PersistenceHarness::new(ManagedMembership::Present)?;
        let initially_persisted = harness.persisted_roles()?;
        let primary_record = initially_persisted
            .get(&harness.primary_role)
            .ok_or_else(|| String::from("the primary role was never persisted"))?
            .clone();
        assert!(initially_persisted.contains_key(&harness.managed_role));

        harness
            .app
            .world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  harness.primary_device.clone(),
                mode: ConfiguredDeviceMode::Offline,
                name: ConfiguredDeviceName::NeverDerived,
            });
        harness.app.update();

        let offline_projection = harness.persisted_roles()?;
        assert_eq!(
            offline_projection.get(&harness.primary_role),
            Some(&primary_record)
        );
        assert!(offline_projection.contains_key(&harness.managed_role));

        harness
            .app
            .world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  harness.primary_device.clone(),
                mode: ConfiguredDeviceMode::Managed,
                name: ConfiguredDeviceName::NeverDerived,
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

        harness
            .app
            .world_mut()
            .spawn(ManagedWindowName("inspector".into()));
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
