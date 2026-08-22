use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Changed;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::MessageReader;
use bevy::prelude::Res;
use bevy::prelude::Single;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::debug;
use bevy::prelude::info;
use bevy::window::PrimaryWindow;
use bevy::window::WindowMode;
use bevy::window::WindowPosition;
use bevy::window::WindowScaleFactorChanged;
use bevy::winit::WINIT_WINDOWS;
use hana_clerestory::CurrentMonitorIndex;
use hana_clerestory::Monitors;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum FocusState {
    Focused,
    #[default]
    Unfocused,
}

impl From<bool> for FocusState {
    fn from(focused: bool) -> Self {
        if focused {
            Self::Focused
        } else {
            Self::Unfocused
        }
    }
}

#[derive(Default)]
pub(crate) struct CachedWindowDebug {
    physical_position: Option<WindowPosition>,
    physical_width:    u32,
    physical_height:   u32,
    window_mode:       Option<WindowMode>,
    focus_state:       FocusState,
}

/// Logs every live monitor descriptor and entity at startup.
pub(crate) fn log_monitor_ids(monitors: Res<Monitors>) {
    for live_monitor in monitors.iter() {
        let monitor = live_monitor.descriptor;
        info!(
            "[log_monitor_ids] index={} entity={:?} position={:?} size={} scale={}",
            monitor.index,
            live_monitor.entity,
            monitor.physical_position,
            monitor.physical_size,
            monitor.scale
        );
    }
}

pub(crate) fn debug_winit_monitor(
    window: Single<Entity, With<PrimaryWindow>>,
    monitors: Res<Monitors>,
    mut cached_monitor: Local<Option<CurrentMonitorIndex>>,
    _: NonSendMarker,
) {
    let window_entity = *window;

    let winit_monitor_index: Option<CurrentMonitorIndex> = WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        winit_windows
            .get_window(window_entity)
            .and_then(|winit_window| {
                winit_window.current_monitor().and_then(|current_monitor| {
                    let physical_position = current_monitor.position();
                    monitors
                        .at(physical_position.x, physical_position.y)
                        .map(|monitor| monitor.index)
                })
            })
    });

    if *cached_monitor != winit_monitor_index {
        debug!(
            "[debug_winit_monitor] Monitor changed: {:?} -> {:?}",
            *cached_monitor, winit_monitor_index
        );
        *cached_monitor = winit_monitor_index;
    }
}

pub(crate) fn debug_window_changed(
    window: Single<&Window, (With<PrimaryWindow>, Changed<Window>)>,
    mut cached: Local<CachedWindowDebug>,
) {
    let window = *window;

    let position_changed = cached.physical_position.as_ref() != Some(&window.position);
    let size_changed = cached.physical_width != window.physical_width()
        || cached.physical_height != window.physical_height();
    let mode_changed = cached.window_mode.as_ref() != Some(&window.mode);
    let focus_state = FocusState::from(window.focused);
    let focused_changed = cached.focus_state != focus_state;

    let mut changes = Vec::new();
    if position_changed {
        changes.push(format!(
            "position: {:?} -> {:?}",
            cached.physical_position, window.position
        ));
    }
    if size_changed {
        changes.push(format!(
            "size: {}x{} -> {}x{}",
            cached.physical_width,
            cached.physical_height,
            window.physical_width(),
            window.physical_height()
        ));
    }
    if mode_changed {
        changes.push(format!(
            "mode: {:?} -> {:?}",
            cached.window_mode, window.mode
        ));
    }
    if focused_changed {
        changes.push(format!(
            "focused: {:?} -> {:?}",
            cached.focus_state, focus_state
        ));
    }

    if !changes.is_empty() {
        debug!("[debug_window_changed] {}", changes.join(", "));
    }

    cached.physical_position = Some(window.position);
    cached.physical_width = window.physical_width();
    cached.physical_height = window.physical_height();
    cached.window_mode = Some(window.mode);
    cached.focus_state = focus_state;
}

pub(crate) fn debug_scale_factor_changed(mut messages: MessageReader<WindowScaleFactorChanged>) {
    for message in messages.read() {
        debug!(
            "[debug_scale_factor_changed] WindowScaleFactorChanged received: scale_factor={}",
            message.scale_factor
        );
    }
}
