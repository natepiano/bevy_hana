use bevy::prelude::Added;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::ChildOf;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Mesh;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::With;

use super::GlyphMetricVisibility;
use super::OverlayContainer;
use super::TypographyOverlay;
use super::glyph;
use super::metric_lines;
use crate::layout::LineMetricsSnapshot;
use crate::layout::MeasureTextFn;
use crate::layout::ShapedTextCache;
use crate::render::ComputedWorldText;
use crate::text;
use crate::text::FontId;
use crate::text::FontMetrics;
use crate::text::FontRegistry;

/// Shared overlay parameters that every spawn helper threads through.
/// Exists to keep helper argument lists under the "context struct when > 7
/// parameters" style threshold.
pub(super) struct OverlayContext<'w, 's, 'a> {
    pub(super) commands:  &'a mut Commands<'w, 's>,
    pub(super) entity:    Entity,
    pub(super) overlay:   &'a TypographyOverlay,
    pub(super) anchor_y:  f32,
    pub(super) font_size: f32,
    pub(super) scale:     f32,
}

/// Font-level metrics shared by helpers that draw glyph-level or line-level
/// guides. Exists to reduce helper parameter counts.
pub(super) struct FontContext<'a> {
    pub(super) font: &'a FontMetrics,
    pub(super) line: &'a LineMetricsSnapshot,
}

/// Asset store handles for overlay mesh/material spawning. Exists to reduce
/// helper parameter counts.
pub(super) struct OverlayAssets<'a> {
    pub(super) meshes:    &'a mut Assets<Mesh>,
    pub(super) materials: &'a mut Assets<StandardMaterial>,
}

/// Text-shaping services (measurer + cache) passed to helpers that measure
/// overlay labels. Exists to reduce helper parameter counts.
pub(super) struct TextServices<'a> {
    pub(super) measure_text: &'a MeasureTextFn,
    pub(super) cache:        &'a ShapedTextCache,
}

/// Horizontal extents of the glyph run and the uniform arrow column spacing.
pub(super) struct GlyphExtents {
    pub(super) first_left:    f32,
    pub(super) last_right:    f32,
    pub(super) arrow_spacing: f32,
}

pub(in crate::debug) fn build_typography_overlay(
    query: Query<(Entity, &TypographyOverlay, &ComputedWorldText)>,
    text_changed: Query<
        Entity,
        (
            With<TypographyOverlay>,
            Or<(
                Added<TypographyOverlay>,
                Changed<TypographyOverlay>,
                Changed<ComputedWorldText>,
            )>,
        ),
    >,
    containers: Query<(Entity, &ChildOf, Option<&Children>), With<OverlayContainer>>,
    font_registry: Res<FontRegistry>,
    cache: Res<ShapedTextCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut dot_materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let changed_entities: Vec<Entity> = text_changed.iter().collect();
    let measure_text =
        text::create_parley_measurer(font_registry.font_context(), font_registry.family_names());

    for (entity, overlay, computed) in &query {
        if !changed_entities.contains(&entity) {
            continue;
        }
        if computed.glyphs.is_empty() {
            continue;
        }

        let Some(container_entity) = overlay_container_entity(&containers, entity) else {
            continue;
        };

        despawn_overlay_children(&mut commands, &containers, container_entity);

        let font_id = FontId(computed.font_id);
        let Some(font) = font_registry.font(font_id) else {
            continue;
        };

        // `ComputedWorldText` already carries the panel child's render scale,
        // point-space font size, anchor, and line metrics, so the overlay draws
        // in the same coordinates as the rendered glyphs without re-deriving any
        // of them. `font_size` is in layout points; `scale` converts to meters.
        let font_metrics = font.metrics(computed.font_size);

        let mut ctx = OverlayContext {
            commands: &mut commands,
            entity: container_entity,
            overlay,
            anchor_y: computed.anchor_y,
            font_size: computed.font_size,
            scale: computed.scale,
        };
        let font_context = FontContext {
            font: &font_metrics,
            line: &computed.line_metrics,
        };
        let mut assets = OverlayAssets {
            meshes:    &mut meshes,
            materials: &mut dot_materials,
        };
        let text_services = TextServices {
            measure_text: &measure_text,
            cache:        &cache,
        };

        if overlay.font_metrics == GlyphMetricVisibility::Shown {
            metric_lines::spawn_font_metric_guides(
                &mut ctx,
                font.name(),
                &font_context,
                computed,
                &text_services,
                &mut assets,
            );
        }

        if overlay.glyph_metrics == GlyphMetricVisibility::Shown {
            glyph::spawn_glyph_metric_guides(&mut ctx, &font_context, computed, &mut assets);
        }
    }
}

fn overlay_container_entity(
    containers: &Query<(Entity, &ChildOf, Option<&Children>), With<OverlayContainer>>,
    entity: Entity,
) -> Option<Entity> {
    containers.iter().find_map(|(child_entity, child_of, _)| {
        (child_of.parent() == entity).then_some(child_entity)
    })
}

fn despawn_overlay_children(
    commands: &mut Commands,
    containers: &Query<(Entity, &ChildOf, Option<&Children>), With<OverlayContainer>>,
    container_entity: Entity,
) {
    if let Some((_, _, Some(children))) = containers
        .iter()
        .find(|(entity, _, _)| *entity == container_entity)
    {
        for child in children {
            commands.entity(*child).despawn();
        }
    }
}
