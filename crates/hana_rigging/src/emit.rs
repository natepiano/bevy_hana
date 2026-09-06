//! Turning reconciled state changes into the events in `crate::events`.
//!
//! Device changes run at the end of `crate::RiggingSystems::Reconcile`, after
//! `crate::reconcile::project_device_entities` has written this pass's conclusions onto entities.
//! Role status changes also run between lifecycle systems so an `Applying -> Waiting -> Applying`
//! sequence in one frame publishes both edges.

use bevy::ecs::change_detection::Ref;
use bevy::ecs::observer::On;
use bevy::ecs::system::Commands;
use bevy::ecs::system::Query;
use bevy::ecs::system::ResMut;
use bevy::log::info;
use bevy::prelude::Added;
use bevy::prelude::Entity;
use bevy::prelude::Resource;
use bevy::prelude::World;

use crate::Bindings;
use crate::DeviceArrived;
use crate::DeviceChange;
use crate::DeviceKey;
use crate::Devices;
use crate::DiscoveryControl;
use crate::DiscoveryFinished;
use crate::DiscoveryProgressChanged;
use crate::IdentityChanged;
use crate::IdentityVerdict;
use crate::KernelRolePresentation;
use crate::KeyAvailability;
use crate::LiveRoleChange;
use crate::LiveRoleChanged;
use crate::ReapplyConfiguration;
use crate::ReporterActivation;
use crate::RetireRole;
use crate::RoleKey;
use crate::RoleStatus;
use crate::StartupDiscoveryChanged;
use crate::WaitingWork;
use crate::apply;
use crate::devices::DeviceChangeAnnouncements;
use crate::devices::PriorKeyAvailability;
use crate::discovery::DiscoveryTransition;
use crate::discovery::DiscoveryTransitionJournal;
use crate::presentation;
use crate::registration::Reporters;

enum RoleStatusWriteTarget {
    LiveMirror(Entity),
    MirrorBeingRebuilt,
}

/// Roles whose missing devices have already requested reporter discovery.
#[derive(Default, Resource)]
pub(crate) struct ReporterWaitDiscoveryRequests {
    waiting_roles: Vec<RoleKey>,
}

fn live_binding_entity(world: &World, binding: Entity) -> RoleStatusWriteTarget {
    if world.get_entity(binding).is_ok() {
        RoleStatusWriteTarget::LiveMirror(binding)
    } else {
        RoleStatusWriteTarget::MirrorBeingRebuilt
    }
}

/// Write every queued role status and emit its typed edge from the same boundary.
pub(crate) fn publish_role_status_changes(world: &mut World) {
    let changes = if world.resource::<Bindings>().has_status_changes() {
        world.resource_mut::<Bindings>().take_status_changes()
    } else {
        Vec::new()
    };
    for change in changes {
        let Ok(indexed_binding) = world.resource::<Bindings>().role_entity(&change.role) else {
            continue;
        };
        let RoleStatusWriteTarget::LiveMirror(binding) =
            live_binding_entity(world, indexed_binding)
        else {
            // `reconcile_role_entities` replaces the missing entity on the next frame. The
            // consumed `LiveRoleChanged` edge is not replayed; the backfill below inserts the
            // current `Bindings::projected_role_status` on the replacement.
            continue;
        };
        world
            .entity_mut(binding)
            .insert(RoleStatus::from_view(change.to.clone()));
        world.trigger(LiveRoleChanged {
            binding,
            role: change.role.clone(),
            change: LiveRoleChange::Status {
                from: crate::RoleStatusBeforeChange::new(change.from),
                to:   crate::RoleStatusAfterChange::new(change.to),
            },
        });
    }

    let registered_roles = world
        .resource::<Bindings>()
        .registered_role_entities()
        .map(|(role, entity)| (role.clone(), entity))
        .collect::<Vec<_>>();
    let mut missing_statuses = Vec::new();
    for (role, indexed_binding) in registered_roles {
        let RoleStatusWriteTarget::LiveMirror(binding) =
            live_binding_entity(world, indexed_binding)
        else {
            continue;
        };
        if world.get::<RoleStatus>(binding).is_some() {
            continue;
        }
        if let Ok(status) = world.resource::<Bindings>().projected_status(&role) {
            missing_statuses.push((binding, status));
        }
    }
    for (binding, status) in missing_statuses {
        world
            .entity_mut(binding)
            .insert(RoleStatus::from_view(status));
    }
    request_discovery_for_waiting_roles(world);
    apply::publish_pending_attempt_endings(world);
}

/// Write the derived operator vocabulary for every registered role whose presentation moved.
///
/// Registered as the immediate successor of `publish_role_status_changes` at each of the kernel's
/// three role-status publication boundaries, so the derivation reads the `crate::RoleStatus` that
/// boundary just wrote rather than projecting a second one. A presentation therefore cannot
/// disagree with the status published beside it, and no pass pays for a duplicate projection.
pub(crate) fn publish_role_presentation_changes(world: &mut World) {
    let bound_roles = {
        let bindings = world.resource::<Bindings>();
        bindings
            .registered_role_entities()
            .filter_map(|(role, entity)| {
                let binding = bindings.binding(role).ok()?;
                Some((binding.endpoint.device.clone(), entity))
            })
            .collect::<Vec<_>>()
    };
    let mut moved_presentations = Vec::new();
    for (device, indexed_binding) in bound_roles {
        let RoleStatusWriteTarget::LiveMirror(binding) =
            live_binding_entity(world, indexed_binding)
        else {
            continue;
        };
        // Neither half of this read can go missing under a live mirror, so the kernel never
        // withdraws a presentation it has published: `publish_role_status_changes` backfills
        // `RoleStatus` onto every live mirror in the immediately preceding system, and the
        // retained availability table is only ever inserted into — `crate::reconcile` forces a
        // merge pass whenever any bound key still lacks a conclusion. The two `let ... else`
        // arms below are how the derivation borrows its evidence, not a withdrawal path.
        let Some(status) = world.get::<RoleStatus>(binding) else {
            continue;
        };
        let PriorKeyAvailability::Published(availability) =
            world.resource::<Devices>().key_availability(&device)
        else {
            continue;
        };
        let derived = presentation::derive_role_presentation(status.view(), availability.clone());
        let published = world.get::<KernelRolePresentation>(binding);
        if !published.is_some_and(|published| *published.view() == derived) {
            moved_presentations.push((binding, derived));
        }
    }
    for (binding, derived) in moved_presentations {
        world
            .entity_mut(binding)
            .insert(KernelRolePresentation::from_view(derived));
    }
}

/// Emit application-observable device entity arrivals and identity changes.
pub(crate) fn announce_device_changes(
    mut commands: Commands,
    arrived: Query<(Entity, &DeviceKey), Added<DeviceKey>>,
    verdicts: Query<(Entity, Ref<'_, IdentityVerdict>)>,
) {
    for (device, key) in &arrived {
        commands.trigger(DeviceArrived {
            device,
            key: key.clone(),
        });
    }
    for (device, verdict) in &verdicts {
        if bevy::ecs::change_detection::DetectChanges::is_changed(&verdict) {
            commands.trigger(IdentityChanged {
                device,
                verdict: (*verdict).clone(),
            });
        }
    }
}

/// Emit retained device availability changes.
pub(crate) fn announce_reconciled_facts(
    mut commands: Commands,
    mut device_change_announcements: ResMut<DeviceChangeAnnouncements>,
) {
    for change in device_change_announcements.availability.drain(..) {
        if let KeyAvailability::Absent { established_by, .. } = &change.to {
            info!(
                "device `{:?}` entered Absent; established by reporter {} batch {}",
                change.key,
                established_by.reporter.get(),
                established_by.batch.get()
            );
        }
        commands.trigger(DeviceChange::Availability {
            key:  change.key,
            from: change.from,
            to:   change.to,
        });
    }
    device_change_announcements.connections.clear();
}

fn endpoint_has_live_device(devices: &Devices, device: &DeviceKey) -> bool {
    matches!(
        devices.key_availability(device),
        crate::devices::PriorKeyAvailability::Published(crate::KeyAvailability::Present(_))
    )
}

/// Request the enabled reporter covering every newly unresolved role.
fn request_discovery_for_waiting_roles(world: &mut World) {
    let unresolved_roles = {
        let bindings = world.resource::<Bindings>();
        let devices = world.resource::<Devices>();
        bindings
            .registered_role_entities()
            .filter_map(|(role, _)| {
                let binding = bindings.binding(role).ok()?;
                (!endpoint_has_live_device(devices, &binding.endpoint.device)).then(|| role.clone())
            })
            .collect::<Vec<_>>()
    };
    let new_requests = {
        let mut requests = world.resource_mut::<ReporterWaitDiscoveryRequests>();
        let new_requests = unresolved_roles
            .iter()
            .filter(|role| !requests.waiting_roles.contains(role))
            .cloned()
            .collect::<Vec<_>>();
        requests.waiting_roles = unresolved_roles;
        new_requests
    };
    for role in new_requests {
        request_discovery_for_hardware_wait(world, &role);
    }
}

/// Request the enabled reporter covering one role's hardware endpoint.
fn request_discovery_for_hardware_wait(world: &mut World, role: &RoleKey) {
    let Some(device_key) = world
        .resource::<Bindings>()
        .binding(role)
        .ok()
        .map(|binding| binding.endpoint.device.clone())
    else {
        return;
    };
    let Some(covering_reporter) = world
        .resource::<Reporters>()
        .registered_reporters()
        .find(|registered_reporter| {
            registered_reporter.activation == ReporterActivation::Enabled
                && registered_reporter
                    .coverage
                    .establishes_absence_for(&device_key)
        })
        .map(|registered_reporter| registered_reporter.reporter)
    else {
        return;
    };
    drop(
        world
            .resource_mut::<DiscoveryControl>()
            .request(covering_reporter),
    );
}

/// Retire a role because application code asked for it, rather than because a device left.
///
/// Global rather than entity-targeted so a role registered and retired inside one frame — which
/// never had a binding entity — can still be retired. A role that was never bound is not an error
/// here: the request and the outcome agree.
pub(crate) fn on_retire_role(retire_role: On<RetireRole>, mut bindings: ResMut<Bindings>) {
    drop(bindings.retire(&retire_role.role));
}

/// Answer an application's request to re-apply a role's saved configuration.
///
/// Honoured for exactly the role the kernel is holding a saved value for:
/// `crate::WaitingWork::ReapplyRequestOwed`. The hold is what the request answers, so the hold is
/// what this reads — reaching back for the `crate::RecoveryPolicy` would re-derive a conclusion the
/// departure already recorded, and the two derivations would eventually disagree.
///
/// `crate::WaitingWork::RegistrationOwed` is refused: that role's saved value was dropped at the
/// departure and there is nothing left to re-apply, so its hold is cleared by registering a binding
/// carrying a fresh configuration instead. Every other state is refused because nothing is owed.
pub(crate) fn on_reapply_configuration(
    reapply_configuration: On<ReapplyConfiguration>,
    roles: Query<&RoleKey>,
    mut bindings: ResMut<Bindings>,
) {
    let Ok(role) = roles.get(reapply_configuration.binding).cloned() else {
        return;
    };
    if bindings.waiting_work(&role) != WaitingWork::ReapplyRequestOwed {
        return;
    }
    bindings.request_reapply(&role);
}

/// Emit every discovery transition the scheduler recorded this frame, and empty the journal.
///
/// Ordered after device publication so a consumer that watches both sees this frame's device
/// conclusions before the discovery bookkeeping that produced them. Draining here is what keeps the
/// journal bounded — it is a record of one frame's transitions, never a history.
///
/// `crate::DiscoveryProgressChanged` carries the reporter's own report and its batch counts from
/// one recorded transition rather than from two reads of the retained status, so the per-reporter
/// and the aggregate view cannot disagree about the same batch.
pub(crate) fn announce_discovery_transitions(
    mut commands: Commands,
    mut discovery_transition_journal: ResMut<DiscoveryTransitionJournal>,
) {
    for discovery_transition in discovery_transition_journal.drain() {
        match discovery_transition {
            DiscoveryTransition::Progressed {
                batch,
                reporter,
                progress,
                completed,
                total,
                running,
                queued,
            } => {
                commands.trigger(DiscoveryProgressChanged {
                    batch,
                    reporter,
                    progress,
                    completed,
                    total,
                    running,
                    queued,
                });
            },
            DiscoveryTransition::Finished {
                batch,
                reporter,
                outcome,
            } => {
                commands.trigger(DiscoveryFinished {
                    batch,
                    reporter,
                    outcome,
                });
            },
            DiscoveryTransition::StartupChanged { startup } => {
                commands.trigger(StartupDiscoveryChanged { state: startup });
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use bevy::MinimalPlugins;
    use bevy::app::App;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::On;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Update;
    use bevy::prelude::With;
    use bevy::prelude::World;
    use bevy::time::TimeUpdateStrategy;

    use crate::Applied;
    use crate::ApplyContext;
    use crate::AttemptInvalidation;
    use crate::AttemptRef;
    use crate::AuthoritativeReporterCoverage;
    use crate::Binding;
    use crate::BindingAuthoring;
    use crate::BindingPolicy;
    use crate::BindingRegistration;
    use crate::Bindings;
    use crate::CoveredDeviceIdentitySpace;
    use crate::DeviceEndpoint;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DeviceReporter;
    use crate::DeviceScan;
    use crate::DiscoveryCadence;
    use crate::DiscoveryControl;
    use crate::DiscoveryWork;
    use crate::DriverCleanupRoleEntity;
    use crate::DriverCompletion;
    use crate::EndpointDriver;
    use crate::EndpointId;
    use crate::EstablishedContext;
    use crate::FlowExpectation;
    use crate::LastKnownGoodConfiguration;
    use crate::LiveRoleChanged;
    use crate::MainThreadDiscoveryJob;
    use crate::OnAbort;
    use crate::OnSessionLoss;
    use crate::RecoveryPolicy;
    use crate::ReporterActivation;
    use crate::ReporterCoverage;
    use crate::ReporterId;
    use crate::ReporterRegistration;
    use crate::RequestedConfiguration;
    use crate::RetryOn;
    use crate::RiggingAppExt;
    use crate::RiggingPlugin;
    use crate::RoleEndpoint;
    use crate::RoleKey;
    use crate::RoleStatus;
    use crate::SessionRef;
    use crate::SessionReleaseCause;
    use crate::TargetResolution;
    use crate::TargetResolutionContext;
    use crate::WaitingWork;
    use crate::binding::ApplyDeadline;
    use crate::register_binding;
    use crate::registration::DriverId;
    use crate::scheme::AuthoredId;

    /// Long enough that a periodic reporter never becomes due on its own inside a test.
    const NEVER_DUE_UNAIDED: Duration = Duration::from_hours(1);

    struct CountingReporter {
        scans: Arc<AtomicUsize>,
    }

    #[derive(Component)]
    #[relationship(relationship_target = RecoveryTestClients)]
    struct RecoveryTestRiggingRole(Entity);

    #[derive(Component)]
    #[relationship_target(relationship = RecoveryTestRiggingRole)]
    struct RecoveryTestClients(Vec<Entity>);

    #[derive(Default, Resource)]
    struct ChangedRoleEntities(Vec<Entity>);

    fn record_changed_role_entity(
        event: On<LiveRoleChanged>,
        mut changed: ResMut<ChangedRoleEntities>,
    ) {
        changed.0.push(event.binding);
    }

    #[derive(Component, bevy::prelude::Reflect)]
    struct RecoveryTestConfiguration;

    struct RecoveryTestDriver;

    impl EndpointDriver for RecoveryTestDriver {
        type Configuration = RecoveryTestConfiguration;
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

    #[derive(Resource)]
    struct RecoveryTestDriverRegistration(
        crate::EndpointDriverRegistration<RecoveryTestConfiguration>,
    );

    #[derive(Default, Resource)]
    struct RegistrationResults {
        registered: Option<Entity>,
        conflict:   Option<String>,
    }

    fn register_role_and_conflict(
        driver: Res<RecoveryTestDriverRegistration>,
        mut registration: BindingRegistration,
        mut results: ResMut<RegistrationResults>,
    ) {
        if results.registered.is_some() {
            return;
        }
        let Ok(role) = RoleKey::new("same-frame-role") else {
            results.conflict = Some("the test role is invalid".to_owned());
            return;
        };
        let Ok(first) = recovery_test_authoring(role.clone(), driver.0) else {
            results.conflict = Some("the first test authoring is invalid".to_owned());
            return;
        };
        let Ok(conflict) = recovery_test_authoring(role, driver.0) else {
            results.conflict = Some("the conflicting test authoring is invalid".to_owned());
            return;
        };
        results.registered = registration.register(first).ok();
        results.conflict = registration
            .register(conflict)
            .err()
            .map(|error| error.to_string());
    }

    fn recovery_test_authoring(
        role: RoleKey,
        driver: crate::EndpointDriverRegistration<RecoveryTestConfiguration>,
    ) -> Result<BindingAuthoring<RecoveryTestConfiguration>, Box<dyn Error>> {
        let endpoint = DeviceEndpoint {
            device: DeviceKey {
                kind: DeviceKind::Camera,
                id:   DeviceIdSource::Authored {
                    value: AuthoredId::new(role.as_str())?,
                },
            },
            id:     EndpointId::Whole,
        };
        Ok(BindingAuthoring::new(
            role,
            endpoint,
            driver,
            RecoveryTestConfiguration,
            BindingPolicy::new(
                RecoveryPolicy::Forget,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ))
    }

    #[test]
    fn typed_registration_returns_its_complete_entity_and_conflicts_spawn_nothing()
    -> Result<(), &'static str> {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let driver = app.add_endpoint_driver(RecoveryTestDriver);
        app.insert_resource(RecoveryTestDriverRegistration(driver))
            .init_resource::<RegistrationResults>()
            .add_systems(Update, register_role_and_conflict);

        app.update();

        let results = app.world().resource::<RegistrationResults>();
        let role_entity = results
            .registered
            .ok_or("registration did not return an entity")?;
        assert!(results.conflict.is_some());
        assert!(app.world().get::<RoleEndpoint>(role_entity).is_some());
        let role_entities = app
            .world_mut()
            .query_filtered::<Entity, With<RoleEndpoint>>()
            .iter(app.world())
            .count();
        assert_eq!(role_entities, 1);
        Ok(())
    }

    impl DeviceReporter for CountingReporter {
        fn discover(&mut self) -> DiscoveryWork {
            self.scans.fetch_add(1, Ordering::Relaxed);
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(|_| {
                DeviceScan::Complete(Vec::new())
            }))
        }
    }

    fn app_with_disabled_reporter_covering(
        kind: DeviceKind,
    ) -> (App, ReporterId, Arc<AtomicUsize>) {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let scans = Arc::new(AtomicUsize::new(0));
        let reporter = app.add_device_reporter(
            CountingReporter {
                scans: Arc::clone(&scans),
            },
            ReporterRegistration::optional(
                DiscoveryCadence::Periodic {
                    interval: NEVER_DUE_UNAIDED,
                },
                ReporterActivation::Disabled,
                ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
                    CoveredDeviceIdentitySpace::AllKeysOfKind { kind },
                )),
                std::time::Duration::from_secs(10),
            ),
        );

        (app, reporter, scans)
    }

    fn unresolved_camera_binding(role: &str) -> Result<Binding, Box<dyn Error>> {
        Ok(Binding {
            role:             RoleKey::new(role)?,
            endpoint:         DeviceEndpoint {
                device: DeviceKey {
                    kind: DeviceKind::Camera,
                    id:   DeviceIdSource::Authored {
                        value: AuthoredId::new(role)?,
                    },
                },
                id:     EndpointId::Whole,
            },
            driver:           DriverId(0),
            recovery:         RecoveryPolicy::Forget,
            retry:            RetryOn::NewRevision,
            on_abort:         OnAbort::default(),
            on_loss:          OnSessionLoss::default(),
            requested:        RequestedConfiguration::new(()),
            last_known_good:  LastKnownGoodConfiguration::default(),
            apply_deadline:   ApplyDeadline::ProcessDefault,
            flow_expectation: FlowExpectation::NotMonitored,
        })
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "the recovery scenario keeps each frame and assertion visible in one test"
    )]
    fn a_registered_role_rebuilds_a_despawned_mirror_and_resumes_status_publication()
    -> Result<(), Box<dyn Error>> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(1)))
            .add_plugins(RiggingPlugin)
            .register_rigging_role_relationship::<RecoveryTestRiggingRole>()
            .init_resource::<ChangedRoleEntities>()
            .add_observer(record_changed_role_entity);
        let repaired_role = RoleKey::new("repaired-role")?;
        let untouched_role = RoleKey::new("untouched-role")?;
        let driver = app.add_endpoint_driver(RecoveryTestDriver);
        let repaired_binding = recovery_test_authoring(repaired_role.clone(), driver)?;
        let repaired_endpoint = repaired_binding.endpoint().clone();
        let repaired_recovery = RecoveryPolicy::Forget;
        let repaired_entity = register_binding(app.world_mut(), repaired_binding)?;
        let untouched_entity = register_binding(
            app.world_mut(),
            recovery_test_authoring(untouched_role.clone(), driver)?,
        )?;
        let client = app
            .world_mut()
            .spawn(RecoveryTestRiggingRole(repaired_entity))
            .id();
        app.update();

        let despawned_mirror = repaired_entity;
        let untouched_mirror = untouched_entity;
        let untouched_status = app
            .world()
            .get::<RoleStatus>(untouched_mirror)
            .ok_or("the untouched role has no published status")?
            .view()
            .clone();

        app.world_mut().entity_mut(despawned_mirror).despawn();
        let queued_status = {
            let mut bindings = app.world_mut().resource_mut::<Bindings>();
            bindings.set_waiting_work(&repaired_role, WaitingWork::ReapplyRequestOwed);
            assert!(bindings.has_status_changes());
            bindings.projected_status(&repaired_role)?
        };

        app.update();

        let rebuilt_mirror = app
            .world()
            .resource::<Bindings>()
            .role_entity(&repaired_role)?;
        assert_ne!(rebuilt_mirror, despawned_mirror);
        assert!(app.world().get_entity(rebuilt_mirror).is_ok());
        assert_eq!(
            app.world().get::<RoleKey>(rebuilt_mirror),
            Some(&repaired_role)
        );
        assert_eq!(
            app.world().get::<RecoveryPolicy>(rebuilt_mirror),
            Some(&repaired_recovery)
        );
        assert_eq!(
            app.world()
                .get::<RoleEndpoint>(rebuilt_mirror)
                .map(RoleEndpoint::endpoint),
            Some(&repaired_endpoint)
        );
        assert_eq!(
            app.world()
                .get::<RoleStatus>(rebuilt_mirror)
                .ok_or("the rebuilt mirror has no published status")?
                .view(),
            &queued_status
        );
        assert_eq!(
            app.world()
                .get::<RecoveryTestRiggingRole>(client)
                .map(|relationship| relationship.0),
            Some(rebuilt_mirror)
        );

        assert_eq!(
            app.world()
                .resource::<Bindings>()
                .role_entity(&untouched_role),
            Ok(untouched_mirror)
        );
        assert!(app.world().get_entity(untouched_mirror).is_ok());
        assert_eq!(
            app.world()
                .get::<RoleStatus>(untouched_mirror)
                .ok_or("the untouched role lost its published status")?
                .view(),
            &untouched_status
        );

        app.world_mut()
            .resource_mut::<ChangedRoleEntities>()
            .0
            .clear();
        app.world_mut()
            .resource_mut::<Bindings>()
            .set_waiting_work(&repaired_role, WaitingWork::Nothing);
        app.update();
        let changed_entities = &app.world().resource::<ChangedRoleEntities>().0;
        assert!(!changed_entities.is_empty());
        assert!(
            changed_entities
                .iter()
                .all(|changed| *changed == rebuilt_mirror)
        );
        Ok(())
    }

    #[test]
    fn an_unresolved_role_does_not_enable_the_covering_disabled_reporter()
    -> Result<(), Box<dyn Error>> {
        let (mut app, reporter, scans) = app_with_disabled_reporter_covering(DeviceKind::Camera);
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(unresolved_camera_binding("camera-tool")?)?;

        app.update();
        assert_eq!(
            app.world()
                .resource::<DiscoveryControl>()
                .activation(reporter),
            ReporterActivation::Disabled,
            "application code owns optional reporter activation"
        );

        app.update();
        assert_eq!(
            scans.load(Ordering::Relaxed),
            0,
            "an unresolved role must not run an application-disabled reporter"
        );

        Ok(())
    }

    #[test]
    fn a_claim_no_reporter_covers_arms_nothing() -> Result<(), Box<dyn Error>> {
        let (mut app, reporter, scans) = app_with_disabled_reporter_covering(DeviceKind::Display);
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(unresolved_camera_binding("camera-tool")?)?;

        app.update();
        app.update();
        assert_eq!(
            app.world()
                .resource::<DiscoveryControl>()
                .activation(reporter),
            ReporterActivation::Disabled,
            "a display-only reporter proves nothing about a camera key and must stay disabled"
        );
        assert_eq!(scans.load(Ordering::Relaxed), 0);

        Ok(())
    }
}
