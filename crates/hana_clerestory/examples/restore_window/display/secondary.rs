use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FontSize;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::TextFont;
use bevy::prelude::Window;
use bevy::prelude::default;
use bevy::window::Monitor;
use hana_clerestory::CurrentMonitor;
use hana_clerestory::ManagedWindowName;

use super::super::constants::DEFAULT_COLOR;
use super::super::constants::FONT_SIZE;
use super::super::constants::SECONDARY_WINDOW_CONTROLS;
use super::super::constants::SECONDARY_WINDOW_NAME_LABEL;
use super::super::constants::UNKNOWN_MANAGED_WINDOW_NAME;
use super::super::constants::VIDEO_MODES_HEADER;
use super::super::events::MismatchStates;
use super::super::events::RestoredStates;
use super::super::input;
use super::super::input::SelectedVideoModes;
use super::comparison::add_span;
use super::comparison::build_comparison_spans;

#[derive(Component)]
pub(crate) struct SecondaryDisplay(pub(crate) Entity);

#[expect(
    clippy::too_many_arguments,
    reason = "Bevy system — each param is a distinct system resource"
)]
pub(crate) fn update_secondary_displays(
    mut displays: Query<(Entity, &SecondaryDisplay)>,
    windows: Query<(&Window, Option<&CurrentMonitor>)>,
    managed_query: Query<&ManagedWindowName>,
    bevy_monitors: Query<(Entity, &Monitor)>,
    mut selected_video_modes: ResMut<SelectedVideoModes>,
    restored_states: Res<RestoredStates>,
    mismatch_states: Res<MismatchStates>,
    mut commands: Commands,
) {
    for (display_entity, display) in &mut displays {
        let Ok((window, maybe_current_monitor)) = windows.get(display.0) else {
            continue;
        };
        let Some(current_monitor) = input::resolve_current_monitor(maybe_current_monitor) else {
            continue;
        };

        let name = managed_query
            .get(display.0)
            .map_or(UNKNOWN_MANAGED_WINDOW_NAME, |managed_name| &managed_name.0);
        let cached_restored_state = restored_states.by_entity.get(&display.0);
        let cached_mismatch_state = mismatch_states.by_entity.get(&display.0);

        let (video_modes, refresh_rate) =
            input::get_video_modes_for_monitor(&bevy_monitors, &current_monitor);
        let refresh_display = input::format_refresh_rate(window, refresh_rate);
        let active_mode_idx = input::find_active_video_mode_index(window, &video_modes);
        input::sync_selected_to_active(
            window,
            &current_monitor,
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
                let monitor_row = input::format_monitor_row(&current_monitor, &refresh_display);
                add_span(
                    child_spawner,
                    &text_font,
                    &format!("{SECONDARY_WINDOW_NAME_LABEL} {name}\n{monitor_row}\n\n"),
                    DEFAULT_COLOR,
                );

                build_comparison_spans(
                    child_spawner,
                    cached_restored_state,
                    cached_mismatch_state,
                    window,
                    &current_monitor,
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
                    SECONDARY_WINDOW_CONTROLS,
                    DEFAULT_COLOR,
                );
            });
    }
}
