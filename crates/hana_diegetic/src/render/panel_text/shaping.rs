use std::collections::HashSet;
use std::time::Instant;

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
use bevy::prelude::With;

use super::PanelTextLayout;
use super::PreparedPanelText;
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
use crate::render::world_text::TextContent;
use crate::text::FontRegistry;
use crate::text::GlyphCache;

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
            )>,
        ),
    >,
    mut texts: Query<(
        Ref<TextContent>,
        Ref<TextStyle>,
        Ref<PanelTextLayout>,
        &ChildOf,
        Option<&mut PreparedPanelText>,
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
        let Ok((world_text, style, panel_text_child, child_of, prepared)) = texts.get_mut(entity)
        else {
            continue;
        };

        if world_text.text().is_empty() {
            clear_panel_text_output(entity, &mut commands);
            continue;
        }

        let config = style.for_shaping(Anchor::Center);
        if let Some(mut prepared) = prepared
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
        .remove::<(PreparedPanelText, AwaitingReady)>();
}

fn build_panel_text(
    text: &str,
    config: &TextStyle,
    placement: &QuadPlacement,
    backend: &mut GlyphCache,
    font_registry: &FontRegistry,
    shaping_cx: &TextShapingContext,
    cache: &ShapedTextCache,
) -> (Option<PreparedPanelText>, TextBuildStats) {
    let mut stats = TextBuildStats {
        texts: 1,
        ..Default::default()
    };
    let layout_start = Instant::now();
    let layout_run =
        text_shaping::shape_text_cached(text, config, font_registry, shaping_cx, cache);
    stats.shape_ms = layout_start.elapsed().as_secs_f32() * MILLISECONDS_PER_SECOND;
    stats.glyphs = layout_run.glyphs.len();
    let positioned_glyphs =
        text_shaping::positioned_glyphs(&layout_run.glyphs, font_registry, &mut stats);
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
            bevy::log::warn!("panel text unsupported: {err}");
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
            em_height: config.size() * placement.scale.y.abs(),
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
            commands
                .entity(entity)
                .insert(panel_text)
                .insert(AwaitingReady);
        },
        GlyphReadiness::Invisible | GlyphReadiness::Failed => {
            clear_panel_text_output(entity, commands);
        },
        GlyphReadiness::Idle => {},
    }
}
