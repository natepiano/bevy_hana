//! Emits [`ComputedWorldText`] for the typography debug overlay.
//!
//! [`DiegeticText`](crate::DiegeticText) labels are one-element panels: the
//! root carries the [`DiegeticText`](crate::DiegeticText) marker and the user's
//! [`TypographyOverlay`], while the actual run is stored on a
//! [`TextContent`](super::TextContent) child with a
//! [`PanelTextLayout`](crate::render::PanelTextLayout).
//!
//! This system reads that child's layout — the same `points_to_world` scale and
//! anchor the panel-text mesh was built from — recomputes the per-glyph ink boxes
//! in those exact coordinates, and writes the result back onto the overlay root.
//! Sourcing the scale from the child (rather than re-deriving it from the root's
//! font unit) is what keeps the overlay boxes and metric lines on the rendered
//! glyphs.

use std::collections::HashSet;

use bevy::prelude::Added;
use bevy::prelude::Changed;
use bevy::prelude::ChildOf;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::RelationshipTarget;
use bevy::prelude::Res;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use ttf_parser::Face;

use super::ComputedGlyphMetrics;
use super::ComputedWorldText;
use super::TextContent;
use crate::TypographyOverlay;
use crate::layout::Anchor;
use crate::layout::ShapedTextCache;
use crate::layout::TextStyle;
use crate::render::PanelTextLayout;
use crate::render::text_shaping;
use crate::render::text_shaping::TextBuildStats;
use crate::render::text_shaping::TextShapingContext;
use crate::text;
use crate::text::FontRegistry;
use crate::text::PositionedGlyph;

/// Writes [`ComputedWorldText`] onto every [`TypographyOverlay`] root whose
/// panel-text child changed (or all of them when a font registration lands).
pub(crate) fn emit_computed_world_text(
    overlay_roots: Query<(Entity, &Children), With<TypographyOverlay>>,
    added_overlays: Query<Entity, Added<TypographyOverlay>>,
    text_children: Query<(&TextContent, &TextStyle, &PanelTextLayout), With<TextContent>>,
    changed_children: Query<
        &ChildOf,
        (
            With<TextContent>,
            Or<(
                Changed<PanelTextLayout>,
                Changed<TextContent>,
                Changed<TextStyle>,
            )>,
        ),
    >,
    font_registry: Res<FontRegistry>,
    shaping_cx: Res<TextShapingContext>,
    cache: Res<ShapedTextCache>,
    mut commands: Commands,
) {
    // A just-registered font can change the run, so a font registration recomputes
    // every overlay rather than only the children flagged changed this frame.
    let recompute_all = font_registry.is_changed();
    let changed_roots: HashSet<Entity> = changed_children.iter().map(ChildOf::parent).collect();

    for (root, children) in &overlay_roots {
        if !recompute_all && !added_overlays.contains(root) && !changed_roots.contains(&root) {
            continue;
        }
        let Some((text, style, layout)) = children
            .iter()
            .find_map(|child| text_children.get(child).ok())
        else {
            continue;
        };
        if text.text().is_empty() {
            continue;
        }
        if let Some(computed) = compute_world_text(
            text.text(),
            style,
            layout,
            &font_registry,
            &shaping_cx,
            &cache,
        ) {
            commands.entity(root).insert(computed);
        }
    }
}

/// Runs text shaping in the panel child's point space and converts the run to the
/// world-unit [`ComputedWorldText`] the overlay reads, or `None` if a glyph
/// failed to resolve or the run produced no lines.
fn compute_world_text(
    text: &str,
    style: &TextStyle,
    layout: &PanelTextLayout,
    font_registry: &FontRegistry,
    shaping_cx: &TextShapingContext,
    cache: &ShapedTextCache,
) -> Option<ComputedWorldText> {
    // Match `shape_panel_text_children`, which lays out the child style at
    // `Anchor::Center`.
    let config = style.for_shaping(Anchor::Center);
    let mut stats = TextBuildStats::default();
    let run = text_shaping::shape_text_cached(text, &config, font_registry, shaping_cx, cache);
    let positioned = text_shaping::positioned_glyphs(&run.glyphs, font_registry, &mut stats);
    if stats.failed_glyphs > 0 {
        return None;
    }
    let line_metrics = run.line_metrics.first().copied()?;

    let font_size = config.size();
    let scale = layout.scale_x;
    // The panel positions glyphs about this layout-local anchor — see
    // `panel_layout_anchor` in the panel-text shaping path.
    let anchor = Vec2::new(
        layout.anchor_x / layout.scale_x - layout.bounds.x,
        layout.anchor_y / layout.scale_y - layout.bounds.y,
    );
    let glyphs = overlay_glyph_metrics(&positioned, font_size, anchor, scale);

    Some(ComputedWorldText {
        anchor_y: anchor.y,
        scale,
        font_size,
        font_id: style.font_id(),
        line_metrics,
        glyphs,
    })
}

/// Builds the per-glyph world-unit metrics the overlay draws boxes, origin dots,
/// and advancement arrows from, using the panel's anchor and `points_to_world`
/// scale so each box lands on its rendered glyph.
fn overlay_glyph_metrics(
    positioned: &[PositionedGlyph<'_>],
    font_size: f32,
    anchor: Vec2,
    scale: f32,
) -> Vec<ComputedGlyphMetrics> {
    let mut glyphs = Vec::with_capacity(positioned.len());
    for positioned_glyph in positioned {
        let glyph = positioned_glyph.glyph;
        if let Some(rect) = ink_rect(
            positioned_glyph.font.data(),
            positioned_glyph.collection_index,
            glyph.id,
            font_size,
            glyph.x,
            glyph.baseline + glyph.y,
            anchor,
            scale,
        ) {
            let origin_x = (glyph.x - anchor.x) * scale;
            glyphs.push(ComputedGlyphMetrics {
                rect,
                origin_x: origin_x.min(rect[0]),
                advance_x: glyph.advance * scale,
            });
        }
    }
    glyphs
}

/// Ink bounding box of one glyph as `[x, y, width, height]` in world units, or
/// `None` if the font face or glyph bbox is unavailable.
fn ink_rect(
    font_data: &[u8],
    collection_index: u32,
    glyph_id: u16,
    font_size: f32,
    glyph_x: f32,
    baseline_offset: f32,
    anchor: Vec2,
    scale: f32,
) -> Option<[f32; 4]> {
    let face = Face::parse(font_data, collection_index).ok()?;
    let ink = text::glyph_ink_extents(&face, glyph_id)?;
    let font_scale = font_size / ink.units_per_em;

    let ink_width = (ink.max_x - ink.min_x) * font_scale;
    let ink_height = (ink.max_y - ink.min_y) * font_scale;
    let ink_x = ink.min_x.mul_add(font_scale, glyph_x) - anchor.x;
    let ink_top = ink.max_y.mul_add(-font_scale, baseline_offset) - anchor.y;

    Some([
        ink_x * scale,
        -ink_top * scale,
        ink_width * scale,
        ink_height * scale,
    ])
}
