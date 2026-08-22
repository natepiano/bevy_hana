//! Kernel endpoint driver for Clerestory-managed windows.

use std::collections::HashMap;

use bevy::prelude::Entity;
use bevy::prelude::IVec2;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::World;
use bevy::window::WindowPosition;
use hana_rigging::prelude::ApplyPermit;
use hana_rigging::prelude::AttemptId;
use hana_rigging::prelude::AttemptLookup;
use hana_rigging::prelude::AttemptOutcome;
use hana_rigging::prelude::AttemptProgress;
use hana_rigging::prelude::Attempts;
use hana_rigging::prelude::CaptureOutcome;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DriverId;
use hana_rigging::prelude::EndpointDriver;

use crate::Platform;
use crate::managed;
use crate::managed::WindowBindingAuthoring;
use crate::monitors::CurrentMonitor;
use crate::persistence::EstablishedWindowPlacement;
use crate::restore::RestorePreparation;
use crate::restore::WindowApplyConfiguration;

/// Completion records emitted by window settling for the kernel-issued attempt that started it.
///
/// The table exists only while this Bevy world exists. `AttemptId` values never repeat during one
/// process, so removing a result from this table is the only way a driver poll can complete an
/// attempt and a delayed result cannot complete a later operation.
#[derive(Default, Resource)]
pub(crate) struct WindowDriverAttemptResults {
    outcomes: HashMap<AttemptId, AttemptOutcome>,
}

/// Process-local identifier the rigging kernel issued for `WindowEndpointDriver`.
#[derive(Resource)]
pub(crate) struct WindowDriverId(pub(crate) DriverId);

impl WindowDriverAttemptResults {
    pub(crate) fn record(&mut self, attempt: AttemptId, outcome: AttemptOutcome) {
        self.outcomes.entry(attempt).or_insert(outcome);
    }

    fn take(&mut self, attempt: AttemptId) -> Option<AttemptOutcome> {
        self.outcomes.remove(&attempt)
    }

    fn discard_finished_attempts(&mut self, attempts: &Attempts) {
        self.outcomes.retain(|attempt, _| {
            matches!(attempts.in_flight(*attempt), AttemptLookup::InFlight(_))
        });
    }
}

/// Drops an unconsumed result after the kernel has already ended its attempt.
///
/// An invalidated attempt can end between target preparation and the next driver poll. Keeping its
/// terminal result would not change another attempt because ids are unique, but retaining it would
/// leave private runtime state behind after the work it describes no longer exists.
pub(crate) fn discard_finished_window_attempt_results(
    mut results: ResMut<WindowDriverAttemptResults>,
    attempts: Res<Attempts>,
) {
    results.discard_finished_attempts(&attempts);
}

/// Applies `EstablishedWindowPlacement` values after the rigging kernel authorizes a display.
pub(crate) struct WindowEndpointDriver;

impl EndpointDriver for WindowEndpointDriver {
    type Configuration = EstablishedWindowPlacement;

    fn capture(
        &mut self,
        world: &mut World,
        endpoint: &DeviceEndpoint,
    ) -> CaptureOutcome<Self::Configuration> {
        let entity = match resolve_window_endpoint_projection(world, endpoint) {
            WindowEndpointProjectionMatch::Unique(entity) => entity,
            WindowEndpointProjectionMatch::Absent => {
                return CaptureOutcome::ReadFailed(DeviceAccessError::Absent {
                    detail: format!(
                        "no Clerestory window projects registered endpoint {endpoint:?}"
                    ),
                });
            },
            WindowEndpointProjectionMatch::Ambiguous => {
                return CaptureOutcome::ReadFailed(DeviceAccessError::Transport {
                    detail: format!(
                        "multiple Clerestory windows project registered endpoint {endpoint:?}"
                    ),
                });
            },
        };
        let Some(window) = world.get::<Window>(entity) else {
            return CaptureOutcome::ReadFailed(DeviceAccessError::Absent {
                detail: format!("Clerestory window entity {entity:?} no longer has Window"),
            });
        };
        let Some(current_monitor) = world.get::<CurrentMonitor>(entity) else {
            return CaptureOutcome::ReadFailed(DeviceAccessError::Transport {
                detail: format!("Clerestory window entity {entity:?} has no current monitor"),
            });
        };
        let physical_position = match window.position {
            WindowPosition::At(position) => Some(IVec2::new(position.x, position.y)),
            _ => None,
        };

        CaptureOutcome::Read(EstablishedWindowPlacement::from_readback(
            window,
            current_monitor,
            physical_position,
            *world.resource::<Platform>(),
        ))
    }

    fn start_apply(
        &mut self,
        world: &mut World,
        endpoint: &DeviceEndpoint,
        configuration: &Self::Configuration,
        attempt: AttemptId,
        _: ApplyPermit,
    ) {
        let attempt = match world.resource::<Attempts>().in_flight(attempt) {
            AttemptLookup::InFlight(attempt) => attempt.clone(),
            AttemptLookup::Finished => return,
        };
        if &attempt.endpoint != endpoint {
            return;
        }
        let Some(entity) = managed::window_entity_for_role(world, &attempt.role) else {
            return;
        };

        world.entity_mut(entity).insert((
            RestorePreparation::for_attempt(&attempt),
            WindowApplyConfiguration::from(configuration.clone()),
        ));
    }

    fn poll(&mut self, world: &mut World, attempt: AttemptId) -> AttemptProgress {
        world
            .resource_mut::<WindowDriverAttemptResults>()
            .take(attempt)
            .map_or(AttemptProgress::Pending, AttemptProgress::Finished)
    }
}

enum WindowEndpointProjectionMatch {
    Absent,
    Unique(Entity),
    Ambiguous,
}

fn resolve_window_endpoint_projection(
    world: &mut World,
    endpoint: &DeviceEndpoint,
) -> WindowEndpointProjectionMatch {
    let mut projections = world.query::<(Entity, &WindowBindingAuthoring)>();
    let mut matches = projections
        .iter(world)
        .filter_map(|(entity, authoring)| match authoring {
            WindowBindingAuthoring::Registered {
                endpoint: registered,
            } if registered == endpoint => Some(entity),
            WindowBindingAuthoring::Registered { .. } | WindowBindingAuthoring::Rejected => None,
        });
    let Some(entity) = matches.next() else {
        return WindowEndpointProjectionMatch::Absent;
    };
    if matches.next().is_some() {
        WindowEndpointProjectionMatch::Ambiguous
    } else {
        WindowEndpointProjectionMatch::Unique(entity)
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::World;
    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;
    use hana_rigging::prelude::EndpointDriver;
    use hana_rigging::prelude::EndpointId;

    use super::AttemptId;
    use super::AttemptOutcome;
    use super::CaptureOutcome;
    use super::DeviceAccessError;
    use super::DeviceEndpoint;
    use super::WindowBindingAuthoring;
    use super::WindowDriverAttemptResults;
    use super::WindowEndpointDriver;

    fn endpoint() -> Result<DeviceEndpoint, String> {
        let authored_id = AuthoredId::new("window-projection-test")
            .map_err(|error| format!("failed to create test device ID: {error}"))?;
        Ok(DeviceEndpoint {
            device: DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored { value: authored_id },
            },
            id:     EndpointId::Whole,
        })
    }

    #[test]
    fn terminal_result_is_first_writer_and_one_shot() {
        let attempt = AttemptId::default();
        let mut results = WindowDriverAttemptResults::default();

        results.record(attempt, AttemptOutcome::Succeeded);
        results.record(attempt, AttemptOutcome::Substituted);

        assert!(matches!(
            results.take(attempt),
            Some(AttemptOutcome::Succeeded)
        ));
        assert!(results.take(attempt).is_none());
    }

    #[test]
    fn capture_reports_an_absent_projection_as_a_read_failure() -> Result<(), String> {
        let mut world = World::new();
        let endpoint = endpoint()?;
        let outcome = WindowEndpointDriver.capture(&mut world, &endpoint);

        let CaptureOutcome::ReadFailed(DeviceAccessError::Absent { detail }) = outcome else {
            return Err(String::from(
                "capture did not report an absent endpoint projection",
            ));
        };
        assert!(detail.contains("no Clerestory window projects registered endpoint"));
        Ok(())
    }

    #[test]
    fn capture_refuses_ambiguous_endpoint_projections() -> Result<(), String> {
        let mut world = World::new();
        let endpoint = endpoint()?;
        world.spawn(WindowBindingAuthoring::Registered {
            endpoint: endpoint.clone(),
        });
        world.spawn(WindowBindingAuthoring::Registered {
            endpoint: endpoint.clone(),
        });

        let outcome = WindowEndpointDriver.capture(&mut world, &endpoint);

        let CaptureOutcome::ReadFailed(DeviceAccessError::Transport { detail }) = outcome else {
            return Err(String::from(
                "capture did not reject ambiguous endpoint projections",
            ));
        };
        assert!(detail.contains("multiple Clerestory windows project registered endpoint"));
        Ok(())
    }
}
