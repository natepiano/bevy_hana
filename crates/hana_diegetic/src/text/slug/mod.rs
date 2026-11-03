//! Slug analytic Bézier glyph renderer.
//!
//! Renders glyphs from their quadratic Bézier contours with per-pixel
//! analytic coverage. This module owns text-specific shaping, glyph lookup,
//! and outline extraction; `render` owns the shared renderer, material, shader,
//! packing, and batch store.

mod glyph;
mod render;
mod runtime;
#[cfg(test)]
mod support;

use bevy::prelude::App;
use bevy::prelude::Plugin;
pub(in crate::text) use glyph::font_glyph_id_has_visible_outline;
pub(in crate::text) use glyph::load_glyph_by_id_from_face;
pub(crate) use render::GlyphQuadExtents;
pub(crate) use render::glyph_quad_extents;
pub(crate) use runtime::GlyphCache;
pub(crate) use runtime::PositionedGlyph;
pub(crate) use runtime::PreparedTextRun;
pub(crate) use runtime::RunStorageKey;

#[cfg(test)]
use crate::render::PathExtendedMaterial;

pub(super) struct SlugPlugin;

#[cfg(test)]
pub(super) const fn text_material_oit_paint_order(material: &PathExtendedMaterial) -> f32 {
    render::text_material_oit_paint_order(material)
}

impl Plugin for SlugPlugin {
    fn build(&self, app: &mut App) { app.init_resource::<GlyphCache>(); }
}
