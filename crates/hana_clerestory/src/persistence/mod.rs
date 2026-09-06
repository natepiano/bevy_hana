//! Window-state serialization adapter and filesystem I/O.

mod constants;
mod established_window_placement;
mod format;
mod load;
mod save;
mod window_state;

use std::collections::HashMap;
use std::collections::HashSet;

use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreStartup;
#[cfg(test)]
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Update;
use bevy::prelude::warn;
pub(crate) use established_window_placement::EstablishedWindowPlacement;
pub(crate) use established_window_placement::EstablishedWindowPosition;
pub(crate) use established_window_placement::RestorableWindowPosition;
pub(crate) use format::PersistedWindowIdentityMigrationOutcome;
pub(crate) use format::PersistedWindowStateDecodeOutcome;
use hana_rigging::prelude::BindingPolicy;
#[cfg(test)]
use hana_rigging::prelude::Bindings;
#[cfg(test)]
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::RegisteredSchemes;
use hana_rigging::prelude::RiggingSystems;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleKeyError;
#[cfg(test)]
use hana_rigging::prelude::RoleStatus;
pub(crate) use load::get_default_state_path;
pub(crate) use load::get_state_path_for_app;
pub(crate) use window_state::LoadedBindingPolicy;
#[cfg(test)]
pub(crate) use window_state::PersistedDisplayFingerprintV4;
pub(crate) use window_state::PersistedDisplayIdentityV4;
pub(crate) use window_state::PersistedPosition;
pub(crate) use window_state::PersistedWindowPlacement;
pub(crate) use window_state::PersistedWindowPlacementLookup;
pub(crate) use window_state::PersistedWindowState;
pub(crate) use window_state::PersistedWindowTargetV5;
#[cfg(test)]
pub(crate) use window_state::SavedFullscreenVideoMode;
pub(crate) use window_state::SavedWindowMode;
#[cfg(test)]
pub(crate) use window_state::UnrebasedDesktopPosition;

use crate::ClerestoryPreStartupSet;
use crate::ClerestoryUpdateSet;
#[cfg(test)]
use crate::managed::ManagedWindowRegistry;
use crate::monitors::LiveDisplayEndpointLookup;
#[cfg(test)]
use crate::restore_window_config::RestoreWindowConfig;

pub(super) const PRIMARY_WINDOW_ROLE: &str = "window:primary";
pub(super) const MANAGED_WINDOW_ROLE_PREFIX: &str = "window:managed:";

pub(crate) struct PersistencePlugin;

impl Plugin for PersistencePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PersistedWindowPlacements>()
            .add_systems(
                PreStartup,
                load::load_persisted_window_placements
                    .in_set(ClerestoryPreStartupSet::PersistenceLoaded),
            )
            .add_systems(
                Update,
                (
                    resolve_pending_legacy_targets.after(RiggingSystems::Reconcile),
                    save::write_established_window_configurations,
                )
                    .chain()
                    .in_set(ClerestoryUpdateSet::Persistence),
            );
    }
}

/// Builds the stable persistence role for the application's primary window.
///
/// # Errors
///
/// Returns [`RoleKeyError`] if Clerestory's built-in primary role spelling is invalid.
pub fn primary_window_role() -> Result<RoleKey, RoleKeyError> { RoleKey::new(PRIMARY_WINDOW_ROLE) }

/// Builds the stable persistence role for a managed window.
///
/// Pass the canonicalized [`crate::ManagedWindowName`] stored on the managed entity. Duplicate
/// registration can rewrite that component value, so callers observing an existing window should
/// use its current component name instead of the originally requested name.
///
/// # Errors
///
/// Returns [`RoleKeyError`] when the managed-window prefix plus `name` is not a valid role key.
pub fn managed_window_role(name: &str) -> Result<RoleKey, RoleKeyError> {
    RoleKey::new(format!("{MANAGED_WINDOW_ROLE_PREFIX}{name}"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StartupLoadStatus {
    #[default]
    Pending,
    Read,
}

/// Roles that have had serialized window configuration during this process.
///
/// `PersistedWindowPlacements::seed` initializes this record from loaded keys, and
/// `PersistedWindowPlacements::replace_serialized` unions each successful projection's keys before
/// replacing `PersistedWindowPlacements::states`. Those are the only writers of `states`, so every
/// key in `states` is always present here.
///
/// This session record deliberately grows only. An `ActiveOnly` projection can remove a role from
/// `states` after its window is no longer managed, but that removal cannot mean the role was never
/// saved. No removal path should be added. A same-name role remanaged during this process is
/// therefore treated as returning rather than new.
#[derive(Default)]
struct RolesWithSavedConfiguration {
    roles: HashSet<RoleKey>,
}

impl RolesWithSavedConfiguration {
    fn contains(&self, role: &RoleKey) -> bool { self.roles.contains(role) }

    fn record_serialized_roles(&mut self, states: &HashMap<RoleKey, PersistedWindowPlacement>) {
        self.roles.extend(states.keys().cloned());
    }
}

impl From<&HashMap<RoleKey, PersistedWindowPlacement>> for RolesWithSavedConfiguration {
    fn from(states: &HashMap<RoleKey, PersistedWindowPlacement>) -> Self {
        Self {
            roles: states.keys().cloned().collect(),
        }
    }
}

/// Mutable RON projection and session saved-configuration evidence.
///
/// This resource is an adapter cache only. Once a binding exists, its
/// `LastKnownGoodConfiguration` is the captured-configuration authority.
///
/// `states` follows successful write projections and can shrink after window management ends under
/// `ManagedWindowPersistence::ActiveOnly`. `roles_with_saved_configuration` retains the roles
/// loaded or successfully projected during this process.
#[derive(Default, Resource)]
pub(crate) struct PersistedWindowPlacements {
    states:                         HashMap<RoleKey, PersistedWindowPlacement>,
    roles_with_saved_configuration: RolesWithSavedConfiguration,
    startup_load_status:            StartupLoadStatus,
    pending_write:                  PendingWrite,
}

/// Whether the projection this process holds still owes the file a write.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum PendingWrite {
    #[default]
    Settled,
    Owed,
}

impl PersistedWindowPlacements {
    pub(crate) fn seed<Value>(&mut self, states: HashMap<RoleKey, Value>)
    where
        Value: Into<PersistedWindowPlacement>,
    {
        if self.startup_load_status == StartupLoadStatus::Read {
            return;
        }
        let states: HashMap<_, _> = states
            .into_iter()
            .map(|(role, value)| (role, value.into()))
            .collect();
        self.roles_with_saved_configuration = RolesWithSavedConfiguration::from(&states);
        self.states = states;
        self.startup_load_status = StartupLoadStatus::Read;
        self.pending_write = PendingWrite::Settled;
    }

    #[must_use]
    const fn startup_load_status(&self) -> StartupLoadStatus { self.startup_load_status }

    #[must_use]
    pub(crate) fn get(&self, role: &RoleKey) -> PersistedWindowPlacementLookup<'_> {
        self.states.get(role).map_or(
            PersistedWindowPlacementLookup::NotSaved,
            PersistedWindowPlacementLookup::Saved,
        )
    }

    /// Whether this process has loaded or written a saved configuration for `role`.
    #[must_use]
    pub(crate) fn has_saved_configuration(&self, role: &RoleKey) -> bool {
        self.roles_with_saved_configuration.contains(role)
    }

    fn replace_serialized(&mut self, states: HashMap<RoleKey, PersistedWindowPlacement>) {
        self.roles_with_saved_configuration
            .record_serialized_roles(&states);
        self.states = states;
    }

    #[must_use]
    const fn pending_write(&self) -> PendingWrite { self.pending_write }

    const fn mark_written(&mut self) { self.pending_write = PendingWrite::Settled; }

    pub(crate) fn resolve_loaded_policy(&mut self, role: &RoleKey, binding_policy: BindingPolicy) {
        let Some(placement) = self.states.get_mut(role) else {
            return;
        };
        placement.loaded_binding_policy = LoadedBindingPolicy::Saved(binding_policy);
        self.pending_write = PendingWrite::Owed;
    }

    /// Adopt reporter keys for retained v4 evidence after an exact live association appears.
    ///
    /// A rejected fresh key removes only the affected record. Every other persisted role stays
    /// available for its own later association or normal save.
    fn resolve_pending(
        &mut self,
        live_displays: &LiveDisplayEndpointLookup,
        registered_schemes: &RegisteredSchemes,
    ) {
        let mut pending_write = self.pending_write;
        self.states.retain(|role, placement| {
            let PersistedWindowTargetV5::AwaitingLegacyEvidence(legacy_identity) =
                &placement.window_state.target
            else {
                return true;
            };
            match format::resolve_legacy_target(*legacy_identity, live_displays, registered_schemes)
            {
                PersistedWindowIdentityMigrationOutcome::Resolved(device_key) => {
                    placement.window_state.target = PersistedWindowTargetV5::Classified(device_key);
                    pending_write = PendingWrite::Owed;
                    true
                },
                PersistedWindowIdentityMigrationOutcome::AwaitingLiveEvidence => true,
                PersistedWindowIdentityMigrationOutcome::Rejected(error) => {
                    warn!(
                        "[resolve_pending_legacy_targets] Rejected retained role {role}: {error}"
                    );
                    pending_write = PendingWrite::Owed;
                    false
                },
            }
        });
        self.pending_write = pending_write;
    }

    fn iter(&self) -> impl Iterator<Item = (&RoleKey, &PersistedWindowPlacement)> {
        self.states.iter()
    }
}

/// Resolve retained legacy targets only through the ready monitor association.
fn resolve_pending_legacy_targets(
    live_displays: LiveDisplayEndpointLookup,
    registered_schemes: Res<RegisteredSchemes>,
    mut persisted: ResMut<PersistedWindowPlacements>,
) {
    persisted.resolve_pending(&live_displays, &registered_schemes);
}

/// Crate-visible handle on the module-private save system for tests outside this module.
#[cfg(test)]
pub(crate) fn write_established_window_configurations_for_test(
    config: Res<RestoreWindowConfig>,
    bindings: Res<Bindings>,
    inventory: Res<HardwareInventory>,
    role_statuses: Query<(&RoleKey, &RoleStatus)>,
    managed_windows: Res<ManagedWindowRegistry>,
    retention: Res<crate::ManagedWindowPersistence>,
    persisted: ResMut<PersistedWindowPlacements>,
) {
    save::write_established_window_configurations(
        config,
        bindings,
        inventory,
        role_statuses,
        managed_windows,
        retention,
        persisted,
    );
}

#[cfg(test)]
pub(crate) fn decode_persisted_state_for_test(contents: &str) -> PersistedWindowStateDecodeOutcome {
    load::decode_roles(contents, &RegisteredSchemes::default())
}
