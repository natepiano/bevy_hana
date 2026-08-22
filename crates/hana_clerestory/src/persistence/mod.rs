//! Window-state serialization adapter and filesystem I/O.

mod constants;
mod established_window_placement;
mod format;
mod load;
mod save;
mod window_state;

use std::collections::HashMap;

use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreStartup;
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
use hana_rigging::prelude::RegisteredSchemes;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleKeyError;
pub(crate) use load::get_default_state_path;
pub(crate) use load::get_state_path_for_app;
#[cfg(test)]
pub(crate) use window_state::PersistedPanelFingerprintV4;
pub(crate) use window_state::PersistedPanelIdentityV4;
pub(crate) use window_state::PersistedPosition;
pub(crate) use window_state::PersistedWindowState;
pub(crate) use window_state::PersistedWindowTargetV5;
#[cfg(test)]
pub(crate) use window_state::SavedFullscreenVideoMode;
pub(crate) use window_state::SavedWindowMode;
#[cfg(test)]
pub(crate) use window_state::UnrebasedDesktopPosition;

use crate::ClerestoryPreStartupSet;
use crate::ClerestoryUpdateSet;
use crate::monitors::MonitorDeviceAssociation;

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
                    resolve_pending_legacy_targets,
                    save::write_established_window_configurations,
                )
                    .chain()
                    .in_set(ClerestoryUpdateSet::Persistence),
            );
    }
}

pub(crate) fn primary_window_role() -> Result<RoleKey, RoleKeyError> {
    RoleKey::new(PRIMARY_WINDOW_ROLE)
}

pub(crate) fn managed_window_role(name: &str) -> Result<RoleKey, RoleKeyError> {
    RoleKey::new(format!("{MANAGED_WINDOW_ROLE_PREFIX}{name}"))
}

/// RON records loaded for startup preparation before a window binding exists.
///
/// This resource is an adapter cache only. Once a binding exists, its
/// `LastKnownGoodConfiguration` is the captured-configuration authority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StartupLoadStatus {
    #[default]
    Pending,
    Read,
}

#[derive(Default, Resource)]
pub(crate) struct PersistedWindowPlacements {
    states:              HashMap<RoleKey, PersistedWindowState>,
    startup_load_status: StartupLoadStatus,
    requires_save:       bool,
}

impl PersistedWindowPlacements {
    fn seed(&mut self, states: HashMap<RoleKey, PersistedWindowState>) {
        if self.startup_load_status == StartupLoadStatus::Read {
            return;
        }
        self.states = states;
        self.startup_load_status = StartupLoadStatus::Read;
        self.requires_save = false;
    }

    #[must_use]
    const fn startup_load_status(&self) -> StartupLoadStatus { self.startup_load_status }

    #[must_use]
    pub(crate) fn get(&self, role: &RoleKey) -> Option<&PersistedWindowState> {
        self.states.get(role)
    }

    fn replace_serialized(&mut self, states: HashMap<RoleKey, PersistedWindowState>) {
        self.states = states;
    }

    #[must_use]
    const fn requires_save(&self) -> bool { self.requires_save }

    const fn mark_saved(&mut self) { self.requires_save = false; }

    /// Adopt reporter keys for retained v4 evidence after an exact live association appears.
    ///
    /// A rejected fresh key removes only the affected record. Every other persisted role stays
    /// available for its own later association or normal save.
    fn resolve_pending(
        &mut self,
        association: &MonitorDeviceAssociation,
        registered_schemes: &RegisteredSchemes,
    ) {
        let mut changed = false;
        self.states.retain(|role, state| {
            let PersistedWindowTargetV5::AwaitingLegacyEvidence(panel_identity) = &state.target
            else {
                return true;
            };
            match format::resolve_legacy_target(*panel_identity, association, registered_schemes) {
                PersistedWindowIdentityMigrationOutcome::Resolved(device_key) => {
                    state.target = PersistedWindowTargetV5::Classified(device_key);
                    changed = true;
                    true
                },
                PersistedWindowIdentityMigrationOutcome::AwaitingLiveEvidence(_) => true,
                PersistedWindowIdentityMigrationOutcome::Rejected(error) => {
                    warn!(
                        "[resolve_pending_legacy_targets] Rejected retained role {role}: {error}"
                    );
                    changed = true;
                    false
                },
            }
        });
        self.requires_save |= changed;
    }

    fn iter(&self) -> impl Iterator<Item = (&RoleKey, &PersistedWindowState)> { self.states.iter() }
}

/// Resolve retained legacy targets only through the ready monitor association.
fn resolve_pending_legacy_targets(
    association: Res<MonitorDeviceAssociation>,
    registered_schemes: Res<RegisteredSchemes>,
    mut persisted: ResMut<PersistedWindowPlacements>,
) {
    persisted.resolve_pending(&association, &registered_schemes);
}

#[cfg(test)]
pub(crate) fn decode_persisted_state_for_test(contents: &str) -> PersistedWindowStateDecodeOutcome {
    load::decode_roles(
        contents,
        &MonitorDeviceAssociation::default(),
        &RegisteredSchemes::default(),
    )
}
