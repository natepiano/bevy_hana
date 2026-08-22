//! Managed window registration and startup preparation.

use std::collections::HashMap;
use std::collections::HashSet;

use bevy::prelude::Add;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
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
use bevy::prelude::debug;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy::window::OnMonitor;
use bevy::window::PrimaryWindow;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::Binding;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DriverId;
use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::LastKnownGoodConfiguration;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::PartName;
use hana_rigging::prelude::PartNameError;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::RequestedConfiguration;
use hana_rigging::prelude::RetireRole;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleState;

use crate::constants::FIRST_DUPLICATE_SUFFIX;
use crate::constants::MANAGED_WINDOW_NAME_SEPARATOR;
use crate::constants::PRIMARY_WINDOW_KEY;
use crate::driver::WindowDriverId;
use crate::monitors;
use crate::monitors::CurrentMonitor;
use crate::monitors::MonitorDeviceAssociation;
use crate::monitors::MonitorDeviceKeyLookup;
use crate::monitors::Monitors;
use crate::persistence;
use crate::persistence::EstablishedWindowPlacement;
use crate::persistence::PersistedPanelIdentityV4;
use crate::persistence::PersistedWindowPlacements;
use crate::persistence::PersistedWindowTargetV5;
use crate::platform::Platform;
use crate::recovery::StrandedWindowObservation;
use crate::recovery::StrandedWindowPlacements;
use crate::restore::RestorePreparation;
use crate::restore::WindowApplyConfiguration;

/// Marks a secondary window whose persistence role is its unique name.
#[derive(Component, Clone, Reflect)]
#[reflect(Component)]
pub struct ManagedWindow {
    /// Name used to derive a stable, namespaced `RoleKey` and the RON adapter key.
    pub name: String,
}

/// Marks a managed window whose application explicitly authorizes restoration after departure.
///
/// Insert this before Clerestory authors the window's binding. Without it, the binding uses
/// `RecoveryPolicy::ReapplyOnReturn` and the kernel restores automatically when the exact display
/// returns. With it, the kernel retains the departure debt until application code targets the
/// binding entity with `ReapplyConfiguration`; this keeps an application-controlled return from
/// becoming a general on-demand window move API.
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct ManagedWindowReapplyOnRequest;

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
    entities: HashMap<Entity, ManagedWindowRegistration>,
}

struct ManagedWindowRegistration {
    name: String,
    role: RoleKey,
}

impl ManagedWindowRegistry {
    #[must_use]
    fn name(&self, entity: Entity) -> Option<&str> {
        self.entities
            .get(&entity)
            .map(|registration| registration.name.as_str())
    }

    #[must_use]
    fn role(&self, entity: Entity) -> Option<&RoleKey> {
        self.entities
            .get(&entity)
            .map(|registration| &registration.role)
    }

    pub(crate) fn roles(&self) -> impl Iterator<Item = &RoleKey> {
        self.entities
            .values()
            .map(|registration| &registration.role)
    }

    #[must_use]
    fn entity_for_role(&self, role: &RoleKey) -> Option<Entity> {
        self.entities
            .iter()
            .find_map(|(entity, registration)| (&registration.role == role).then_some(*entity))
    }

    fn register(&mut self, entity: Entity, name: String, role: RoleKey) {
        self.names.insert(name.clone());
        self.entities
            .insert(entity, ManagedWindowRegistration { name, role });
    }

    fn release(&mut self, entity: Entity) -> Option<RoleKey> {
        let registration = self.entities.remove(&entity)?;
        self.names.remove(&registration.name);
        Some(registration.role)
    }
}

/// Records whether Clerestory registered or rejected the one binding for a window entity.
///
/// The registered endpoint is a local dispatch projection of `Bindings`, retained only on the
/// window entity whose checked registration established it. It neither authorizes kernel work nor
/// replaces the role-owned binding.
#[derive(Component)]
pub(crate) enum WindowBindingAuthoring {
    /// The entity's stable role now owns the exact display endpoint in `Bindings`.
    Registered {
        /// Exact endpoint accepted by or reused from the checked binding registry.
        endpoint: DeviceEndpoint,
    },
    /// The role was invalid or checked registration rejected its endpoint; the error was reported.
    Rejected,
}

fn window_endpoint_id(role: &RoleKey) -> Result<EndpointId, PartNameError> {
    PartName::new(role.as_str()).map(EndpointId::Part)
}

enum WindowBindingRoleResolution {
    Available(RoleKey),
    AwaitingManagedRegistration,
    InvalidPrimaryRole,
}

enum WindowBindingRoleSource {
    Primary,
    Managed(Entity),
}

fn resolve_window_binding_role(
    source: WindowBindingRoleSource,
    registry: &ManagedWindowRegistry,
) -> WindowBindingRoleResolution {
    match source {
        WindowBindingRoleSource::Managed(entity) => registry.role(entity).cloned().map_or(
            WindowBindingRoleResolution::AwaitingManagedRegistration,
            WindowBindingRoleResolution::Available,
        ),
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
    role_source:     WindowBindingRoleSource,
    recovery:        RecoveryPolicy,
}

struct WindowBindingResources<'a> {
    registry:    &'a ManagedWindowRegistry,
    persisted:   &'a PersistedWindowPlacements,
    association: &'a MonitorDeviceAssociation,
    platform:    Platform,
    driver:      &'a WindowDriverId,
    bindings:    &'a mut Bindings,
}

fn author_window_binding(
    candidate: WindowBindingCandidate<'_>,
    resources: &mut WindowBindingResources<'_>,
) -> Option<WindowBindingAuthoring> {
    let role = match resolve_window_binding_role(candidate.role_source, resources.registry) {
        WindowBindingRoleResolution::Available(role) => role,
        WindowBindingRoleResolution::AwaitingManagedRegistration => return None,
        WindowBindingRoleResolution::InvalidPrimaryRole => {
            return Some(WindowBindingAuthoring::Rejected);
        },
    };
    if let Ok(binding) = resources.bindings.binding(&role) {
        return Some(WindowBindingAuthoring::Registered {
            endpoint: binding.endpoint.clone(),
        });
    }

    let persisted_state = resources.persisted.get(&role);
    let placement = persisted_state.map_or_else(
        || {
            let physical_position = match candidate.window.position {
                WindowPosition::At(position) => Some(IVec2::new(position.x, position.y)),
                _ => None,
            };
            crate::persistence::EstablishedWindowPlacement::from_readback(
                candidate.window,
                candidate.current_monitor,
                physical_position,
                resources.platform,
            )
        },
        crate::persistence::EstablishedWindowPlacement::from,
    );
    let device = match persisted_state {
        Some(state) => match &state.target {
            PersistedWindowTargetV5::Classified(device_key) => {
                MonitorDeviceKeyLookup::Exact(device_key.clone())
            },
            // `device_for_legacy_panel` matches a saved fingerprint against a live one, so an
            // `Anonymous` record — every v1 and v2 file, which `convert_v1_state_to_v4` and
            // `convert_v2_state_to_v4` write without panel evidence — can never match any monitor
            // and `PersistedWindowPlacements::resolve_pending` can never upgrade it. Adopting the
            // monitor the window already occupies completes that migration: the authored `Binding`
            // is what lets `write_established_window_configurations` rewrite the record as
            // `Classified`. `placement` above comes from the persisted state independently of the
            // device, so the saved position survives.
            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedPanelIdentityV4::Anonymous,
            ) => resources
                .association
                .device_for_descriptor(candidate.current_monitor.descriptor),
            PersistedWindowTargetV5::AwaitingLegacyEvidence(panel_identity) => resources
                .association
                .device_for_legacy_panel(*panel_identity),
        },
        None => resources
            .association
            .device_for_descriptor(candidate.current_monitor.descriptor),
    };
    let MonitorDeviceKeyLookup::Exact(device) = device else {
        debug!(
            "[author_window_bindings] role {role} is waiting for an exact reporter-issued display endpoint"
        );
        return None;
    };
    let endpoint_id = match window_endpoint_id(&role) {
        Ok(endpoint_id) => endpoint_id,
        Err(error) => {
            error!("[author_window_bindings] role {role} cannot name its window endpoint: {error}");
            return Some(WindowBindingAuthoring::Rejected);
        },
    };
    let endpoint = DeviceEndpoint {
        device,
        id: endpoint_id,
    };
    let binding = window_binding(
        role.clone(),
        endpoint.clone(),
        resources.driver.0,
        candidate.recovery,
        placement,
    );
    match resources.bindings.register(binding) {
        Ok(()) => Some(WindowBindingAuthoring::Registered { endpoint }),
        Err(error) => {
            error!(
                "[author_window_bindings] rejected role {role} as a Clerestory configuration error: {error}"
            );
            Some(WindowBindingAuthoring::Rejected)
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
            Has<PrimaryWindow>,
            Has<ManagedWindowReapplyOnRequest>,
        ),
        Without<WindowBindingAuthoring>,
    >,
    registry: Res<ManagedWindowRegistry>,
    persisted: Res<PersistedWindowPlacements>,
    association: Res<MonitorDeviceAssociation>,
    platform: Res<Platform>,
    driver: Res<WindowDriverId>,
    mut bindings: ResMut<Bindings>,
) {
    let mut resources = WindowBindingResources {
        registry:    &registry,
        persisted:   &persisted,
        association: &association,
        platform:    *platform,
        driver:      &driver,
        bindings:    &mut bindings,
    };
    for (entity, window, current_monitor, primary, reapply_on_request) in &windows {
        let role_source = if primary {
            WindowBindingRoleSource::Primary
        } else {
            WindowBindingRoleSource::Managed(entity)
        };
        let recovery = if reapply_on_request {
            RecoveryPolicy::ReapplyOnRequest
        } else {
            RecoveryPolicy::ReapplyOnReturn
        };
        let candidate = WindowBindingCandidate {
            window,
            current_monitor,
            role_source,
            recovery,
        };
        if let Some(authoring) = author_window_binding(candidate, &mut resources) {
            commands.entity(entity).insert(authoring);
        }
    }
}

/// Build the one kernel binding a Clerestory window role owns on a display endpoint.
///
/// `author_window_binding` and `rebind_window_to_its_current_display` both produce this value,
/// and a binding that differed between them would let a window keep authored intent from the
/// display it left. The state is `RoleState::Waiting` because both callers hand the binding to
/// `Bindings`, which owns every later transition.
fn window_binding(
    role: RoleKey,
    endpoint: DeviceEndpoint,
    driver: DriverId,
    recovery: RecoveryPolicy,
    placement: EstablishedWindowPlacement,
) -> Binding {
    Binding {
        role,
        endpoint,
        driver,
        recovery,
        retry: RetryOn::NewRevision,
        on_abort: OnAbort::default(),
        on_loss: OnSessionLoss::default(),
        state: RoleState::Waiting,
        requested: RequestedConfiguration::new(placement),
        last_known_good: LastKnownGoodConfiguration::NotEstablished,
        apply_deadline: ApplyDeadline::ProcessDefault,
    }
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
/// `Bindings::replace` returns the role to `RoleState::Waiting`, which dispatches one apply, and an
/// apply that targets the position the window already occupies cannot pull it off the display it
/// was dropped on.
pub(crate) fn rebind_window_to_its_current_display(
    monitor_changed: On<Insert, CurrentMonitor>,
    windows: Query<
        (
            &Window,
            &CurrentMonitor,
            Has<PrimaryWindow>,
            Has<ManagedWindowReapplyOnRequest>,
        ),
        With<WindowBindingAuthoring>,
    >,
    registry: Res<ManagedWindowRegistry>,
    association: Res<MonitorDeviceAssociation>,
    platform: Res<Platform>,
    driver: Res<WindowDriverId>,
    mut bindings: ResMut<Bindings>,
) {
    let entity = monitor_changed.entity;
    let Ok((window, current_monitor, primary, reapply_on_request)) = windows.get(entity) else {
        return;
    };
    let role_source = if primary {
        WindowBindingRoleSource::Primary
    } else {
        WindowBindingRoleSource::Managed(entity)
    };
    let WindowBindingRoleResolution::Available(role) =
        resolve_window_binding_role(role_source, &registry)
    else {
        return;
    };
    let Ok(existing) = bindings.binding(&role) else {
        return;
    };
    // No state guard: moving a window is itself what puts the role into `Applying`, so the role is
    // never `Ready` at the moment `CurrentMonitor` changes. `Bindings::replace` takes a reserved
    // transition slot and attempt results are isolated by `AttemptId`, so superseding the in-flight
    // attempt is the supported handoff rather than a stranded attempt.
    let MonitorDeviceKeyLookup::Exact(device) =
        association.device_for_descriptor(current_monitor.descriptor)
    else {
        return;
    };
    // The same display reporting new geometry also reinserts `CurrentMonitor`, which is not a move.
    if existing.endpoint.device == device {
        return;
    }
    let endpoint = DeviceEndpoint {
        device,
        id: existing.endpoint.id.clone(),
    };
    let recovery = if reapply_on_request {
        RecoveryPolicy::ReapplyOnRequest
    } else {
        RecoveryPolicy::ReapplyOnReturn
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
    let replacement = window_binding(role.clone(), endpoint, driver.0, recovery, placement);
    if let Err(error) = bindings.replace(replacement) {
        error!(
            "[rebind_window_to_its_current_display] role {role} could not follow its window to its current display: {error}"
        );
    }
}

/// Hand a stranded window's role to the display it is actually on, once the user moves it.
///
/// A window whose saved display is absent keeps a binding naming that display, so
/// `RecoveryPolicy::ReapplyOnReturn` carries it home when the display returns. The cost is that
/// the role stays in `RoleState::Waiting`, and `write_established_window_configurations` writes
/// nothing for a role that is not ready — so while the window is stranded nothing the user does to
/// it is saved, and the same absent display is waited for again on every launch.
///
/// Moving the window is the user overriding that pending return. Replacing the binding with one on
/// the live display lets the role resolve, which both cancels the return and reopens persistence,
/// so where the user put the window becomes the position that is remembered.
///
/// [`StrandedWindowPlacements`] is what separates the user's move from the fallback's own
/// placement; see its documentation for why the baseline is the window at rest rather than the
/// geometry the fallback requested.
pub(crate) fn adopt_live_display_for_stranded_window(
    windows: Query<
        (
            Entity,
            &Window,
            &CurrentMonitor,
            Has<PrimaryWindow>,
            Has<ManagedWindowReapplyOnRequest>,
        ),
        With<WindowBindingAuthoring>,
    >,
    registry: Res<ManagedWindowRegistry>,
    association: Res<MonitorDeviceAssociation>,
    platform: Res<Platform>,
    driver: Res<WindowDriverId>,
    mut bindings: ResMut<Bindings>,
    mut stranded: ResMut<StrandedWindowPlacements>,
) {
    if stranded.is_empty() {
        return;
    }
    for (entity, window, current_monitor, primary, reapply_on_request) in &windows {
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
        // Leaving `Waiting` means the kernel resolved the saved display and is carrying the window
        // home. The fallback placement stops being a baseline at that point, and reading the
        // arrival as a user move would rebind the role to the display it is being taken off.
        if existing.state != RoleState::Waiting {
            stranded.forget(&role);
            continue;
        }
        let MonitorDeviceKeyLookup::Exact(device) =
            association.device_for_descriptor(current_monitor.descriptor)
        else {
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
        if stranded.observe(&role, &live) == StrandedWindowObservation::Unmoved {
            continue;
        }
        let endpoint = DeviceEndpoint {
            device,
            id: endpoint_id,
        };
        let recovery = if reapply_on_request {
            RecoveryPolicy::ReapplyOnRequest
        } else {
            RecoveryPolicy::ReapplyOnReturn
        };
        let replacement = window_binding(role.clone(), endpoint, driver.0, recovery, live);
        if let Err(error) = bindings.replace(replacement) {
            error!(
                "[adopt_live_display_for_stranded_window] role {role} could not adopt the display its window now sits on: {error}"
            );
            continue;
        }
        stranded.forget(&role);
    }
}

/// Forget a role's captured configuration when its window's live geometry no longer matches it.
///
/// The kernel reads a role's configuration once, right after the role becomes ready, and
/// `write_established_window_configurations` persists only that captured value. A move or resize
/// that stays on one display changes nothing the kernel watches, so without this system the saved
/// record keeps the launch-time geometry forever. Forgetting the stale capture reopens the safe
/// readback: the kernel re-reads the window on the next pass and the persistence projection writes
/// the fresh value.
///
/// Cross-display moves are not this system's job — `rebind_window_to_its_current_display` replaces
/// the whole binding there, and the replacement's apply leaves the role outside `RoleState::Ready`
/// until it settles, which is exactly what the state guard here skips.
pub(crate) fn forget_stale_window_captures(
    windows: Query<
        (Entity, &Window, &CurrentMonitor, Has<PrimaryWindow>),
        (Changed<Window>, With<WindowBindingAuthoring>),
    >,
    registry: Res<ManagedWindowRegistry>,
    platform: Res<Platform>,
    mut bindings: ResMut<Bindings>,
) {
    for (entity, window, current_monitor, primary) in &windows {
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
        let Ok(binding) = bindings.as_ref().binding(&role) else {
            continue;
        };
        let LastKnownGoodConfiguration::Known(configuration) = &binding.last_known_good else {
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
        if binding.state != RoleState::Ready {
            continue;
        }
        bindings.forget_last_known_good(&role);
    }
}

/// Refresh registered window endpoint projections from their role-owned kernel bindings.
pub(crate) fn synchronize_registered_window_binding_projections(
    mut windows: Query<(Entity, Has<PrimaryWindow>, &mut WindowBindingAuthoring), With<Window>>,
    registry: Res<ManagedWindowRegistry>,
    bindings: Res<Bindings>,
) {
    for (entity, primary, mut authoring) in &mut windows {
        let WindowBindingAuthoring::Registered { endpoint } = &*authoring else {
            continue;
        };
        let source = if primary {
            WindowBindingRoleSource::Primary
        } else {
            WindowBindingRoleSource::Managed(entity)
        };
        let WindowBindingRoleResolution::Available(role) =
            resolve_window_binding_role(source, &registry)
        else {
            continue;
        };
        let Ok(binding) = bindings.binding(&role) else {
            continue;
        };
        if endpoint != &binding.endpoint {
            *authoring = WindowBindingAuthoring::Registered {
                endpoint: binding.endpoint.clone(),
            };
        }
    }
}

/// Locate the window entity whose retained binding role matches a kernel attempt.
pub(crate) fn window_entity_for_role(world: &mut World, role: &RoleKey) -> Option<Entity> {
    if persistence::primary_window_role().ok().as_ref() == Some(role) {
        let mut windows = world.query_filtered::<Entity, With<PrimaryWindow>>();
        return windows.iter(world).next();
    }
    world
        .resource::<ManagedWindowRegistry>()
        .entity_for_role(role)
}

/// Register and deduplicate a managed name before deriving its kernel role.
pub(crate) fn on_managed_window_added(
    add: On<Add, ManagedWindow>,
    mut commands: Commands,
    mut managed: Query<&mut ManagedWindow>,
    mut registry: ResMut<ManagedWindowRegistry>,
    primary_query: Query<(), With<PrimaryWindow>>,
) {
    let entity = add.entity;
    let Ok(mut managed_window) = managed.get_mut(entity) else {
        return;
    };
    let name = managed_window.name.clone();
    if registry.name(entity) == Some(name.as_str()) {
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
    let role = match persistence::managed_window_role(&unique_name) {
        Ok(role) => role,
        Err(error) => {
            warn!(
                "[on_managed_window_added] rejected managed window name {unique_name:?}: {error}"
            );
            commands.entity(entity).remove::<ManagedWindow>();
            return;
        },
    };
    managed_window.name.clone_from(&unique_name);
    registry.register(entity, unique_name, role);
}

/// Retire the kernel role when a managed window lifetime ends.
pub(crate) fn on_managed_window_removed(
    remove: On<Remove, ManagedWindow>,
    mut commands: Commands,
    mut registry: ResMut<ManagedWindowRegistry>,
    primary_windows: Query<(), With<PrimaryWindow>>,
) {
    if !primary_windows.contains(remove.entity) {
        commands
            .entity(remove.entity)
            .try_remove::<CurrentMonitor>();
    }
    let Some(role) = registry.release(remove.entity) else {
        return;
    };
    commands.trigger(RetireRole { role });
}

/// Retire the primary window role when its entity lifetime ends.
pub(crate) fn on_primary_window_removed(remove: On<Remove, PrimaryWindow>, mut commands: Commands) {
    let Ok(role) = persistence::primary_window_role() else {
        return;
    };
    commands
        .entity(remove.entity)
        .try_remove::<(CurrentMonitor, RestorePreparation, WindowApplyConfiguration)>();
    commands.trigger(RetireRole { role });
}

/// Prepare a newly-created managed window for the kernel-backed binding authoring path.
pub(crate) fn on_managed_window_load(
    add: On<Add, ManagedWindow>,
    mut commands: Commands,
    registry: Res<ManagedWindowRegistry>,
    mut windows: Query<(&mut Window, Option<&OnMonitor>)>,
    monitors: Res<Monitors>,
    platform: Res<Platform>,
) {
    let entity = add.entity;
    if registry.role(entity).is_none() {
        return;
    }
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
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::UVec2;
    use bevy::prelude::Update;
    use bevy::prelude::default;
    use bevy::window::WindowMode;
    use hana_rigging::prelude::ApplyPermit;
    use hana_rigging::prelude::AttemptId;
    use hana_rigging::prelude::AttemptProgress;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::CaptureOutcome;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::RiggingSystems;

    use super::*;
    use crate::MonitorDescriptor;
    use crate::persistence::EstablishedWindowPosition;
    use crate::persistence::SavedWindowMode;

    #[derive(Default, Resource)]
    struct RetiredRoles(Vec<RoleKey>);

    #[derive(Component, Reflect)]
    struct ProjectionBindingConfiguration;

    struct ProjectionBindingDriver;

    impl EndpointDriver for ProjectionBindingDriver {
        type Configuration = ProjectionBindingConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            CaptureOutcome::Read(ProjectionBindingConfiguration)
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
            AttemptProgress::Pending
        }
    }

    fn record_retired_role(retired: On<RetireRole>, mut roles: ResMut<RetiredRoles>) {
        roles.0.push(retired.role.clone());
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
            app.add_plugins(RiggingPlugin);
            let driver = app.add_endpoint_driver(ProjectionBindingDriver);
            app.insert_resource(WindowDriverId(driver))
                .init_resource::<ManagedWindowRegistry>()
                .insert_resource(Platform::detect())
                .init_resource::<StrandedWindowPlacements>()
                .insert_resource(MonitorDeviceAssociation::from_test_live_displays([(
                    live_device,
                    descriptor,
                )]))
                .add_systems(Update, adopt_live_display_for_stranded_window);

            let binding = window_binding(
                role.clone(),
                DeviceEndpoint {
                    device: absent_device,
                    id:     endpoint_id,
                },
                driver,
                RecoveryPolicy::ReapplyOnReturn,
                EstablishedWindowPlacement {
                    position:          EstablishedWindowPosition::Restorable {
                        logical_offset: IVec2::new(0, 30),
                    },
                    logical_size:      UVec2::new(800, 600),
                    saved_window_mode: SavedWindowMode::Windowed,
                },
            );
            let endpoint = binding.endpoint.clone();
            app.world_mut()
                .resource_mut::<Bindings>()
                .register(binding)
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
                    WindowBindingAuthoring::Registered { endpoint },
                ))
                .id();

            app.world_mut()
                .resource_mut::<StrandedWindowPlacements>()
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

    /// The window does not land where the fallback asked: macOS clamps it below the menu bar, and
    /// winit reports the real coordinate a frame or two later. That writeback is the fallback's own
    /// placement finishing, not the user moving the window, and reading it as a move spends the one
    /// adoption on nobody's decision.
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
                .resource::<StrandedWindowPlacements>()
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
            .init_resource::<RetiredRoles>()
            .add_observer(on_managed_window_added)
            .add_observer(on_managed_window_removed)
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
    fn registered_projection_tracks_authoritative_endpoint_after_identity_adoption()
    -> Result<(), String> {
        let role = persistence::primary_window_role()
            .map_err(|error| format!("failed to create primary role: {error}"))?;
        let endpoint_id = window_endpoint_id(&role)
            .map_err(|error| format!("failed to create primary endpoint ID: {error}"))?;
        let saved_device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("saved-display")
                    .map_err(|error| format!("failed to create saved device ID: {error}"))?,
            },
        };
        let adopted_device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("adopted-display")
                    .map_err(|error| format!("failed to create adopted device ID: {error}"))?,
            },
        };
        let stale_endpoint = DeviceEndpoint {
            device: saved_device,
            id:     endpoint_id.clone(),
        };
        let authoritative_endpoint = DeviceEndpoint {
            device: adopted_device,
            id:     endpoint_id,
        };
        let mut app = App::new();
        let driver = app.add_endpoint_driver(ProjectionBindingDriver);
        let binding = Binding {
            role,
            endpoint: authoritative_endpoint.clone(),
            driver,
            recovery: RecoveryPolicy::ReapplyOnReturn,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::Waiting,
            requested: RequestedConfiguration::new(ProjectionBindingConfiguration),
            last_known_good: LastKnownGoodConfiguration::NotEstablished,
            apply_deadline: ApplyDeadline::ProcessDefault,
        };
        app.add_plugins(RiggingPlugin)
            .init_resource::<ManagedWindowRegistry>()
            .add_systems(
                Update,
                synchronize_registered_window_binding_projections
                    .after(RiggingSystems::Reconcile)
                    .in_set(RiggingSystems::Prepare),
            );
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(binding)
            .map_err(|error| format!("failed to register adopted binding: {error}"))?;
        let entity = app
            .world_mut()
            .spawn((
                Window::default(),
                PrimaryWindow,
                WindowBindingAuthoring::Registered {
                    endpoint: stale_endpoint.clone(),
                },
            ))
            .id();

        app.update();

        let projection = app
            .world()
            .get::<WindowBindingAuthoring>(entity)
            .ok_or_else(|| String::from("primary window lost its binding projection"))?;
        let WindowBindingAuthoring::Registered { endpoint } = projection else {
            return Err(String::from("registered projection became rejected"));
        };
        assert_eq!(endpoint, &authoritative_endpoint);
        assert_ne!(endpoint, &stale_endpoint);
        Ok(())
    }

    #[test]
    fn duplicate_managed_names_are_canonicalized_before_role_derivation() {
        let mut app = managed_lifecycle_app();
        let first = app
            .world_mut()
            .spawn(ManagedWindow {
                name: "inspector".into(),
            })
            .id();
        let second = app
            .world_mut()
            .spawn(ManagedWindow {
                name: "inspector".into(),
            })
            .id();

        let first_name = app
            .world()
            .get::<ManagedWindow>(first)
            .map(|window| window.name.clone());
        let second_name = app
            .world()
            .get::<ManagedWindow>(second)
            .map(|window| window.name.clone());
        assert_eq!(first_name.as_deref(), Some("inspector"));
        assert!(
            second_name
                .as_deref()
                .is_some_and(|name| name != "inspector")
        );
        assert_ne!(first_name, second_name);
        let registry = app.world().resource::<ManagedWindowRegistry>();
        assert_eq!(registry.names.len(), 2);
        assert_eq!(
            registry.role(first),
            first_name
                .as_deref()
                .and_then(|name| persistence::managed_window_role(name).ok())
                .as_ref()
        );
        assert_eq!(
            registry.role(second),
            second_name
                .as_deref()
                .and_then(|name| persistence::managed_window_role(name).ok())
                .as_ref()
        );
    }

    #[test]
    fn primary_window_is_not_registered_as_a_second_managed_role() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn((
                PrimaryWindow,
                ManagedWindow {
                    name: "inspector".into(),
                },
            ))
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
            .spawn(ManagedWindow {
                name: "inspector\nwindow".into(),
            })
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
    fn registration_retains_the_role_derived_from_the_managed_name() {
        let mut app = managed_lifecycle_app();
        let role = persistence::managed_window_role("inspector");
        assert!(role.is_ok());
        let Ok(role) = role else {
            return;
        };
        let entity = app
            .world_mut()
            .spawn(ManagedWindow {
                name: "inspector".into(),
            })
            .id();

        assert_eq!(
            app.world().resource::<ManagedWindowRegistry>().role(entity),
            Some(&role)
        );
    }

    #[test]
    fn managed_removal_retires_the_kernel_role_once() {
        let mut app = managed_lifecycle_app();
        let entity = app
            .world_mut()
            .spawn(ManagedWindow {
                name: "inspector".into(),
            })
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
}
