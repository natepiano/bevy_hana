use bevy::light::NotShadowCaster;
use bevy::picking::Pickable;
use bevy::prelude::AlphaMode;
use bevy::prelude::ChildOf;
use bevy::prelude::Color;
use bevy::prelude::Entity;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Name;
use bevy::prelude::Rectangle;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::default;

use super::OverlayBoundingBox;
use super::constants::LEFT_OUTER_ARROW_SLOT;
use super::constants::NO_LINE_GAP_LABEL_PREFIX;
use super::constants::OVERLAY_BOUNDING_BOX_NAME;
use super::constants::RIGHT_OUTER_ARROW_SLOT;
use super::pipeline::FontContext;
use super::pipeline::GlyphExtents;
use super::pipeline::OverlayAssets;
use super::pipeline::OverlayContext;
use super::pipeline::TextServices;
use super::scaling;
use crate::debug::constants::CALLOUT_Z_OFFSET;
use crate::debug::constants::LABEL_ADVANCEMENT;
use crate::debug::constants::LABEL_ASCENT;
use crate::debug::constants::LABEL_BASELINE;
use crate::debug::constants::LABEL_BOTTOM;
use crate::debug::constants::LABEL_CAP_HEIGHT;
use crate::debug::constants::LABEL_DESCENT;
use crate::debug::constants::LABEL_LINE_HEIGHT;
use crate::debug::constants::LABEL_SIZE_RATIO;
use crate::debug::constants::LABEL_TOP;
use crate::debug::constants::LABEL_X_HEIGHT;
use crate::debug::constants::METRIC_LINE_Z_OFFSET;
use crate::layout::Anchor;
use crate::layout::MeasureTextFn;
use crate::layout::ShapedTextCache;
use crate::layout::TextDimensions;
use crate::layout::TextStyle;
use crate::layout::Unit;

/// Visual parameters shared across arrow-label spawners. Exists to reduce
/// helper parameter counts.
pub(super) struct LabelStyle {
    pub(super) size:     f32,
    pub(super) color:    Color,
    pub(super) z_offset: f32,
    pub(super) gap:      f32,
}

/// Precomputed layout Y-coordinates for the horizontal metric guides. Each arrow
/// label spawner reads the guides it needs; unused fields are ignored.
/// Exists to reduce helper parameter counts.
pub(super) struct MetricGuideYs {
    pub(super) baseline:   f32,
    pub(super) ascent:     f32,
    pub(super) cap_height: f32,
    pub(super) x_height:   f32,
    pub(super) descent:    f32,
}

/// Spawns labels for metric lines and dimension arrows.
///
/// Left-side labels sit outside their arrows (`CenterRight` anchor).
/// Right-side labels sit outside their arrows (`CenterLeft` anchor).
pub(super) fn spawn_metric_labels(
    ctx: &mut OverlayContext<'_, '_, '_>,
    font_name: &str,
    font_context: &FontContext<'_>,
    metric_lines: &[(&str, f32)],
    extents: &GlyphExtents,
) {
    let style = LabelStyle {
        size:     scaling::font_scale(ctx.font_size, ctx.scale) * LABEL_SIZE_RATIO,
        color:    ctx.overlay.color,
        z_offset: METRIC_LINE_Z_OFFSET,
        gap:      scaling::label_gap(ctx.font_size, ctx.scale),
    };

    let line_metrics = font_context.line;
    let font_metrics = font_context.font;
    let guides = MetricGuideYs {
        baseline:   line_metrics.baseline,
        ascent:     line_metrics.baseline - line_metrics.ascent,
        cap_height: line_metrics.baseline - font_metrics.cap_height,
        x_height:   line_metrics.baseline - font_metrics.x_height,
        descent:    line_metrics.baseline + line_metrics.descent,
    };

    let left_1 = extents.first_left - extents.arrow_spacing;
    let left_2 = LEFT_OUTER_ARROW_SLOT.mul_add(-extents.arrow_spacing, extents.first_left);

    let right_1 = extents.last_right + extents.arrow_spacing;
    let right_2 = RIGHT_OUTER_ARROW_SLOT.mul_add(extents.arrow_spacing, extents.last_right);

    spawn_line_edge_labels(ctx, metric_lines, &style, left_2);
    spawn_left_arrow_labels(
        ctx,
        font_context,
        font_name,
        &style,
        &guides,
        left_1,
        left_2,
    );
    spawn_right_arrow_labels(ctx, &style, &guides, right_1, right_2);
}

fn measure_overlay_label(
    cache: &ShapedTextCache,
    measure_text: &MeasureTextFn,
    text: &str,
    size: f32,
    boost: f32,
    scale: f32,
) -> TextDimensions {
    let measure = TextStyle::new(size)
        .for_shaping(Anchor::Center)
        .scaled(boost)
        .as_measure();
    if let Some(dims) = cache.get_measurement(text, &measure) {
        return TextDimensions {
            width:       dims.width * scale,
            height:      dims.height * scale,
            line_height: dims.line_height * scale,
        };
    }
    let dims = (measure_text)(text, &measure);
    cache.insert_measurement(text, &measure, dims);
    TextDimensions {
        width:       dims.width * scale,
        height:      dims.height * scale,
        line_height: dims.line_height * scale,
    }
}

pub(super) fn spawn_overlay_bounds_target(
    ctx: &mut OverlayContext<'_, '_, '_>,
    font_name: &str,
    font_context: &FontContext<'_>,
    extents: &GlyphExtents,
    text_services: &TextServices<'_>,
    assets: &mut OverlayAssets<'_>,
) -> Entity {
    let label_size = scaling::font_scale(ctx.font_size, ctx.scale) * LABEL_SIZE_RATIO;
    let gap = scaling::label_gap(ctx.font_size, ctx.scale);
    let boost = if Unit::Points.meters_per_unit() > 0.0 {
        1.0 / Unit::Points.meters_per_unit()
    } else {
        1.0
    };

    let line_height_dims = measure_overlay_label(
        text_services.cache,
        text_services.measure_text,
        LABEL_LINE_HEIGHT,
        label_size,
        boost,
        ctx.scale,
    );
    let cap_height_dims = measure_overlay_label(
        text_services.cache,
        text_services.measure_text,
        LABEL_CAP_HEIGHT,
        label_size,
        boost,
        ctx.scale,
    );
    let advancement_dims = measure_overlay_label(
        text_services.cache,
        text_services.measure_text,
        LABEL_ADVANCEMENT,
        label_size,
        boost,
        ctx.scale,
    );

    let line_metrics = font_context.line;
    let baseline_y = line_metrics.baseline;
    let ascent_y = baseline_y - line_metrics.ascent;
    let top_y = line_metrics.top;
    let left_2 = LEFT_OUTER_ARROW_SLOT.mul_add(-extents.arrow_spacing, extents.first_left);
    let right_2 = RIGHT_OUTER_ARROW_SLOT.mul_add(extents.arrow_spacing, extents.last_right);

    let line_height_anchor_x = left_2 - gap;
    let cap_height_anchor_x = right_2 + gap;
    let line_height_left = line_height_anchor_x - line_height_dims.width;
    let cap_height_right = cap_height_anchor_x + cap_height_dims.width;

    let descent_world =
        scaling::layout_to_world_y(baseline_y + line_metrics.descent, ctx.anchor_y, ctx.scale);
    let spacing = extents.arrow_spacing;
    let arrow_y = descent_world - spacing;
    let advancement_anchor_y = spacing.mul_add(-0.5, arrow_y);
    let advancement_bottom = advancement_anchor_y - advancement_dims.height;

    let has_line_gap = (line_metrics.top - ascent_y).abs() > 0.5;
    let top_line_y = if has_line_gap { top_y } else { ascent_y };
    let mut top_extent = scaling::layout_to_world_y(top_line_y, ctx.anchor_y, ctx.scale);
    if !has_line_gap {
        let no_gap_label = format!("{NO_LINE_GAP_LABEL_PREFIX}{font_name}");
        let no_gap_dims = measure_overlay_label(
            text_services.cache,
            text_services.measure_text,
            &no_gap_label,
            label_size,
            boost,
            ctx.scale,
        );
        let no_gap_top =
            scaling::layout_to_world_y(ascent_y, ctx.anchor_y, ctx.scale) + no_gap_dims.height;
        top_extent = top_extent.max(no_gap_top);
    }

    let left = line_height_left;
    let right = cap_height_right;
    let top = top_extent;
    let bottom = advancement_bottom;
    let width = (right - left).max(0.001);
    let height = (top - bottom).max(0.001);
    let center = Vec3::new(
        f32::midpoint(left, right),
        f32::midpoint(top, bottom),
        CALLOUT_Z_OFFSET,
    );

    let mesh = Mesh3d(assets.meshes.add(Rectangle::new(width, height)));
    let material = MeshMaterial3d(assets.materials.add(StandardMaterial {
        base_color: Color::NONE,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    }));

    ctx.commands
        .spawn((
            Name::new(OVERLAY_BOUNDING_BOX_NAME),
            OverlayBoundingBox,
            Pickable::IGNORE,
            NotShadowCaster,
            mesh,
            material,
            Transform::from_translation(center),
            Visibility::Inherited,
            ChildOf(ctx.entity),
        ))
        .id()
}

/// Spawns Top/Bottom line-edge labels.
fn spawn_line_edge_labels(
    ctx: &mut OverlayContext<'_, '_, '_>,
    metric_lines: &[(&str, f32)],
    style: &LabelStyle,
    label_x: f32,
) {
    for &(label, layout_y) in metric_lines {
        if label != LABEL_TOP && label != LABEL_BOTTOM {
            continue;
        }
        let line_world_y = scaling::layout_to_world_y(layout_y, ctx.anchor_y, ctx.scale);
        super::spawn_overlay_label(
            ctx.commands,
            ctx.entity,
            label,
            TextStyle::new(style.size)
                .with_color(style.color)
                .with_anchor(Anchor::CenterRight)
                .with_shadow_mode(ctx.overlay.label_shadow_mode()),
            Transform::from_xyz(label_x - style.gap, line_world_y, style.z_offset),
        );
    }
}

/// Spawns Ascent, Descent, Line Height, and optional "no line gap" labels.
fn spawn_left_arrow_labels(
    ctx: &mut OverlayContext<'_, '_, '_>,
    font_context: &FontContext<'_>,
    font_name: &str,
    style: &LabelStyle,
    guides: &MetricGuideYs,
    left_1: f32,
    left_2: f32,
) {
    let label_y_mid = f32::midpoint(guides.baseline, guides.x_height);
    let label_y_mid_world = scaling::layout_to_world_y(label_y_mid, ctx.anchor_y, ctx.scale);
    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_ASCENT,
        TextStyle::new(style.size)
            .with_color(style.color)
            .with_anchor(Anchor::CenterRight)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(left_1 - style.gap, label_y_mid_world, style.z_offset),
    );

    let descent_mid = f32::midpoint(guides.baseline, guides.descent);
    let descent_mid_world = scaling::layout_to_world_y(descent_mid, ctx.anchor_y, ctx.scale);
    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_DESCENT,
        TextStyle::new(style.size)
            .with_color(style.color)
            .with_anchor(Anchor::CenterRight)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(left_1 - style.gap, descent_mid_world, style.z_offset),
    );

    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_LINE_HEIGHT,
        TextStyle::new(style.size)
            .with_color(style.color)
            .with_anchor(Anchor::CenterRight)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(left_2 - style.gap, label_y_mid_world, style.z_offset),
    );

    // Baseline label: offset down by half the label's descent so the visual
    // center of the text sits on the red line.
    let line_metrics = font_context.line;
    let label_descent_offset = line_metrics.descent * LABEL_SIZE_RATIO * ctx.scale / 2.0;
    let baseline_label_world =
        scaling::layout_to_world_y(guides.baseline, ctx.anchor_y, ctx.scale) - label_descent_offset;
    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_BASELINE,
        TextStyle::new(style.size)
            .with_color(style.color)
            .with_anchor(Anchor::CenterRight)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(left_2 - style.gap, baseline_label_world, style.z_offset),
    );

    let has_line_gap =
        (line_metrics.top - (line_metrics.baseline - line_metrics.ascent)).abs() > 0.5;
    if !has_line_gap {
        let ascent_world = scaling::layout_to_world_y(guides.ascent, ctx.anchor_y, ctx.scale);
        let no_gap_label = format!("{NO_LINE_GAP_LABEL_PREFIX}{font_name}");
        super::spawn_overlay_label(
            ctx.commands,
            ctx.entity,
            no_gap_label,
            TextStyle::new(style.size)
                .with_color(style.color)
                .with_anchor(Anchor::BottomLeft)
                .with_shadow_mode(ctx.overlay.label_shadow_mode()),
            Transform::from_xyz(left_2, ascent_world, style.z_offset),
        );
    }
}

/// Spawns x-Height and Cap Height labels on the right side.
fn spawn_right_arrow_labels(
    ctx: &mut OverlayContext<'_, '_, '_>,
    style: &LabelStyle,
    guides: &MetricGuideYs,
    right_1: f32,
    right_2: f32,
) {
    let x_height_mid = f32::midpoint(guides.x_height, guides.baseline);
    let x_height_mid_world = scaling::layout_to_world_y(x_height_mid, ctx.anchor_y, ctx.scale);
    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_X_HEIGHT,
        TextStyle::new(style.size)
            .with_color(style.color)
            .with_anchor(Anchor::CenterLeft)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(right_1 + style.gap, x_height_mid_world, style.z_offset),
    );

    let cap_mid = f32::midpoint(guides.cap_height, guides.x_height);
    let cap_mid_world = scaling::layout_to_world_y(cap_mid, ctx.anchor_y, ctx.scale);
    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_CAP_HEIGHT,
        TextStyle::new(style.size)
            .with_color(style.color)
            .with_anchor(Anchor::CenterLeft)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(right_2 + style.gap, cap_mid_world, style.z_offset),
    );
}
