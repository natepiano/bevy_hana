//! Managed window registration and startup preparation.

use std::collections::HashMap;
use std::collections::HashSet;

use bevy::ecs::query::QueryData;
use bevy::ecs::query::QueryEntityError;
use bevy::prelude::Add;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Has;
use bevy::prelude::IVec2;
use bevy::prelude::Insert;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectResource;
use bevy::prelude::Remove;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::WindowPosition;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::World;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy::window::OnMonitor;
use bevy::window::PrimaryWindow;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::BindingAuthoring;
use hana_rigging::prelude::BindingPolicy;
use hana_rigging::prelude::BindingRegistration;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::EndpointDriverRegistration;
use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::OutputConfirmationSource;
use hana_rigging::prelude::PartName;
use hana_rigging::prelude::PartNameError;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::ResolvedToDevice;
use hana_rigging::prelude::RetireRole;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleKeyError;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;
use hana_rigging::prelude::WaitingStatusView;
#[cfg(test)]
use hana_rigging::prelude::register_binding;

use crate::WindowRevealDisposition;
use crate::constants::FIRST_DUPLICATE_SUFFIX;
use crate::constants::MANAGED_WINDOW_NAME_SEPARATOR;
use crate::constants::PRIMARY_WINDOW_KEY;
use crate::driver::WindowDriverId;
use crate::driver::WindowRoleDriverState;
use crate::monitors;
use crate::monitors::CurrentMonitor;
use crate::monitors::CurrentMonitorEntity;
use crate::monitors::LiveDisplayDevices;
use crate::monitors::LiveDisplayEndpointLookup;
use crate::monitors::LiveDisplayMatchError;
use crate::monitors::Monitors;
use crate::output_proof::OnScreenConfirmation;
use crate::persistence;
use crate::persistence::EstablishedWindowPlacement;
use crate::persistence::LoadedBindingPolicy;
use crate::persistence::PersistedDisplayIdentityV4;
use crate::persistence::PersistedWindowPlacementLookup;
use crate::persistence::PersistedWindowPlacements;
use crate::persistence::PersistedWindowTargetV5;
use crate::platform::Platform;
use crate::recovery::StrandedWindowDisplayObservation;
use crate::recovery::StrandedWindowMovementBaselines;
use crate::recovery::StrandedWindowObservation;
use crate::restore::WindowRestoreAttempt;
use crate::visibility::PlacementAbandoned;
use crate::visibility::SavedDisplayRevealWait;

/// Marks a window whose placement Clerestory restores, follows, and persists.
///
/// Every managed window carries it, the primary window included; the plugin inserts it there.
/// A secondary window carries a [`ManagedWindowName`] beside it, which is what gives that window a
/// persistence role of its own.
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct ManagedWindow;

/// The app-chosen name that gives a secondary window its durable persistence role.
///
/// The name derives a stable, namespaced `RoleKey` and the RON adapter key. Adding it puts the
/// window under Clerestory: [`ManagedWindow`] arrives with it, and a name already held by another
/// window is rewritten here with a numeric suffix before the role is derived.
///
/// The primary window never carries this. Its role is `window:primary`, and naming the primary
/// logs one warning and derives nothing.
#[derive(Component, Clone, Reflect)]
#[reflect(Component)]
#[require(ManagedWindow)]
pub struct ManagedWindowName(pub String);

/// Signals that `ManagedWindowRegistry` accepted and registered one managed window lifetime.
#[derive(EntityEvent)]
pub(crate) struct ManagedWindowRegistered {
    entity: Entity,
}

/// Move the window back to its saved display on its own, once that display returns.
///
/// This is authoring-time configuration. Insert it before Clerestory authors the window's binding;
/// later changes do not alter that binding's recovery policy.
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct RecoverOnReturn;

/// Move the window back to its saved display only when application code requests the return.
///
/// This is authoring-time configuration. Insert it before Clerestory authors the window's binding;
/// later changes do not alter that binding's recovery policy.
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct RecoverOnRequest;

#[derive(Component)]
pub(crate) struct RecoveryMarkerConflictWarned;

/// Counts entries into `warn_conflicting_recovery_markers`' warning path in tests.
#[cfg(test)]
#[derive(Default, Resource)]
pub(crate) struct RecoveryMarkerWarningPathExecutions(usize);

#[derive(QueryData)]
pub(crate) struct RecoveryMarkers {
    recover_on_return:  Has<RecoverOnReturn>,
    recover_on_request: Has<RecoverOnRequest>,
}

const fn recovery_policy(markers: RecoveryMarkersItem<'_, '_>) -> RecoveryPolicy {
    match (markers.recover_on_return, markers.recover_on_request) {
        (false, false) => RecoveryPolicy::Forget,
        (true, false) => RecoveryPolicy::ReapplyOnReturn,
        (_, true) => RecoveryPolicy::ReapplyOnRequest,
    }
}

pub(crate) fn warn_conflicting_recovery_markers(
    added: On<Add, (RecoverOnReturn, RecoverOnRequest)>,
    managed: Query<(), With<ManagedWindow>>,
    conflicts: Query<(), (With<RecoverOnReturn>, With<RecoverOnRequest>)>,
    warned: Query<(), With<RecoveryMarkerConflictWarned>>,
    mut commands: Commands,
    #[cfg(test)] mut warning_paths: Option<ResMut<RecoveryMarkerWarningPathExecutions>>,
) {
    let entity = added.entity;
    if warned.contains(entity) {
        return;
    }
    if !conflicts.contains(entity) && managed.contains(entity) {
        return;
    }
    #[cfg(test)]
    if let Some(warning_paths) = &mut warning_paths {
        warning_paths.0 += 1;
    }
    if conflicts.contains(entity) {
        warn!(
            "[warn_conflicting_recovery_markers] entity {entity} carries both RecoverOnReturn and RecoverOnRequest; treating it as RecoverOnRequest"
        );
    } else {
        warn!(
            "[warn_conflicting_recovery_markers] entity {entity} has a recovery marker but is not a ManagedWindow"
        );
    }
    commands.entity(entity).insert(RecoveryMarkerConflictWarned);
}

/// Controls whether serialized records survive after managed windows close.
#[derive(Resource, Default, Clone, Debug, PartialEq, Eq, Reflect)]
#[reflect(Resource)]
pub enum ManagedWindowPersistence {
    /// Keep the latest serialized record even while no window entity exists.
    #[default]
    RememberAll,
    /// Write only roles with active window bindings.
    ActiveOnly,
}

/// Names currently assigned to managed window entities.
#[derive(Resource, Default)]
pub(crate) struct ManagedWindowRegistry {
    names:    HashSet<String>,
    entities: HashMap<Entity, RegisteredManagedWindow>,
}

struct RegisteredManagedWindow {
    name: String,
}

/// The registry's record for one window entity.
#[derive(Debug, PartialEq, Eq)]
enum ManagedWindowRegistration<'a> {
    /// Clerestory registered this entity under this validated name.
    Registered { name: &'a str },
    /// The entity is not a managed window.
    NotAManagedWindow,
}

/// Which entity currently holds one role in managed-window tests.
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
enum RoleOccupancy {
    Held(Entity),
    Unoccupied,
}

/// What releasing a window entity gave back.
#[derive(Debug, PartialEq, Eq)]
enum ManagedWindowRelease {
    /// The registration was removed; this role is now free.
    Released(RoleKey),
    /// The retained name no longer forms the role validated at registration.
    InvalidRegisteredName { name: String, error: RoleKeyError },
    /// Nothing was registered for that entity.
    NotAManagedWindow,
}

impl ManagedWindowRegistry {
    #[must_use]
    fn registration(&self, entity: Entity) -> ManagedWindowRegistration<'_> {
        self.entities.get(&entity).map_or(
            ManagedWindowRegistration::NotAManagedWindow,
            |registration| ManagedWindowRegistration::Registered {
                name: registration.name.as_str(),
            },
        )
    }

    pub(crate) fn roles(&self) -> Vec<RoleKey> {
        let mut roles = Vec::with_capacity(self.entities.len());
        for registration in self.entities.values() {
            match persistence::managed_window_role(&registration.name) {
                Ok(role) => roles.push(role),
                Err(error) => error!(
                    "[ManagedWindowRegistry::roles] registered name {:?} no longer forms its role: {error}",
                    registration.name
                ),
            }
        }
        roles
    }

    #[cfg(test)]
    #[must_use]
    fn entity_for_role(&self, role: &RoleKey) -> RoleOccupancy {
        self.entities
            .iter()
            .find_map(|(entity, registration)| {
                persistence::managed_window_role(&registration.name)
                    .is_ok_and(|registered_role| registered_role == *role)
                    .then_some(*entity)
            })
            .map_or(RoleOccupancy::Unoccupied, RoleOccupancy::Held)
    }

    fn register(&mut self, entity: Entity, name: String) {
        self.names.insert(name.clone());
        self.entities
            .insert(entity, RegisteredManagedWindow { name });
    }

    fn release(&mut self, entity: Entity) -> ManagedWindowRelease {
        let Some(registration) = self.entities.remove(&entity) else {
            return ManagedWindowRelease::NotAManagedWindow;
        };
        self.names.remove(&registration.name);
        match persistence::managed_window_role(&registration.name) {
            Ok(role) => ManagedWindowRelease::Released(role),
            Err(error) => ManagedWindowRelease::InvalidRegisteredName {
                name: registration.name,
                error,
            },
        }
    }
}

/// Records whether Clerestory registered or rejected the one binding for a window entity.
#[derive(Component)]
pub(crate) enum WindowBindingAuthoring {
    /// The entity's stable role now owns its display endpoint in `Bindings`.
    Registered,
    /// The role was invalid or checked registration rejected its endpoint; the error was reported.
    Rejected,
}

/// Window entity's relationship to its role entity in the rigging kernel.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[relationship(relationship_target = WindowsWithRiggingRole)]
#[reflect(Component, PartialEq)]
#[require(
    OutputConfirmationSource<OnScreenConfirmation> =
        OutputConfirmationSource::new(OnScreenConfirmation)
)]
pub(crate) struct WindowRiggingRole(Entity);

impl WindowRiggingRole {
    pub(crate) const fn new(role_entity: Entity) -> Self { Self(role_entity) }

    pub(crate) const fn entity(self) -> Entity { self.0 }
}

/// Window entities whose local relationship names this rigging role entity.
#[derive(Component, Debug, Reflect)]
#[relationship_target(relationship = WindowRiggingRole)]
#[reflect(Component)]
pub(crate) struct WindowsWithRiggingRole(Vec<Entity>);

/// Marks a current-monitor change that still needs an exact live-device lookup.
#[derive(Component)]
pub(crate) struct WindowDisplayRebindPending;

/// Defer display rebind work until reporter capabilities have been projected.
pub(crate) fn queue_window_display_rebind(
    current_monitor_added: On<Insert, CurrentMonitor>,
    mut commands: Commands,
) {
    commands
        .entity(current_monitor_added.entity)
        .insert(WindowDisplayRebindPending);
}

fn window_endpoint_id(role: &RoleKey) -> Result<EndpointId, PartNameError> {
    PartName::new(role.as_str()).map(EndpointId::Part)
}

enum WindowBindingRoleResolution {
    Available(RoleKey),
    AwaitingManagedRegistration,
    InvalidPrimaryRole,
}

enum WindowBindingAuthoringResolution {
    AwaitingManagedRegistration,
    /// The role has no exact reporter-issued display endpoint yet.
    AwaitingDisplayEndpoint,
    /// The registered role entity is being recovered after external removal.
    AwaitingRoleEntity,
    Registered {
        role_entity: Entity,
    },
    Rejected,
}

#[derive(Clone, Copy)]
enum WindowBindingRoleSource {
    Primary,
    Managed(Entity),
}

fn resolve_window_binding_role(
    source: WindowBindingRoleSource,
    registry: &ManagedWindowRegistry,
) -> WindowBindingRoleResolution {
    match source {
        WindowBindingRoleSource::Managed(entity) => match registry.registration(entity) {
            ManagedWindowRegistration::Registered { name } => {
                persistence::managed_window_role(name).map_or(
                    WindowBindingRoleResolution::InvalidPrimaryRole,
                    WindowBindingRoleResolution::Available,
                )
            },
            ManagedWindowRegistration::NotAManagedWindow => {
                WindowBindingRoleResolution::AwaitingManagedRegistration
            },
        },
        WindowBindingRoleSource::Primary => match persistence::primary_window_role() {
            Ok(role) => WindowBindingRoleResolution::Available(role),
            Err(error) => {
                error!("[author_window_bindings] primary window role is invalid: {error}");
                WindowBindingRoleResolution::InvalidPrimaryRole
            },
        },
    }
}

struct WindowBindingCandidate<'a> {
    window:          &'a Window,
    current_monitor: &'a CurrentMonitor,
    monitor_entity:  CurrentMonitorEntity,
    role_source:     WindowBindingRoleSource,
    recovery:        RecoveryPolicy,
}

struct WindowBindingAuthoringContext<'a, 'world, 'state, 'live_display_data, 'device_key_data> {
    registry:             &'a ManagedWindowRegistry,
    persisted:            &'a mut PersistedWindowPlacements,
    live_displays:        &'a LiveDisplayEndpointLookup<'world, 'state>,
    live_display_devices: &'a Query<'world, 'state, &'live_display_data LiveDisplayDevices>,
    device_keys:          &'a Query<'world, 'state, &'device_key_data DeviceKey>,
    platform:             Platform,
    driver:               &'a WindowDriverId,
    stranded:             &'a mut StrandedWindowMovementBaselines,
}

impl WindowBindingAuthoringContext<'_, '_, '_, '_, '_> {
    fn device_for_current_monitor(
        &self,
        current_monitor_entity: CurrentMonitorEntity,
    ) -> Result<DeviceKey, LiveDisplayMatchError> {
        let live_display_devices = self
            .live_display_devices
            .get(current_monitor_entity.entity())
            .map_err(|_| LiveDisplayMatchError::NoLiveDisplayMatches)?;
        let device_entity = live_display_devices.device()?;
        self.device_keys
            .get(device_entity)
            .cloned()
            .map_err(|_| LiveDisplayMatchError::NoLiveDisplayMatches)
    }
}

enum WindowBindingDeviceEvidence {
    Classified(DeviceKey),
    CurrentDisplay,
    LegacyIdentity(PersistedDisplayIdentityV4),
}

enum WindowBindingPolicySaveAction {
    NoChange,
    WriteExplicitPolicy,
}

struct PreparedWindowBindingPersistence {
    placement:          EstablishedWindowPlacement,
    device_evidence:    WindowBindingDeviceEvidence,
    binding_policy:     BindingPolicy,
    policy_save_action: WindowBindingPolicySaveAction,
}

enum StrandedWindowRoleObservation {
    StatusUnavailable,
    WaitingWithoutResolvedDevice,
    WaitingWithResolvedDevice,
    ActiveWithoutResolvedDevice,
    ActiveWithResolvedDevice,
}

const fn stranded_window_role_observation(
    observation: Result<(&RoleStatus, Option<&ResolvedToDevice>), QueryEntityError>,
) -> StrandedWindowRoleObservation {
    let Ok((status, resolved_to_device)) = observation else {
        return StrandedWindowRoleObservation::StatusUnavailable;
    };
    match (
        matches!(status.view(), RoleStatusView::Waiting(_)),
        resolved_to_device.is_some(),
    ) {
        (true, false) => StrandedWindowRoleObservation::WaitingWithoutResolvedDevice,
        (true, true) => StrandedWindowRoleObservation::WaitingWithResolvedDevice,
        (false, false) => StrandedWindowRoleObservation::ActiveWithoutResolvedDevice,
        (false, true) => StrandedWindowRoleObservation::ActiveWithResolvedDevice,
    }
}

fn resolve_window_binding_device(
    evidence: WindowBindingDeviceEvidence,
    current_monitor_entity: CurrentMonitorEntity,
    context: &WindowBindingAuthoringContext<'_, '_, '_, '_, '_>,
) -> Result<DeviceKey, LiveDisplayMatchError> {
    match evidence {
        WindowBindingDeviceEvidence::Classified(device_key) => Ok(device_key),
        WindowBindingDeviceEvidence::CurrentDisplay => {
            context.device_for_current_monitor(current_monitor_entity)
        },
        WindowBindingDeviceEvidence::LegacyIdentity(legacy_identity) => context
            .live_displays
            .key_for_legacy_identity(legacy_identity.into()),
    }
}

fn prepare_window_binding_persistence(
    candidate: &WindowBindingCandidate<'_>,
    persisted: PersistedWindowPlacementLookup<'_>,
    platform: Platform,
) -> PreparedWindowBindingPersistence {
    match persisted {
        PersistedWindowPlacementLookup::Saved(saved) => {
            let device_evidence = match &saved.window_state.target {
                PersistedWindowTargetV5::Classified(device_key) => {
                    WindowBindingDeviceEvidence::Classified(device_key.clone())
                },
                PersistedWindowTargetV5::AwaitingLegacyEvidence(
                    PersistedDisplayIdentityV4::Anonymous,
                ) => WindowBindingDeviceEvidence::CurrentDisplay,
                PersistedWindowTargetV5::AwaitingLegacyEvidence(legacy_identity) => {
                    WindowBindingDeviceEvidence::LegacyIdentity(*legacy_identity)
                },
            };
            let (binding_policy, policy_save_action) = match saved.loaded_binding_policy {
                LoadedBindingPolicy::Saved(binding_policy) => {
                    (binding_policy, WindowBindingPolicySaveAction::NoChange)
                },
                LoadedBindingPolicy::LegacyWindowMarkers => (
                    default_window_binding_policy(candidate.recovery),
                    WindowBindingPolicySaveAction::WriteExplicitPolicy,
                ),
            };
            PreparedWindowBindingPersistence {
                placement: EstablishedWindowPlacement::from(&saved.window_state),
                device_evidence,
                binding_policy,
                policy_save_action,
            }
        },
        PersistedWindowPlacementLookup::NotSaved => {
            let physical_position = match candidate.window.position {
                WindowPosition::At(position) => Some(IVec2::new(position.x, position.y)),
                _ => None,
            };
            PreparedWindowBindingPersistence {
                placement:          EstablishedWindowPlacement::from_readback(
                    candidate.window,
                    candidate.current_monitor,
                    physical_position,
                    platform,
                ),
                device_evidence:    WindowBindingDeviceEvidence::CurrentDisplay,
                binding_policy:     default_window_binding_policy(candidate.recovery),
                policy_save_action: WindowBindingPolicySaveAction::NoChange,
            }
        },
    }
}

const fn default_window_binding_policy(recovery: RecoveryPolicy) -> BindingPolicy {
    BindingPolicy::new(
        recovery,
        RetryOn::NewRevision,
        OnAbort::LeaveAsIs,
        OnSessionLoss::Recreate,
        ApplyDeadline::ProcessDefault,
    )
}

fn author_window_binding(
    candidate: WindowBindingCandidate<'_>,
    context: &mut WindowBindingAuthoringContext<'_, '_, '_, '_, '_>,
    role_entities: &Query<'_, '_, (Entity, &RoleKey)>,
    registration: &mut BindingRegistration<'_, '_>,
) -> WindowBindingAuthoringResolution {
    let role = match resolve_window_binding_role(candidate.role_source, context.registry) {
        WindowBindingRoleResolution::Available(role) => role,
        WindowBindingRoleResolution::AwaitingManagedRegistration => {
            return WindowBindingAuthoringResolution::AwaitingManagedRegistration;
        },
        WindowBindingRoleResolution::InvalidPrimaryRole => {
            return WindowBindingAuthoringResolution::Rejected;
        },
    };
    if let Ok(binding) = registration.binding(&role) {
        let endpoint = binding.endpoint.clone();
        let reattaches_to_stranded_endpoint = matches!(
            binding.recovery,
            RecoveryPolicy::ReapplyOnRequest | RecoveryPolicy::ReapplyOnReturn
        ) && matches!(
            context
                .device_for_current_monitor(candidate.monitor_entity),
            Ok(device) if device != endpoint.device
        );
        if reattaches_to_stranded_endpoint {
            context.stranded.begin(role.clone());
        }
        let Some(role_entity) = role_entities
            .iter()
            .find_map(|(entity, candidate)| (candidate == &role).then_some(entity))
        else {
            return WindowBindingAuthoringResolution::AwaitingRoleEntity;
        };
        return WindowBindingAuthoringResolution::Registered { role_entity };
    }

    let prepared = prepare_window_binding_persistence(
        &candidate,
        context.persisted.get(&role),
        context.platform,
    );
    if matches!(
        prepared.policy_save_action,
        WindowBindingPolicySaveAction::WriteExplicitPolicy
    ) {
        context
            .persisted
            .resolve_loaded_policy(&role, prepared.binding_policy);
    }
    let Ok(device) =
        resolve_window_binding_device(prepared.device_evidence, candidate.monitor_entity, context)
    else {
        return WindowBindingAuthoringResolution::AwaitingDisplayEndpoint;
    };
    let endpoint_id = match window_endpoint_id(&role) {
        Ok(endpoint_id) => endpoint_id,
        Err(error) => {
            error!("[author_window_bindings] role {role} cannot name its window endpoint: {error}");
            return WindowBindingAuthoringResolution::Rejected;
        },
    };
    let endpoint = DeviceEndpoint {
        device,
        id: endpoint_id,
    };
    let binding_policy = prepared.binding_policy;
    let binding = window_binding(
        role.clone(),
        endpoint,
        context.driver.0,
        prepared.placement,
        binding_policy,
    );
    match registration.register(binding) {
        Ok(role_entity) => WindowBindingAuthoringResolution::Registered { role_entity },
        Err(error) => {
            error!(
                "[author_window_bindings] rejected role {role} as a Clerestory configuration error: {error}"
            );
            WindowBindingAuthoringResolution::Rejected
        },
    }
}

/// Register every available Clerestory window through the kernel's checked binding registry.
pub(crate) fn author_window_bindings(
    mut commands: Commands,
    windows: Query<
        (
            Entity,
            &Window,
            &CurrentMonitor,
            &CurrentMonitorEntity,
            Has<PrimaryWindow>,
            RecoveryMarkers,
        ),
        (Without<WindowBindingAuthoring>, With<ManagedWindow>),
    >,
    registry: Res<ManagedWindowRegistry>,
    mut persisted: ResMut<PersistedWindowPlacements>,
    live_displays: LiveDisplayEndpointLookup,
    live_display_devices: Query<&LiveDisplayDevices>,
    device_keys: Query<&DeviceKey>,
    role_entities: Query<(Entity, &RoleKey)>,
    platform: Res<Platform>,
    driver: Res<WindowDriverId>,
    mut registration: BindingRegistration,
    mut stranded: ResMut<StrandedWindowMovementBaselines>,
) {
    let mut context = WindowBindingAuthoringContext {
        registry:             &registry,
        persisted:            &mut persisted,
        live_displays:        &live_displays,
        live_display_devices: &live_display_devices,
        device_keys:          &device_keys,
        platform:             *platform,
        driver:               &driver,
        stranded:             &mut stranded,
    };
    for (entity, window, current_monitor, current_monitor_entity, primary, markers) in &windows {
        let role_source = if primary {
            WindowBindingRoleSource::Primary
        } else {
            WindowBindingRoleSource::Managed(entity)
        };
        let candidate = WindowBindingCandidate {
            window,
            current_monitor,
            monitor_entity: *current_monitor_entity,
            role_source,
            recovery: recovery_policy(markers),
        };
        match author_window_binding(candidate, &mut context, &role_entities, &mut registration) {
            WindowBindingAuthoringResolution::AwaitingManagedRegistration
            | WindowBindingAuthoringResolution::AwaitingDisplayEndpoint
            | WindowBindingAuthoringResolution::AwaitingRoleEntity => {},
            WindowBindingAuthoringResolution::Registered { role_entity } => {
                registration
                    .insert_client_relationship(entity, WindowRiggingRole::new(role_entity));
                commands
                    .entity(entity)
                    .insert(WindowBindingAuthoring::Registered)
                    .remove::<WindowDisplayRebindPending>();
            },
            WindowBindingAuthoringResolution::Rejected => {
                commands
                    .entity(entity)
                    .insert(WindowBindingAuthoring::Rejected)
                    .remove::<WindowDisplayRebindPending>();
            },
        }
    }
}

/// Build the one kernel binding a Clerestory window role owns on a display endpoint.
///
/// `author_window_binding` and `rebind_window_to_its_current_display` both produce this value, so
/// a rebound window's binding differs from a freshly authored one only in the endpoint, recovery
/// policy, and placement passed in here. The role starts waiting because both callers hand the
/// binding to `Bindings`, which owns every later transition.
const fn window_binding(
    role: RoleKey,
    endpoint: DeviceEndpoint,
    driver: EndpointDriverRegistration<EstablishedWindowPlacement>,
    placement: EstablishedWindowPlacement,
    binding_policy: BindingPolicy,
) -> BindingAuthoring<EstablishedWindowPlacement> {
    BindingAuthoring::new(role, endpoint, driver, placement, binding_policy)
}

/// Move a role's binding to the display its window now occupies.
///
/// `author_window_bindings` filters on `Without<WindowBindingAuthoring>`, so it authors one
/// endpoint per window and never revisits it. A window dragged to another display therefore keeps
/// the endpoint it was authored with, and `write_established_window_configurations` saves that
/// stale `Binding::endpoint`'s device beside an offset that
/// `EstablishedWindowPlacement::from_readback` measured against `CurrentMonitor` — the display the
/// window actually occupies. The saved record then names one display and describes a position on
/// another, and the next restore resolves the offset against the display named in the record.
///
/// The replacement carries the current readback as its `RequestedConfiguration`.
/// `Bindings::replace` returns the role to waiting, which dispatches one apply, and an
/// apply that targets the position the window already occupies cannot pull it off the display it
/// was dropped on.
pub(crate) fn rebind_window_to_its_current_display(
    mut commands: Commands,
    windows: Query<
        (
            Entity,
            &Window,
            &CurrentMonitor,
            &WindowRiggingRole,
            Has<PrimaryWindow>,
        ),
        (
            With<WindowBindingAuthoring>,
            With<WindowDisplayRebindPending>,
        ),
    >,
    registry: Res<ManagedWindowRegistry>,
    live_displays: LiveDisplayEndpointLookup,
    platform: Res<Platform>,
    driver: Res<WindowDriverId>,
    role_statuses: Query<&RoleStatus>,
    resolved_devices: Query<&ResolvedToDevice>,
    mut bindings: ResMut<Bindings>,
    mut stranded: ResMut<StrandedWindowMovementBaselines>,
) {
    for (entity, window, current_monitor, window_rigging_role, primary) in &windows {
        let role_source = if primary {
            WindowBindingRoleSource::Primary
        } else {
            WindowBindingRoleSource::Managed(entity)
        };
        let WindowBindingRoleResolution::Available(role) =
            resolve_window_binding_role(role_source, &registry)
        else {
            continue;
        };
        let Ok(existing) = bindings.binding(&role) else {
            continue;
        };
        // No state guard: moving a window is itself what puts the role into `Applying`, so the
        // role is never `Ready` at the moment `CurrentMonitor` changes. `Bindings::replace` takes
        // a reserved transition slot and driver work is isolated by `AttemptRef`, so
        // superseding the in-flight attempt is the supported handoff rather than a stranded
        // attempt.
        let Ok(device) = live_displays.key_for_descriptor(current_monitor.descriptor) else {
            continue;
        };
        commands
            .entity(entity)
            .remove::<WindowDisplayRebindPending>();
        // The same display reporting new geometry also reinserts `CurrentMonitor`, which is not a
        // move.
        if existing.endpoint.device == device {
            continue;
        }
        // A returning display can re-enumerate the fallback display and reinsert `CurrentMonitor`
        // without moving the stranded window. Only that same-device insertion keeps the saved
        // endpoint; a different exact device means the window moved between live displays.
        if stranded.is_tracked(&role) {
            match stranded.observe_display(&role, &device) {
                StrandedWindowDisplayObservation::NotTracked
                | StrandedWindowDisplayObservation::BaselineRecorded
                | StrandedWindowDisplayObservation::MatchesBaseline => continue,
                StrandedWindowDisplayObservation::DiffersFromBaseline => {},
            }
        }
        let binding_policy = existing.policy();
        let recovery = binding_policy.recovery();
        let availability_blocks_service = role_statuses
            .get(window_rigging_role.entity())
            .is_ok_and(|status| {
                matches!(
                    status.view(),
                    RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
                )
            });
        let endpoint_unavailable = availability_blocks_service
            || resolved_devices.get(window_rigging_role.entity()).is_err();
        if endpoint_unavailable
            && matches!(
                recovery,
                RecoveryPolicy::ReapplyOnRequest | RecoveryPolicy::ReapplyOnReturn
            )
        {
            // The compositor moved this window only because the display its binding names left
            // the live topology. Keeping the endpoint preserves the kernel's departure debt;
            // stranded tracking makes a later user move an explicit decision to adopt the
            // fallback display.
            stranded.restart_placement(role);
            continue;
        }
        let endpoint = DeviceEndpoint {
            device,
            id: existing.endpoint.id.clone(),
        };
        let physical_position = match window.position {
            WindowPosition::At(position) => Some(IVec2::new(position.x, position.y)),
            _ => None,
        };
        let placement = crate::persistence::EstablishedWindowPlacement::from_readback(
            window,
            current_monitor,
            physical_position,
            *platform,
        );
        let replacement =
            window_binding(role.clone(), endpoint, driver.0, placement, binding_policy);
        if let Err(error) = bindings.replace_authoring(replacement) {
            error!(
                "[rebind_window_to_its_current_display] role {role} could not follow its window to its current display: {error}"
            );
            continue;
        }
        stranded.forget(&role);
    }
}

/// Hand a stranded window's role to the display it is actually on, once the user moves it.
///
/// A window whose saved display is absent keeps a binding naming that display, so
/// `RecoveryPolicy::ReapplyOnReturn` moves it back there when the display returns. The cost is that
/// the role stays waiting, and `write_established_window_configurations` writes
/// nothing for a role that is not ready — so while the window is stranded nothing the user does to
/// it is saved, and the same absent display is waited for again on every launch.
///
/// Moving the window is the user overriding that pending return. Replacing the binding with one on
/// the live display lets the role resolve, which both cancels the return and reopens persistence,
/// so where the user put the window becomes the position that is saved.
///
/// [`StrandedWindowMovementBaselines`] separates the user's move from the fallback's own
/// placement; see its documentation for why the baseline is the window at rest rather than the
/// geometry the fallback requested.
pub(crate) fn adopt_live_display_for_stranded_window(
    windows: Query<
        (
            Entity,
            &Window,
            &CurrentMonitor,
            &CurrentMonitorEntity,
            &WindowRiggingRole,
            Has<PrimaryWindow>,
        ),
        With<WindowBindingAuthoring>,
    >,
    registry: Res<ManagedWindowRegistry>,
    live_display_devices: Query<&LiveDisplayDevices>,
    device_keys: Query<&DeviceKey>,
    platform: Res<Platform>,
    driver: Res<WindowDriverId>,
    role_statuses: Query<(&RoleStatus, Option<&ResolvedToDevice>)>,
    mut bindings: ResMut<Bindings>,
    mut stranded: ResMut<StrandedWindowMovementBaselines>,
) {
    if stranded.is_empty() {
        return;
    }
    for (entity, window, current_monitor, current_monitor_entity, window_rigging_role, primary) in
        &windows
    {
        let role_source = if primary {
            WindowBindingRoleSource::Primary
        } else {
            WindowBindingRoleSource::Managed(entity)
        };
        let WindowBindingRoleResolution::Available(role) =
            resolve_window_binding_role(role_source, &registry)
        else {
            continue;
        };
        // Reads go through `bindings.as_ref()` so a settled frame never marks the resource
        // changed; only an actual adoption takes the mutable path.
        let Ok(existing) = bindings.as_ref().binding(&role) else {
            stranded.forget(&role);
            continue;
        };
        // Topology can move the window before the kernel publishes the resulting departure. Keep
        // the fallback baseline during that short interval while the binding's endpoint is still
        // absent; once the endpoint is live again, a non-waiting role is carrying the window back
        // to its saved display, and its arrival must not be read as a user move.
        match stranded_window_role_observation(role_statuses.get(window_rigging_role.entity())) {
            StrandedWindowRoleObservation::StatusUnavailable
            | StrandedWindowRoleObservation::ActiveWithoutResolvedDevice => continue,
            StrandedWindowRoleObservation::ActiveWithResolvedDevice => {
                stranded.forget(&role);
                continue;
            },
            StrandedWindowRoleObservation::WaitingWithoutResolvedDevice
            | StrandedWindowRoleObservation::WaitingWithResolvedDevice => {},
        }
        let Ok(live_display_device) = live_display_devices.get(current_monitor_entity.entity())
        else {
            continue;
        };
        let Ok(device_entity) = live_display_device.device() else {
            continue;
        };
        let Ok(device) = device_keys.get(device_entity).cloned() else {
            continue;
        };
        // The saved display returned under the window without moving it; the kernel owns the role
        // from here.
        if existing.endpoint.device == device {
            stranded.forget(&role);
            continue;
        }
        let endpoint_id = existing.endpoint.id.clone();
        let physical_position = match window.position {
            WindowPosition::At(position) => Some(IVec2::new(position.x, position.y)),
            _ => None,
        };
        let live = persistence::EstablishedWindowPlacement::from_readback(
            window,
            current_monitor,
            physical_position,
            *platform,
        );
        let display_changed = match stranded.observe_display(&role, &device) {
            StrandedWindowDisplayObservation::NotTracked => continue,
            StrandedWindowDisplayObservation::BaselineRecorded
            | StrandedWindowDisplayObservation::MatchesBaseline => false,
            StrandedWindowDisplayObservation::DiffersFromBaseline => true,
        };
        if !display_changed
            && stranded.observe_placement(&role, &live) == StrandedWindowObservation::Unmoved
        {
            continue;
        }
        let endpoint = DeviceEndpoint {
            device,
            id: endpoint_id,
        };
        let binding_policy = existing.policy();
        let replacement = window_binding(role.clone(), endpoint, driver.0, live, binding_policy);
        if let Err(error) = bindings.replace_authoring(replacement) {
            error!(
                "[adopt_live_display_for_stranded_window] role {role} could not adopt the display its window now sits on: {error}"
            );
            continue;
        }
        stranded.forget(&role);
    }
}

/// Report a role's live placement when its window geometry changes.
///
/// `WindowRoleDriverState::configuration_changed` sends the new
/// `EstablishedWindowPlacement` through the role's `SessionLease`, and
/// `write_established_window_configurations` persists that established value. A move or resize on
/// one display changes no binding, so this system reports the placement directly.
///
/// Cross-display moves are not this system's job — `rebind_window_to_its_current_display` replaces
/// the whole binding there, and the replacement's apply leaves the role outside its established
/// state until it settles, which is exactly what the state guard here skips.
pub(crate) fn forget_stale_window_captures(
    windows: Query<
        (
            Entity,
            &Window,
            &CurrentMonitor,
            &WindowRiggingRole,
            Has<PrimaryWindow>,
        ),
        (Changed<Window>, With<WindowBindingAuthoring>),
    >,
    registry: Res<ManagedWindowRegistry>,
    platform: Res<Platform>,
    role_statuses: Query<&RoleStatus>,
    bindings: Res<Bindings>,
    mut driver_state: ResMut<WindowRoleDriverState>,
) {
    for (entity, window, current_monitor, window_rigging_role, primary) in &windows {
        let role_source = if primary {
            WindowBindingRoleSource::Primary
        } else {
            WindowBindingRoleSource::Managed(entity)
        };
        let WindowBindingRoleResolution::Available(role) =
            resolve_window_binding_role(role_source, &registry)
        else {
            continue;
        };
        // Reads go through `bindings.as_ref()` so a settled frame never marks the resource
        // changed; only an actual forget takes the mutable path.
        let Ok(binding) = bindings.binding(&role) else {
            continue;
        };
        let Ok(configuration) = binding.last_known_good() else {
            continue;
        };
        let Some(captured) = configuration
            .as_any()
            .downcast_ref::<EstablishedWindowPlacement>()
        else {
            continue;
        };
        let physical_position = match window.position {
            WindowPosition::At(position) => Some(IVec2::new(position.x, position.y)),
            _ => None,
        };
        let live = persistence::EstablishedWindowPlacement::from_readback(
            window,
            current_monitor,
            physical_position,
            *platform,
        );
        if *captured == live {
            continue;
        }
        let established = role_statuses
            .get(window_rigging_role.entity())
            .is_ok_and(|status| matches!(status.view(), RoleStatusView::Established { .. }));
        if !established {
            continue;
        }
        driver_state.configuration_changed(&role, live);
    }
}

/// Locate the window whose relationship targets the current role entity.
pub(crate) fn window_for_role_entity(world: &World, role_entity: Entity) -> Option<Entity> {
    world
        .get::<WindowsWithRiggingRole>(role_entity)
        .and_then(|windows| windows.0.first().copied())
}

/// Register and deduplicate a managed name before deriving its kernel role.
///
/// The marker and the name each trigger this, so the pair arriving together registers once and a
/// name inserted after the marker still registers.
pub(crate) fn on_managed_window_added(
    add: On<Add, (ManagedWindow, ManagedWindowName)>,
    mut commands: Commands,
    mut names: Query<&mut ManagedWindowName>,
    mut registry: ResMut<ManagedWindowRegistry>,
    primary_query: Query<(), With<PrimaryWindow>>,
) {
    let entity = add.entity;
    let Ok(mut managed_name) = names.get_mut(entity) else {
        return;
    };
    let name = managed_name.0.clone();
    if let ManagedWindowRegistration::Registered {
        name: registered_name,
        ..
    } = registry.registration(entity)
        && registered_name == name
    {
        return;
    }
    if primary_query.contains(entity) {
        warn!(
            "[on_managed_window_added] the primary window is already managed under {PRIMARY_WINDOW_KEY}"
        );
        return;
    }

    let unique_name = if registry.names.contains(&name) {
        let mut suffix = FIRST_DUPLICATE_SUFFIX;
        loop {
            let candidate = format!("{name}{MANAGED_WINDOW_NAME_SEPARATOR}{suffix}");
            if !registry.names.contains(&candidate) {
                break candidate;
            }
            suffix += 1;
        }
    } else {
        name
    };
    if let Err(error) = persistence::managed_window_role(&unique_name) {
        warn!("[on_managed_window_added] rejected managed window name {unique_name:?}: {error}");
        commands
            .entity(entity)
            .remove::<(ManagedWindow, ManagedWindowName)>();
        return;
    }
    managed_name.0.clone_from(&unique_name);
    registry.register(entity, unique_name);
    commands.trigger(ManagedWindowRegistered { entity });
}

/// Retire the kernel role when a managed window lifetime ends.
pub(crate) fn on_managed_window_removed(
    remove: On<Remove, (ManagedWindow, ManagedWindowName)>,
    mut commands: Commands,
    mut registry: ResMut<ManagedWindowRegistry>,
    primary_windows: Query<(), With<PrimaryWindow>>,
    bindings: Res<Bindings>,
    mut driver_state: ResMut<WindowRoleDriverState>,
    mut stranded: ResMut<StrandedWindowMovementBaselines>,
) {
    // Queued first, so the strip still finds each aborted attempt's own marker: the removals
    // below issue against this same command queue.
    driver_state
        .abort_window(remove.entity)
        .strip(&mut commands);
    if primary_windows.contains(remove.entity) {
        commands
            .entity(remove.entity)
            .try_remove::<WindowBindingAuthoring>();
    } else {
        commands.entity(remove.entity).try_remove::<(
            CurrentMonitor,
            OutputConfirmationSource<OnScreenConfirmation>,
            PlacementAbandoned,
            SavedDisplayRevealWait,
            WindowBindingAuthoring,
            WindowRiggingRole,
        )>();
    }
    let role = match registry.release(remove.entity) {
        ManagedWindowRelease::Released(role) => role,
        ManagedWindowRelease::InvalidRegisteredName { name, error } => {
            error!(
                "[on_managed_window_removed] registered name {name:?} no longer forms its role: {error}"
            );
            return;
        },
        ManagedWindowRelease::NotAManagedWindow => return,
    };
    stranded.forget(&role);
    let recovery = bindings
        .binding(&role)
        .map_or(RecoveryPolicy::Forget, |binding| binding.recovery);
    match recovery {
        RecoveryPolicy::ReapplyOnRequest | RecoveryPolicy::ReapplyOnReturn => {},
        RecoveryPolicy::Forget => commands.trigger(RetireRole { role }),
    }
}

/// Retire the primary window role when its entity lifetime ends.
///
/// A registered name gives the window a second role that outlives the primary marker, so that
/// window stays managed and keeps the reveal wait its hidden lifetime is still counting down. An
/// unregistered window has `window:primary` and nothing else, so losing the marker leaves it with
/// no role at all and Clerestory takes back everything it attached, management included.
pub(crate) fn on_primary_window_removed(
    remove: On<Remove, PrimaryWindow>,
    mut commands: Commands,
    registry: Res<ManagedWindowRegistry>,
    bindings: Res<Bindings>,
    mut driver_state: ResMut<WindowRoleDriverState>,
    mut stranded: ResMut<StrandedWindowMovementBaselines>,
) {
    // A registered name keeps this entity managed, so the window survives losing its primary
    // marker and every restore marker an aborted attempt left would stand on it forever. Queued
    // first, before the `WindowRestoreAttempt` removals below, which the strip matches on.
    driver_state
        .abort_window(remove.entity)
        .strip(&mut commands);
    let Ok(role) = persistence::primary_window_role() else {
        return;
    };
    match registry.registration(remove.entity) {
        ManagedWindowRegistration::Registered { .. } => {
            commands.entity(remove.entity).try_remove::<(
                CurrentMonitor,
                WindowRestoreAttempt,
                PlacementAbandoned,
                WindowBindingAuthoring,
            )>();
        },
        ManagedWindowRegistration::NotAManagedWindow => {
            commands.entity(remove.entity).try_remove::<(
                CurrentMonitor,
                ManagedWindow,
                OutputConfirmationSource<OnScreenConfirmation>,
                WindowRestoreAttempt,
                PlacementAbandoned,
                SavedDisplayRevealWait,
                WindowBindingAuthoring,
            )>();
        },
    }
    stranded.forget(&role);
    let recovery = bindings
        .binding(&role)
        .map_or(RecoveryPolicy::Forget, |binding| binding.recovery);
    match recovery {
        RecoveryPolicy::ReapplyOnRequest | RecoveryPolicy::ReapplyOnReturn => {},
        RecoveryPolicy::Forget => commands.trigger(RetireRole { role }),
    }
}

/// Remove the proof source when relationship cleanup stops a window from being an output.
pub(crate) fn on_window_rigging_role_removed(
    remove: On<Remove, WindowRiggingRole>,
    mut commands: Commands,
) {
    commands
        .entity(remove.entity)
        .try_remove::<OutputConfirmationSource<OnScreenConfirmation>>();
}

/// Prepare a managed window only after `ManagedWindowRegistry` accepted its registration.
pub(crate) fn on_managed_window_load(
    registered: On<ManagedWindowRegistered>,
    mut commands: Commands,
    mut windows: Query<(&mut Window, Option<&OnMonitor>)>,
    monitors: Res<Monitors>,
    platform: Res<Platform>,
) {
    let entity = registered.entity;
    let Ok((mut window, on_monitor)) = windows.get_mut(entity) else {
        return;
    };
    let current_monitor = on_monitor.and_then(|association| {
        monitors::current_monitor_from_association(&window, association, &monitors)
    });
    let mut entity_commands = commands.entity(entity);
    if let Some(current_monitor) = current_monitor {
        entity_commands.insert(current_monitor);
    } else {
        entity_commands.remove::<CurrentMonitor>();
    }

    if platform.should_hide_on_startup() {
        window.visible = false;
        entity_commands
            .remove::<WindowRevealDisposition>()
            .insert(SavedDisplayRevealWait::default());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bevy::prelude::App;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::UVec2;
    use bevy::prelude::Update;
    use bevy::prelude::default;
    use bevy::window::WindowMode;
    use hana_rigging::prelude::ApplyContext;
    use hana_rigging::prelude::AttemptInvalidation;
    use hana_rigging::prelude::AttemptRef;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::DriverCleanupRoleEntity;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EstablishedContext;
    use hana_rigging::prelude::FlowExpectation;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RiggingSystems;
    use hana_rigging::prelude::SessionRef;
    use hana_rigging::prelude::SessionReleaseCause;
    use hana_rigging::prelude::TargetResolution;
    use hana_rigging::prelude::TargetResolutionContext;

    use super::*;
    use crate::MonitorDescriptor;
    use crate::monitors::DisplayIdentity;
    use crate::monitors::LiveDisplayEndpoint;
    use crate::monitors::LiveDisplayMonitor;
    use crate::output_proof::ScriptedWindowOnScreenReadings;
    use crate::output_proof::WindowOnScreenReading;
    use crate::persistence::EstablishedWindowPosition;
    use crate::persistence::LoadedBindingPolicy;
    use crate::persistence::PersistedDisplayFingerprintV4;
    use crate::persistence::PersistedDisplayIdentityV4;
    use crate::persistence::PersistedPosition;
    use crate::persistence::PersistedWindowPlacementLookup;
    use crate::persistence::PersistedWindowState;
    use crate::persistence::PersistedWindowTargetV5;
    use crate::persistence::SavedWindowMode;
    use crate::restore::InjectedWinitWindows;

    #[derive(Default, Resource)]
    struct RetiredRoles(Vec<RoleKey>);

    #[derive(Default, Resource)]
    struct RecoveryMarkerWarnings(usize);

    struct ProjectionBindingDriver;

    impl EndpointDriver for ProjectionBindingDriver {
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
            _: ApplyContext<'_, Self::Configuration>,
            _: &Self::Configuration,
            (): Self::Target,
        ) {
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

    fn record_retired_role(retired: On<RetireRole>, mut roles: ResMut<RetiredRoles>) {
        roles.0.push(retired.role.clone());
    }

    fn record_recovery_marker_warning(
        _added: On<Add, RecoveryMarkerConflictWarned>,
        mut warnings: ResMut<RecoveryMarkerWarnings>,
    ) {
        warnings.0 += 1;
    }

    struct AuthoringHarness {
        app:        App,
        descriptor: MonitorDescriptor,
        monitor:    Entity,
    }

    impl AuthoringHarness {
        fn new() -> Result<Self, String> {
            let live_device = DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored {
                    value: AuthoredId::new("authoring-display").map_err(|error| {
                        format!("failed to create authoring device ID: {error}")
                    })?,
                },
            };
            let descriptor = MonitorDescriptor::for_current_enumeration(
                0,
                1.0,
                IVec2::ZERO,
                UVec2::new(1_920, 1_080),
            );
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, RiggingPlugin));
            crate::output_proof::register_window_output_proof(&mut app);
            let driver = app.add_endpoint_driver(ProjectionBindingDriver);
            app.insert_resource(WindowDriverId(driver))
                .init_resource::<WindowRoleDriverState>()
                .init_resource::<ManagedWindowRegistry>()
                .init_resource::<PersistedWindowPlacements>()
                .init_resource::<RetiredRoles>()
                .init_resource::<StrandedWindowMovementBaselines>()
                .insert_resource(Platform::detect())
                .init_resource::<RecoveryMarkerWarnings>()
                .init_resource::<RecoveryMarkerWarningPathExecutions>()
                .add_observer(on_managed_window_added)
                .add_observer(on_managed_window_removed)
                .add_observer(on_primary_window_removed)
                .add_observer(crate::mark_primary_window_as_managed)
                .add_observer(record_retired_role)
                .add_observer(warn_conflicting_recovery_markers)
                .add_observer(record_recovery_marker_warning)
                .add_systems(Update, author_window_bindings);
            let monitor = app.world_mut().spawn_empty().id();
            app.world_mut().spawn((
                live_device,
                LiveDisplayEndpoint {
                    monitor,
                    descriptor,
                    legacy_identity: DisplayIdentity::Anonymous,
                },
                LiveDisplayMonitor::new(monitor),
            ));
            Ok(Self {
                app,
                descriptor,
                monitor,
            })
        }

        fn primary_window(&mut self) -> Entity {
            self.app
                .world_mut()
                .spawn((
                    Window::default(),
                    PrimaryWindow,
                    CurrentMonitor {
                        descriptor:            self.descriptor,
                        effective_window_mode: WindowMode::Windowed,
                    },
                    CurrentMonitorEntity::new(self.monitor),
                ))
                .id()
        }

        fn recovery(&self) -> Result<RecoveryPolicy, String> {
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to create primary role: {error}"))?;
            self.app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .map(|binding| binding.recovery)
                .map_err(|error| format!("the primary window lost its binding: {error}"))
        }

        fn warning_path_count(&self) -> usize {
            self.app
                .world()
                .resource::<RecoveryMarkerWarningPathExecutions>()
                .0
        }

        fn warning_count(&self) -> usize { self.app.world().resource::<RecoveryMarkerWarnings>().0 }
    }

    #[test]
    fn window_without_recovery_markers_authors_forget() -> Result<(), String> {
        let mut harness = AuthoringHarness::new()?;
        harness.primary_window();

        harness.app.update();

        assert_eq!(harness.recovery()?, RecoveryPolicy::Forget);
        Ok(())
    }

    #[test]
    fn recovery_markers_author_their_corresponding_policies() -> Result<(), String> {
        let mut return_harness = AuthoringHarness::new()?;
        let return_window = return_harness.primary_window();
        return_harness
            .app
            .world_mut()
            .entity_mut(return_window)
            .insert(RecoverOnReturn);
        return_harness.app.update();
        assert_eq!(return_harness.recovery()?, RecoveryPolicy::ReapplyOnReturn);

        let mut request_harness = AuthoringHarness::new()?;
        let request_window = request_harness.primary_window();
        request_harness
            .app
            .world_mut()
            .entity_mut(request_window)
            .insert(RecoverOnRequest);
        request_harness.app.update();
        assert_eq!(
            request_harness.recovery()?,
            RecoveryPolicy::ReapplyOnRequest
        );
        Ok(())
    }

    #[test]
    fn offline_legacy_display_resolves_its_binding_policy_before_endpoint_arrives()
    -> Result<(), String> {
        let mut harness = AuthoringHarness::new()?;
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let retained_target = PersistedWindowTargetV5::AwaitingLegacyEvidence(
            PersistedDisplayIdentityV4::Fingerprinted(PersistedDisplayFingerprintV4(0xA850_6988)),
        );
        harness
            .app
            .world_mut()
            .resource_mut::<PersistedWindowPlacements>()
            .seed(HashMap::from([(
                role.clone(),
                PersistedWindowState {
                    target:            retained_target.clone(),
                    position:          PersistedPosition::Unpositioned,
                    logical_width:     800,
                    logical_height:    600,
                    saved_window_mode: SavedWindowMode::Windowed,
                    app_name:          String::from("offline legacy display"),
                },
            )]));
        let window = harness.primary_window();
        harness
            .app
            .world_mut()
            .entity_mut(window)
            .insert(RecoverOnRequest);

        harness.app.update();

        assert!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .is_err()
        );
        let persisted = harness.app.world().resource::<PersistedWindowPlacements>();
        let PersistedWindowPlacementLookup::Saved(placement) = persisted.get(&role) else {
            return Err(String::from("the offline legacy placement was discarded"));
        };
        assert_eq!(placement.window_state.target, retained_target);
        assert_eq!(
            placement.loaded_binding_policy,
            LoadedBindingPolicy::Saved(default_window_binding_policy(
                RecoveryPolicy::ReapplyOnRequest,
            ))
        );
        Ok(())
    }

    #[test]
    fn the_window_binding_stays_unmonitored_under_every_recovery_policy() {
        for recovery in [
            RecoveryPolicy::Forget,
            RecoveryPolicy::ReapplyOnRequest,
            RecoveryPolicy::ReapplyOnReturn,
        ] {
            // Exhaustive on purpose: a recovery policy added later stops compiling here rather
            // than escaping this pin unexamined.
            match recovery {
                RecoveryPolicy::Forget
                | RecoveryPolicy::ReapplyOnRequest
                | RecoveryPolicy::ReapplyOnReturn => {},
            }

            assert_eq!(
                default_window_binding_policy(recovery).flow_expectation(),
                FlowExpectation::NotMonitored,
                "window placement is a one-shot arrangement, not a stream, so it has no datum to \
                 testify to and must never be judged for silence — {recovery:?} is no exception"
            );
        }
    }

    #[test]
    fn conflicting_recovery_markers_prefer_request_and_warn_once() -> Result<(), String> {
        let mut harness = AuthoringHarness::new()?;
        let entity = harness
            .app
            .world_mut()
            .spawn((
                Window::default(),
                PrimaryWindow,
                CurrentMonitor {
                    descriptor:            harness.descriptor,
                    effective_window_mode: WindowMode::Windowed,
                },
                CurrentMonitorEntity::new(harness.monitor),
                RecoverOnReturn,
                RecoverOnRequest,
            ))
            .id();

        harness.app.update();

        assert_eq!(harness.recovery()?, RecoveryPolicy::ReapplyOnRequest);
        assert!(
            harness
                .app
                .world()
                .get::<RecoveryMarkerConflictWarned>(entity)
                .is_some()
        );
        assert_eq!(harness.warning_path_count(), 1);
        Ok(())
    }

    #[test]
    fn recovery_markers_added_after_authoring_do_not_change_policy() -> Result<(), String> {
        let mut harness = AuthoringHarness::new()?;
        let window = harness.primary_window();
        harness.app.update();

        harness
            .app
            .world_mut()
            .entity_mut(window)
            .insert(RecoverOnRequest);
        harness.app.update();

        assert_eq!(harness.recovery()?, RecoveryPolicy::Forget);
        Ok(())
    }

    #[test]
    fn an_unmanaged_secondary_window_is_not_authored() -> Result<(), String> {
        let mut harness = AuthoringHarness::new()?;
        let entity = harness
            .app
            .world_mut()
            .spawn((
                Window::default(),
                CurrentMonitor {
                    descriptor:            harness.descriptor,
                    effective_window_mode: WindowMode::Windowed,
                },
                CurrentMonitorEntity::new(harness.monitor),
                RecoverOnReturn,
            ))
            .id();

        harness.app.update();

        assert!(
            harness
                .app
                .world()
                .get::<WindowBindingAuthoring>(entity)
                .is_none()
        );
        assert!(
            harness
                .app
                .world()
                .get::<RecoveryMarkerConflictWarned>(entity)
                .is_some()
        );
        assert_eq!(harness.warning_count(), 1);
        Ok(())
    }

    /// A stranded window: its role's binding names a display that is not plugged in, while the
    /// window itself sits on the one display that is.
    struct StrandedHarness {
        app:    App,
        window: Entity,
    }

    impl StrandedHarness {
        fn new() -> Result<Self, String> {
            let role = persistence::primary_window_role()
                .map_err(|error| format!("failed to create primary role: {error}"))?;
            let endpoint_id = window_endpoint_id(&role)
                .map_err(|error| format!("failed to create primary endpoint ID: {error}"))?;
            let absent_device = DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored {
                    value: AuthoredId::new("absent-display")
                        .map_err(|error| format!("failed to create absent device ID: {error}"))?,
                },
            };
            let live_device = DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored {
                    value: AuthoredId::new("live-display")
                        .map_err(|error| format!("failed to create live device ID: {error}"))?,
                },
            };
            let descriptor = MonitorDescriptor::for_current_enumeration(
                0,
                1.0,
                IVec2::ZERO,
                UVec2::new(1_920, 1_080),
            );

            let mut app = App::new();
            app.add_plugins((MinimalPlugins, RiggingPlugin));
            crate::output_proof::register_window_output_proof(&mut app);
            let driver = app.add_endpoint_driver(ProjectionBindingDriver);
            app.insert_resource(WindowDriverId(driver))
                .init_resource::<ManagedWindowRegistry>()
                .insert_resource(Platform::detect())
                .init_resource::<StrandedWindowMovementBaselines>()
                .add_systems(
                    Update,
                    adopt_live_display_for_stranded_window.after(RiggingSystems::Reconcile),
                );
            let monitor = app.world_mut().spawn_empty().id();
            app.world_mut().spawn((
                live_device,
                LiveDisplayEndpoint {
                    monitor,
                    descriptor,
                    legacy_identity: DisplayIdentity::Anonymous,
                },
                LiveDisplayMonitor::new(monitor),
            ));

            let binding = window_binding(
                role.clone(),
                DeviceEndpoint {
                    device: absent_device,
                    id:     endpoint_id,
                },
                driver,
                EstablishedWindowPlacement {
                    position:          EstablishedWindowPosition::Restorable {
                        logical_offset: IVec2::new(0, 30),
                    },
                    logical_size:      UVec2::new(800, 600),
                    saved_window_mode: SavedWindowMode::Windowed,
                },
                BindingPolicy::new(
                    RecoveryPolicy::ReapplyOnReturn,
                    RetryOn::NewRevision,
                    OnAbort::default(),
                    OnSessionLoss::default(),
                    ApplyDeadline::ProcessDefault,
                ),
            );
            let role_entity = register_binding(app.world_mut(), binding)
                .map_err(|error| format!("failed to register the stranded binding: {error}"))?;

            let mut window = Window {
                position: WindowPosition::At(IVec2::new(0, 30)),
                ..default()
            };
            window.resolution.set(800.0, 600.0);
            let window = app
                .world_mut()
                .spawn((
                    window,
                    PrimaryWindow,
                    CurrentMonitor {
                        descriptor,
                        effective_window_mode: WindowMode::Windowed,
                    },
                    CurrentMonitorEntity::new(monitor),
                    WindowRiggingRole::new(role_entity),
                    OnMonitor(monitor),
                    WindowBindingAuthoring::Registered,
                ))
                .id();

            app.world_mut()
                .resource_mut::<StrandedWindowMovementBaselines>()
                .begin(role);

            Ok(Self { app, window })
        }

        fn set_window_position(&mut self, position: IVec2) -> Result<(), String> {
            let mut window = self
                .app
                .world_mut()
                .get_mut::<Window>(self.window)
                .ok_or_else(|| String::from("the primary window lost its Window component"))?;
            window.position = WindowPosition::At(position);
            Ok(())
        }

        fn adopted_device(&self, role: &RoleKey) -> Result<DeviceKey, String> {
            self.app
                .world()
                .resource::<Bindings>()
                .binding(role)
                .map(|binding| binding.endpoint.device.clone())
                .map_err(|error| format!("the role lost its binding: {error}"))
        }
    }

    fn assert_window_output_proof_poll_runs(app: &mut App, window: Entity) {
        app.world_mut().init_resource::<InjectedWinitWindows>();
        app.world_mut()
            .resource_mut::<ScriptedWindowOnScreenReadings>()
            .script(window, WindowOnScreenReading::ReportedVisible);
        app.update();

        assert_eq!(
            app.world()
                .resource::<ScriptedWindowOnScreenReadings>()
                .readings_taken(),
            1
        );
    }

    #[test]
    fn managed_test_harnesses_run_the_window_output_proof_poll() -> Result<(), String> {
        let mut authoring_harness = AuthoringHarness::new()?;
        let authoring_window = authoring_harness.primary_window();
        assert_window_output_proof_poll_runs(&mut authoring_harness.app, authoring_window);

        let mut stranded_harness = StrandedHarness::new()?;
        let stranded_window = stranded_harness.window;
        assert_window_output_proof_poll_runs(&mut stranded_harness.app, stranded_window);
        Ok(())
    }

    /// The window does not land where the fallback asked: macOS clamps it below the menu bar, and
    /// winit reports the real coordinate a frame or two later. That writeback is the fallback's own
    /// placement finishing, not the user moving the window; reading it as a move would adopt the
    /// fallback display even though the user never touched the window.
    #[test]
    fn the_fallbacks_own_settling_is_not_a_user_move() -> Result<(), String> {
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let mut harness = StrandedHarness::new()?;

        // Frame one observes the geometry the fallback requested.
        harness.app.update();
        // The compositor answers with the coordinate it actually used.
        harness.set_window_position(IVec2::new(0, 36))?;
        harness.app.update();
        harness.app.update();

        assert!(
            harness
                .app
                .world()
                .resource::<StrandedWindowMovementBaselines>()
                .is_tracked(&role),
            "the compositor's own writeback was spent as if the user had moved the window"
        );
        Ok(())
    }

    /// Moving a stranded window is the user overriding the pending return, so its role has to
    /// follow the window onto the display it now occupies.
    #[test]
    fn moving_a_stranded_window_adopts_the_display_it_sits_on() -> Result<(), String> {
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let mut harness = StrandedHarness::new()?;

        // Two frames with the window untouched make the fallback placement the baseline.
        harness.app.update();
        harness.app.update();
        harness.set_window_position(IVec2::new(400, 200))?;
        harness.app.update();

        let adopted = harness.adopted_device(&role)?;
        assert!(
            matches!(&adopted.id, DeviceIdSource::Authored { value } if value.as_str() == "live-display"),
            "the stranded role kept {adopted:?} instead of adopting the live display"
        );
        Ok(())
    }

    fn managed_lifecycle_app() -> App {
        let mut app = App::new();
        app.init_resource::<ManagedWindowRegistry>()
            .init_resource::<Bindings>()
            .init_resource::<WindowRoleDriverState>()
            .init_resource::<RetiredRoles>()
            .init_resource::<StrandedWindowMovementBaselines>()
            .insert_resource(Monitors::from_test_monitors([]))
            .insert_resource(Platform::MacOs)
            .add_observer(on_managed_window_load)
            .add_observer(on_managed_window_added)
            .add_observer(on_managed_window_removed)
            .add_observer(on_primary_window_removed)
            .add_observer(crate::mark_primary_window_as_managed)
            .add_observer(record_retired_role);
        app
    }
    #[test]
    fn window_roles_name_distinct_endpoints_on_one_display_device() {
        let primary = persistence::primary_window_role();
        let managed = persistence::managed_window_role("automatic");
        assert!(primary.is_ok());
        assert!(managed.is_ok());
        let (Ok(primary), Ok(managed)) = (primary, managed) else {
            return;
        };

        assert_ne!(window_endpoint_id(&primary), window_endpoint_id(&managed));
    }

    #[test]
    fn duplicate_managed_names_are_canonicalized_before_role_derivation() {
        let mut app = managed_lifecycle_app();
        let first = app
            .world_mut()
            .spawn(ManagedWindowName("inspector".into()))
            .id();
        let second = app
            .world_mut()
            .spawn(ManagedWindowName("inspector".into()))
            .id();

        let first_name = app
            .world()
            .get::<ManagedWindowName>(first)
            .map(|managed_name| managed_name.0.clone());
        let second_name = app
            .world()
            .get::<ManagedWindowName>(second)
            .map(|managed_name| managed_name.0.clone());
        assert_eq!(first_name.as_deref(), Some("inspector"));
        assert!(
            second_name
                .as_deref()
                .is_some_and(|name| name != "inspector")
        );
        assert_ne!(first_name, second_name);
        let registry = app.world().resource::<ManagedWindowRegistry>();
        assert_eq!(registry.names.len(), 2);
        let first_role = first_name
            .as_deref()
            .and_then(|name| persistence::managed_window_role(name).ok());
        let second_role = second_name
            .as_deref()
            .and_then(|name| persistence::managed_window_role(name).ok());
        assert!(matches!(
            registry.registration(first),
            ManagedWindowRegistration::Registered { name }
                if Some(name) == first_name.as_deref()
        ));
        assert!(matches!(
            registry.registration(second),
            ManagedWindowRegistration::Registered { name }
                if Some(name) == second_name.as_deref()
        ));
        assert!(
            registry
                .roles()
                .into_iter()
                .any(|role| Some(&role) == first_role.as_ref())
        );
        assert!(
            registry
                .roles()
                .into_iter()
                .any(|role| Some(&role) == second_role.as_ref())
        );
    }

    #[test]
    fn the_primary_window_carries_the_managed_marker() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();

        assert!(app.world().get::<ManagedWindow>(entity).is_some());
        assert!(app.world().get::<ManagedWindowName>(entity).is_none());
        assert!(
            app.world()
                .resource::<ManagedWindowRegistry>()
                .entities
                .is_empty()
        );
    }

    #[test]
    fn primary_window_is_not_registered_as_a_second_managed_role() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn((PrimaryWindow, ManagedWindowName("inspector".into())))
            .id();

        assert!(app.world().get::<ManagedWindow>(entity).is_some());
        assert!(
            app.world()
                .resource::<ManagedWindowRegistry>()
                .entities
                .is_empty()
        );
    }

    #[test]
    fn invalid_managed_name_leaves_no_component_registration_or_retirement() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn(ManagedWindowName("inspector\nwindow".into()))
            .id();

        assert!(app.world().get::<ManagedWindow>(entity).is_none());
        assert!(
            app.world()
                .resource::<ManagedWindowRegistry>()
                .entities
                .is_empty()
        );

        let despawned = app.world_mut().despawn(entity);
        assert!(despawned);
        app.update();
        assert!(app.world().resource::<RetiredRoles>().0.is_empty());
    }

    #[test]
    fn registration_retains_the_name_that_defines_the_managed_role() {
        let mut app = managed_lifecycle_app();
        let role = persistence::managed_window_role("inspector");
        assert!(role.is_ok());
        let Ok(role) = role else {
            return;
        };
        let entity = app
            .world_mut()
            .spawn(ManagedWindowName("inspector".into()))
            .id();

        assert!(matches!(
            app.world()
                .resource::<ManagedWindowRegistry>()
                .registration(entity),
            ManagedWindowRegistration::Registered { name: "inspector" }
        ));
        assert!(
            app.world()
                .resource::<ManagedWindowRegistry>()
                .roles()
                .into_iter()
                .any(|registered_role| registered_role == role)
        );
    }

    #[test]
    fn managed_registration_hides_the_window_and_starts_its_reveal_wait() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn((Window::default(), ManagedWindowName("inspector".into())))
            .id();

        assert!(matches!(
            app.world()
                .resource::<ManagedWindowRegistry>()
                .registration(entity),
            ManagedWindowRegistration::Registered { .. }
        ));
        assert_eq!(
            app.world()
                .get::<Window>(entity)
                .map(|window| window.visible),
            Some(false)
        );
        assert!(app.world().get::<SavedDisplayRevealWait>(entity).is_some());
    }

    #[test]
    fn a_readded_managed_window_is_revealed_during_each_hidden_lifetime() {
        let mut app = managed_lifecycle_app();
        app.add_systems(Update, crate::show_window_once_placement_settles);
        let entity = app
            .world_mut()
            .spawn((Window::default(), ManagedWindowName("inspector".into())))
            .id();

        app.world_mut()
            .entity_mut(entity)
            .insert(WindowRevealDisposition::NothingSaved);
        app.update();
        assert_eq!(
            app.world()
                .get::<Window>(entity)
                .map(|window| window.visible),
            Some(true)
        );

        app.world_mut().entity_mut(entity).remove::<ManagedWindow>();
        app.world_mut().flush();
        app.world_mut()
            .entity_mut(entity)
            .insert(ManagedWindowName("inspector".into()));
        app.world_mut().flush();
        assert_eq!(
            app.world()
                .get::<Window>(entity)
                .map(|window| window.visible),
            Some(false)
        );
        assert!(app.world().get::<WindowRevealDisposition>(entity).is_none());

        app.world_mut()
            .entity_mut(entity)
            .insert(WindowRevealDisposition::NothingSaved);
        app.update();
        assert_eq!(
            app.world()
                .get::<Window>(entity)
                .map(|window| window.visible),
            Some(true)
        );
    }

    #[test]
    fn removing_a_managed_name_clears_its_saved_display_wait() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn((
                Window::default(),
                ManagedWindowName("inspector".into()),
                SavedDisplayRevealWait::default(),
            ))
            .id();

        app.world_mut().entity_mut(entity).remove::<ManagedWindow>();
        app.world_mut().flush();

        assert!(app.world().get::<SavedDisplayRevealWait>(entity).is_none());
    }

    #[test]
    fn removing_a_managed_name_keeps_a_primary_window_reveal_wait() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn((
                Window::default(),
                ManagedWindowName("inspector".into()),
                SavedDisplayRevealWait::default(),
            ))
            .id();
        app.world_mut().entity_mut(entity).insert(PrimaryWindow);

        app.world_mut().entity_mut(entity).remove::<ManagedWindow>();
        app.world_mut().flush();

        assert!(app.world().get::<SavedDisplayRevealWait>(entity).is_some());
    }

    #[test]
    fn removing_the_primary_marker_unmanages_an_unnamed_window() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn((
                Window::default(),
                PrimaryWindow,
                SavedDisplayRevealWait::default(),
            ))
            .id();

        app.world_mut().entity_mut(entity).remove::<PrimaryWindow>();
        app.world_mut().flush();

        assert!(app.world().get::<SavedDisplayRevealWait>(entity).is_none());
        assert!(app.world().get::<ManagedWindow>(entity).is_none());
    }

    #[test]
    fn removing_the_primary_marker_keeps_a_named_window_managed() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn((
                Window::default(),
                ManagedWindowName("inspector".into()),
                SavedDisplayRevealWait::default(),
            ))
            .id();
        app.world_mut().entity_mut(entity).insert(PrimaryWindow);

        app.world_mut().entity_mut(entity).remove::<PrimaryWindow>();
        app.world_mut().flush();

        assert!(app.world().get::<SavedDisplayRevealWait>(entity).is_some());
        assert!(app.world().get::<ManagedWindow>(entity).is_some());
    }

    #[test]
    fn managed_removal_retires_the_kernel_role_once() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn(ManagedWindowName("inspector".into()))
            .id();
        let despawned = app.world_mut().despawn(entity);
        assert!(despawned);
        app.update();

        let expected = persistence::managed_window_role("inspector");
        assert!(expected.is_ok());
        assert_eq!(
            app.world().resource::<RetiredRoles>().0,
            expected.into_iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn live_primary_marker_removal_and_readd_reauthors_the_binding() -> Result<(), String> {
        let mut harness = AuthoringHarness::new()?;
        let entity = harness.primary_window();
        harness.app.update();
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;

        harness
            .app
            .world_mut()
            .entity_mut(entity)
            .remove::<PrimaryWindow>();
        harness.app.update();
        assert!(
            harness
                .app
                .world()
                .get::<WindowBindingAuthoring>(entity)
                .is_none()
        );
        assert!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .is_err()
        );

        harness.app.world_mut().entity_mut(entity).insert((
            PrimaryWindow,
            CurrentMonitor {
                descriptor:            harness.descriptor,
                effective_window_mode: WindowMode::Windowed,
            },
        ));
        harness.app.update();

        assert!(
            harness
                .app
                .world()
                .get::<WindowBindingAuthoring>(entity)
                .is_some()
        );
        assert!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn opted_in_managed_window_respawn_reattaches_to_its_retained_role() -> Result<(), String> {
        let mut harness = AuthoringHarness::new()?;
        let role = persistence::managed_window_role("inspector")
            .map_err(|error| format!("failed to create managed role: {error}"))?;
        let first = harness
            .app
            .world_mut()
            .spawn((
                Window::default(),
                ManagedWindowName("inspector".into()),
                RecoverOnReturn,
                CurrentMonitor {
                    descriptor:            harness.descriptor,
                    effective_window_mode: WindowMode::Windowed,
                },
                CurrentMonitorEntity::new(harness.monitor),
            ))
            .id();
        harness.app.update();
        harness
            .app
            .world()
            .resource::<Bindings>()
            .binding(&role)
            .map_err(|error| format!("managed binding was not authored: {error}"))?;

        assert!(harness.app.world_mut().despawn(first));
        harness.app.update();
        assert!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .is_ok()
        );

        let second = harness
            .app
            .world_mut()
            .spawn((
                Window::default(),
                ManagedWindowName("inspector".into()),
                RecoverOnReturn,
                CurrentMonitor {
                    descriptor:            harness.descriptor,
                    effective_window_mode: WindowMode::Windowed,
                },
                CurrentMonitorEntity::new(harness.monitor),
            ))
            .id();
        harness.app.update();

        assert_eq!(
            harness
                .app
                .world()
                .resource::<ManagedWindowRegistry>()
                .entity_for_role(&role),
            RoleOccupancy::Held(second)
        );
        assert!(matches!(
            harness.app.world().get::<WindowBindingAuthoring>(second),
            Some(WindowBindingAuthoring::Registered)
        ));
        assert_eq!(
            harness
                .app
                .world()
                .resource::<Bindings>()
                .binding(&role)
                .map_err(|error| format!("retained managed binding disappeared: {error}"))?
                .recovery,
            RecoveryPolicy::ReapplyOnReturn
        );
        Ok(())
    }
}
