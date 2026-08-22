use bevy::diagnostic::FrameCount;
use bevy::prelude::Add;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use hana_clerestory::WindowRestoreMismatch;
use hana_clerestory::WindowRestored;
use hana_rigging::prelude::AttemptFinished;
use hana_rigging::prelude::DeviceArrived;
use hana_rigging::prelude::DeviceDeparted;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::IdentityQuestionRaised;
use hana_rigging::prelude::RecoveryPolicyChanged;
use hana_rigging::prelude::RoleAvailable;
use hana_rigging::prelude::RoleAwaiting;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleStateChanged;

use super::constants::APPLICATION_WINDOW_KEY;
use super::constants::APPLICATION_WINDOW_ROLE;
use super::constants::AUTOMATIC_WINDOW_KEY;
use super::constants::AUTOMATIC_WINDOW_ROLE;
use super::constants::FIELD_ATTEMPT;
use super::constants::FIELD_CANDIDATE;
use super::constants::FIELD_MONITOR;
use super::constants::FIELD_OUTCOME;
use super::constants::FIELD_RECOVERY_POLICY;
use super::constants::FIELD_ROLE;
use super::constants::FIELD_ROLE_STATE;
use super::constants::FIELD_WINDOW;
use super::constants::FIELD_WINDOW_KEY;
use super::constants::KIND_ATTEMPT_FINISHED;
use super::constants::KIND_IDENTITY_QUESTION_RAISED;
use super::constants::KIND_MONITOR_CONNECTED;
use super::constants::KIND_MONITOR_DISCONNECTED;
use super::constants::KIND_RECOVERY_ACCEPTED;
use super::constants::KIND_RECOVERY_AVAILABLE;
use super::constants::KIND_RECOVERY_MISMATCH;
use super::constants::KIND_RECOVERY_PENDING;
use super::constants::KIND_RECOVERY_RESTORED;
use super::constants::KIND_ROLE_STATE_CHANGED;
use super::constants::KIND_WINDOW_CREATED;
use super::constants::PRIMARY_WINDOW_ROLE;
use super::constants::PRODUCER_ATTEMPT_FINISHED;
use super::constants::PRODUCER_IDENTITY_QUESTION_RAISED;
use super::constants::PRODUCER_MONITOR_CONNECTED;
use super::constants::PRODUCER_MONITOR_DISCONNECTED;
use super::constants::PRODUCER_RECOVERY_ACCEPTED;
use super::constants::PRODUCER_RECOVERY_AVAILABLE;
use super::constants::PRODUCER_RECOVERY_MISMATCH;
use super::constants::PRODUCER_RECOVERY_PENDING;
use super::constants::PRODUCER_RECOVERY_RESTORED;
use super::constants::PRODUCER_ROLE_STATE_CHANGED;
use super::constants::PRODUCER_WINDOW_CREATED;
use super::setup::ProbeWindowRole;
pub(crate) use super::trace_store::ProbeTrace;
pub(crate) use super::trace_store::TraceRecord;

pub(super) fn wire_window_key(role: &RoleKey) -> Option<&'static str> {
    match role.as_str() {
        PRIMARY_WINDOW_ROLE => Some("primary"),
        AUTOMATIC_WINDOW_ROLE => Some(AUTOMATIC_WINDOW_KEY),
        APPLICATION_WINDOW_ROLE => Some(APPLICATION_WINDOW_KEY),
        _ => None,
    }
}

fn record_role(trace: &ProbeTrace, frame_count: u32, producer: &str, kind: &str, role: &RoleKey) {
    let Some(window_key) = wire_window_key(role) else {
        return;
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

/// Projects an exact kernel display arrival under the controller's established record name.
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

/// Projects an exact kernel display departure under the controller's established record name.
pub(crate) fn on_device_departed(
    event: On<DeviceDeparted>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    record_monitor_lifetime(
        &trace,
        frame_count.0,
        PRODUCER_MONITOR_DISCONNECTED,
        KIND_MONITOR_DISCONNECTED,
        &event.key,
    );
}

/// Records the binding policy edge that establishes one recovery registration.
pub(crate) fn on_recovery_policy_changed(
    event: On<RecoveryPolicyChanged>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let Some(window_key) = wire_window_key(&event.role) else {
        return;
    };
    trace.record(
        frame_count.0,
        PRODUCER_RECOVERY_ACCEPTED,
        KIND_RECOVERY_ACCEPTED,
        vec![
            (FIELD_WINDOW_KEY.into(), window_key.into()),
            (
                FIELD_RECOVERY_POLICY.into(),
                format!("{:?}", event.recovery),
            ),
        ],
    );
}

/// Records a finished kernel apply instead of inferring completion from window components.
pub(crate) fn on_attempt_finished(
    event: On<AttemptFinished>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let Some(window_key) = wire_window_key(&event.role) else {
        return;
    };
    trace.record(
        frame_count.0,
        PRODUCER_ATTEMPT_FINISHED,
        KIND_ATTEMPT_FINISHED,
        vec![
            (FIELD_WINDOW_KEY.into(), window_key.into()),
            (FIELD_ATTEMPT.into(), format!("{:?}", event.attempt)),
            (FIELD_OUTCOME.into(), format!("{:?}", event.outcome)),
        ],
    );
}

/// Records an operator-owned identity question without answering it.
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

/// Records the edge where a bound display has no live device.
pub(crate) fn on_role_awaiting(
    event: On<RoleAwaiting>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    record_role(
        &trace,
        frame_count.0,
        PRODUCER_RECOVERY_PENDING,
        KIND_RECOVERY_PENDING,
        &event.role,
    );
}

/// Records the edge where a role's exact endpoint resolves again.
pub(crate) fn on_role_available(
    event: On<RoleAvailable>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    record_role(
        &trace,
        frame_count.0,
        PRODUCER_RECOVERY_AVAILABLE,
        KIND_RECOVERY_AVAILABLE,
        &event.role,
    );
}

/// Records kernel state transitions, including the deliberate stopped state.
pub(crate) fn on_role_state_changed(
    event: On<RoleStateChanged>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let Some(window_key) = wire_window_key(&event.role) else {
        return;
    };
    trace.record(
        frame_count.0,
        PRODUCER_ROLE_STATE_CHANGED,
        KIND_ROLE_STATE_CHANGED,
        vec![
            (FIELD_WINDOW_KEY.into(), window_key.into()),
            (FIELD_ROLE_STATE.into(), format!("{:?}", event.state)),
        ],
    );
}

/// Records the observer-facing successful window projection.
pub(crate) fn on_window_restored(
    event: On<WindowRestored>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let Some(window_key) = wire_window_key(&event.role) else {
        return;
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

/// Records the observer-facing mismatch projection without changing its payload.
pub(crate) fn on_window_restore_mismatch(
    event: On<WindowRestoreMismatch>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let Some(window_key) = wire_window_key(&event.role) else {
        return;
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

/// Records creation of one controller-visible probe role.
pub(crate) fn on_probe_window_added(
    event: On<Add, ProbeWindowRole>,
    roles: Query<&ProbeWindowRole>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    let Ok(role) = roles.get(event.entity) else {
        return;
    };
    if *role == ProbeWindowRole::Control {
        return;
    }
    trace.record(
        frame_count.0,
        PRODUCER_WINDOW_CREATED,
        KIND_WINDOW_CREATED,
        vec![
            (FIELD_WINDOW_KEY.into(), role.key().into()),
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
