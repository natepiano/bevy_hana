use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Node;
use bevy::prelude::Query;
use bevy::prelude::Text;
use bevy::prelude::TextColor;
use bevy::prelude::Vec2;
use bevy::ui::UiTargetCamera;

use super::config::FitTargetOverlayConfig;
use crate::fit::constants::TOLERANCE;
use crate::fit::geometry::ScreenSpaceBounds;
use crate::fit::overlay::geometry;
use crate::fit::overlay::geometry::Edge;
use crate::fit::overlay::geometry::MarginBalance;
use crate::fit::overlay::render::labels;
use crate::fit::overlay::render::labels::MarginLabel;
use crate::fit::overlay::render::labels::MarginLabelParameters;
use crate::fit::overlay::render::lines::FitOverlayLineContext;
use crate::fit::overlay::render::visual::FitOverlayVisual;
use crate::fit::overlay::render::visual::FitOverlayVisualKind;

/// Camera-derived drawing parameters shared across margin/bounds rendering.
pub(super) struct DrawContext<'a> {
    pub(super) camera:        Entity,
    pub(super) ui_camera:     Entity,
    pub(super) bounds:        &'a ScreenSpaceBounds,
    pub(super) viewport_size: Option<Vec2>,
}

/// Calculates the color for an edge based on balance state.
const fn calculate_edge_color(
    edge: Edge,
    horizontal_balance: MarginBalance,
    vertical_balance: MarginBalance,
    config: &FitTargetOverlayConfig,
) -> Color {
    let balance = match edge {
        Edge::Left | Edge::Right => horizontal_balance,
        Edge::Top | Edge::Bottom => vertical_balance,
    };
    match balance {
        MarginBalance::Balanced => config.balanced_color,
        MarginBalance::Unbalanced => config.unbalanced_color,
    }
}

/// Draws margin lines from boundary edges to screen edges and updates margin labels.
/// Returns the set of edges that had visible margins.
pub(super) fn draw_margin_lines_and_labels(
    line_context: &mut FitOverlayLineContext<'_, '_, '_>,
    label_query: &mut Query<(
        Entity,
        &MarginLabel,
        &FitOverlayVisual,
        &mut Text,
        &mut Node,
        &mut TextColor,
        &mut UiTargetCamera,
    )>,
    draw_context: &DrawContext,
    config: &FitTargetOverlayConfig,
) -> Vec<Edge> {
    let camera = draw_context.camera;
    let bounds = draw_context.bounds;
    let viewport_size = draw_context.viewport_size;
    let horizontal_balance = geometry::horizontal_balance(bounds, TOLERANCE);
    let vertical_balance = geometry::vertical_balance(bounds, TOLERANCE);

    let mut visible_edges: Vec<Edge> = Vec::new();

    for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
        let Some((boundary_x, boundary_y)) = geometry::boundary_edge_center(bounds, edge) else {
            continue;
        };
        visible_edges.push(edge);

        let (screen_x, screen_y) = geometry::screen_edge_center(bounds, edge);
        let color = calculate_edge_color(edge, horizontal_balance, vertical_balance, config);
        line_context.upsert_polyline(
            FitOverlayVisual {
                camera,
                kind: FitOverlayVisualKind::MarginLine { edge },
            },
            &[
                Vec2::new(boundary_x, boundary_y),
                Vec2::new(screen_x, screen_y),
            ],
            false,
            color,
            config.line_width,
        );

        let Some(viewport_size) = viewport_size else {
            continue;
        };
        let percentage = geometry::margin_percentage(bounds, edge);
        let text = format!("margin: {percentage:.3}%");
        let label_screen_position =
            labels::calculate_label_pixel_position(edge, bounds, viewport_size);

        labels::update_or_create_margin_label(
            line_context.commands,
            label_query,
            MarginLabelParameters {
                camera,
                ui_camera: draw_context.ui_camera,
                edge,
                text,
                color,
                screen_position: label_screen_position,
                viewport_size,
            },
        );
    }

    visible_edges
}

/// Removes margin labels for edges no longer visible, scoped to a specific camera.
pub(super) fn cleanup_stale_margin_labels(
    commands: &mut Commands,
    label_query: &Query<(
        Entity,
        &MarginLabel,
        &FitOverlayVisual,
        &mut Text,
        &mut Node,
        &mut TextColor,
        &mut UiTargetCamera,
    )>,
    camera: Entity,
    visible_edges: &[Edge],
) {
    for (entity, _, visual, _, _, _, _) in label_query {
        let FitOverlayVisualKind::MarginLabel { edge } = visual.kind else {
            continue;
        };

        if visual.camera == camera && !visible_edges.contains(&edge) {
            commands.entity(entity).despawn();
        }
    }
}
