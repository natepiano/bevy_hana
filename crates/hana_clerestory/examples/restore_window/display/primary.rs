use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FontSize;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Single;
use bevy::prelude::TextFont;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::window::Monitor;
use bevy::window::PrimaryWindow;
use bevy::window::WindowPosition;
use hana_clerestory::CurrentMonitor;
use hana_clerestory::ManagedWindow;
use hana_clerestory::ManagedWindowPersistence;
use hana_kana::ToU32;

use super::super::constants::AUTOMATIC_TEXT;
use super::super::constants::DEFAULT_COLOR;
use super::super::constants::FONT_SIZE;
use super::super::constants::MANAGED_WINDOWS_HEADER;
use super::super::constants::MONITOR_INDEX_LABEL;
use super::super::constants::NO_MANAGED_WINDOWS_TEXT;
use super::super::constants::SCALE_LABEL;
use super::super::constants::VIDEO_MODES_HEADER;
use super::super::events::MismatchStates;
use super::super::events::RestoredStates;
use super::super::input;
use super::super::input::SelectedVideoModes;
use super::comparison::add_span;
use super::comparison::build_comparison_spans;

#[derive(Component)]
pub(crate) struct PrimaryDisplay;

#[expect(
    clippy::too_many_arguments,
    reason = "Bevy system — each param is a distinct system resource"
)]
pub(crate) fn update_primary_display(
    primary_display: Single<Entity, With<PrimaryDisplay>>,
    window_query: Single<(Entity, &Window, &CurrentMonitor), With<PrimaryWindow>>,
    bevy_monitors: Query<(Entity, &Monitor)>,
    mut selected_video_modes: ResMut<SelectedVideoModes>,
    managed_window_persistence: Res<ManagedWindowPersistence>,
    managed_query: Query<(&Window, &ManagedWindow, Option<&CurrentMonitor>)>,
    restored_states: Res<RestoredStates>,
    mismatch_states: Res<MismatchStates>,
    mut commands: Commands,
) {
    let display_entity = *primary_display;
    let (window_entity, window, current_monitor) = *window_query;

    let cached_restored_state = restored_states.by_entity.get(&window_entity);
    let cached_mismatch_state = mismatch_states.by_entity.get(&window_entity);

    let (video_modes, refresh_rate) =
        input::get_video_modes_for_monitor(&bevy_monitors, current_monitor);
    let refresh_display = input::format_refresh_rate(window, refresh_rate);
    let active_mode_idx = input::find_active_video_mode_index(window, &video_modes);
    input::sync_selected_to_active(
        window,
        current_monitor,
        active_mode_idx,
        &mut selected_video_modes,
    );
    let selected_idx = selected_video_modes.get(current_monitor.index);
    let video_modes_display =
        input::build_video_modes_display(&video_modes, selected_idx, active_mode_idx);

    let text_font = TextFont {
        font_size: FontSize::Px(FONT_SIZE),
        ..default()
    };

    commands.entity(display_entity).despawn_children();
    commands
        .entity(display_entity)
        .with_children(|child_spawner| {
            let monitor_row = input::format_monitor_row(current_monitor, &refresh_display);
            add_span(
                child_spawner,
                &text_font,
                &format!("{monitor_row}\n\n"),
                DEFAULT_COLOR,
            );

            build_comparison_spans(
                child_spawner,
                cached_restored_state,
                cached_mismatch_state,
                window,
                current_monitor,
                &text_font,
            );

            add_span(
                child_spawner,
                &text_font,
                &format!("{VIDEO_MODES_HEADER}{video_modes_display}\n"),
                DEFAULT_COLOR,
            );

            add_span(
                child_spawner,
                &text_font,
                &format!(
                    "\nControls:\n\
                 [Enter] Exclusive Fullscreen\n\
                 [B] Borderless Fullscreen\n\
                 [W] Windowed\n\
                 [Space] Spawn managed window\n\
                 [P] Toggle persistence ({managed_window_persistence:?})\n\
                 [Ctrl+Shift+Backspace] Clear state and quit\n\
                 [Q] Quit\n"
                ),
                DEFAULT_COLOR,
            );

            let mut managed_lines = Vec::new();
            for (managed_window, managed, current_monitor) in &managed_query {
                let monitor_index = current_monitor.map_or_else(
                    || String::from("unavailable"),
                    |current_monitor| current_monitor.index.to_string(),
                );
                let position = match managed_window.position {
                    WindowPosition::At(managed_position) => {
                        format!("({}, {})", managed_position.x, managed_position.y)
                    },
                    _ => AUTOMATIC_TEXT.to_string(),
                };
                managed_lines.push(format!(
                    "  {}: position={position} physical={}x{} logical={}x{} {SCALE_LABEL} {} {MONITOR_INDEX_LABEL} {}\n",
                    managed.name,
                    managed_window.physical_width(),
                    managed_window.physical_height(),
                    managed_window.resolution.width().to_u32(),
                    managed_window.resolution.height().to_u32(),
                    managed_window.resolution.scale_factor(),
                    monitor_index,
                ));
            }
            add_span(child_spawner, &text_font, MANAGED_WINDOWS_HEADER, DEFAULT_COLOR);
            if managed_lines.is_empty() {
                add_span(child_spawner, &text_font, NO_MANAGED_WINDOWS_TEXT, DEFAULT_COLOR);
            } else {
                for line in &managed_lines {
                    add_span(child_spawner, &text_font, line, DEFAULT_COLOR);
                }
            }
        });
}
