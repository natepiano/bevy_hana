//! Per-glyph overlay spawners: glyph bounding boxes, the "Bounding Box"
//! callout, origin dots, and the advancement dimension arrow.

use bevy::prelude::AlphaMode;
use bevy::prelude::Color;
use bevy::prelude::Handle;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use hana_kana::ToF32;
use hana_kana::ToUsize;

use super::GlyphMetricVisibility;
use super::pipeline::FontContext;
use super::pipeline::OverlayAssets;
use super::pipeline::OverlayContext;
use super::scaling;
use crate::callouts::CalloutCap;
use crate::debug::constants::BASELINE_COLOR;
use crate::debug::constants::BBOX_COLOR;
use crate::debug::constants::BBOX_MIN_WORLD_RATIO;
use crate::debug::constants::BRACKET_DASH_RATIO;
use crate::debug::constants::BRACKET_GAP_RATIO;
use crate::debug::constants::CALLOUT_Z_OFFSET;
use crate::debug::constants::LABEL_ADVANCEMENT;
use crate::debug::constants::LABEL_BOUNDING_BOX;
use crate::debug::constants::LABEL_ORIGIN;
use crate::debug::constants::LABEL_SIZE_RATIO;
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
use crate::layout::TextStyle;
use crate::panel::DiegeticPanel;
use crate::render::ComputedWorldText;
use crate::render::HairlineFade;

/// Geometry inputs for the horizontal advancement dimension arrow. Exists to
/// reduce helper parameter counts.
struct ArrowGeometry {
    origin_x:      f32,
    origin_y:      f32,
    advance_end_x: f32,
    descent_world: f32,
    spacing:       f32,
    z:             f32,
}

/// One guide segment in overlay-container world space (x right, y up).
/// [`spawn_guide_panel`] converts these into element-owned [`PanelLine`]s.
struct GuideSegment {
    start:       Vec2,
    end:         Vec2,
    width:       f32,
    color:       Color,
    start_inset: f32,
    end_inset:   f32,
    start_cap:   CalloutCap,
    end_cap:     CalloutCap,
}

impl GuideSegment {
    const fn new(start: Vec2, end: Vec2, width: f32, color: Color) -> Self {
        Self {
            start,
            end,
            width,
            color,
            start_inset: 0.0,
            end_inset: 0.0,
            start_cap: CalloutCap::None,
            end_cap: CalloutCap::None,
        }
    }
}

fn transparent_panel_material(assets: &mut OverlayAssets<'_>) -> Handle<StandardMaterial> {
    let mut material = default_panel_material();
    material.base_color = Color::NONE;
    material.alpha_mode = AlphaMode::Blend;
    material.unlit = true;
    assets.materials.add(material)
}

/// Spawns per-glyph bounding boxes, origin dots, and the advancement arrow.
pub(super) fn spawn_glyph_metric_guides(
    ctx: &mut OverlayContext<'_, '_, '_>,
    font_context: &FontContext<'_>,
    computed: &ComputedWorldText,
    assets: &mut OverlayAssets<'_>,
) {
    spawn_glyph_box_panels(ctx, computed, BBOX_COLOR, assets);

    // "Bounding Box" callout from the first glyph's bbox.
    if !computed.glyphs.is_empty() && ctx.overlay.labels == GlyphMetricVisibility::Shown {
        spawn_bounding_box_callout(ctx, font_context, computed, BBOX_COLOR, assets);
    }

    // Origin dots + Advancement arrow below the first glyph.
    if !computed.glyphs.is_empty() && ctx.overlay.labels == GlyphMetricVisibility::Shown {
        spawn_origin_and_advancement(ctx, font_context, computed, assets);
    }
}

/// Spawns one transparent world panel per glyph whose root element draws the
/// bounding box outline as four [`PanelLine`]s.
fn spawn_glyph_box_panels(
    ctx: &mut OverlayContext<'_, '_, '_>,
    computed: &ComputedWorldText,
    bbox_color: Color,
    assets: &mut OverlayAssets<'_>,
) {
    let material = transparent_panel_material(assets);

    let line_width = scaling::bbox_border_width(ctx.overlay, ctx.font_size, ctx.scale);

    for glyph in &computed.glyphs {
        let [x, y, width, height] = glyph.rect;
        if width <= 0.0 || height <= 0.0 {
            continue;
        }

        let tree = build_box_outline_tree(width, height, line_width, bbox_color);

        let Ok(panel) = DiegeticPanel::world()
            .size(width, height)
            .anchor(Anchor::Center)
            .shadow_casting(ctx.overlay.shadow_casting)
            .material(material.clone())
            .with_tree(tree)
            .build()
        else {
            continue;
        };

        ctx.commands.entity(ctx.entity).with_child((
            panel,
            Transform::from_xyz(x + width / 2.0, y - height / 2.0, CALLOUT_Z_OFFSET),
        ));
    }
}

/// Rectangle outline: each line is centered half a stroke inside its box edge.
fn build_box_outline_tree(width: f32, height: f32, line_width: f32, color: Color) -> LayoutTree {
    let inset = line_width * 0.5;
    let lines = [
        ((0.0, inset), (width, inset)),
        ((0.0, height - inset), (width, height - inset)),
        ((inset, 0.0), (inset, height)),
        ((width - inset, 0.0), (width - inset, height)),
    ]
    .map(|((x0, y0), (x1, y1))| {
        PanelLine::new(PanelPoint::new(x0, y0), PanelPoint::new(x1, y1))
            .width(line_width)
            .color(color)
    });

    LayoutBuilder::with_root(
        El::new()
            .size(width, height)
            .hairline_fade(HairlineFade::Full)
            .draw(PanelDraw::lines(lines).overflow(DrawOverflow::Visible)),
    )
    .build()
}

/// Spawns one transparent world panel sized to the segments' bounding box,
/// with every segment authored as an element-owned [`PanelLine`]. The element
/// pins [`HairlineFade::Full`] so debug guides never fade with distance.
fn spawn_guide_panel(
    ctx: &mut OverlayContext<'_, '_, '_>,
    segments: &[GuideSegment],
    z: f32,
    assets: &mut OverlayAssets<'_>,
) {
    let Some(first) = segments.first() else {
        return;
    };

    let mut min = first.start.min(first.end);
    let mut max = first.start.max(first.end);
    for segment in segments {
        min = min.min(segment.start.min(segment.end));
        max = max.max(segment.start.max(segment.end));
    }

    // A purely horizontal or vertical cluster has zero extent on one axis;
    // pad to the widest stroke so the panel size stays positive. The lines
    // overflow visibly, so padding never shifts them.
    let pad = segments
        .iter()
        .map(|segment| segment.width)
        .fold(f32::EPSILON, f32::max);
    let size = (max - min).max(Vec2::splat(pad));

    let lines = segments.iter().map(|segment| {
        let local = |point: Vec2| PanelPoint::new(point.x - min.x, max.y - point.y);
        PanelLine::new(local(segment.start), local(segment.end))
            .width(segment.width)
            .color(segment.color)
            .start_inset(segment.start_inset)
            .end_inset(segment.end_inset)
            .start_cap(segment.start_cap)
            .end_cap(segment.end_cap)
    });

    let tree = LayoutBuilder::with_root(
        El::new()
            .size(size.x, size.y)
            .hairline_fade(HairlineFade::Full)
            .draw(PanelDraw::lines(lines).overflow(DrawOverflow::Visible)),
    )
    .build();

    let material = transparent_panel_material(assets);

    let Ok(panel) = DiegeticPanel::world()
        .size(size.x, size.y)
        .anchor(Anchor::TopLeft)
        .shadow_casting(ctx.overlay.shadow_casting)
        .material(material)
        .with_tree(tree)
        .build()
    else {
        return;
    };

    ctx.commands
        .entity(ctx.entity)
        .with_child((panel, Transform::from_xyz(min.x, max.y, z)));
}

/// Spawns the "Bounding Box" callout label with shelf and riser lines.
fn spawn_bounding_box_callout(
    ctx: &mut OverlayContext<'_, '_, '_>,
    font_context: &FontContext<'_>,
    computed: &ComputedWorldText,
    bbox_color: Color,
    assets: &mut OverlayAssets<'_>,
) {
    let label_size = scaling::font_scale(ctx.font_size, ctx.scale) * LABEL_SIZE_RATIO;
    let callout_thickness = scaling::font_scale(ctx.font_size, ctx.scale) * BBOX_MIN_WORLD_RATIO;
    let z = CALLOUT_Z_OFFSET;

    let Some(last) = computed.glyphs.last() else {
        return;
    };
    let [last_x, last_y, last_width, last_height] = last.rect;

    // Shelf starts at right edge of last bbox, at vertical midpoint.
    let shelf_right_x = last_x + last_width;
    let shelf_y = last_y - last_height / 2.0;

    // Shelf extends rightward, then riser goes up. Label sits to the
    // left of the riser (CenterRight anchor) so it's always clear of
    // adjacent glyphs even when bounding boxes overlap.
    let shelf_len = computed
        .glyphs
        .first()
        .map_or(0.0, |glyph| scaling::arrow_spacing(glyph.advance_x) / 2.0);
    let shelf_end_x = shelf_right_x + shelf_len;

    // Vertical line goes up to halfway between Cap Height and Ascent.
    let baseline_y_layout = font_context.line.baseline;
    let ascent_y_layout = baseline_y_layout - font_context.line.ascent;
    let cap_height_y_layout = baseline_y_layout - font_context.font.cap_height;
    let callout_top_layout = f32::midpoint(cap_height_y_layout, ascent_y_layout);
    let callout_top_world = scaling::layout_to_world_y(callout_top_layout, ctx.anchor_y, ctx.scale);

    spawn_guide_panel(
        ctx,
        &[
            GuideSegment::new(
                Vec2::new(shelf_right_x, shelf_y),
                Vec2::new(shelf_end_x, shelf_y),
                callout_thickness,
                bbox_color,
            ),
            GuideSegment::new(
                Vec2::new(shelf_end_x, shelf_y),
                Vec2::new(shelf_end_x, callout_top_world),
                callout_thickness,
                bbox_color,
            ),
        ],
        z,
        assets,
    );

    // Label at the top of the riser, to the left (CenterRight anchor).
    let ascent_mid_layout = f32::midpoint(cap_height_y_layout, ascent_y_layout);
    let ascent_mid_world = scaling::layout_to_world_y(ascent_mid_layout, ctx.anchor_y, ctx.scale);
    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_BOUNDING_BOX,
        TextStyle::new(label_size)
            .with_color(bbox_color)
            .with_anchor(Anchor::CenterRight)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(
            shelf_end_x - scaling::label_gap(ctx.font_size, ctx.scale),
            ascent_mid_world,
            z,
        ),
    );
}

/// World-space placement of the origin and advancement-end dots, shared by the
/// standalone dot panels here and the metric-guide panel that layers the dots
/// above its baseline `PanelLine` when font metrics are also shown.
pub(super) struct DotGeometry {
    pub(super) radius:        f32,
    pub(super) origin_x:      f32,
    pub(super) advance_end_x: f32,
    pub(super) baseline_y:    f32,
}

/// Derives the origin and advancement-end dot placement from the first glyph.
/// `baseline_y` sits half a stroke above the true baseline, where
/// `metric_guide_lines` centers the rendered baseline line, so a dot drawn at
/// this center aligns with that line.
pub(super) fn dot_geometry(
    ctx: &OverlayContext<'_, '_, '_>,
    font_context: &FontContext<'_>,
    computed: &ComputedWorldText,
) -> Option<DotGeometry> {
    let first = computed.glyphs.first()?;
    let baseline_line_width =
        scaling::metric_line_border_width(ctx.overlay, ctx.font_size, ctx.scale);
    let baseline_y = baseline_line_width.mul_add(
        0.5,
        scaling::layout_to_world_y(font_context.line.baseline, ctx.anchor_y, ctx.scale),
    );
    Some(DotGeometry {
        radius: scaling::dot_radius(ctx.font_size, ctx.scale),
        origin_x: first.origin_x,
        advance_end_x: first.origin_x + first.advance_x,
        baseline_y,
    })
}

/// Spawns origin dots, origin label, advancement end dot, and advancement arrow.
fn spawn_origin_and_advancement(
    ctx: &mut OverlayContext<'_, '_, '_>,
    font_context: &FontContext<'_>,
    computed: &ComputedWorldText,
    assets: &mut OverlayAssets<'_>,
) {
    let label_size = scaling::font_scale(ctx.font_size, ctx.scale) * LABEL_SIZE_RATIO;
    let z = CALLOUT_Z_OFFSET;

    let Some(dots) = dot_geometry(ctx, font_context, computed) else {
        return;
    };
    let DotGeometry {
        radius: dot_radius,
        origin_x,
        advance_end_x,
        baseline_y: origin_y,
    } = dots;

    let first = &computed.glyphs[0];
    let first_mid_x = first.rect[0] + first.rect[2] / 2.0;

    let line_metrics = font_context.line;
    let descent_world = scaling::layout_to_world_y(
        line_metrics.baseline + line_metrics.descent,
        ctx.anchor_y,
        ctx.scale,
    );

    // The metric-guide panel draws these dots above its baseline line when font
    // metrics are shown; spawn standalone dot panels only when it does not.
    let draw_standalone_dots = ctx.overlay.font_metrics == GlyphMetricVisibility::Hidden;

    // Origin dot — small filled circle at (origin, baseline).
    if draw_standalone_dots {
        spawn_dot_panel(
            ctx,
            dot_radius,
            Vec3::new(origin_x, origin_y, z),
            Color::WHITE,
            assets,
        );
    }

    // Origin label — centered between the bottom of the first
    // glyph's bbox and the Descent line.
    let first_bbox_bottom = first.rect[1] - first.rect[3];
    let origin_label_y = f32::midpoint(first_bbox_bottom, descent_world);

    // Callout line from just above the label toward the origin
    // dot, touching the circle edge. The label's cap height in
    // world units gives the visual top of the text.
    let label_ascent_world = line_metrics.ascent * LABEL_SIZE_RATIO * ctx.scale;
    let label_top_y = origin_label_y + label_ascent_world;
    let dx = origin_x - first_mid_x;
    let dy = origin_y - label_top_y;
    let len = dx.hypot(dy);
    let edge_x = (dx / len).mul_add(-dot_radius, origin_x);
    let edge_y = (dy / len).mul_add(-dot_radius, origin_y);
    spawn_guide_panel(
        ctx,
        &[GuideSegment::new(
            Vec2::new(edge_x, edge_y),
            Vec2::new(first_mid_x, label_top_y),
            scaling::callout_line_thickness(ctx.overlay, ctx.font_size, ctx.scale),
            BASELINE_COLOR,
        )],
        z,
        assets,
    );
    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_ORIGIN,
        TextStyle::new(label_size)
            .with_color(ctx.overlay.color)
            .with_anchor(Anchor::Center)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(first_mid_x, origin_label_y, z),
    );

    // Advancement end dot — filled circle at (origin + advance, baseline).
    if draw_standalone_dots {
        spawn_dot_panel(
            ctx,
            dot_radius,
            Vec3::new(advance_end_x, origin_y, z),
            Color::WHITE,
            assets,
        );
    }

    // Advancement arrow — horizontal double-headed arrow below descent.
    let spacing = scaling::arrow_spacing(first.advance_x);
    spawn_advancement_arrow(
        ctx,
        &ArrowGeometry {
            origin_x,
            origin_y,
            advance_end_x,
            descent_world,
            spacing,
            z,
        },
        assets,
    );
}

/// Spawns the horizontal advancement arrow, its dashed bracket lines, and the
/// label. The arrow and both dash groups live in one guide panel element.
fn spawn_advancement_arrow(
    ctx: &mut OverlayContext<'_, '_, '_>,
    geometry: &ArrowGeometry,
    assets: &mut OverlayAssets<'_>,
) {
    let arrow_y = geometry.descent_world - geometry.spacing;
    let head = scaling::arrowhead_size(ctx.font_size, ctx.scale);
    let gap = scaling::arrow_gap(ctx.font_size, ctx.scale);
    let label_size = scaling::font_scale(ctx.font_size, ctx.scale) * LABEL_SIZE_RATIO;
    let thickness = scaling::callout_line_thickness(ctx.overlay, ctx.font_size, ctx.scale);

    // Dashed vertical bracket lines — from the arrow body up to the
    // origin/advance dots on the baseline.
    let tick_above = geometry.origin_y;
    let tick_below = arrow_y;
    let dash_len = geometry.spacing * BRACKET_DASH_RATIO;
    let gap_len = geometry.spacing * BRACKET_GAP_RATIO;
    let mut segments = dashed_segments(
        Vec2::new(geometry.origin_x, tick_below),
        Vec2::new(geometry.origin_x, tick_above),
        dash_len,
        gap_len,
        thickness,
        ctx.overlay.color,
    );
    segments.extend(dashed_segments(
        Vec2::new(geometry.advance_end_x, tick_below),
        Vec2::new(geometry.advance_end_x, tick_above),
        dash_len,
        gap_len,
        thickness,
        ctx.overlay.color,
    ));

    // Horizontal dimension arrow.
    let arrow_cap = CalloutCap::arrow().solid().length(head).width(head);
    segments.push(GuideSegment {
        start_inset: gap,
        end_inset: gap,
        start_cap: arrow_cap,
        end_cap: arrow_cap,
        ..GuideSegment::new(
            Vec2::new(geometry.origin_x, arrow_y),
            Vec2::new(geometry.advance_end_x, arrow_y),
            thickness,
            ctx.overlay.color,
        )
    });

    spawn_guide_panel(ctx, &segments, geometry.z, assets);

    // "Advancement" label centered below the arrow.
    let advance_mid_x = f32::midpoint(geometry.origin_x, geometry.advance_end_x);
    let advance_label_y = geometry.spacing.mul_add(-0.5, arrow_y);
    super::spawn_overlay_label(
        ctx.commands,
        ctx.entity,
        LABEL_ADVANCEMENT,
        TextStyle::new(label_size)
            .with_color(ctx.overlay.color)
            .with_anchor(Anchor::TopCenter)
            .with_shadow_mode(ctx.overlay.label_shadow_mode()),
        Transform::from_xyz(advance_mid_x, advance_label_y, geometry.z),
    );
}

/// Spawns a transparent world panel that draws one filled [`PanelCircle`] of
/// `radius` centered at `position`. Pins [`HairlineFade::Full`] so the dot
/// never fades with distance, matching [`spawn_guide_panel`].
fn spawn_dot_panel(
    ctx: &mut OverlayContext<'_, '_, '_>,
    radius: f32,
    position: Vec3,
    color: Color,
    assets: &mut OverlayAssets<'_>,
) {
    let size = radius * 2.0;
    let tree = LayoutBuilder::with_root(
        El::new()
            .size(size, size)
            .hairline_fade(HairlineFade::Full)
            .draw(
                PanelDraw::shapes([PanelShape::Circle(
                    PanelCircle::new(PanelPoint::new(radius, radius), radius).color(color),
                )])
                .overflow(DrawOverflow::Visible),
            ),
    )
    .build();

    let material = transparent_panel_material(assets);

    let Ok(panel) = DiegeticPanel::world()
        .size(size, size)
        .anchor(Anchor::Center)
        .shadow_casting(ctx.overlay.shadow_casting)
        .material(material)
        .with_tree(tree)
        .build()
    else {
        return;
    };

    ctx.commands
        .entity(ctx.entity)
        .with_child((panel, Transform::from_translation(position)));
}

/// Splits one line into dash segments. All dashes of the line stay in one
/// guide-panel element.
fn dashed_segments(
    start: Vec2,
    end: Vec2,
    dash_len: f32,
    gap_len: f32,
    width: f32,
    color: Color,
) -> Vec<GuideSegment> {
    let delta = end - start;
    let total_len = delta.length();
    if total_len < f32::EPSILON {
        return Vec::new();
    }
    let dir = delta / total_len;
    let stride = dash_len + gap_len;
    let count = (total_len / stride).ceil().to_usize();
    (0..count)
        .map(|i| {
            let t = i.to_f32() * stride;
            let dash_end = (t + dash_len).min(total_len);
            GuideSegment::new(start + dir * t, start + dir * dash_end, width, color)
        })
        .collect()
}
