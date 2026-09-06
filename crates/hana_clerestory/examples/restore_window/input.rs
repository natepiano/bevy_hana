use std::collections::HashMap;
use std::env::current_exe;
use std::path::PathBuf;

use bevy::prelude::AppExit;
use bevy::prelude::ButtonInput;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MessageWriter;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::Monitor;
use bevy::window::VideoMode;
use bevy::window::VideoModeSelection;
use bevy::window::WindowMode;
use dirs::config_dir;
use hana_clerestory::CurrentMonitor;
use hana_clerestory::CurrentMonitorIndex;
use hana_clerestory::ManagedWindowName;
#[cfg(target_os = "linux")]
use hana_clerestory::Platform;

use super::constants::ACTIVE_VIDEO_MODE_SUFFIX;
use super::constants::BACKWARD_SCROLL_OFFSET;
use super::constants::DEFAULT_VIDEO_MODE_INDEX;
use super::constants::FORWARD_SCROLL_OFFSET;
use super::constants::MILLIHERTZ_PER_HERTZ;
use super::constants::MONITOR_ID_LABEL;
use super::constants::MONITOR_INDEX_LABEL;
use super::constants::NO_VIDEO_MODES_TEXT;
use super::constants::NON_PRIMARY_MONITOR_MARKER;
use super::constants::NOT_AVAILABLE_TEXT;
use super::constants::PRIMARY_MONITOR_INDEX;
use super::constants::PRIMARY_MONITOR_MARKER;
use super::constants::REFRESH_RATE_LABEL;
use super::constants::SCALE_LABEL;
use super::constants::SELECTED_VIDEO_MODE_MARKER;
use super::constants::STATE_FILE;
use super::constants::UNSELECTED_VIDEO_MODE_MARKER;
use super::constants::VIDEO_MODE_CENTER_PADDING;
use super::constants::VISIBLE_VIDEO_MODE_COUNT;
#[cfg(target_os = "linux")]
use super::constants::WAYLAND_PLATFORM_SUFFIX;
#[cfg(target_os = "linux")]
use super::constants::X11_PLATFORM_SUFFIX;
use super::events::ClearStateAndQuit;
use super::events::QuitApp;
use super::events::RestoredStates;
use super::events::SetBorderlessFullscreen;
use super::events::SetExclusiveFullscreen;
use super::events::SetWindowed;
use super::events::SpawnManagedWindow;
use super::events::TogglePersistence;

#[derive(Resource, Clone, Copy)]
pub(crate) enum KeyboardInputMode {
    Enabled,
    Disabled,
}

impl From<bool> for KeyboardInputMode {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct SelectedVideoModes {
    indices:   HashMap<CurrentMonitorIndex, usize>,
    last_sync: Option<(UVec2, u32)>,
}

impl SelectedVideoModes {
    pub(crate) fn get(&self, monitor_index: CurrentMonitorIndex) -> usize {
        self.indices
            .get(&monitor_index)
            .copied()
            .unwrap_or(DEFAULT_VIDEO_MODE_INDEX)
    }

    fn set(&mut self, monitor_index: CurrentMonitorIndex, index: usize) {
        self.indices.insert(monitor_index, index);
    }
}

pub(crate) fn keyboard_enabled(input_mode: Res<KeyboardInputMode>) -> bool {
    matches!(*input_mode, KeyboardInputMode::Enabled)
}

pub(crate) fn handle_global_input(
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    mut commands: Commands,
) {
    if !windows.iter().any(|window| window.focused) {
        return;
    }

    if keys.just_pressed(KeyCode::Space) {
        commands.trigger(SpawnManagedWindow);
    }
    if keys.just_pressed(KeyCode::KeyP) {
        commands.trigger(TogglePersistence);
    }
    if keys.just_pressed(KeyCode::Backspace)
        && keys.pressed(KeyCode::ShiftLeft)
        && keys.pressed(KeyCode::ControlLeft)
    {
        commands.trigger(ClearStateAndQuit);
    }
    if keys.just_pressed(KeyCode::KeyQ) {
        commands.trigger(QuitApp);
    }
}

pub(crate) fn despawn_managed_and_exit(
    managed_entities: &Query<Entity, With<ManagedWindowName>>,
    commands: &mut Commands,
    app_exit: &mut MessageWriter<AppExit>,
) {
    for entity in managed_entities.iter() {
        commands.entity(entity).despawn();
    }
    app_exit.write(AppExit::Success);
}

pub(crate) fn get_state_file_path() -> Option<PathBuf> {
    let executable_name = current_exe().ok()?.file_stem()?.to_str()?.to_string();
    config_dir().map(|config_dir| config_dir.join(executable_name).join(STATE_FILE))
}

pub(crate) const fn resolve_current_monitor(
    current_monitor: Option<&CurrentMonitor>,
) -> Option<CurrentMonitor> {
    current_monitor.copied()
}

pub(crate) fn handle_window_mode_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut windows: Query<(Entity, &mut Window, Option<&CurrentMonitor>)>,
    bevy_monitors: Query<(Entity, &Monitor)>,
    mut selected_video_modes: ResMut<SelectedVideoModes>,
    restored_states: Res<RestoredStates>,
    mut commands: Commands,
) {
    let Some((entity, mut window, maybe_current_monitor)) =
        windows.iter_mut().find(|(_, window, _)| window.focused)
    else {
        return;
    };

    let Some(current_monitor) = resolve_current_monitor(maybe_current_monitor) else {
        return;
    };

    let is_fullscreen = !matches!(window.mode, WindowMode::Windowed);
    let restore_complete = restored_states.by_entity.contains_key(&entity);
    let mode_desynced = window.mode != current_monitor.effective_window_mode;
    if restore_complete && !is_fullscreen && mode_desynced {
        window.mode = current_monitor.effective_window_mode;
    }

    let video_modes: Vec<VideoMode> = bevy_monitors
        .iter()
        .find(|(_, bevy_monitor)| {
            bevy_monitor.physical_position == current_monitor.physical_position
        })
        .map(|(_, bevy_monitor)| bevy_monitor.video_modes.clone())
        .unwrap_or_default();

    let current_idx = selected_video_modes.get(current_monitor.index);
    if keys.just_pressed(KeyCode::ArrowUp) && current_idx > 0 {
        selected_video_modes.set(current_monitor.index, current_idx - 1);
    }
    if keys.just_pressed(KeyCode::ArrowDown) && current_idx < video_modes.len().saturating_sub(1) {
        selected_video_modes.set(current_monitor.index, current_idx + 1);
    }

    if keys.just_pressed(KeyCode::Enter) {
        commands.trigger(SetExclusiveFullscreen);
    }
    if keys.just_pressed(KeyCode::KeyB) {
        commands.trigger(SetBorderlessFullscreen);
    }
    if keys.just_pressed(KeyCode::KeyW) {
        commands.trigger(SetWindowed);
    }
}

pub(crate) fn get_video_modes_for_monitor<'a>(
    bevy_monitors: &'a Query<(Entity, &Monitor)>,
    current_monitor: &CurrentMonitor,
) -> (Vec<&'a VideoMode>, Option<u32>) {
    bevy_monitors
        .iter()
        .find(|(_, bevy_monitor)| {
            bevy_monitor.physical_position == current_monitor.physical_position
        })
        .map(|(_, bevy_monitor)| {
            (
                bevy_monitor.video_modes.iter().collect(),
                bevy_monitor
                    .refresh_rate_millihertz
                    .map(|rate| rate / MILLIHERTZ_PER_HERTZ),
            )
        })
        .unwrap_or_default()
}

pub(crate) fn format_refresh_rate(window: &Window, monitor_refresh: Option<u32>) -> String {
    let active_refresh = match &window.mode {
        WindowMode::Fullscreen(_, VideoModeSelection::Specific(mode)) => {
            Some(mode.refresh_rate_millihertz / MILLIHERTZ_PER_HERTZ)
        },
        _ => monitor_refresh,
    };
    active_refresh.map_or_else(|| NOT_AVAILABLE_TEXT.into(), |hz| format!("{hz}Hz"))
}

pub(crate) fn find_active_video_mode_index(
    window: &Window,
    video_modes: &[&VideoMode],
) -> Option<usize> {
    match &window.mode {
        WindowMode::Fullscreen(_, VideoModeSelection::Specific(active)) => {
            video_modes.iter().position(|mode| {
                mode.physical_size == active.physical_size
                    && mode.refresh_rate_millihertz == active.refresh_rate_millihertz
            })
        },
        _ => None,
    }
}

pub(crate) fn sync_selected_to_active(
    window: &Window,
    current_monitor: &CurrentMonitor,
    active_mode_idx: Option<usize>,
    selected_video_modes: &mut SelectedVideoModes,
) {
    if let WindowMode::Fullscreen(_, VideoModeSelection::Specific(active)) = &window.mode {
        let current_mode = (active.physical_size, active.refresh_rate_millihertz);
        if selected_video_modes.last_sync != Some(current_mode)
            && let Some(active_idx) = active_mode_idx
        {
            selected_video_modes.set(current_monitor.index, active_idx);
            selected_video_modes.last_sync = Some(current_mode);
        }
    } else {
        selected_video_modes.last_sync = None;
    }
}

#[cfg(target_os = "linux")]
fn platform_suffix() -> &'static str {
    if Platform::detect().is_wayland() {
        WAYLAND_PLATFORM_SUFFIX
    } else {
        X11_PLATFORM_SUFFIX
    }
}

#[cfg(not(target_os = "linux"))]
const fn platform_suffix() -> &'static str { "" }

pub(crate) fn format_monitor_row(
    current_monitor: &CurrentMonitor,
    refresh_display: &str,
) -> String {
    let primary_marker = if current_monitor.index
        == CurrentMonitorIndex::from_current_enumeration(PRIMARY_MONITOR_INDEX)
    {
        PRIMARY_MONITOR_MARKER
    } else {
        NON_PRIMARY_MONITOR_MARKER
    };
    format!(
        "{MONITOR_ID_LABEL} {:?}\n{MONITOR_INDEX_LABEL} {}{primary_marker} {SCALE_LABEL} {} - {REFRESH_RATE_LABEL} {refresh_display}{}",
        "live display",
        current_monitor.index,
        current_monitor.scale,
        platform_suffix()
    )
}

pub(crate) fn build_video_modes_display(
    video_modes: &[&VideoMode],
    selected_idx: usize,
    active_mode_idx: Option<usize>,
) -> String {
    if video_modes.is_empty() {
        return NO_VIDEO_MODES_TEXT.into();
    }

    let selected_idx = selected_idx.min(video_modes.len().saturating_sub(1));
    let len = video_modes.len();

    let start = if len <= VISIBLE_VIDEO_MODE_COUNT {
        0
    } else {
        let center_target = active_mode_idx.unwrap_or(selected_idx);
        let ideal_start = center_target.saturating_sub(VIDEO_MODE_CENTER_PADDING);
        let ideal_end = ideal_start + VISIBLE_VIDEO_MODE_COUNT;

        if selected_idx < ideal_start {
            selected_idx.saturating_sub(BACKWARD_SCROLL_OFFSET)
        } else if selected_idx >= ideal_end {
            (selected_idx + FORWARD_SCROLL_OFFSET).saturating_sub(VISIBLE_VIDEO_MODE_COUNT)
        } else {
            ideal_start
        }
        .min(len.saturating_sub(VISIBLE_VIDEO_MODE_COUNT))
    };
    let end = (start + VISIBLE_VIDEO_MODE_COUNT).min(len);

    video_modes[start..end]
        .iter()
        .enumerate()
        .map(|(i, mode)| {
            let actual_idx = start + i;
            let left_marker = if actual_idx == selected_idx {
                SELECTED_VIDEO_MODE_MARKER
            } else {
                UNSELECTED_VIDEO_MODE_MARKER
            };
            let right_marker = if Some(actual_idx) == active_mode_idx {
                ACTIVE_VIDEO_MODE_SUFFIX
            } else {
                ""
            };
            format!(
                "  {left_marker} {}x{} @ {}Hz{right_marker}",
                mode.physical_size.x,
                mode.physical_size.y,
                mode.refresh_rate_millihertz / MILLIHERTZ_PER_HERTZ
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
