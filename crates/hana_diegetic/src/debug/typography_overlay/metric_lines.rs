use bevy::prelude::AlphaMode;
use bevy::prelude::Color;
use bevy::prelude::Transform;

use super::GlyphMetricVisibility;
use super::TypographyOverlay;
use super::constants::LEFT_OUTER_ARROW_SLOT;
use super::constants::METRIC_RECT_WIDTH_SLOTS;
use super::glyph;
use super::labels;
use super::pipeline::FontContext;
use super::pipeline::GlyphExtents;
use super::pipeline::OverlayAssets;
use super::pipeline::OverlayContext;
use super::pipeline::TextServices;
use super::scaling;
use crate::callouts;
use crate::callouts::CalloutCap;
use crate::debug::constants::BASELINE_COLOR;
use crate::debug::constants::LABEL_ASCENT;
use crate::debug::constants::LABEL_BASELINE;
use crate::debug::constants::LABEL_BOTTOM;
use crate::debug::constants::LABEL_CAP_HEIGHT;
use crate::debug::constants::LABEL_DESCENT;
use crate::debug::constants::LABEL_TOP;
use crate::debug::constants::LABEL_X_HEIGHT;
use crate::debug::constants::METRIC_LINE_Z_OFFSET;
use crate::default_panel_material;
use crate::layout::Anchor;
use crate::layout::DrawOverflow;
use crate::layout::El;
use crate::layout::LayoutBuilder;
use crate::layout::LayoutTree;
use crate::layout::PanelCircle;
use crate::layout::PanelDraw;
use crate::layout::PanelLine;
use crate::layout::PanelPoint;
use crate::layout::PanelShape;
use crate::layout::ShapedLineMetrics;
use crate::panel::DiegeticPanel;
use crate::render::ComputedWorldText;
use crate::render::HairlineFade;
use crate::text::FontMetrics;

pub(super) struct MetricLineSpec {
    pub(super) offset_y: f32,
    pub(super) color:    Color,
}

/// Spawns horizontal metric lines and dimension arrows for font-level metrics.
pub(super) fn spawn_font_metric_guides(
    ctx: &mut OverlayContext<'_, '_, '_>,
    font_name: &str,
    font_context: &FontContext<'_>,
    computed: &ComputedWorldText,
    text_services: &TextServices<'_>,
    assets: &mut OverlayAssets<'_>,
) {
    let first_glyph = computed.glyphs.first();
    let last_glyph = computed.glyphs.last();
    let extents = GlyphExtents {
        first_left:    first_glyph.map_or(0.0, |glyph| glyph.rect[0]),
        last_right:    last_glyph.map_or(0.0, |glyph| glyph.rect[0] + glyph.rect[2]),
        arrow_spacing: first_glyph.map_or(0.0, |glyph| scaling::arrow_spacing(glyph.advance_x)),
    };

    spawn_metric_guide_panel(ctx, font_context, computed, &extents, assets);

    if ctx.overlay.labels == GlyphMetricVisibility::Shown {
        let metric_lines = metric_line_labels(font_context.font, font_context.line);
        labels::spawn_metric_labels(ctx, font_name, font_context, &metric_lines, &extents);
    }

    labels::spawn_overlay_bounds_target(
        ctx,
        font_name,
        font_context,
        &extents,
        text_services,
        assets,
    );
}

/// Spawns horizontal font metric lines and the vertical dimension arrows as a
/// single transparent world panel whose root element owns every guide as a
/// [`PanelLine`].
fn spawn_metric_guide_panel(
    ctx: &mut OverlayContext<'_, '_, '_>,
    font_context: &FontContext<'_>,
    computed: &ComputedWorldText,
    extents: &GlyphExtents,
    assets: &mut OverlayAssets<'_>,
) {
    let line_specs = metric_line_specs(
        font_context.font,
        font_context.line,
        ctx.overlay,
        ctx.anchor_y,
        ctx.scale,
    );
    if line_specs.len() < 2 {
        return;
    }

    let width = METRIC_RECT_WIDTH_SLOTS.mul_add(
        extents.arrow_spacing,
        extents.last_right - extents.first_left,
    );
    if width <= 0.0 {
        return;
    }

    let height = line_specs.last().map_or(0.0, |line| line.offset_y)
        - line_specs.first().map_or(0.0, |line| line.offset_y);
    if height <= 0.0 {
        return;
    }

    let line_width = scaling::metric_line_border_width(ctx.overlay, ctx.font_size, ctx.scale);
    let mut lines = metric_guide_lines(width, &line_specs, line_width);
    lines.extend(metric_arrow_lines(ctx, font_context, extents, width));

    let mut material = default_panel_material();
    material.base_color = Color::NONE;
    material.alpha_mode = AlphaMode::Blend;
    material.unlit = true;
    let material = assets.materials.add(material);

    let x = LEFT_OUTER_ARROW_SLOT.mul_add(-extents.arrow_spacing, extents.first_left);
    let line_metrics = font_context.line;
    let top_layout =
        if (line_metrics.top - (line_metrics.baseline - line_metrics.ascent)).abs() > 0.5 {
            line_metrics.top
        } else {
            line_metrics.baseline - line_metrics.ascent
        };
    let top_world = scaling::layout_to_world_y(top_layout, ctx.anchor_y, ctx.scale);

    let dot_shapes = origin_dot_shapes(ctx, font_context, computed, x, top_world);
    let tree = build_metric_guide_tree(width, height, lines, dot_shapes);

    let Ok(panel) = DiegeticPanel::world()
        .size(width, height)
        .anchor(Anchor::TopLeft)
        .shadow_casting(ctx.overlay.shadow_casting)
        .material(material)
        .with_tree(tree)
        .build()
    else {
        return;
    };

    ctx.commands.entity(ctx.entity).with_child((
        panel,
        Transform::from_xyz(x, top_world, METRIC_LINE_Z_OFFSET),
    ));
}

fn metric_line_specs(
    font_metrics: &FontMetrics,
    line_metrics: &ShapedLineMetrics,
    overlay: &TypographyOverlay,
    anchor_y: f32,
    scale: f32,
) -> Vec<MetricLineSpec> {
    let baseline_y = line_metrics.baseline;
    let ascent_y = baseline_y - line_metrics.ascent;
    let descent_y = baseline_y + line_metrics.descent;
    let top_y = line_metrics.top;
    let bottom_y = line_metrics.bottom;

    let include_top = (top_y - ascent_y).abs() > 0.5;
    let top_layout = if include_top { top_y } else { ascent_y };
    let top_world = scaling::layout_to_world_y(top_layout, anchor_y, scale);
    let offset = |layout_y: f32| top_world - scaling::layout_to_world_y(layout_y, anchor_y, scale);

    let mut specs = Vec::with_capacity(7);
    if include_top {
        specs.push(MetricLineSpec {
            offset_y: 0.0,
            color:    overlay.color,
        });
    }
    specs.push(MetricLineSpec {
        offset_y: offset(ascent_y),
        color:    overlay.color,
    });
    specs.push(MetricLineSpec {
        offset_y: offset(baseline_y - font_metrics.cap_height),
        color:    overlay.color,
    });
    specs.push(MetricLineSpec {
        offset_y: offset(baseline_y - font_metrics.x_height),
        color:    overlay.color,
    });
    specs.push(MetricLineSpec {
        offset_y: offset(baseline_y),
        color:    BASELINE_COLOR,
    });
    specs.push(MetricLineSpec {
        offset_y: offset(descent_y),
        color:    overlay.color,
    });
    if (bottom_y - descent_y).abs() > 0.5 {
        specs.push(MetricLineSpec {
            offset_y: offset(bottom_y),
            color:    overlay.color,
        });
    }
    specs
}

/// One horizontal [`PanelLine`] per metric: the first line hangs below the
/// panel top edge, every other line sits above its spec offset. Specs at
/// non-increasing offsets are skipped.
fn metric_guide_lines(
    width: f32,
    line_specs: &[MetricLineSpec],
    line_width: f32,
) -> Vec<PanelLine> {
    let mut lines = Vec::with_capacity(line_specs.len());
    let mut previous_offset = f32::NEG_INFINITY;
    for (index, spec) in line_specs.iter().enumerate() {
        if index > 0 && spec.offset_y <= previous_offset {
            continue;
        }
        previous_offset = spec.offset_y;
        let center_y = if index == 0 {
            line_width * 0.5
        } else {
            line_width.mul_add(-0.5, spec.offset_y)
        };
        lines.push(
            PanelLine::new(
                PanelPoint::new(0.0, center_y),
                PanelPoint::new(width, center_y),
            )
            .width(line_width)
            .color(spec.color),
        );
    }
    lines
}

const fn solid_arrow_cap(head: f32, tint: Option<Color>) -> CalloutCap {
    let cap = callouts::CalloutCap::arrow()
        .solid()
        .length(head)
        .width(head);
    if let Some(color) = tint {
        cap.color(color)
    } else {
        cap
    }
}

/// Vertical dimension arrows between metric lines, authored in the metric
/// guide panel's local space (origin at the panel's top-left, y down).
fn metric_arrow_lines(
    ctx: &OverlayContext<'_, '_, '_>,
    font_context: &FontContext<'_>,
    extents: &GlyphExtents,
    width: f32,
) -> Vec<PanelLine> {
    let line_metrics = font_context.line;
    let font_metrics = font_context.font;
    let baseline_y = line_metrics.baseline;
    let ascent_y = baseline_y - line_metrics.ascent;
    let descent_y = baseline_y + line_metrics.descent;

    let top_layout = if (line_metrics.top - ascent_y).abs() > 0.5 {
        line_metrics.top
    } else {
        ascent_y
    };
    let top_world = scaling::layout_to_world_y(top_layout, ctx.anchor_y, ctx.scale);
    let local_y =
        |layout_y: f32| top_world - scaling::layout_to_world_y(layout_y, ctx.anchor_y, ctx.scale);

    let ascent = local_y(ascent_y);
    let baseline = local_y(baseline_y);
    let descent = local_y(descent_y);
    let x_height = local_y(baseline_y - font_metrics.x_height);
    let cap_height = local_y(baseline_y - font_metrics.cap_height);

    let spacing = extents.arrow_spacing;
    let left_1 = 2.0 * spacing;
    let left_2 = 0.0;
    let right_1 = width - spacing;
    let right_2 = width;
    let head = scaling::arrowhead_size(ctx.font_size, ctx.scale);
    let gap = scaling::arrow_gap(ctx.font_size, ctx.scale);
    let thickness = scaling::callout_line_thickness(ctx.overlay, ctx.font_size, ctx.scale);

    let plain = || solid_arrow_cap(head, None);
    let tinted = || solid_arrow_cap(head, Some(BASELINE_COLOR));

    [
        (left_1, ascent, baseline, plain(), tinted()),
        (left_1, baseline, descent, tinted(), plain()),
        (left_2, ascent, descent, plain(), plain()),
        (right_1, x_height, baseline, plain(), tinted()),
        (right_2, cap_height, baseline, plain(), tinted()),
    ]
    .into_iter()
    .map(|(x, from_y, to_y, start_cap, end_cap)| {
        PanelLine::new(PanelPoint::new(x, from_y), PanelPoint::new(x, to_y))
            .width(thickness)
            .color(ctx.overlay.color)
            .start_inset(gap)
            .end_inset(gap)
            .start_cap(start_cap)
            .end_cap(end_cap)
    })
    .collect()
}

/// Origin and advancement-end dots authored in the metric panel's local space
/// (top-left origin, y down). Empty unless both glyph metrics and labels are
/// shown, matching the standalone dots in
/// [`glyph::spawn_origin_and_advancement`](super::glyph). The metric panel's
/// world transform places local `(0, 0)` at `(panel_x, top_world)`.
fn origin_dot_shapes(
    ctx: &OverlayContext<'_, '_, '_>,
    font_context: &FontContext<'_>,
    computed: &ComputedWorldText,
    panel_x: f32,
    top_world: f32,
) -> Vec<PanelShape> {
    if ctx.overlay.glyph_metrics != GlyphMetricVisibility::Shown
        || ctx.overlay.labels != GlyphMetricVisibility::Shown
    {
        return Vec::new();
    }
    let Some(dots) = glyph::dot_geometry(ctx, font_context, computed) else {
        return Vec::new();
    };
    let local_y = top_world - dots.baseline_y;
    [dots.origin_x, dots.advance_end_x]
        .into_iter()
        .map(|world_x| {
            PanelShape::Circle(
                PanelCircle::new(PanelPoint::new(world_x - panel_x, local_y), dots.radius)
                    .color(Color::WHITE),
            )
        })
        .collect()
}

/// Overlay tree owning every metric guide line plus the origin dots. The lines
/// and dots are separate full-panel children so they share the panel's local
/// coordinate space; the dot child is raised one [`DrawZIndex`] level so the
/// dots composite above the baseline line. Both children pin
/// [`HairlineFade::Full`] so debug guides never fade with distance, and their
/// draws overflow visibly because the outermost arrow columns sit exactly on
/// the panel's left and right edges.
fn build_metric_guide_tree(
    width: f32,
    height: f32,
    lines: Vec<PanelLine>,
    dot_shapes: Vec<PanelShape>,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::overlay().size(width, height));
    builder.with(
        El::new()
            .size(width, height)
            .hairline_fade(HairlineFade::Full)
            .draw(PanelDraw::lines(lines).overflow(DrawOverflow::Visible)),
        |_| {},
    );
    if !dot_shapes.is_empty() {
        builder.with(
            El::new()
                .size(width, height)
                .hairline_fade(HairlineFade::Full)
                .z_index(1)
                .draw(PanelDraw::shapes(dot_shapes).overflow(DrawOverflow::Visible)),
            |_| {},
        );
    }
    builder.build()
}

/// `(label, layout_y)` pairs for the metric label spawner, in top-to-bottom
/// order.
fn metric_line_labels(
    font_metrics: &FontMetrics,
    line_metrics: &ShapedLineMetrics,
) -> Vec<(&'static str, f32)> {
    let baseline_y = line_metrics.baseline;
    let ascent_y = baseline_y - line_metrics.ascent;
    let descent_y = baseline_y + line_metrics.descent;
    let top_y = line_metrics.top;
    let bottom_y = line_metrics.bottom;

    let mut metric_lines: Vec<(&'static str, f32)> = Vec::with_capacity(7);
    if (top_y - ascent_y).abs() > 0.5 {
        metric_lines.push((LABEL_TOP, top_y));
    }
    metric_lines.push((LABEL_ASCENT, ascent_y));
    metric_lines.push((LABEL_CAP_HEIGHT, baseline_y - font_metrics.cap_height));
    metric_lines.push((LABEL_X_HEIGHT, baseline_y - font_metrics.x_height));
    metric_lines.push((LABEL_BASELINE, baseline_y));
    metric_lines.push((LABEL_DESCENT, descent_y));
    if (bottom_y - descent_y).abs() > 0.5 {
        metric_lines.push((LABEL_BOTTOM, bottom_y));
    }
    metric_lines
}
