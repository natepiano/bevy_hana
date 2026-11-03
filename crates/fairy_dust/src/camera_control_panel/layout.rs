//! Diegetic layout-tree builders for the camera control panel.

use bevy::prelude::Color;
use hana_diegetic::AlignY;
use hana_diegetic::ChildDivider;
use hana_diegetic::Column;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::El;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::PanelDraw;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_lagrange::ActionColumnLabel;
use hana_lagrange::CameraControlActivation;
use hana_lagrange::CameraControlBinding;
use hana_lagrange::CameraControlBindingKind;
use hana_lagrange::ControlSpeed;
use hana_lagrange::FreeCamActiveDirections;
use hana_lagrange::InteractionSources;
use hana_lagrange::SlowModeHint;

use super::constants::ACTIVE_COLOR;
use super::constants::CONNECTOR_CAP_SIZE;
use super::constants::CONNECTOR_LEVEL_EPSILON;
use super::constants::CONNECTOR_LINE_WIDTH;
use super::constants::FEEDER_CELL_MIN;
use super::constants::FEEDER_START_GAP;
use super::constants::HEADER_COLOR;
use super::constants::LABEL_COLOR;
use super::constants::SPACER_WIDTH;
use super::constants::SPEED_LABEL_COLUMN_WIDTH;
use super::constants::TABLE_DIVIDER_WIDTH;
use super::constants::TRUNK_END_GAP;
use super::display::ActionSpeed;
use super::display::CameraGuidanceDisplay;
use super::display::LastZoomDirection;
use super::guidance::CameraGuidanceAction;
use super::guidance::CameraGuidanceRow;
use super::snapshot;
use super::snapshot::CameraGuidanceSnapshot;
use crate::connector;
use crate::connector::ConnectorColors;
use crate::connector::SpacerLayout;
use crate::constants;
use crate::constants::BORDER_DIM;
use crate::constants::LABEL_SIZE;
use crate::constants::SCREEN_PANEL_ROW_HEIGHT;
use crate::constants::TITLE_COLOR;
use crate::constants::TITLE_SIZE;
use crate::screen_panels;
use crate::screen_panels::PanelTextEdge;

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

fn action_value(row: &CameraGuidanceRow) -> &str {
    match row.action_label() {
        ActionColumnLabel::ActionName => row.action().label(),
        ActionColumnLabel::Replacement(value) => value,
    }
}

/// The same surrogate sizes every value cell, so the column is aligned and
/// each cell reserves the advance of the widest value in this snapshot.
fn widest_value<'a>(
    snapshot: &'a CameraGuidanceSnapshot,
    style: &TextStyle,
    measurer: &DiegeticTextMeasurer,
) -> &'a str {
    let measure = style.as_measure();
    let slow_value = match snapshot.slow_mode {
        SlowModeHint::ToggledBy(_) => Some(snapshot::speed_label(ControlSpeed::Slow)),
        SlowModeHint::NoDisplayableSlowToggle => None,
    };
    snapshot
        .rows
        .iter()
        .map(action_value)
        .chain(
            snapshot
                .settings
                .iter()
                .filter_map(|binding| setting_value(binding).map(|(value, _)| value)),
        )
        .chain(slow_value)
        .max_by(|left, right| {
            (measurer.measure_fn)(left, &measure)
                .width
                .total_cmp(&(measurer.measure_fn)(right, &measure).width)
        })
        .unwrap_or("")
}

pub(super) fn build_guidance_tree(
    snapshot: &CameraGuidanceSnapshot,
    display: CameraGuidanceDisplay,
    background: Color,
    fonts: &FontRegistry,
    measurer: &DiegeticTextMeasurer,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    build_guidance_layout(&mut builder, snapshot, display, background, fonts, measurer);
    builder.build()
}

fn build_guidance_layout(
    builder: &mut LayoutBuilder,
    snapshot: &CameraGuidanceSnapshot,
    display: CameraGuidanceDisplay,
    background: Color,
    fonts: &FontRegistry,
    measurer: &DiegeticTextMeasurer,
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
                .width(Sizing::GROW)
                .height(Sizing::FIT)
                .gap(constants::screen_spacing().title_gap),
            |builder| {
                builder.text(
                    screen_panels::screen_panel_ink_text(
                        snapshot.camera_label.clone(),
                        title.clone(),
                        fonts,
                        screen_panels::integral_advance_size(fonts, TITLE_SIZE),
                        PanelTextEdge::First,
                    )
                    .wrap(TextWrap::None),
                );
                builder.text(
                    Text::new(
                        format!("{}: {}", snapshot.mode_label, snapshot.mode_value),
                        header.clone(),
                    )
                    .wrap(TextWrap::None),
                );
                build_guidance_table(
                    builder, snapshot, display, &label, &active, fonts, label_size, measurer,
                );
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
    fonts: &FontRegistry,
    label_size: Pt,
    measurer: &DiegeticTextMeasurer,
) {
    let value_column = widest_value(snapshot, label, measurer);
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
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(constants::screen_spacing().card_gap)
            .child_divider(ChildDivider::new(TABLE_DIVIDER_WIDTH, BORDER_DIM)),
        |builder| {
            for (index, speed) in speeds.iter().copied().enumerate() {
                build_speed_block(
                    builder,
                    snapshot,
                    speed,
                    speed_column,
                    display,
                    label,
                    active,
                    fonts,
                    label_size,
                    index + 1 == speeds.len()
                        && matches!(snapshot.slow_mode, SlowModeHint::NoDisplayableSlowToggle),
                    value_column,
                );
            }
            if let SlowModeHint::ToggledBy(binding_label) = &snapshot.slow_mode {
                build_slow_mode_row(
                    builder,
                    binding_label,
                    display.slow_mode_active(),
                    label,
                    active,
                    fonts,
                    label_size,
                    value_column,
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
    fonts: &FontRegistry,
    label_size: Pt,
    value_column: &str,
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
                    .height(Sizing::FIT)
                    .align_y(AlignY::Center),
                |builder| {
                    builder.text(
                        screen_panels::screen_panel_ink_text(
                            binding_label,
                            style.clone(),
                            fonts,
                            label_size,
                            PanelTextEdge::Last,
                        )
                        .wrap(TextWrap::None),
                    );
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
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .align_y(AlignY::Center),
                |builder| {
                    builder.text(
                        screen_panels::screen_panel_ink_text(
                            snapshot::speed_label(ControlSpeed::Slow),
                            style.clone(),
                            fonts,
                            label_size,
                            PanelTextEdge::Last,
                        )
                        .wrap(TextWrap::None)
                        .measure_as(value_column),
                    );
                },
            );
        },
    );
}

fn guidance_spacer_layout(word_count: usize) -> SpacerLayout {
    SpacerLayout {
        word_count,
        label_line_height: SCREEN_PANEL_ROW_HEIGHT,
        row_gap: constants::screen_spacing().row_gap,
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
    fonts: &FontRegistry,
    label_size: Pt,
    ends_panel: bool,
    value_column: &str,
) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(constants::screen_spacing().row_gap)
            .align_y(AlignY::Center),
        |builder| {
            if matches!(speed_column, SpeedColumn::Shown) {
                let block_active = snapshot.rows.iter().any(|row| {
                    row.speed() == speed
                        && display.speed(row.action()) == ActionSpeed::At(speed)
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
                        builder.text(
                            Text::new(snapshot::speed_label(speed), speed_style.clone())
                                .wrap(TextWrap::None),
                        );
                    },
                );
            }
            builder.with(action_rows_element(speed_column), |builder| {
                let actions = actions_for_speed(snapshot, speed);
                for (index, action) in actions.iter().copied().enumerate() {
                    build_action_row(
                        builder,
                        snapshot,
                        action,
                        speed,
                        display,
                        label,
                        active,
                        fonts,
                        label_size,
                        ends_panel && index + 1 == actions.len(),
                        value_column,
                    );
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
        .gap(constants::screen_spacing().card_gap);
    match speed_column {
        SpeedColumn::Hidden => {
            element.child_divider(ChildDivider::new(TABLE_DIVIDER_WIDTH, BORDER_DIM))
        },
        SpeedColumn::Shown => element,
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
    fonts: &FontRegistry,
    label_size: Pt,
    ends_panel: bool,
    value_column: &str,
) {
    let active_sources = display.sources(action);
    let live_zoom_direction = display.zoom_direction();
    let live_free_directions = display.free_directions();
    // `speed_matches` gates highlight so the slow row stays dim at normal speed.
    let speed_matches = display.speed(action) == ActionSpeed::At(speed);
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
            .gap(constants::screen_spacing().row_gap),
        |builder| {
            let row_count = rows.len();
            for (index, row) in rows.into_iter().enumerate() {
                build_guidance_table_row(
                    builder,
                    row,
                    label,
                    active,
                    fonts,
                    label_size,
                    if ends_panel && index + 1 == row_count {
                        PanelTextEdge::Last
                    } else {
                        PanelTextEdge::Interior
                    },
                    value_column,
                );
            }
        },
    );
}

fn build_guidance_table_row(
    builder: &mut LayoutBuilder,
    row: GuidanceTableRow<'_>,
    label: &TextStyle,
    active: &TextStyle,
    fonts: &FontRegistry,
    label_size: Pt,
    edge: PanelTextEdge,
    value_column: &str,
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
    let row_height = Sizing::fixed(SCREEN_PANEL_ROW_HEIGHT);
    let row_alignment = if matches!(edge, PanelTextEdge::Last) {
        AlignY::Bottom
    } else {
        AlignY::Center
    };

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
                    .height(row_height)
                    .align_y(row_alignment),
                |builder| {
                    builder.text(
                        screen_panels::screen_panel_ink_text(
                            row.label,
                            style_for_activation(row.label_activation, label, active).clone(),
                            fonts,
                            label_size,
                            edge,
                        )
                        .wrap(TextWrap::None),
                    );
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
                    .width(Sizing::FIT)
                    .height(row_height)
                    .align_y(row_alignment),
                |builder| {
                    builder.text(
                        screen_panels::screen_panel_ink_text(
                            row.value,
                            style_for_activation(row.value_activation, label, active).clone(),
                            fonts,
                            label_size,
                            edge,
                        )
                        .wrap(TextWrap::None)
                        .measure_as(value_column),
                    );
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
    live_zoom_direction: LastZoomDirection,
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
                value:                action_value(row),
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

#[cfg(test)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::MinimalPlugins;
    use hana_diegetic::ComputedDiegeticPanel;
    use hana_diegetic::DiegeticPanel;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::Fit;
    use hana_diegetic::FontId;
    use hana_diegetic::FontRegistry;
    use hana_diegetic::HeadlessDiegeticUiPlugin;
    use hana_diegetic::TextStyle;
    use hana_diegetic::Unit;
    use hana_lagrange::ActionColumnLabel;
    use hana_lagrange::CameraControlAction;
    use hana_lagrange::CameraControlActivation;
    use hana_lagrange::CameraControlBinding;
    use hana_lagrange::CameraControlBindingKind;
    use hana_lagrange::ControlSpeed;
    use hana_lagrange::FreeCamDirectionFilter;
    use hana_lagrange::InteractionSources;
    use hana_lagrange::SlowModeHint;

    use super::build_guidance_tree;
    use crate::camera_control_panel::display::CameraGuidanceDisplay;
    use crate::camera_control_panel::guidance::CameraGuidanceAction;
    use crate::camera_control_panel::guidance::CameraGuidanceRow;
    use crate::camera_control_panel::snapshot::CameraGuidanceSnapshot;

    #[test]
    fn camera_panel_top_and_bottom_padding_follow_visible_text_ink() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let mut no_slow_mode = alignment_snapshot();
        no_slow_mode.slow_mode = SlowModeHint::NoDisplayableSlowToggle;
        for snapshot in [alignment_snapshot(), no_slow_mode] {
            let tree = build_guidance_tree(
                &snapshot,
                CameraGuidanceDisplay::default(),
                crate::constants::DEFAULT_PANEL_BACKGROUND,
                &fonts,
                &DiegeticTextMeasurer::default(),
            );
            let panel = DiegeticPanel::screen()
                .size(Fit, Fit)
                .with_tree(tree)
                .build()
                .map_err(|error| error.to_string())?;
            let mut app = crate::test_app();
            app.add_plugins(MinimalPlugins)
                .insert_resource(DiegeticTextMeasurer::default())
                .add_plugins(HeadlessDiegeticUiPlugin);
            let entity = app.world_mut().spawn(panel).id();
            app.update();
            let solved = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(|panel| panel.layout().solved())
                .ok_or("camera panel did not solve")?;
            let bounds = solved.computed[0].bounds;
            let title = solved
                .commands
                .iter()
                .find(|command| {
                    format!("{:?}", command.kind).starts_with("Text { text: \"Camera\",")
                })
                .ok_or("camera title missing")?;
            let point_per_pixel = Unit::Pixels.to_points();
            let font = fonts
                .font(FontId::MONOSPACE)
                .map_err(|error| error.to_string())?;
            let bottom = solved
                .commands
                .iter()
                .filter(|command| format!("{:?}", command.kind).starts_with("Text {"))
                .map(|command| {
                    let text = command.bounds;
                    let size = if command.element_idx == title.element_idx {
                        crate::screen_panels::integral_advance_size(
                            &fonts,
                            crate::constants::TITLE_SIZE,
                        )
                    } else {
                        crate::screen_panels::integral_advance_size(
                            &fonts,
                            crate::constants::LABEL_SIZE,
                        )
                    };
                    let metrics = font.metrics(size.0 * 4.0 / 3.0);
                    metrics
                        .line_gap
                        .mul_add(0.5, metrics.descent)
                        .mul_add(-point_per_pixel, text.y + text.height)
                })
                .fold(f32::NEG_INFINITY, f32::max);
            let title_metrics = font.metrics(
                crate::screen_panels::integral_advance_size(&fonts, crate::constants::TITLE_SIZE).0
                    * 4.0
                    / 3.0,
            );
            let top_offset = title_metrics.line_gap.mul_add(0.5, title_metrics.ascent)
                - title_metrics.cap_height;
            let top_margin = top_offset.mul_add(point_per_pixel, title.bounds.y) - bounds.y;
            let bottom_margin = bounds.y + bounds.height - bottom;
            let expected = crate::constants::screen_spacing().panel_padding.0 * point_per_pixel;
            assert!(
                (top_margin - expected).abs() <= 1.0,
                "top ink margin {top_margin} vs {expected}"
            );
            assert!(
                (bottom_margin - expected).abs() <= 1.0,
                "bottom ink margin {bottom_margin} vs {expected}"
            );
        }
        Ok(())
    }

    #[test]
    fn free_camera_actions_reach_the_header_right_inset() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let snapshot = CameraGuidanceSnapshot {
            camera_label: "Free camera".into(),
            mode_label:   "Controls".into(),
            mode_value:   "Mouse and keyboard".into(),
            slow_mode:    SlowModeHint::NoDisplayableSlowToggle,
            settings:     Vec::new(),
            rows:         vec![
                CameraGuidanceRow::new(CameraGuidanceAction::Look, "Right drag"),
                CameraGuidanceRow::new(CameraGuidanceAction::Translate, "WASD"),
            ],
        };
        let tree = build_guidance_tree(
            &snapshot,
            CameraGuidanceDisplay::default(),
            Color::BLACK,
            &fonts,
            &DiegeticTextMeasurer::default(),
        );
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(tree)
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("camera panel did not solve")?;
        let right_edge = |text: &str| -> Result<f32, String> {
            let command = solved
                .commands
                .iter()
                .find(|command| {
                    format!("{:?}", command.kind).starts_with(&format!("Text {{ text: {text:?},"))
                })
                .ok_or_else(|| format!("{text} missing"))?;
            Ok(command.bounds.x + command.bounds.width)
        };
        let header = right_edge("Controls: Mouse and keyboard")?;
        let action = right_edge("Move")?;
        assert!(
            (header - action).abs() < 1.0,
            "header {header} vs action {action}"
        );
        Ok(())
    }

    #[test]
    fn last_camera_action_keeps_the_inner_row_height() -> Result<(), String> {
        let fonts = FontRegistry::new().map_err(|error| error.to_string())?;
        let mut snapshot = alignment_snapshot();
        snapshot.slow_mode = SlowModeHint::NoDisplayableSlowToggle;
        let tree = build_guidance_tree(
            &snapshot,
            CameraGuidanceDisplay::default(),
            Color::BLACK,
            &fonts,
            &DiegeticTextMeasurer::default(),
        );
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(tree.clone())
            .build()
            .map_err(|error| error.to_string())?;
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let solved = app
            .world()
            .get::<ComputedDiegeticPanel>(entity)
            .and_then(|panel| panel.layout().solved())
            .ok_or("camera panel did not solve")?;
        let row_height = |label: &str| -> Result<f32, String> {
            let command = solved
                .commands
                .iter()
                .find(|command| {
                    format!("{:?}", command.kind).starts_with(&format!("Text {{ text: {label:?},"))
                })
                .ok_or_else(|| format!("{label} missing"))?;
            let cell = (0..solved.computed.len())
                .find(|&index| tree.children_of(index).contains(&command.element_idx))
                .ok_or_else(|| format!("{label} cell missing"))?;
            Ok(solved.computed[cell].bounds.height)
        };
        let expected = super::SCREEN_PANEL_ROW_HEIGHT.0 * Unit::Pixels.to_points();
        for label in ["wheel", "mouse"] {
            let height = row_height(label)?;
            assert!(
                (height - expected).abs() < 0.1,
                "{label} row height {height} vs {expected}"
            );
        }
        Ok(())
    }

    #[test]
    fn zoom_out_value_does_not_add_a_camera_panel_line() {
        let fonts = FontRegistry::new();
        assert!(fonts.is_ok());
        let Ok(fonts) = fonts else { return };
        let snapshot = |value| CameraGuidanceSnapshot {
            camera_label: "Camera".to_string(),
            mode_label:   "Mode".to_string(),
            mode_value:   "Orbit".to_string(),
            slow_mode:    SlowModeHint::NoDisplayableSlowToggle,
            settings:     Vec::new(),
            rows:         vec![
                CameraGuidanceRow::new(CameraGuidanceAction::Zoom, "Zoom").with_action_label(value),
            ],
        };
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);

        let panel_height = |app: &mut App, value| {
            let tree = build_guidance_tree(
                &snapshot(value),
                CameraGuidanceDisplay::default(),
                Color::BLACK,
                &fonts,
                app.world().resource::<DiegeticTextMeasurer>(),
            );
            let panel = DiegeticPanel::screen()
                .size(Fit, Fit)
                .with_tree(tree)
                .build();
            assert!(panel.is_ok());
            let Ok(panel) = panel else { return 0.0 };
            let entity = app.world_mut().spawn(panel).id();
            app.update();
            let bounds = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .and_then(ComputedDiegeticPanel::content_bounds);
            assert!(bounds.is_some());
            bounds.map_or(0.0, |bounds| bounds.height)
        };

        let short_height = panel_height(&mut app, "Zoom");
        let long_height = panel_height(&mut app, "Zoom out");
        assert!((long_height - short_height).abs() < 1e-4);
    }

    #[test]
    fn value_cells_align_and_fit_the_widest_action_setting_and_slow_value() {
        let fonts = FontRegistry::new();
        assert!(fonts.is_ok());
        let Ok(fonts) = fonts else { return };
        let style = TextStyle::new(crate::screen_panels::integral_advance_size(
            &fonts,
            crate::constants::LABEL_SIZE,
        ));
        let snapshot = alignment_snapshot();
        let measurer = DiegeticTextMeasurer::default();
        let tree = build_guidance_tree(
            &snapshot,
            CameraGuidanceDisplay::default(),
            Color::BLACK,
            &fonts,
            &measurer,
        );
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .with_tree(tree.clone())
            .build();
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessDiegeticUiPlugin);
        assert!(panel.is_ok());
        let Ok(panel) = panel else { return };
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        let computed = app.world().get::<ComputedDiegeticPanel>(entity);
        assert!(computed.is_some());
        let Some(computed) = computed else { return };
        let solved = computed.layout().solved();
        assert!(solved.is_some());
        let Some(solved) = solved else { return };
        let values = [
            "Zoom out",
            "Replacement label",
            "Very long setting value",
            "Slow",
        ];
        let expected_width = values
            .iter()
            .map(|value| (measurer.measure_fn)(value, &style.as_measure()).width)
            .fold(0.0_f32, f32::max);
        let mut cells = Vec::new();
        for command in &solved.commands {
            let rendered = format!("{:?}", command.kind);
            let Some(value) = values
                .iter()
                .find(|&&value| rendered.starts_with(&format!("Text {{ text: {value:?},")))
            else {
                continue;
            };
            let cell = (0..solved.computed.len())
                .find(|&parent| tree.children_of(parent).contains(&command.element_idx));
            assert!(cell.is_some(), "value text has a cell parent");
            let Some(cell) = cell else { return };
            cells.push((*value, solved.computed[cell].bounds, command.bounds));
        }
        assert_eq!(cells.len(), values.len());
        let start_x = cells[0].1.x;
        for (value, cell, drawn) in cells {
            assert!(
                (cell.x - start_x).abs() < 0.01,
                "{value} starts in another column"
            );
            assert!(
                (cell.width - expected_width).abs() < 0.01,
                "{value} has a different cell width"
            );
            assert!(
                drawn.x + drawn.width <= cell.x + cell.width + 0.01,
                "{value} overflows its cell"
            );
        }

        assert_value_text_baselines_match_labels(computed);
    }

    fn assert_value_text_baselines_match_labels(computed: &ComputedDiegeticPanel) {
        let solved = computed.layout().solved();
        assert!(solved.is_some());
        let Some(solved) = solved else { return };
        let text_bounds = |text: &str| {
            solved.commands.iter().find_map(|command| {
                format!("{:?}", command.kind)
                    .starts_with(&format!("Text {{ text: {text:?},"))
                    .then_some(command.bounds)
            })
        };
        for (label, value) in [("wheel", "Zoom out"), ("Alt S", "Slow")] {
            let label_bounds = text_bounds(label);
            let value_bounds = text_bounds(value);
            assert!(label_bounds.is_some(), "missing camera label: {label}");
            assert!(value_bounds.is_some(), "missing camera value: {value}");
            let (Some(label_bounds), Some(value_bounds)) = (label_bounds, value_bounds) else {
                return;
            };
            assert!(
                (label_bounds.y - value_bounds.y).abs() < 0.01,
                "{label} and {value} do not share a text baseline"
            );
        }
    }

    fn alignment_snapshot() -> CameraGuidanceSnapshot {
        CameraGuidanceSnapshot {
            camera_label: "Camera".into(),
            mode_label:   "Controls".into(),
            mode_value:   "Custom".into(),
            slow_mode:    SlowModeHint::ToggledBy("Alt S".into()),
            settings:     vec![CameraControlBinding {
                action:              CameraControlAction::Look,
                label:               "Alt I".into(),
                interaction_sources: InteractionSources::KEYBOARD,
                speed:               ControlSpeed::Normal,
                kind:                CameraControlBindingKind::Setting {
                    value:      "Very long setting value".into(),
                    activation: CameraControlActivation::Inactive,
                },
                action_label:        ActionColumnLabel::ActionName,
                direction:           FreeCamDirectionFilter::AnySourceEngagement,
            }],
            rows:         vec![
                CameraGuidanceRow::new(CameraGuidanceAction::ZoomOut, "wheel"),
                CameraGuidanceRow::new(CameraGuidanceAction::Look, "mouse")
                    .with_action_label("Replacement label"),
            ],
        }
    }
}
