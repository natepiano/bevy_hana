use std::time::Instant;

use bevy::app::App;
use bevy::app::Plugin;
use bevy::app::Update;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::SystemSet;
use bevy::log::warn;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::World;
use bevy::time::Real;
use bevy::time::Time;

use crate::Attempts;
use crate::Bindings;
use crate::Devices;
use crate::DiscoveryControl;
use crate::DiscoveryStatus;
use crate::HardwareInventory;
use crate::IdentityDecisions;
use crate::KernelRolePresentation;
use crate::RegisteredSchemes;
use crate::RiggingLimits;
use crate::RiggingRevision;
use crate::RiggingRuntimeClock;
use crate::apply;
use crate::apply::PendingAttemptEndingPublications;
use crate::binding;
use crate::binding::BindingTransitionBatch;
use crate::binding::LostRegisteredRoleEntities;
use crate::binding::LostRoleRelationships;
use crate::binding::RiggingRoleRelationshipRepairs;
use crate::contract::DriverReports;
use crate::devices::DepartureGraceDeadlineStatus;
use crate::devices::DeviceChangeAnnouncements;
use crate::devices::ReconciledDeviceChanges;
use crate::discovery::DiscoveryLimits;
use crate::discovery::DiscoveryTransitionJournal;
use crate::emit;
use crate::emit::ReporterWaitDiscoveryRequests;
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
    /// Role entities and enrolled relationships have been repaired for this frame.
    RoleEntityRecovery,
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
        install_resources(app);
        // `RolePresentation` is generic, and Bevy's `reflect_auto_register` submits only
        // non-generic types, so this concrete alias is invisible to the type registry — and
        // therefore to BRP and to every scene or serialization path — unless it is named here.
        // `ContinuousFlowExpiryCause` and the rest of the crate's reflected types are
        // non-generic and need no such call.
        app.register_type::<KernelRolePresentation>();
        app.configure_sets(
            Update,
            (
                RiggingSystems::Collect,
                RiggingSystems::RoleEntityRecovery,
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
                refresh_driver_report_time.in_set(RiggingSystems::Collect),
                collect_departure_grace_deadlines
                    .after(refresh_driver_report_time)
                    .in_set(RiggingSystems::Collect),
                collect
                    .after(collect_departure_grace_deadlines)
                    .in_set(RiggingSystems::Collect),
                write_reporter_health
                    .after(collect)
                    .in_set(RiggingSystems::Collect),
                cross_role_wait_bounds
                    .after(write_reporter_health)
                    .in_set(RiggingSystems::Collect),
                (
                    binding::drain_binding_transitions,
                    binding::reconcile_role_entities,
                    apply::cleanup_binding_transition_driver_state,
                    binding::retire_role_entities,
                )
                    .chain()
                    .before(reconcile::reconcile)
                    .in_set(RiggingSystems::RoleEntityRecovery),
                reconcile::reconcile.in_set(RiggingSystems::Reconcile),
                // Adjudication's `Bindings::readdress` replaces the bound device key outright,
                // so it must not land between the status write and the presentation derived
                // from it: the derivation would read availability under the adopted key while
                // the `RoleStatus` beside it still described the pre-adoption wait. Ordering the
                // whole publication chain ahead of adjudication keeps the pair coherent; the
                // adoption's own edges publish at the `Apply` boundary, which republishes both.
                (
                    reconcile::project_device_entities,
                    emit::publish_role_status_changes,
                    emit::publish_role_presentation_changes,
                )
                    .chain()
                    .after(reconcile::reconcile)
                    .before(identity_decisions::adjudicate_identity_questions)
                    .in_set(RiggingSystems::Reconcile),
                identity_decisions::adjudicate_identity_questions
                    .after(reconcile::project_device_entities)
                    .before(emit::announce_reconciled_facts)
                    .in_set(RiggingSystems::Reconcile),
                (
                    emit::announce_device_changes,
                    emit::announce_reconciled_facts,
                )
                    .after(reconcile::project_device_entities)
                    .in_set(RiggingSystems::Reconcile),
                emit::announce_discovery_transitions
                    .after(emit::announce_reconciled_facts)
                    .in_set(RiggingSystems::Reconcile),
                apply::judge_continuous_flow.in_set(RiggingSystems::SessionLoss),
                (
                    apply::advance_attempt_lifecycle,
                    emit::publish_role_status_changes,
                    emit::publish_role_presentation_changes,
                )
                    .chain()
                    .in_set(RiggingSystems::Apply),
                apply::clear_binding_transitions.after(RiggingSystems::Apply),
            ),
        )
        .add_observer(emit::on_retire_role)
        .add_observer(emit::on_reapply_configuration);
        install_role_status_clock(app);
    }
}

fn install_resources(app: &mut App) {
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
    app.init_resource::<Time<Real>>();
    app.init_resource::<PendingAttemptEndingPublications>();
    app.init_resource::<ReporterWaitDiscoveryRequests>();
    let runtime_started_at = app.world().resource::<Time<Real>>().startup();
    app.world_mut()
        .get_resource_or_insert_with(|| RiggingRuntimeClock::starting_at(runtime_started_at));
    app.world_mut()
        .get_resource_or_insert_with(|| DriverReports::starting_at(runtime_started_at));
    app.init_resource::<Attempts>()
        .init_resource::<BindingTransitionBatch>()
        .init_resource::<Bindings>()
        .init_resource::<DeviceChangeAnnouncements>()
        .init_resource::<DepartureGraceDeadlineStatus>()
        .init_resource::<Devices>()
        .init_resource::<DiscoveryControl>()
        .init_resource::<DiscoveryLimits>()
        .init_resource::<DiscoveryStatus>()
        .init_resource::<DiscoveryTransitionJournal>()
        .init_resource::<HardwareInventory>()
        .init_resource::<IdentityDecisions>()
        .init_resource::<LostRegisteredRoleEntities>()
        .init_resource::<LostRoleRelationships>()
        .init_resource::<ReconciledDeviceChanges>()
        .init_resource::<RegisteredSchemes>()
        .init_resource::<Reporters>()
        .init_resource::<RiggingRoleRelationshipRepairs>()
        .init_resource::<RiggingLimits>()
        .init_resource::<RiggingRevision>();
}

fn install_role_status_clock(app: &mut App) {
    let runtime_clock = *app.world().resource::<RiggingRuntimeClock>();
    app.world_mut()
        .resource_mut::<Bindings>()
        .install_role_status_clock(runtime_clock);
}

fn collect(world: &mut World) {
    let now = rigging_frame_instant(world);
    world.resource_scope::<Reporters, _>(|world, mut reporters| {
        let discovery_limits = world.resource::<DiscoveryLimits>().clone();
        world.resource_scope::<DiscoveryControl, _>(|world, mut discovery_control| {
            world.resource_scope::<DiscoveryStatus, _>(|world, mut discovery_status| {
                world.resource_scope::<DiscoveryTransitionJournal, _>(
                    |world, mut discovery_transition_journal| {
                        reporters.collect(
                            world,
                            now,
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

fn collect_departure_grace_deadlines(
    devices: Res<crate::Devices>,
    runtime_clock: Res<RiggingRuntimeClock>,
    time: Res<Time<Real>>,
    mut deadline_status: ResMut<DepartureGraceDeadlineStatus>,
) {
    let observed_at = time.last_update().unwrap_or_else(|| time.startup());
    deadline_status.refresh(&devices, runtime_clock.time_at(observed_at));
}

fn refresh_driver_report_time(world: &mut World) {
    let frame_instant = rigging_frame_instant(world);
    world
        .resource::<DriverReports>()
        .set_frame_instant(frame_instant);
}

fn write_reporter_health(world: &mut World) {
    let now = rigging_frame_instant(world);
    let runtime_clock = *world.resource::<RiggingRuntimeClock>();
    world.resource_scope::<Reporters, _>(|world, mut reporters| {
        reporters.write_health(world, now, runtime_clock);
    });
}

fn cross_role_wait_bounds(world: &mut World) {
    let now = rigging_frame_instant(world);
    if !world.resource::<Bindings>().wait_bound_requires_update(now) {
        return;
    }
    let crossed = world.resource_mut::<Bindings>().cross_wait_bounds(now);
    for role in crossed {
        warn!("role `{role}` exceeded its waiting bound");
    }
    emit::publish_role_status_changes(world);
    emit::publish_role_presentation_changes(world);
}

fn rigging_frame_instant(world: &World) -> Instant {
    let time = world.resource::<Time<Real>>();
    time.last_update().unwrap_or_else(|| time.startup())
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
