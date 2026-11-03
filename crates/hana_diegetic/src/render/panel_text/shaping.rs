use std::collections::HashSet;
use std::time::Instant;

use bevy::camera::primitives::Aabb;
use bevy::math::Vec4;
use bevy::prelude::Changed;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;

use super::PanelTextLayout;
use super::PreparedPanelText;
use super::reify::PanelGlyphSource;
use super::reify::RichRunGlyphs;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::constants::MILLISECONDS_PER_SECOND;
use crate::layout::Anchor;
use crate::layout::BoundingBox;
use crate::layout::ShapedTextCache;
use crate::layout::TextStyle;
use crate::panel::DiegeticPerfStats;
use crate::panel::FrameWork;
use crate::render::DEFAULT_BAND_COUNT;
use crate::render::constants;
use crate::render::text_shaping;
use crate::render::text_shaping::GlyphReadiness;
use crate::render::text_shaping::TextBuildStats;
use crate::render::text_shaping::TextShapingContext;
use crate::render::world_text::AwaitingReady;
use crate::render::world_text::DrawnTextBounds;
use crate::render::world_text::TextContent;
use crate::text;
use crate::text::FontRegistry;
use crate::text::GlyphCache;
use crate::text::PreparedTextRun;

/// Shapes text for panel [`TextContent`] children that are changed or pending.
pub(super) fn shape_panel_text_children(
    changed_texts: Query<
        Entity,
        (
            With<TextContent>,
            Or<(
                Changed<TextContent>,
                Changed<TextStyle>,
                Changed<PanelTextLayout>,
                Changed<RichRunGlyphs>,
            )>,
        ),
    >,
    mut texts: Query<(
        Ref<TextContent>,
        Ref<TextStyle>,
        Ref<PanelTextLayout>,
        &ChildOf,
        Option<&mut PreparedPanelText>,
        Option<Ref<RichRunGlyphs>>,
    )>,
    font_registry: Res<FontRegistry>,
    shaping_cx: Res<TextShapingContext>,
    cache: Res<ShapedTextCache>,
    mut backend: ResMut<GlyphCache>,
    mut perf: ResMut<DiegeticPerfStats>,
    mut commands: Commands,
) {
    let shape_stage_start = Instant::now();
    let mut aggregate = TextBuildStats::default();
    let mut shaped_panels: HashSet<Entity> = HashSet::new();

    let to_process: Vec<Entity> = changed_texts.iter().collect();

    if to_process.is_empty() {
        perf.panel_text.shape_ms = 0.0;
        perf.panel_text.parley_ms = 0.0;
        perf.panel_text.shaped_panels = FrameWork::default();
        perf.panel_text.total_ms = perf.panel_text.mesh_build_ms;
        return;
    }

    for entity in to_process {
        let Ok((world_text, style, panel_text_child, child_of, prepared, rich_glyphs)) =
            texts.get_mut(entity)
        else {
            continue;
        };

        if world_text.text().is_empty() {
            clear_panel_text_output(entity, &mut commands);
            continue;
        }

        let config = style.for_shaping(Anchor::Center);
        if let Some(mut prepared) = prepared
            && !rich_glyphs.as_ref().is_some_and(DetectChanges::is_changed)
            && text_render_only_refresh(
                &world_text,
                &style,
                &panel_text_child,
                &config,
                &mut prepared,
            )
        {
            continue;
        }
        let placement = QuadPlacement {
            bounds:    panel_text_child.bounds,
            scale:     Vec2::new(panel_text_child.scale_x, panel_text_child.scale_y),
            anchor:    Vec2::new(panel_text_child.anchor_x, panel_text_child.anchor_y),
            clip_rect: panel_text_child.clip_rect,
        };

        let (panel_text, stats) = build_panel_text(
            world_text.text(),
            &config,
            &placement,
            &mut backend,
            &font_registry,
            &shaping_cx,
            &cache,
            rich_glyphs
                .as_deref()
                .map_or(PanelGlyphSource::Plain, PanelGlyphSource::Rich),
        );
        aggregate.accumulate(&stats);
        shaped_panels.insert(child_of.parent());
        let readiness = GlyphReadiness::from(&stats);
        apply_panel_result(entity, panel_text, readiness, &mut commands);
    }

    perf.panel_text.shape_ms = shape_stage_start.elapsed().as_secs_f32() * MILLISECONDS_PER_SECOND;
    perf.panel_text.parley_ms = aggregate.shape_ms;
    perf.panel_text.shaped_panels = shaped_panels.len().into();
    perf.panel_text.total_ms = perf.panel_text.shape_ms + perf.panel_text.mesh_build_ms;
}

fn text_render_only_refresh(
    world_text: &Ref<'_, TextContent>,
    style: &Ref<'_, TextStyle>,
    panel_text_child: &Ref<'_, PanelTextLayout>,
    config: &TextStyle,
    prepared: &mut PreparedPanelText,
) -> bool {
    if world_text.is_changed() || panel_text_child.is_changed() {
        return false;
    }
    if !style.is_changed() || !prepared.style_gate.gating_eq(config) {
        return false;
    }

    let render_mode = config.render_mode();
    let shadow_mode = config.shadow_mode();
    let fill_color = config.color();
    let render_fields_changed = prepared.render_mode != render_mode
        || prepared.shadow_mode != shadow_mode
        || prepared.fill_color != fill_color;
    if render_fields_changed {
        prepared.style_gate = config.clone();
        prepared.render_mode = render_mode;
        prepared.shadow_mode = shadow_mode;
        prepared.fill_color = fill_color;
        prepared.render_only = true;
    }
    true
}

/// Placement parameters that position glyphs into panel-local space.
struct QuadPlacement {
    bounds:    BoundingBox,
    scale:     Vec2,
    anchor:    Vec2,
    clip_rect: Option<BoundingBox>,
}

fn clear_panel_text_output(entity: Entity, commands: &mut Commands) {
    commands
        .entity(entity)
        .remove::<(PreparedPanelText, DrawnTextBounds, AwaitingReady)>();
}

fn build_panel_text(
    text: &str,
    config: &TextStyle,
    placement: &QuadPlacement,
    backend: &mut GlyphCache,
    font_registry: &FontRegistry,
    shaping_cx: &TextShapingContext,
    cache: &ShapedTextCache,
    glyph_source: PanelGlyphSource<&RichRunGlyphs>,
) -> (Option<PreparedPanelText>, TextBuildStats) {
    let mut stats = TextBuildStats {
        texts: 1,
        ..Default::default()
    };
    let layout_start = Instant::now();
    let plain_run;
    let glyphs = match glyph_source {
        PanelGlyphSource::Plain => {
            plain_run =
                text_shaping::shape_text_cached(text, config, font_registry, shaping_cx, cache);
            stats.shape_ms = layout_start.elapsed().as_secs_f32() * MILLISECONDS_PER_SECOND;
            &plain_run.glyphs[..]
        },
        PanelGlyphSource::Rich(run) => &run.glyphs[..],
    };
    stats.glyphs = glyphs.len();
    let positioned_glyphs = text_shaping::positioned_glyphs(glyphs, font_registry, &mut stats);
    if stats.failed_glyphs > 0 {
        return (None, stats);
    }

    let run_start = Instant::now();
    let anchor = panel_layout_anchor(placement);
    let prepared = match backend.prepare_positioned_run_with_scale(
        &positioned_glyphs,
        anchor,
        config.size(),
        placement.scale,
        DEFAULT_BAND_COUNT,
    ) {
        Ok(prepared) => prepared,
        Err(err) => {
            bevy::log::warn!(target: DIAGNOSTIC_LOG_TARGET, "panel text unsupported: {err}");
            stats.failed_glyphs += positioned_glyphs.len().max(1);
            stats.atlas_ms = run_start.elapsed().as_secs_f32() * MILLISECONDS_PER_SECOND;
            return (None, stats);
        },
    };

    text_shaping::record_prepared_glyphs(
        &mut stats,
        positioned_glyphs.len(),
        prepared.glyph_count(),
    );
    stats.atlas_ms = run_start.elapsed().as_secs_f32() * MILLISECONDS_PER_SECOND;

    let clip_rect = placement.clip_rect.map(|clip_rect| {
        panel_clip_rect_local(
            Some(clip_rect),
            placement.scale.x,
            placement.scale.y,
            placement.anchor.x,
            placement.anchor.y,
        )
        .to_array()
    });
    (
        Some(PreparedPanelText {
            prepared,
            style_gate: config.clone(),
            render_mode: config.render_mode(),
            shadow_mode: config.shadow_mode(),
            fill_color: config.color(),
            clip_rect,
            render_only: false,
        }),
        stats,
    )
}

fn panel_layout_anchor(placement: &QuadPlacement) -> Vec2 {
    Vec2::new(
        placement.anchor.x / placement.scale.x - placement.bounds.x,
        placement.anchor.y / placement.scale.y - placement.bounds.y,
    )
}

fn panel_clip_rect_local(
    clip_rect: Option<BoundingBox>,
    scale_x: f32,
    scale_y: f32,
    anchor_x: f32,
    anchor_y: f32,
) -> Vec4 {
    clip_rect.map_or(constants::UNCLIPPED_TEXT_CLIP_RECT, |clip| {
        Vec4::new(
            clip.x.mul_add(scale_x, -anchor_x),
            (clip.y + clip.height).mul_add(-scale_y, anchor_y),
            (clip.x + clip.width).mul_add(scale_x, -anchor_x),
            clip.y.mul_add(-scale_y, anchor_y),
        )
    })
}

/// Stores the prepared run and readiness markers for one label.
///
/// The label's `Resolved<TextAlpha>` is owned by the cascade (seeded by
/// `seed_panel_child_alpha`, kept current by the propagation pass), not
/// computed here.
fn apply_panel_result(
    entity: Entity,
    panel_text: Option<PreparedPanelText>,
    readiness: GlyphReadiness,
    commands: &mut Commands,
) {
    match readiness {
        GlyphReadiness::Ready => {
            let Some(panel_text) = panel_text else {
                return;
            };
            let bounds = drawn_text_bounds(&panel_text.prepared, panel_text.clip_rect);
            commands
                .entity(entity)
                .insert(panel_text)
                .insert(AwaitingReady);
            if let Some(bounds) = bounds {
                commands.entity(entity).insert(bounds);
            } else {
                commands.entity(entity).remove::<DrawnTextBounds>();
            }
        },
        GlyphReadiness::Invisible | GlyphReadiness::Failed => {
            clear_panel_text_output(entity, commands);
        },
        GlyphReadiness::Idle => {},
    }
}

/// Unites exactly the clipped quads emitted into the text batch for this run.
fn drawn_text_bounds(
    prepared: &PreparedTextRun,
    clip_rect: Option<[f32; 4]>,
) -> Option<DrawnTextBounds> {
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    let mut has_quad = false;
    for glyph in prepared.glyphs() {
        let Some(quad) = text::glyph_quad_extents(*glyph, 1.0, clip_rect) else {
            continue;
        };
        minimum = minimum.min(Vec3::new(quad.left, quad.bottom, 0.0));
        maximum = maximum.max(Vec3::new(quad.right, quad.top, 0.0));
        has_quad = true;
    }
    has_quad.then(|| DrawnTextBounds::new(Aabb::from_min_max(minimum, maximum)))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests fail loudly when their glyph fixture is missing"
)]
mod bounds_tests {
    use super::*;

    #[test]
    fn drawn_bounds_follow_the_prepared_glyph_quads() {
        const TOLERANCE: f32 = 1.0e-5;

        let registry = FontRegistry::new().expect("embedded fonts");
        let placement = QuadPlacement {
            bounds:    BoundingBox {
                x:      0.0,
                y:      0.0,
                width:  200.0,
                height: 40.0,
            },
            scale:     Vec2::splat(0.008),
            anchor:    Vec2::ZERO,
            clip_rect: None,
        };
        let (prepared, stats) = build_panel_text(
            "Info",
            &TextStyle::new(28.0).for_shaping(Anchor::Center),
            &placement,
            &mut GlyphCache::default(),
            &registry,
            &TextShapingContext::default(),
            &ShapedTextCache::default(),
            PanelGlyphSource::Plain,
        );
        assert_eq!(stats.failed_glyphs, 0);
        let prepared = prepared.expect("prepared text");
        let bounds = drawn_text_bounds(&prepared.prepared, prepared.clip_rect)
            .expect("visible glyph bounds")
            .local_aabb();
        let minimum = bounds.min();
        let maximum = bounds.max();
        let quads = prepared
            .prepared
            .glyphs()
            .iter()
            .filter_map(|glyph| text::glyph_quad_extents(*glyph, 1.0, prepared.clip_rect))
            .collect::<Vec<_>>();
        assert!(!quads.is_empty());
        for quad in &quads {
            assert!(quad.left >= minimum.x - TOLERANCE && quad.right <= maximum.x + TOLERANCE);
            assert!(quad.bottom >= minimum.y - TOLERANCE && quad.top <= maximum.y + TOLERANCE);
        }
        assert!(
            quads
                .iter()
                .any(|quad| (quad.left - minimum.x).abs() <= TOLERANCE)
        );
        assert!(
            quads
                .iter()
                .any(|quad| (quad.right - maximum.x).abs() <= TOLERANCE)
        );
        assert!(
            quads
                .iter()
                .any(|quad| (quad.bottom - minimum.y).abs() <= TOLERANCE)
        );
        assert!(
            quads
                .iter()
                .any(|quad| (quad.top - maximum.y).abs() <= TOLERANCE)
        );
    }
}
