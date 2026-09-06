use bevy::diagnostic::FrameCount;
use bevy::prelude::Add;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::error;
use hana_clerestory::WindowRestoreMismatch;
use hana_clerestory::WindowRestored;
use hana_clerestory::managed_window_role;
use hana_clerestory::primary_window_role;
use hana_rigging::prelude::DeviceArrived;
use hana_rigging::prelude::DeviceChange;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::IdentityQuestionRaised;
use hana_rigging::prelude::KeyAvailability;
use hana_rigging::prelude::LiveRoleChange;
use hana_rigging::prelude::LiveRoleChanged;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleKeyError;
use hana_rigging::prelude::RoleStatusView;
use hana_rigging::prelude::WaitingStatusView;

use super::constants::APPLICATION_WINDOW_KEY;
use super::constants::AUTOMATIC_WINDOW_KEY;
use super::constants::FIELD_ATTEMPT;
use super::constants::FIELD_CANDIDATE;
use super::constants::FIELD_ENDING;
use super::constants::FIELD_MONITOR;
use super::constants::FIELD_ROLE;
use super::constants::FIELD_ROLE_STATUS;
use super::constants::FIELD_WINDOW;
use super::constants::FIELD_WINDOW_KEY;
use super::constants::KIND_ATTEMPT_ENDED;
use super::constants::KIND_IDENTITY_QUESTION_RAISED;
use super::constants::KIND_MONITOR_CONNECTED;
use super::constants::KIND_MONITOR_DISCONNECTED;
use super::constants::KIND_RECOVERY_AVAILABLE;
use super::constants::KIND_RECOVERY_MISMATCH;
use super::constants::KIND_RECOVERY_PENDING;
use super::constants::KIND_RECOVERY_RESTORED;
use super::constants::KIND_ROLE_STATUS_CHANGED;
use super::constants::KIND_WINDOW_CREATED;
use super::constants::PRODUCER_ATTEMPT_ENDED;
use super::constants::PRODUCER_IDENTITY_QUESTION_RAISED;
use super::constants::PRODUCER_MONITOR_CONNECTED;
use super::constants::PRODUCER_MONITOR_DISCONNECTED;
use super::constants::PRODUCER_RECOVERY_AVAILABLE;
use super::constants::PRODUCER_RECOVERY_MISMATCH;
use super::constants::PRODUCER_RECOVERY_PENDING;
use super::constants::PRODUCER_RECOVERY_RESTORED;
use super::constants::PRODUCER_ROLE_STATUS_CHANGED;
use super::constants::PRODUCER_WINDOW_CREATED;
use super::constants::RESTORE_ONLY_WINDOW_KEY;
use super::setup::ProbeWindowScenario;
pub(crate) use super::trace_store::ProbeTrace;
pub(crate) use super::trace_store::TraceRecord;

pub(super) enum TraceRoleTracking {
    Included(&'static str),
    Excluded,
}

pub(super) fn wire_window_key(role: &RoleKey) -> Result<TraceRoleTracking, RoleKeyError> {
    if role == &primary_window_role()? {
        return Ok(TraceRoleTracking::Included("primary"));
    }
    if role == &managed_window_role(AUTOMATIC_WINDOW_KEY)? {
        return Ok(TraceRoleTracking::Included(AUTOMATIC_WINDOW_KEY));
    }
    if role == &managed_window_role(APPLICATION_WINDOW_KEY)? {
        return Ok(TraceRoleTracking::Included(APPLICATION_WINDOW_KEY));
    }
    if role == &managed_window_role(RESTORE_ONLY_WINDOW_KEY)? {
        return Ok(TraceRoleTracking::Included(RESTORE_ONLY_WINDOW_KEY));
    }
    Ok(TraceRoleTracking::Excluded)
}

fn record_role(
    trace: &ProbeTrace,
    frame_count: u32,
    role: &RoleKey,
    observer: &str,
    producer: &str,
    kind: &str,
) {
    let window_key = match wire_window_key(role) {
        Ok(TraceRoleTracking::Included(window_key)) => window_key,
        Ok(TraceRoleTracking::Excluded) => return,
        Err(error) => {
            error!("[{observer}] probe role invariant failed: {error}");
            return;
        },
    };
    trace.record(
        frame_count,
        producer,
        kind,
        vec![(FIELD_WINDOW_KEY.into(), window_key.into())],
    );
}

pub(super) fn record_monitor_lifetime(
    trace: &ProbeTrace,
    frame_count: u32,
    producer: &str,
    kind: &str,
    device_key: &DeviceKey,
) {
    if device_key.kind != DeviceKind::Display {
        return;
    }
    trace.record(
        frame_count,
        producer,
        kind,
        vec![(FIELD_MONITOR.into(), format!("{device_key:?}"))],
    );
}

/// Records a [`DeviceArrived`] for a display as a `monitor-connected` record, the name the
/// controller reads. Arrivals of any other [`DeviceKind`] are skipped.
pub(crate) fn on_device_arrived(
    event: On<DeviceArrived>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    record_monitor_lifetime(
        &trace,
        frame_count.0,
        PRODUCER_MONITOR_CONNECTED,
        KIND_MONITOR_CONNECTED,
        &event.key,
    );
}

/// Records an unavailable [`DeviceChange`] for a display as a `monitor-disconnected` record, the
/// name the controller reads. Departures of any other [`DeviceKind`] are skipped.
pub(crate) fn on_device_departed(
    event: On<DeviceChange>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let DeviceChange::Availability { key, from, to } = event.event();
    if !matches!(from, KeyAvailability::Present(_)) || matches!(to, KeyAvailability::Present(_)) {
        return;
    }
    record_monitor_lifetime(
        &trace,
        frame_count.0,
        PRODUCER_MONITOR_DISCONNECTED,
        KIND_MONITOR_DISCONNECTED,
        key,
    );
}

/// Records role status and attempt-ending changes from the kernel's live-role envelope.
pub(crate) fn on_live_role_changed(
    event: On<LiveRoleChanged>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    match &event.change {
        LiveRoleChange::AttemptEnded { attempt, ending } => {
            let window_key = match wire_window_key(&event.role) {
                Ok(TraceRoleTracking::Included(window_key)) => window_key,
                Ok(TraceRoleTracking::Excluded) => return,
                Err(error) => {
                    error!("[on_live_role_changed] probe role invariant failed: {error}");
                    return;
                },
            };
            trace.record(
                frame_count.0,
                PRODUCER_ATTEMPT_ENDED,
                KIND_ATTEMPT_ENDED,
                vec![
                    (FIELD_WINDOW_KEY.into(), window_key.into()),
                    (FIELD_ATTEMPT.into(), format!("{attempt:?}")),
                    (FIELD_ENDING.into(), format!("{ending:?}")),
                ],
            );
        },
        LiveRoleChange::Status { from, to } => {
            let was_waiting = matches!(
                from.view(),
                RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
            );
            let is_waiting = matches!(
                to.view(),
                RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
            );
            match (was_waiting, is_waiting) {
                (false, true) => record_role(
                    &trace,
                    frame_count.0,
                    &event.role,
                    "on_live_role_changed",
                    PRODUCER_RECOVERY_PENDING,
                    KIND_RECOVERY_PENDING,
                ),
                (true, false) => record_role(
                    &trace,
                    frame_count.0,
                    &event.role,
                    "on_live_role_changed",
                    PRODUCER_RECOVERY_AVAILABLE,
                    KIND_RECOVERY_AVAILABLE,
                ),
                (false, false) | (true, true) => {},
            }

            let window_key = match wire_window_key(&event.role) {
                Ok(TraceRoleTracking::Included(window_key)) => window_key,
                Ok(TraceRoleTracking::Excluded) => return,
                Err(error) => {
                    error!("[on_live_role_changed] probe role invariant failed: {error}");
                    return;
                },
            };
            trace.record(
                frame_count.0,
                PRODUCER_ROLE_STATUS_CHANGED,
                KIND_ROLE_STATUS_CHANGED,
                vec![
                    (FIELD_WINDOW_KEY.into(), window_key.into()),
                    (FIELD_ROLE_STATUS.into(), format!("{:?}", to.view())),
                ],
            );
        },
    }
}

/// Records an [`IdentityQuestionRaised`] with its role and candidate. The question is left
/// unanswered here; the operator answers it.
pub(crate) fn on_identity_question_raised(
    event: On<IdentityQuestionRaised>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    trace.record(
        frame_count.0,
        PRODUCER_IDENTITY_QUESTION_RAISED,
        KIND_IDENTITY_QUESTION_RAISED,
        vec![
            (FIELD_ROLE.into(), event.role.to_string()),
            (FIELD_CANDIDATE.into(), format!("{:?}", event.candidate)),
        ],
    );
}

/// Records a [`WindowRestored`] with the role's window key, the window entity, and the monitor
/// index it landed on.
pub(crate) fn on_window_restored(
    event: On<WindowRestored>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let window_key = match wire_window_key(&event.role) {
        Ok(TraceRoleTracking::Included(window_key)) => window_key,
        Ok(TraceRoleTracking::Excluded) => return,
        Err(error) => {
            error!("[on_window_restored] probe role invariant failed: {error}");
            return;
        },
    };
    trace.record(
        frame_count.0,
        PRODUCER_RECOVERY_RESTORED,
        KIND_RECOVERY_RESTORED,
        vec![
            (FIELD_WINDOW_KEY.into(), window_key.into()),
            (FIELD_WINDOW.into(), format!("{:?}", event.entity)),
            (FIELD_MONITOR.into(), event.monitor_index.to_string()),
        ],
    );
}

/// Records a [`WindowRestoreMismatch`] with the role's window key, the window entity, and the
/// event's expected and actual monitor indices, unaltered.
pub(crate) fn on_window_restore_mismatch(
    event: On<WindowRestoreMismatch>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let window_key = match wire_window_key(&event.role) {
        Ok(TraceRoleTracking::Included(window_key)) => window_key,
        Ok(TraceRoleTracking::Excluded) => return,
        Err(error) => {
            error!("[on_window_restore_mismatch] probe role invariant failed: {error}");
            return;
        },
    };
    trace.record(
        frame_count.0,
        PRODUCER_RECOVERY_MISMATCH,
        KIND_RECOVERY_MISMATCH,
        vec![
            (FIELD_WINDOW_KEY.into(), window_key.into()),
            (FIELD_WINDOW.into(), format!("{:?}", event.entity)),
            (
                FIELD_MONITOR.into(),
                format!("{} -> {}", event.expected_monitor, event.actual_monitor),
            ),
        ],
    );
}

/// Records the creation of a probe window as a `window-created` record. A
/// [`ProbeWindowScenario::UnmanagedControl`] window is skipped, so no record names it.
pub(crate) fn on_probe_window_added(
    event: On<Add, ProbeWindowScenario>,
    scenarios: Query<&ProbeWindowScenario>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let Ok(scenario) = scenarios.get(event.entity) else {
        return;
    };
    if *scenario == ProbeWindowScenario::UnmanagedControl {
        return;
    }
    trace.record(
        frame_count.0,
        PRODUCER_WINDOW_CREATED,
        KIND_WINDOW_CREATED,
        vec![
            (FIELD_WINDOW_KEY.into(), scenario.key().into()),
            (FIELD_WINDOW.into(), format!("{:?}", event.entity)),
        ],
    );
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use hana_rigging::prelude::AuthoredId;
    use hana_rigging::prelude::DeviceIdSource;

    use super::*;

    #[test]
    fn trace_role_tracking_distinguishes_included_and_excluded_roles() -> Result<(), Box<dyn Error>>
    {
        let primary = primary_window_role()?;
        assert!(matches!(
            wire_window_key(&primary)?,
            TraceRoleTracking::Included("primary")
        ));

        let unrelated = RoleKey::new("unrelated-kernel-role")?;
        assert!(matches!(
            wire_window_key(&unrelated)?,
            TraceRoleTracking::Excluded
        ));
        Ok(())
    }

    #[test]
    fn kernel_display_departure_projects_to_controller_record() -> Result<(), Box<dyn Error>> {
        let trace = ProbeTrace::default();
        let device_key = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("controller-display")?,
            },
        };

        record_monitor_lifetime(
            &trace,
            9,
            PRODUCER_MONITOR_DISCONNECTED,
            KIND_MONITOR_DISCONNECTED,
            &device_key,
        );

        let records = trace.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, KIND_MONITOR_DISCONNECTED);
        assert!(
            records[0]
                .fields
                .iter()
                .any(|(name, value)| name == FIELD_MONITOR && value == &format!("{device_key:?}"))
        );
        Ok(())
    }
}
