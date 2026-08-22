mod run_data;

pub(crate) use run_data::GlyphQuadExtents;
pub(crate) use run_data::glyph_quad_extents;

#[cfg(test)]
use crate::render;
#[cfg(test)]
use crate::render::PathExtendedMaterial;

#[cfg(test)]
pub(super) const fn text_material_oit_depth_offset(material: &PathExtendedMaterial) -> f32 {
    render::path_material_oit_depth_offset(material)
}
