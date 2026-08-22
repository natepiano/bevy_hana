use std::fs::remove_file;

use bevy::prelude::AppExit;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::MessageWriter;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::info;
use bevy::prelude::warn;
use bevy::window::Monitor;
use bevy::window::VideoMode;
use bevy::window::VideoModeSelection;
use bevy::window::WindowMode;
use hana_clerestory::CurrentMonitor;
use hana_clerestory::ManagedWindow;
use hana_clerestory::ManagedWindowPersistence;

use super::events::ClearStateAndQuit;
use super::events::QuitApp;
use super::events::SetBorderlessFullscreen;
use super::events::SetExclusiveFullscreen;
use super::events::SetWindowed;
use super::events::TogglePersistence;
use super::input;
use super::input::SelectedVideoModes;

pub(crate) fn on_set_borderless_fullscreen(
    _trigger: On<SetBorderlessFullscreen>,
    mut windows: Query<(&mut Window, Option<&CurrentMonitor>)>,
) {
    let Some((mut window, maybe_current_monitor)) =
        windows.iter_mut().find(|(window, _)| window.focused)
    else {
        return;
    };
    let Some(current_monitor) = input::resolve_current_monitor(maybe_current_monitor) else {
        return;
    };
    window.mode = WindowMode::BorderlessFullscreen(current_monitor.index.selection());
}

pub(crate) fn on_set_windowed(_trigger: On<SetWindowed>, mut windows: Query<&mut Window>) {
    let Some(mut window) = windows.iter_mut().find(|window| window.focused) else {
        return;
    };
    window.mode = WindowMode::Windowed;
}

pub(crate) fn on_set_exclusive_fullscreen(
    _trigger: On<SetExclusiveFullscreen>,
    mut windows: Query<(&mut Window, Option<&CurrentMonitor>)>,
    bevy_monitors: Query<(Entity, &Monitor)>,
    selected_video_modes: Res<SelectedVideoModes>,
) {
    let Some((mut window, maybe_current_monitor)) =
        windows.iter_mut().find(|(window, _)| window.focused)
    else {
        return;
    };
    let Some(current_monitor) = input::resolve_current_monitor(maybe_current_monitor) else {
        return;
    };

    let video_modes: Vec<VideoMode> = bevy_monitors
        .iter()
        .find(|(_, bevy_monitor)| {
            bevy_monitor.physical_position == current_monitor.physical_position
        })
        .map(|(_, bevy_monitor)| bevy_monitor.video_modes.clone())
        .unwrap_or_default();

    let selected_idx = selected_video_modes
        .get(current_monitor.index)
        .min(video_modes.len().saturating_sub(1));
    let video_mode_selection = video_modes
        .get(selected_idx)
        .map_or(VideoModeSelection::Current, |mode| {
            VideoModeSelection::Specific(*mode)
        });

    window.mode = WindowMode::Fullscreen(current_monitor.index.selection(), video_mode_selection);
}

pub(crate) fn on_toggle_persistence(
    _trigger: On<TogglePersistence>,
    mut managed_window_persistence: ResMut<ManagedWindowPersistence>,
) {
    *managed_window_persistence = match *managed_window_persistence {
        ManagedWindowPersistence::RememberAll => ManagedWindowPersistence::ActiveOnly,
        ManagedWindowPersistence::ActiveOnly => ManagedWindowPersistence::RememberAll,
    };
    info!(
        "[restore_window] Persistence mode: {:?}",
        *managed_window_persistence
    );
}

pub(crate) fn on_clear_state_and_quit(
    _trigger: On<ClearStateAndQuit>,
    managed_entities: Query<Entity, With<ManagedWindow>>,
    mut commands: Commands,
    mut app_exit: MessageWriter<AppExit>,
) {
    if let Some(state_path) = input::get_state_file_path() {
        if let Err(error) = remove_file(&state_path) {
            warn!("[restore_window] Failed to remove state file: {error}");
        } else {
            info!("[restore_window] Cleared state file: {state_path:?}");
        }
    }
    input::despawn_managed_and_exit(&managed_entities, &mut commands, &mut app_exit);
}

pub(crate) fn on_quit_app(
    _trigger: On<QuitApp>,
    managed_entities: Query<Entity, With<ManagedWindow>>,
    mut commands: Commands,
    mut app_exit: MessageWriter<AppExit>,
) {
    input::despawn_managed_and_exit(&managed_entities, &mut commands, &mut app_exit);
}
