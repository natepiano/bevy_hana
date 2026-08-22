use std::collections::HashMap;

use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::IVec2;
use bevy::prelude::On;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ReflectResource;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
use bevy::prelude::info;
use bevy::prelude::warn;
use bevy::window::WindowMode;
use hana_clerestory::ExpectedLogicalPosition;
use hana_clerestory::ExpectedPhysicalPosition;
use hana_clerestory::ObservedLogicalPosition;
use hana_clerestory::ObservedPhysicalPosition;
use hana_clerestory::WindowRestoreMismatch;
use hana_clerestory::WindowRestored;

#[derive(Event, Reflect)]
#[reflect(Event)]
pub(crate) struct SpawnManagedWindow;

#[derive(Event, Reflect)]
#[reflect(Event)]
pub(crate) struct SetBorderlessFullscreen;

#[derive(Event, Reflect)]
#[reflect(Event)]
pub(crate) struct SetWindowed;

#[derive(Event, Reflect)]
#[reflect(Event)]
pub(crate) struct SetExclusiveFullscreen;

#[derive(Event, Reflect)]
#[reflect(Event)]
pub(crate) struct TogglePersistence;

#[derive(Event, Reflect)]
#[reflect(Event)]
pub(crate) struct ClearStateAndQuit;

#[derive(Event, Reflect)]
#[reflect(Event)]
pub(crate) struct QuitApp;

#[derive(Debug, Clone, Reflect)]
pub(crate) struct MonitorDifference {
    pub(crate) expected: usize,
    pub(crate) actual:   usize,
}

#[derive(Debug, Clone, Reflect)]
pub(crate) struct WindowModeDifference {
    pub(crate) expected: WindowMode,
    pub(crate) actual:   WindowMode,
}

#[derive(Debug, Clone, Reflect)]
pub(crate) struct PhysicalPositionMismatch {
    pub(crate) expected: Option<IVec2>,
    pub(crate) actual:   Option<IVec2>,
}

#[derive(Debug, Clone, Reflect)]
pub(crate) struct LogicalPositionMismatch {
    pub(crate) expected: Option<IVec2>,
    pub(crate) actual:   Option<IVec2>,
}

#[derive(Debug, Clone, Reflect)]
pub(crate) struct PhysicalSizeMismatch {
    pub(crate) expected: UVec2,
    pub(crate) actual:   UVec2,
}

#[derive(Debug, Clone, Reflect)]
pub(crate) struct LogicalSizeMismatch {
    pub(crate) expected: UVec2,
    pub(crate) actual:   UVec2,
}

#[derive(Debug, Clone, Reflect)]
pub(crate) struct ScaleFactorDifference {
    pub(crate) expected: f64,
    pub(crate) actual:   f64,
}

#[derive(Resource, Debug, Clone, Reflect)]
#[reflect(Resource)]
struct WindowRestoredReceived {
    physical_position: Option<IVec2>,
    physical_size:     UVec2,
    window_mode:       WindowMode,
    monitor:           usize,
}

/// Adapts the flat `expected_*` / `actual_*` shape of `WindowRestoreMismatch` into
/// nested comparison structs for BRP inspection. If the public event's field layout
/// changes, this resource's unpacking (in `on_window_restore_mismatch`) must change with it.
#[derive(Resource, Debug, Clone, Reflect)]
#[reflect(Resource)]
struct WindowRestoreMismatchReceived {
    monitor_difference:     MonitorDifference,
    physical_size_mismatch: PhysicalSizeMismatch,
    window_mode_difference: WindowModeDifference,
}

#[derive(Resource, Debug, Default, Reflect)]
#[reflect(Resource)]
pub(crate) struct WindowsSettledCount {
    value: usize,
}

#[derive(Clone)]
pub(crate) struct CachedMismatchState {
    pub(crate) physical_position_mismatch: PhysicalPositionMismatch,
    pub(crate) logical_position_mismatch:  LogicalPositionMismatch,
    pub(crate) physical_size_mismatch:     PhysicalSizeMismatch,
    pub(crate) logical_size_mismatch:      LogicalSizeMismatch,
    pub(crate) window_mode_difference:     WindowModeDifference,
    pub(crate) monitor_difference:         MonitorDifference,
    pub(crate) scale_factor_difference:    ScaleFactorDifference,
}

#[derive(Resource, Default)]
pub(crate) struct MismatchStates {
    pub(crate) by_entity: HashMap<Entity, CachedMismatchState>,
}

#[derive(Resource, Default)]
pub(crate) struct RestoredStates {
    pub(crate) by_entity: HashMap<Entity, CachedRestoredState>,
}

pub(crate) struct CachedRestoredState {
    pub(crate) physical_position: Option<IVec2>,
    pub(crate) logical_position:  Option<IVec2>,
    pub(crate) physical_size:     UVec2,
    pub(crate) logical_size:      UVec2,
    pub(crate) monitor:           usize,
    pub(crate) window_mode:       WindowMode,
}

pub(crate) fn on_window_restored(
    trigger: On<WindowRestored>,
    mut commands: Commands,
    mut restored_states: ResMut<RestoredStates>,
    mut settled_count: ResMut<WindowsSettledCount>,
) {
    let window_restored = trigger.event();
    info!(
        "[on_window_restored] Restore complete: role={} entity={:?} physical_position={:?} logical_position={:?} physical_size={} logical_size={} mode={:?} monitor={}",
        window_restored.role,
        window_restored.entity,
        window_restored.physical_position,
        window_restored.logical_position,
        window_restored.physical_size,
        window_restored.logical_size,
        window_restored.window_mode,
        window_restored.monitor_index
    );

    restored_states.by_entity.insert(
        window_restored.entity,
        CachedRestoredState {
            physical_position: expected_physical_position(window_restored.physical_position),
            logical_position:  expected_logical_position(window_restored.logical_position),
            physical_size:     window_restored.physical_size,
            logical_size:      window_restored.logical_size,
            monitor:           window_restored.monitor_index,
            window_mode:       window_restored.window_mode,
        },
    );

    commands.insert_resource(WindowRestoredReceived {
        physical_position: expected_physical_position(window_restored.physical_position),
        physical_size:     window_restored.physical_size,
        window_mode:       window_restored.window_mode,
        monitor:           window_restored.monitor_index,
    });
    settled_count.value += 1;
}

pub(crate) fn on_window_restore_mismatch(
    trigger: On<WindowRestoreMismatch>,
    mut commands: Commands,
    mut restored_states: ResMut<RestoredStates>,
    mut mismatch_states: ResMut<MismatchStates>,
    mut settled_count: ResMut<WindowsSettledCount>,
) {
    let window_restore_mismatch = trigger.event();
    warn!(
        "[on_window_restore_mismatch] role={} entity={:?} \
         monitor: {} vs {}, size: {} vs {}, mode: {:?} vs {:?}",
        window_restore_mismatch.role,
        window_restore_mismatch.entity,
        window_restore_mismatch.expected_monitor,
        window_restore_mismatch.actual_monitor,
        window_restore_mismatch.expected_physical_size,
        window_restore_mismatch.actual_physical_size,
        window_restore_mismatch.expected_window_mode,
        window_restore_mismatch.actual_window_mode,
    );

    restored_states.by_entity.insert(
        window_restore_mismatch.entity,
        CachedRestoredState {
            physical_position: expected_physical_position(
                window_restore_mismatch.expected_physical_position,
            ),
            logical_position:  expected_logical_position(
                window_restore_mismatch.expected_logical_position,
            ),
            physical_size:     window_restore_mismatch.expected_physical_size,
            logical_size:      window_restore_mismatch.expected_logical_size,
            monitor:           window_restore_mismatch.expected_monitor,
            window_mode:       window_restore_mismatch.expected_window_mode,
        },
    );

    mismatch_states.by_entity.insert(
        window_restore_mismatch.entity,
        CachedMismatchState {
            physical_position_mismatch: PhysicalPositionMismatch {
                expected: expected_physical_position(
                    window_restore_mismatch.expected_physical_position,
                ),
                actual:   observed_physical_position(
                    window_restore_mismatch.actual_physical_position,
                ),
            },
            logical_position_mismatch:  LogicalPositionMismatch {
                expected: expected_logical_position(
                    window_restore_mismatch.expected_logical_position,
                ),
                actual:   observed_logical_position(
                    window_restore_mismatch.actual_logical_position,
                ),
            },
            physical_size_mismatch:     PhysicalSizeMismatch {
                expected: window_restore_mismatch.expected_physical_size,
                actual:   window_restore_mismatch.actual_physical_size,
            },
            logical_size_mismatch:      LogicalSizeMismatch {
                expected: window_restore_mismatch.expected_logical_size,
                actual:   window_restore_mismatch.actual_logical_size,
            },
            window_mode_difference:     WindowModeDifference {
                expected: window_restore_mismatch.expected_window_mode,
                actual:   window_restore_mismatch.actual_window_mode,
            },
            monitor_difference:         MonitorDifference {
                expected: window_restore_mismatch.expected_monitor,
                actual:   window_restore_mismatch.actual_monitor,
            },
            scale_factor_difference:    ScaleFactorDifference {
                expected: window_restore_mismatch.expected_scale,
                actual:   window_restore_mismatch.actual_scale,
            },
        },
    );

    commands.insert_resource(WindowRestoreMismatchReceived {
        monitor_difference:     MonitorDifference {
            expected: window_restore_mismatch.expected_monitor,
            actual:   window_restore_mismatch.actual_monitor,
        },
        physical_size_mismatch: PhysicalSizeMismatch {
            expected: window_restore_mismatch.expected_physical_size,
            actual:   window_restore_mismatch.actual_physical_size,
        },
        window_mode_difference: WindowModeDifference {
            expected: window_restore_mismatch.expected_window_mode,
            actual:   window_restore_mismatch.actual_window_mode,
        },
    });
    settled_count.value += 1;
}

const fn expected_physical_position(position: ExpectedPhysicalPosition) -> Option<IVec2> {
    match position {
        ExpectedPhysicalPosition::Specified(position) => Some(position),
        ExpectedPhysicalPosition::PlatformCannotPosition
        | ExpectedPhysicalPosition::NotSaved
        | ExpectedPhysicalPosition::DiscardedLegacy => None,
    }
}

const fn expected_logical_position(position: ExpectedLogicalPosition) -> Option<IVec2> {
    match position {
        ExpectedLogicalPosition::Specified(position) => Some(position),
        ExpectedLogicalPosition::PlatformCannotPosition
        | ExpectedLogicalPosition::NotSaved
        | ExpectedLogicalPosition::DiscardedLegacy => None,
    }
}

const fn observed_physical_position(position: ObservedPhysicalPosition) -> Option<IVec2> {
    match position {
        ObservedPhysicalPosition::Observed(position) => Some(position),
        ObservedPhysicalPosition::PlatformCannotReport => None,
    }
}

const fn observed_logical_position(position: ObservedLogicalPosition) -> Option<IVec2> {
    match position {
        ObservedLogicalPosition::Observed(position) => Some(position),
        ObservedLogicalPosition::PlatformCannotReport => None,
    }
}
