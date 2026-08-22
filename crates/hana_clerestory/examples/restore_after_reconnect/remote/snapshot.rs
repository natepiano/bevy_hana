use bevy::diagnostic::FrameCount;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::World;
use bevy::window::OnMonitor;
use bevy::window::WindowPosition;
use hana_clerestory::CurrentMonitor;
use hana_clerestory::MonitorDescriptor;
use hana_clerestory::MonitorDeviceAssociation;
use hana_clerestory::MonitorDeviceKeyLookup;
use hana_clerestory::Monitors;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleState;
use serde::Serialize;

use super::ProbeSession;
use crate::ProbeMonitorIndex;
use crate::ProbeStartupMode;
use crate::constants::APPLICATION_WINDOW_ROLE;
use crate::constants::AUTOMATIC_WINDOW_ROLE;
use crate::constants::FIELD_SELECTED_MONITOR_INDEX;
use crate::constants::FIELD_WINDOW_KEY;
use crate::constants::KIND_RECOVERY_ACCEPTED;
use crate::constants::KIND_RECOVERY_AVAILABLE;
use crate::constants::KIND_RECOVERY_CANCELLATION_REQUESTED;
use crate::constants::KIND_RECOVERY_MISMATCH;
use crate::constants::KIND_RECOVERY_PENDING;
use crate::constants::KIND_RECOVERY_READY;
use crate::constants::KIND_RECOVERY_RESTORED;
use crate::constants::KIND_WINDOW_CREATED;
use crate::constants::PRIMARY_WINDOW_ROLE;
use crate::constants::PROBE_SCHEMA_VERSION;
use crate::constants::PROBE_WINDOW_COUNT;
use crate::constants::PRODUCER_RECOVERY_READY;
use crate::control::CommandReceipt;
use crate::control::CommandReceipts;
use crate::setup::ProbeWindowRegistrationComplete;
use crate::setup::ProbeWindowRole;
use crate::trace::ProbeTrace;
use crate::trace::TraceRecord;

#[derive(Serialize)]
struct MonitorSnapshot {
    entity:            u64,
    name:              Option<String>,
    identity:          String,
    verified_id:       Option<String>,
    index:             usize,
    scale:             f64,
    physical_position: [i32; 2],
    physical_size:     [u32; 2],
}

impl MonitorSnapshot {
    fn from_descriptor(
        entity: Entity,
        name: Option<String>,
        descriptor: MonitorDescriptor,
        association: &MonitorDeviceAssociation,
    ) -> Self {
        let (identity, verified_id) = match association.device_for_descriptor(descriptor) {
            MonitorDeviceKeyLookup::Exact(device_key) => (
                format!("Verified({device_key:?})"),
                Some(format!("{device_key:?}")),
            ),
            MonitorDeviceKeyLookup::Unresolved => (String::from("Unverified"), None),
        };
        Self {
            entity: entity.to_bits(),
            name,
            identity,
            verified_id,
            index: descriptor.index.adapter_value(),
            scale: descriptor.scale,
            physical_position: [
                descriptor.physical_position.x,
                descriptor.physical_position.y,
            ],
            physical_size: [descriptor.physical_size.x, descriptor.physical_size.y],
        }
    }
}

#[derive(Default, Serialize)]
struct RecoveryCounts {
    accepted:     usize,
    pending:      usize,
    available:    usize,
    restored:     usize,
    mismatch:     usize,
    cancellation: usize,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Readiness {
    Ready,
    Pending,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Resource)]
pub(crate) enum ProbeReadiness {
    Ready,
    #[default]
    Pending,
}

impl From<ProbeReadiness> for Readiness {
    fn from(readiness: ProbeReadiness) -> Self {
        match readiness {
            ProbeReadiness::Ready => Self::Ready,
            ProbeReadiness::Pending => Self::Pending,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Presence {
    Present,
    Absent,
}

impl From<bool> for Presence {
    fn from(present: bool) -> Self { if present { Self::Present } else { Self::Absent } }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Focus {
    Focused,
    Unfocused,
}

impl From<bool> for Focus {
    fn from(focused: bool) -> Self {
        if focused {
            Self::Focused
        } else {
            Self::Unfocused
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum MonitorCoverage {
    Full,
    Partial,
}

impl From<bool> for MonitorCoverage {
    fn from(covers: bool) -> Self { if covers { Self::Full } else { Self::Partial } }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum NativeFullscreen {
    Fullscreen,
    Windowed,
    Unavailable,
}

impl From<Option<bool>> for NativeFullscreen {
    fn from(state: Option<bool>) -> Self {
        match state {
            Some(true) => Self::Fullscreen,
            Some(false) => Self::Windowed,
            None => Self::Unavailable,
        }
    }
}

#[derive(Serialize)]
struct WindowSnapshot {
    key:                 String,
    entity:              u64,
    recovery_policy:     Option<String>,
    current_monitor:     Option<MonitorSnapshot>,
    requested_mode:      String,
    effective_mode:      Option<String>,
    position:            String,
    physical_size:       [u32; 2],
    #[serde(rename = "decorated")]
    decoration_presence: Presence,
    #[serde(rename = "focused")]
    focus:               Focus,
    native_fullscreen:   NativeFullscreen,
    #[serde(rename = "covers_current_monitor")]
    monitor_coverage:    MonitorCoverage,
    replacement_count:   usize,
    recovery_counts:     RecoveryCounts,
}

#[derive(Serialize)]
pub(super) struct ProbeSnapshot {
    schema_version:         u32,
    run_id:                 String,
    boot_nonce:             String,
    #[serde(rename = "ready")]
    readiness:              Readiness,
    startup_mode:           &'static str,
    selected_monitor_index: usize,
    topology_revision:      u64,
    record_cursor:          u64,
    monitors:               Vec<MonitorSnapshot>,
    windows:                Vec<WindowSnapshot>,
    terminal_failure:       Presence,
    command_receipts:       Vec<CommandReceipt>,
}

fn role_key(role: ProbeWindowRole) -> Option<RoleKey> {
    let value = match role {
        ProbeWindowRole::Primary => PRIMARY_WINDOW_ROLE,
        ProbeWindowRole::Automatic => AUTOMATIC_WINDOW_ROLE,
        ProbeWindowRole::Application => APPLICATION_WINDOW_ROLE,
        ProbeWindowRole::Control => return None,
    };
    RoleKey::new(value).ok()
}

fn record_count(records: &[TraceRecord], kind: &str, window_key: &str) -> usize {
    records
        .iter()
        .filter(|record| {
            record.kind == kind
                && record
                    .fields
                    .iter()
                    .any(|(name, value)| name == FIELD_WINDOW_KEY && value == window_key)
        })
        .count()
}

fn recovery_counts(records: &[TraceRecord], window_key: &str) -> RecoveryCounts {
    RecoveryCounts {
        accepted:     record_count(records, KIND_RECOVERY_ACCEPTED, window_key),
        pending:      record_count(records, KIND_RECOVERY_PENDING, window_key),
        available:    record_count(records, KIND_RECOVERY_AVAILABLE, window_key),
        restored:     record_count(records, KIND_RECOVERY_RESTORED, window_key),
        mismatch:     record_count(records, KIND_RECOVERY_MISMATCH, window_key),
        cancellation: record_count(records, KIND_RECOVERY_CANCELLATION_REQUESTED, window_key),
    }
}

fn binding_ready(
    bindings: &Bindings,
    role: ProbeWindowRole,
    expected_policy: RecoveryPolicy,
) -> bool {
    role_key(role).is_some_and(|role| {
        bindings.binding(&role).is_ok_and(|binding| {
            binding.recovery == expected_policy && binding.state == RoleState::Ready
        })
    })
}

fn accepted_recorded(records: &[TraceRecord], role: ProbeWindowRole) -> bool {
    record_count(records, KIND_RECOVERY_ACCEPTED, role.key()) == 1
}

/// Records the one-way readiness transition only after exact bindings and monitor placement exist.
pub(crate) fn record_probe_readiness(
    startup_mode: Res<ProbeStartupMode>,
    monitor_index: Res<ProbeMonitorIndex>,
    monitors: Res<Monitors>,
    association: Res<MonitorDeviceAssociation>,
    bindings: Res<Bindings>,
    windows: Query<(
        &ProbeWindowRole,
        &OnMonitor,
        Has<ProbeWindowRegistrationComplete>,
    )>,
    mut readiness: ResMut<ProbeReadiness>,
    trace: Res<ProbeTrace>,
    frame_count: Res<FrameCount>,
) {
    if *readiness == ProbeReadiness::Ready {
        return;
    }
    let Some(target) = monitors
        .iter()
        .find(|monitor| monitor.descriptor.index.adapter_value() == monitor_index.0)
    else {
        return;
    };
    if matches!(
        association.device_for_descriptor(*target.descriptor),
        MonitorDeviceKeyLookup::Unresolved
    ) {
        return;
    }
    let all_windows_placed = windows
        .iter()
        .all(|(_, on_monitor, registered)| registered && on_monitor.0 == target.entity);
    if windows.iter().count() != PROBE_WINDOW_COUNT || !all_windows_placed {
        return;
    }
    let records = trace.records();
    let common_roles_ready = binding_ready(
        &bindings,
        ProbeWindowRole::Primary,
        RecoveryPolicy::ReapplyOnReturn,
    ) && binding_ready(
        &bindings,
        ProbeWindowRole::Application,
        RecoveryPolicy::ReapplyOnRequest,
    ) && accepted_recorded(&records, ProbeWindowRole::Primary)
        && accepted_recorded(&records, ProbeWindowRole::Application);
    if !common_roles_ready {
        return;
    }
    let automatic_ready = match *startup_mode {
        ProbeStartupMode::Exclusive => role_key(ProbeWindowRole::Automatic)
            .is_some_and(|role| bindings.binding(&role).is_err()),
        ProbeStartupMode::Windowed | ProbeStartupMode::Borderless => {
            binding_ready(
                &bindings,
                ProbeWindowRole::Automatic,
                RecoveryPolicy::ReapplyOnReturn,
            ) && accepted_recorded(&records, ProbeWindowRole::Automatic)
        },
    };
    if !automatic_ready {
        return;
    }
    trace.record(
        frame_count.0,
        PRODUCER_RECOVERY_READY,
        KIND_RECOVERY_READY,
        vec![(
            FIELD_SELECTED_MONITOR_INDEX.into(),
            monitor_index.0.to_string(),
        )],
    );
    *readiness = ProbeReadiness::Ready;
}

pub(super) fn snapshot(world: &mut World) -> ProbeSnapshot {
    let window_values = {
        let mut windows_query = world.query::<(
            Entity,
            &Window,
            &ProbeWindowRole,
            Option<&CurrentMonitor>,
            Option<&OnMonitor>,
        )>();
        windows_query
            .iter(world)
            .filter(|(_, _, role, ..)| **role != ProbeWindowRole::Control)
            .map(|(entity, window, role, current_monitor, on_monitor)| {
                (
                    entity,
                    window.clone(),
                    *role,
                    current_monitor.copied(),
                    on_monitor.map(|on_monitor| on_monitor.0),
                )
            })
            .collect::<Vec<_>>()
    };
    let session = world.resource::<ProbeSession>();
    let run_id = session.run_id.clone();
    let boot_nonce = session.boot_nonce.clone();
    let startup_mode = world.resource::<ProbeStartupMode>().selector();
    let selected_monitor_index = world.resource::<ProbeMonitorIndex>().0;
    let topology_revision = world
        .resource::<hana_clerestory::MonitorTopologyRevision>()
        .get();
    let readiness = *world.resource::<ProbeReadiness>();
    let trace = world.resource::<ProbeTrace>().clone();
    let records = trace.records();
    let record_cursor = records.last().map_or(0, |record| record.sequence);
    let terminal_failure: Presence = records
        .iter()
        .any(|record| record.kind == KIND_RECOVERY_MISMATCH)
        .into();
    let command_receipts = world
        .resource::<CommandReceipts>()
        .0
        .values()
        .cloned()
        .collect();
    let association = world.resource::<MonitorDeviceAssociation>();
    let monitors_resource = world.resource::<Monitors>();
    let monitors = monitors_resource
        .iter()
        .map(|monitor| {
            let name = world
                .get::<bevy::window::Monitor>(monitor.entity)
                .and_then(|monitor| monitor.name.clone());
            MonitorSnapshot::from_descriptor(monitor.entity, name, *monitor.descriptor, association)
        })
        .collect();
    let bindings = world.resource::<Bindings>();
    let windows = window_values
        .iter()
        .map(|(entity, window, role, current_monitor, on_monitor)| {
            window_snapshot(
                *entity,
                window,
                *role,
                *current_monitor,
                *on_monitor,
                monitors_resource,
                association,
                bindings,
                &records,
            )
        })
        .collect();
    ProbeSnapshot {
        schema_version: PROBE_SCHEMA_VERSION,
        run_id,
        boot_nonce,
        readiness: readiness.into(),
        startup_mode,
        selected_monitor_index,
        topology_revision,
        record_cursor,
        monitors,
        windows,
        terminal_failure,
        command_receipts,
    }
}

fn current_monitor_snapshot(
    current_monitor: Option<CurrentMonitor>,
    on_monitor: Option<Entity>,
    monitors: &Monitors,
    association: &MonitorDeviceAssociation,
) -> Option<MonitorSnapshot> {
    let descriptor = current_monitor
        .map(|current_monitor| current_monitor.descriptor)
        .or_else(|| {
            on_monitor.and_then(|on_monitor| {
                monitors
                    .iter()
                    .find(|monitor| monitor.entity == on_monitor)
                    .map(|monitor| *monitor.descriptor)
            })
        })?;
    let entity = on_monitor.or_else(|| {
        monitors
            .iter()
            .find(|monitor| *monitor.descriptor == descriptor)
            .map(|monitor| monitor.entity)
    })?;
    Some(MonitorSnapshot::from_descriptor(
        entity,
        None,
        descriptor,
        association,
    ))
}

fn window_snapshot(
    entity: Entity,
    window: &Window,
    role: ProbeWindowRole,
    current_monitor: Option<CurrentMonitor>,
    on_monitor: Option<Entity>,
    monitors: &Monitors,
    association: &MonitorDeviceAssociation,
    bindings: &Bindings,
    records: &[TraceRecord],
) -> WindowSnapshot {
    let binding = role_key(role).and_then(|role| bindings.binding(&role).ok());
    let descriptor = current_monitor
        .map(|current_monitor| current_monitor.descriptor)
        .or_else(|| {
            on_monitor.and_then(|on_monitor| {
                monitors
                    .iter()
                    .find(|monitor| monitor.entity == on_monitor)
                    .map(|monitor| *monitor.descriptor)
            })
        });
    let monitor_coverage: MonitorCoverage = descriptor
        .is_some_and(|descriptor| {
            window.resolution.physical_size() == descriptor.physical_size
                && match window.position {
                    WindowPosition::At(position) => position == descriptor.physical_position,
                    WindowPosition::Automatic | WindowPosition::Centered(_) => false,
                }
        })
        .into();
    let created = record_count(records, KIND_WINDOW_CREATED, role.key());
    WindowSnapshot {
        key: role.key().into(),
        entity: entity.to_bits(),
        recovery_policy: binding.map(|binding| format!("{:?}", binding.recovery)),
        current_monitor: current_monitor_snapshot(
            current_monitor,
            on_monitor,
            monitors,
            association,
        ),
        requested_mode: format!("{:?}", window.mode),
        effective_mode: current_monitor
            .map(|current_monitor| format!("{:?}", current_monitor.effective_window_mode)),
        position: format!("{:?}", window.position),
        physical_size: [
            window.resolution.physical_width(),
            window.resolution.physical_height(),
        ],
        decoration_presence: window.decorations.into(),
        focus: window.focused.into(),
        native_fullscreen: native_fullscreen(entity),
        monitor_coverage,
        replacement_count: created.saturating_sub(1),
        recovery_counts: recovery_counts(records, role.key()),
    }
}

#[cfg(target_os = "macos")]
fn native_fullscreen(entity: Entity) -> NativeFullscreen {
    use bevy::winit::WINIT_WINDOWS;
    use objc2_app_kit::NSView;
    use objc2_app_kit::NSWindowStyleMask;
    use raw_window_handle::HasWindowHandle;
    use raw_window_handle::RawWindowHandle;

    WINIT_WINDOWS
        .with_borrow(|winit_windows| {
            let winit_window = winit_windows.get_window(entity)?;
            let handle = winit_window.window_handle().ok()?;
            let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() else {
                return None;
            };
            // SAFETY: `ns_view` comes from the live winit window handle above.
            let ns_view: &NSView = unsafe { appkit_handle.ns_view.cast().as_ref() };
            let window = ns_view.window()?;
            Some(window.styleMask().contains(NSWindowStyleMask::FullScreen))
        })
        .into()
}

#[cfg(not(target_os = "macos"))]
const fn native_fullscreen(_: Entity) -> NativeFullscreen { NativeFullscreen::Unavailable }

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn snapshot_wire_keeps_readiness_and_nested_monitor_window_fields()
    -> Result<(), serde_json::Error> {
        let snapshot = ProbeSnapshot {
            schema_version:         PROBE_SCHEMA_VERSION,
            run_id:                 "run".into(),
            boot_nonce:             "boot".into(),
            readiness:              Readiness::Ready,
            startup_mode:           "windowed",
            selected_monitor_index: 1,
            topology_revision:      2,
            record_cursor:          3,
            monitors:               vec![MonitorSnapshot {
                entity:            4,
                name:              Some("display".into()),
                identity:          "Verified(display)".into(),
                verified_id:       Some("display".into()),
                index:             1,
                scale:             2.0,
                physical_position: [0, 0],
                physical_size:     [1_920, 1_080],
            }],
            windows:                vec![WindowSnapshot {
                key:                 "primary".into(),
                entity:              5,
                recovery_policy:     Some("ReapplyOnReturn".into()),
                current_monitor:     Some(MonitorSnapshot {
                    entity:            4,
                    name:              None,
                    identity:          "Verified(display)".into(),
                    verified_id:       Some("display".into()),
                    index:             1,
                    scale:             2.0,
                    physical_position: [0, 0],
                    physical_size:     [1_920, 1_080],
                }),
                requested_mode:      "Windowed".into(),
                effective_mode:      Some("Windowed".into()),
                position:            "At(IVec2(0, 0))".into(),
                physical_size:       [800, 540],
                decoration_presence: Presence::Present,
                focus:               Focus::Focused,
                native_fullscreen:   NativeFullscreen::Unavailable,
                monitor_coverage:    MonitorCoverage::Partial,
                replacement_count:   0,
                recovery_counts:     RecoveryCounts::default(),
            }],
            terminal_failure:       Presence::Absent,
            command_receipts:       Vec::new(),
        };

        let value = serde_json::to_value(snapshot)?;
        assert_eq!(value.get("ready"), Some(&Value::String("ready".into())));
        assert!(value.get("selected_monitor_index").is_some());
        assert!(value["monitors"][0].get("verified_id").is_some());
        assert!(value["monitors"][0].get("physical_position").is_some());
        assert!(value["windows"][0].get("key").is_some());
        assert!(
            value["windows"][0]["current_monitor"]
                .get("verified_id")
                .is_some()
        );
        assert!(value["windows"][0].get("recovery_counts").is_some());
        assert!(value.get("terminal_failure").is_some());
        Ok(())
    }
}
