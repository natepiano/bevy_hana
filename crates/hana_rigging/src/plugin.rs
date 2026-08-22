use bevy::app::App;
use bevy::app::Plugin;
use bevy::app::Update;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::SystemSet;
use bevy::prelude::World;
use bevy::time::Real;
use bevy::time::Time;

use crate::Attempts;
use crate::BindingEntities;
use crate::Bindings;
use crate::Devices;
use crate::DiscoveryControl;
use crate::DiscoveryLimits;
use crate::DiscoveryStatus;
use crate::HardwareInventory;
use crate::IdentityDecisions;
use crate::RegisteredSchemes;
use crate::RiggingLimits;
use crate::RiggingRevision;
use crate::SessionLossReports;
use crate::apply;
use crate::binding;
use crate::binding::BindingTransitionBatch;
use crate::devices::DepartureAnnouncements;
use crate::devices::ReconciledDeviceChanges;
use crate::discovery::DiscoveryTransitionJournal;
use crate::emit;
use crate::emit::AnnouncedEdges;
use crate::identity_decisions;
use crate::reconcile;
use crate::registration::Drivers;
use crate::registration::Reporters;

/// Installs the reporter collection system and the ordering sets that integration crates use.
pub struct RiggingPlugin;

/// Ordered kernel phases in the `Update` schedule.
///
/// This enum is non-exhaustive because later kernel phases can add an ordering boundary without
/// making downstream `RiggingSystems` matches incomplete.
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
#[non_exhaustive]
pub enum RiggingSystems {
    /// Discovery cadence, task admission, and bounded completed reports run before reconciliation.
    Collect,
    /// Reconciliation follows the current collect pass and consumes each independently accepted
    /// reporter change; it does not wait for a global reporter completion barrier.
    Reconcile,
    /// Consumer systems prepare their requested configurations after authorization but before any
    /// endpoint driver begins an apply.
    Prepare,
    /// Guarded established-session failures are processed after consumer preparation and before
    /// any replacement apply can be selected or dispatched.
    SessionLoss,
    /// Kernel attempt systems later start, poll, retry, and escalate endpoint applies.
    Apply,
}

impl Plugin for RiggingPlugin {
    fn build(&self, app: &mut App) {
        // `reconcile` reads `Time<Real>` for the freshness lease, and a missing resource makes
        // Bevy skip the system without reporting anything. The resource is initialized here rather
        // than by adding `TimePlugin`, because `DefaultPlugins` and `MinimalPlugins` both panic on
        // a `TimePlugin` that is already installed, which would make plugin order decide whether an
        // application starts.
        //
        // `Drivers` is installed by hand rather than by `init_resource` because it deliberately has
        // no `Default`, and get-or-insert rather than `insert_resource` because a
        // driver-registering plugin may be added before this one; overwriting here would
        // unregister its driver.
        app.world_mut().get_resource_or_insert_with(Drivers::new);
        app.init_resource::<Time<Real>>()
            .init_resource::<Attempts>()
            .init_resource::<BindingEntities>()
            .init_resource::<AnnouncedEdges>()
            .init_resource::<BindingTransitionBatch>()
            .init_resource::<Bindings>()
            .init_resource::<DepartureAnnouncements>()
            .init_resource::<Devices>()
            .init_resource::<DiscoveryControl>()
            .init_resource::<DiscoveryLimits>()
            .init_resource::<DiscoveryStatus>()
            .init_resource::<DiscoveryTransitionJournal>()
            .init_resource::<HardwareInventory>()
            .init_resource::<IdentityDecisions>()
            .init_resource::<ReconciledDeviceChanges>()
            .init_resource::<RegisteredSchemes>()
            .init_resource::<Reporters>()
            .init_resource::<RiggingLimits>()
            .init_resource::<RiggingRevision>()
            .init_resource::<SessionLossReports>()
            .configure_sets(
                Update,
                (
                    RiggingSystems::Collect,
                    RiggingSystems::Reconcile,
                    RiggingSystems::Prepare,
                    RiggingSystems::SessionLoss,
                    RiggingSystems::Apply,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    collect.in_set(RiggingSystems::Collect),
                    (
                        binding::drain_binding_transitions,
                        binding::project_binding_entities,
                    )
                        .chain()
                        .before(reconcile::reconcile)
                        .in_set(RiggingSystems::Reconcile),
                    reconcile::reconcile.in_set(RiggingSystems::Reconcile),
                    reconcile::project_device_entities
                        .after(reconcile::reconcile)
                        .in_set(RiggingSystems::Reconcile),
                    identity_decisions::adjudicate_identity_questions
                        .after(reconcile::project_device_entities)
                        .before(emit::announce_role_availability)
                        .in_set(RiggingSystems::Reconcile),
                    (
                        emit::announce_device_changes,
                        emit::announce_binding_changes,
                        emit::announce_reconciled_facts,
                        emit::announce_role_availability,
                    )
                        .after(reconcile::project_device_entities)
                        .in_set(RiggingSystems::Reconcile),
                    emit::announce_discovery_transitions
                        .after(emit::announce_role_availability)
                        .in_set(RiggingSystems::Reconcile),
                    apply::process_session_loss_reports.in_set(RiggingSystems::SessionLoss),
                    (
                        apply::abort_invalidated_attempts,
                        apply::poll_attempts,
                        apply::start_authorized_applies,
                    )
                        .chain()
                        .in_set(RiggingSystems::Apply),
                    apply::clear_binding_transitions.after(RiggingSystems::Apply),
                ),
            )
            .add_observer(emit::on_retire_role)
            .add_observer(emit::on_reapply_configuration)
            .add_observer(emit::on_role_awaiting);
    }
}

fn collect(world: &mut World) {
    world.resource_scope::<Reporters, _>(|world, mut reporters| {
        let discovery_limits = world.resource::<DiscoveryLimits>().clone();
        world.resource_scope::<DiscoveryControl, _>(|world, mut discovery_control| {
            world.resource_scope::<DiscoveryStatus, _>(|world, mut discovery_status| {
                world.resource_scope::<DiscoveryTransitionJournal, _>(
                    |world, mut discovery_transition_journal| {
                        reporters.collect(
                            world,
                            &mut discovery_control,
                            &discovery_limits,
                            &mut discovery_status,
                            &mut discovery_transition_journal,
                        );
                    },
                );
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use bevy::MinimalPlugins;
    use bevy::app::App;
    use bevy::app::Update;
    use bevy::ecs::schedule::IntoScheduleConfigs;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::time::Real;
    use bevy::time::Time;

    use super::RiggingPlugin;
    use super::RiggingSystems;

    #[derive(Default, Resource)]
    struct StageLog(Vec<&'static str>);

    fn reconcile(mut stage_log: ResMut<StageLog>) { stage_log.0.push("reconcile"); }

    fn prepare(mut stage_log: ResMut<StageLog>) {
        assert_eq!(stage_log.0.as_slice(), ["reconcile"]);
        stage_log.0.push("prepare");
    }

    fn session_loss(mut stage_log: ResMut<StageLog>) {
        assert_eq!(stage_log.0.as_slice(), ["reconcile", "prepare"]);
        stage_log.0.push("session-loss");
    }

    fn apply(mut stage_log: ResMut<StageLog>) {
        assert_eq!(
            stage_log.0.as_slice(),
            ["reconcile", "prepare", "session-loss"]
        );
        stage_log.0.push("apply");
    }

    #[test]
    fn session_loss_runs_after_prepare_and_before_apply() {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin)
            .init_resource::<StageLog>()
            .add_systems(
                Update,
                (
                    reconcile.in_set(RiggingSystems::Reconcile),
                    prepare.in_set(RiggingSystems::Prepare),
                    session_loss.in_set(RiggingSystems::SessionLoss),
                    apply.in_set(RiggingSystems::Apply),
                ),
            );

        app.update();

        assert_eq!(
            app.world().resource::<StageLog>().0.as_slice(),
            ["reconcile", "prepare", "session-loss", "apply"]
        );
    }

    #[test]
    fn an_application_may_add_the_plugin_before_its_default_plugins() {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin).add_plugins(MinimalPlugins);

        app.update();

        // `MinimalPlugins` brings `TimePlugin`, which panics if the kernel had claimed it first.
        // The lease still has the resource it reads either way.
        assert!(app.world().get_resource::<Time<Real>>().is_some());
    }

    #[test]
    fn a_bare_app_still_gets_the_clock_the_freshness_lease_reads() {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);

        app.update();

        assert!(app.world().get_resource::<Time<Real>>().is_some());
    }
}
