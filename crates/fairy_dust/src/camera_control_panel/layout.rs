//! Diegetic layout-tree builders for the camera control panel.

use bevy::prelude::Color;
use hana_diegetic::AlignY;
use hana_diegetic::ChildDivider;
use hana_diegetic::Column;
use hana_diegetic::El;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::PanelDraw;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_lagrange::CameraControlActivation;
use hana_lagrange::CameraControlBinding;
use hana_lagrange::CameraControlBindingKind;
use hana_lagrange::ControlSpeed;
use hana_lagrange::FreeCamActiveDirections;
use hana_lagrange::InteractionSources;
use hana_lagrange::ZoomDirection;

use super::constants::ACTION_COLUMN_WIDTH;
use super::constants::ACTIVE_COLOR;
use super::constants::CONNECTOR_CAP_SIZE;
use super::constants::CONNECTOR_LEVEL_EPSILON;
use super::constants::CONNECTOR_LINE_WIDTH;
use super::constants::FEEDER_CELL_MIN;
use super::constants::FEEDER_START_GAP;
use super::constants::GUIDANCE_CHILD_GAP;
use super::constants::HEADER_COLOR;
use super::constants::LABEL_COLOR;
use super::constants::LABEL_LINE_HEIGHT;
use super::constants::SPACER_WIDTH;
use super::constants::SPEED_LABEL_COLUMN_WIDTH;
use super::constants::TABLE_COLUMN_GAP;
use super::constants::TABLE_DIVIDER_WIDTH;
use super::constants::TABLE_GROUP_GAP;
use super::constants::TABLE_ROW_GAP;
use super::constants::TABLE_SECTION_DIVIDER_GAP;
use super::constants::TRUNK_END_GAP;
use super::display::CameraGuidanceDisplay;
use super::guidance::CameraGuidanceAction;
use super::snapshot;
use super::snapshot::CameraGuidanceSnapshot;
use crate::connector;
use crate::connector::ConnectorColors;
use crate::connector::SpacerLayout;
use crate::constants::BORDER_DIM;
use crate::constants::LABEL_SIZE;
use crate::constants::TITLE_COLOR;
use crate::constants::TITLE_SIZE;
use crate::screen_panels;

/// Whether a speed block renders the `Normal` / `Slow` label column.
#[derive(Clone, Copy)]
enum SpeedColumn {
    Shown,
    Hidden,
}

#[derive(Clone, Copy)]
struct GuidanceTableRow<'a> {
    label:                &'a str,
    value:                &'a str,
    label_activation:     CameraControlActivation,
    value_activation:     CameraControlActivation,
    connector_activation: CameraControlActivation,
}

pub(super) fn build_guidance_tree(
    snapshot: &CameraGuidanceSnapshot,
    display: CameraGuidanceDisplay,
    background: Color,
    fonts: &FontRegistry,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    build_guidance_layout(&mut builder, snapshot, display, background, fonts);
    builder.build()
}

fn build_guidance_layout(
    builder: &mut LayoutBuilder,
    snapshot: &CameraGuidanceSnapshot,
    display: CameraGuidanceDisplay,
    background: Color,
    fonts: &FontRegistry,
) {
    let title = TextStyle::new(screen_panels::integral_advance_size(fonts, TITLE_SIZE))
        .with_color(TITLE_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);
    let label_size = screen_panels::integral_advance_size(fonts, LABEL_SIZE);
    let header = TextStyle::new(label_size)
        .with_color(HEADER_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);
    let label = TextStyle::new(label_size)
        .with_color(LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);
    let active = TextStyle::new(label_size)
        .with_color(ACTIVE_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);

    screen_panels::screen_panel_frame(builder, Sizing::FIT, Sizing::FIT, background, |builder| {
        builder.with(
            El::column()
                .width(Sizing::FIT)
                .height(Sizing::FIT)
                .gap(GUIDANCE_CHILD_GAP),
            |builder| {
                builder.text((format!("CAMERA: {}", snapshot.camera_label), title.clone()));
                builder.text((
                    format!("{}: {}", snapshot.mode_label, snapshot.mode_value),
                    header.clone(),
                ));
                build_guidance_table(builder, snapshot, display, &label, &active);
            },
        );
    });
}

fn build_guidance_table(
    builder: &mut LayoutBuilder,
    snapshot: &CameraGuidanceSnapshot,
    display: CameraGuidanceDisplay,
    label: &TextStyle,
    active: &TextStyle,
) {
    let speeds: Vec<ControlSpeed> = [ControlSpeed::Normal, ControlSpeed::Slow]
        .into_iter()
        .filter(|&speed| {
            snapshot.rows.iter().any(|row| row.speed() == speed)
                || snapshot
                    .settings
                    .iter()
                    .any(|binding| binding.speed == speed)
        })
        .collect();
    // Single-speed presets (mouse, keyboard) drop the speed column entirely.
    let speed_column = if speeds.len() > 1 {
        SpeedColumn::Shown
    } else {
        SpeedColumn::Hidden
    };

    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(Px(TABLE_SECTION_DIVIDER_GAP))
            .child_divider(ChildDivider::new(TABLE_DIVIDER_WIDTH, BORDER_DIM)),
        |builder| {
            for speed in speeds {
                build_speed_block(
                    builder,
                    snapshot,
                    speed,
                    speed_column,
                    display,
                    label,
                    active,
                );
            }
            if let Some(binding_label) = snapshot.slow_mode_binding_label.as_deref() {
                build_slow_mode_row(
                    builder,
                    binding_label,
                    display.slow_mode_active(),
                    label,
                    active,
                );
            }
        },
    );
}

fn build_slow_mode_row(
    builder: &mut LayoutBuilder,
    binding_label: &str,
    slow_mode_active: bool,
    label: &TextStyle,
    active: &TextStyle,
) {
    let style = if slow_mode_active { active } else { label };
    let connector_color = if slow_mode_active {
        ACTIVE_COLOR
    } else {
        Color::NONE
    };
    let spacer_layout = guidance_spacer_layout(1);
    let spacer_lines =
        connector::spacer_lines(spacer_layout, &[slow_mode_active], slow_mode_active);

    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(Px(0.0))
            .align_y(AlignY::Center),
        |builder| {
            builder.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(LABEL_LINE_HEIGHT))
                    .align_y(AlignY::Center),
                |builder| {
                    builder.text((binding_label, style.clone()));
                    builder.with(
                        El::new()
                            .width(Sizing::grow_min(FEEDER_CELL_MIN))
                            .height(Sizing::GROW)
                            .draw(PanelDraw::lines([connector::feeder_line(
                                FEEDER_START_GAP,
                                CONNECTOR_LINE_WIDTH,
                                connector_color,
                            )])),
                        |_| {},
                    );
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(SPACER_WIDTH))
                    .height(Sizing::GROW)
                    .draw(PanelDraw::lines(spacer_lines)),
                |_| {},
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(ACTION_COLUMN_WIDTH))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((snapshot::speed_label(ControlSpeed::Slow), style.clone()));
                },
            );
        },
    );
}

const fn guidance_spacer_layout(word_count: usize) -> SpacerLayout {
    SpacerLayout {
        word_count,
        label_line_height: LABEL_LINE_HEIGHT,
        row_gap: Px(TABLE_ROW_GAP),
        line_width: CONNECTOR_LINE_WIDTH,
        cap_size: CONNECTOR_CAP_SIZE,
        level_epsilon: CONNECTOR_LEVEL_EPSILON,
        trunk_end_gap: TRUNK_END_GAP,
        colors: ConnectorColors {
            active: ACTIVE_COLOR,
            idle:   Color::NONE,
        },
    }
}

fn build_speed_block(
    builder: &mut LayoutBuilder,
    snapshot: &CameraGuidanceSnapshot,
    speed: ControlSpeed,
    speed_column: SpeedColumn,
    display: CameraGuidanceDisplay,
    label: &TextStyle,
    active: &TextStyle,
) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(Px(TABLE_COLUMN_GAP))
            .align_y(AlignY::Center),
        |builder| {
            if matches!(speed_column, SpeedColumn::Shown) {
                let block_active = snapshot.rows.iter().any(|row| {
                    row.speed() == speed
                        && display.speed(row.action()) == Some(speed)
                        && snapshot::row_active(
                            row,
                            display.sources(row.action()),
                            display.zoom_direction(),
                            display.free_directions(),
                        )
                }) || snapshot.settings.iter().any(|binding| {
                    binding.speed == speed
                        && setting_activation(binding) == Some(CameraControlActivation::Active)
                });
                let speed_style = if block_active { active } else { label };
                builder.with(
                    El::new()
                        .width(Sizing::fit_min(SPEED_LABEL_COLUMN_WIDTH))
                        .height(Sizing::FIT),
                    |builder| {
                        builder.text((snapshot::speed_label(speed), speed_style.clone()));
                    },
                );
            }
            builder.with(action_rows_element(speed_column), |builder| {
                for action in actions_for_speed(snapshot, speed) {
                    build_action_row(builder, snapshot, action, speed, display, label, active);
                }
            });
        },
    );
}

/// The action-row column element. Single-speed presets divide their action
/// groups with a border line, while multi-speed presets keep the groups
/// gap-separated and rely on the divider between `Normal` / `Slow` blocks.
fn action_rows_element(speed_column: SpeedColumn) -> El<Column> {
    let element = El::column()
        .width(Sizing::GROW)
        .height(Sizing::FIT)
        .gap(Px(action_row_group_gap(speed_column)));
    match speed_column {
        SpeedColumn::Hidden => {
            element.child_divider(ChildDivider::new(TABLE_DIVIDER_WIDTH, BORDER_DIM))
        },
        SpeedColumn::Shown => element,
    }
}

const fn action_row_group_gap(speed_column: SpeedColumn) -> f32 {
    match speed_column {
        SpeedColumn::Hidden => TABLE_SECTION_DIVIDER_GAP,
        SpeedColumn::Shown => TABLE_GROUP_GAP,
    }
}

fn build_action_row(
    builder: &mut LayoutBuilder,
    snapshot: &CameraGuidanceSnapshot,
    action: CameraGuidanceAction,
    speed: ControlSpeed,
    display: CameraGuidanceDisplay,
    label: &TextStyle,
    active: &TextStyle,
) {
    let active_sources = display.sources(action);
    let live_zoom_direction = display.zoom_direction();
    let live_free_directions = display.free_directions();
    // `speed_matches` gates highlight so the slow row stays dim at normal speed.
    let speed_matches = display.speed(action) == Some(speed);
    let rows = table_rows_for_action(
        snapshot,
        action,
        speed,
        speed_matches,
        active_sources,
        live_zoom_direction,
        live_free_directions,
    );
    if rows.is_empty() {
        return;
    }

    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(Px(TABLE_ROW_GAP)),
        |builder| {
            for row in rows {
                build_guidance_table_row(builder, row, label, active);
            }
        },
    );
}

fn build_guidance_table_row(
    builder: &mut LayoutBuilder,
    row: GuidanceTableRow<'_>,
    label: &TextStyle,
    active: &TextStyle,
) {
    let connector_active = row.connector_activation == CameraControlActivation::Active;
    let connector_color = if connector_active {
        ACTIVE_COLOR
    } else {
        Color::NONE
    };
    let spacer_layout = guidance_spacer_layout(1);
    let spacer_lines =
        connector::spacer_lines(spacer_layout, &[connector_active], connector_active);

    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(Px(0.0))
            .align_y(AlignY::Center),
        |builder| {
            builder.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(LABEL_LINE_HEIGHT))
                    .align_y(AlignY::Center),
                |builder| {
                    builder.text((
                        row.label,
                        style_for_activation(row.label_activation, label, active).clone(),
                    ));
                    builder.with(
                        El::new()
                            .width(Sizing::grow_min(FEEDER_CELL_MIN))
                            .height(Sizing::GROW)
                            .draw(PanelDraw::lines([connector::feeder_line(
                                FEEDER_START_GAP,
                                CONNECTOR_LINE_WIDTH,
                                connector_color,
                            )])),
                        |_| {},
                    );
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(SPACER_WIDTH))
                    .height(Sizing::GROW)
                    .draw(PanelDraw::lines(spacer_lines)),
                |_| {},
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(ACTION_COLUMN_WIDTH))
                    .height(Sizing::FIT),
                |builder| {
                    builder.text((
                        row.value,
                        style_for_activation(row.value_activation, label, active).clone(),
                    ));
                },
            );
        },
    );
}

const fn style_for_activation<'a>(
    activation: CameraControlActivation,
    label: &'a TextStyle,
    active: &'a TextStyle,
) -> &'a TextStyle {
    match activation {
        CameraControlActivation::Active => active,
        CameraControlActivation::Inactive => label,
    }
}

fn table_rows_for_action(
    snapshot: &CameraGuidanceSnapshot,
    action: CameraGuidanceAction,
    speed: ControlSpeed,
    speed_matches: bool,
    active_sources: InteractionSources,
    live_zoom_direction: Option<ZoomDirection>,
    live_free_directions: FreeCamActiveDirections,
) -> Vec<GuidanceTableRow<'_>> {
    let mut rows = snapshot
        .rows
        .iter()
        .filter(|row| row.action() == action && row.speed() == speed)
        .map(|row| {
            let activation = if speed_matches
                && snapshot::row_active(
                    row,
                    active_sources,
                    live_zoom_direction,
                    live_free_directions,
                ) {
                CameraControlActivation::Active
            } else {
                CameraControlActivation::Inactive
            };
            GuidanceTableRow {
                label:                row.label(),
                value:                row.action_label().unwrap_or_else(|| action.label()),
                label_activation:     activation,
                value_activation:     activation,
                connector_activation: activation,
            }
        })
        .collect::<Vec<_>>();

    rows.extend(
        snapshot
            .settings
            .iter()
            .filter(|binding| {
                CameraGuidanceAction::from(binding.action) == action && binding.speed == speed
            })
            .filter_map(|binding| {
                setting_value(binding).map(|(value, activation)| GuidanceTableRow {
                    label: binding.label.as_str(),
                    value,
                    label_activation: CameraControlActivation::Inactive,
                    value_activation: activation,
                    connector_activation: activation,
                })
            }),
    );
    rows
}

const fn setting_value(binding: &CameraControlBinding) -> Option<(&str, CameraControlActivation)> {
    match &binding.kind {
        CameraControlBindingKind::Setting { value, activation } => {
            Some((value.as_str(), *activation))
        },
        CameraControlBindingKind::Direct => None,
    }
}

fn setting_activation(binding: &CameraControlBinding) -> Option<CameraControlActivation> {
    setting_value(binding).map(|(_, activation)| activation)
}

fn actions_for_speed(
    snapshot: &CameraGuidanceSnapshot,
    speed: ControlSpeed,
) -> Vec<CameraGuidanceAction> {
    let mut actions = Vec::new();
    for row in snapshot.rows.iter().filter(|row| row.speed() == speed) {
        if !actions.contains(&row.action()) {
            actions.push(row.action());
        }
    }
    for binding in snapshot
        .settings
        .iter()
        .filter(|binding| binding.speed == speed)
    {
        let action = CameraGuidanceAction::from(binding.action);
        if !actions.contains(&action) {
            actions.push(action);
        }
    }
    actions
}
