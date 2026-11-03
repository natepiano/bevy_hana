#[cfg(test)]
use bevy::math::Vec2;

#[cfg(test)]
use crate::render::PackedPathRecord;
#[cfg(test)]
use crate::render::PathQuadRecord;
use crate::text::slug::runtime::GlyphInstance;

/// Vertex-stage size and expansion measured from a packed glyph record.
#[cfg(test)]
pub(crate) struct VertexTextMargin {
    pub pixels_per_em: f32,
    pub margin_px:     f32,
}

#[cfg(test)]
impl PathQuadRecord {
    /// Mirrors the text branch of `pull_vertex`; the inputs are the GPU record
    /// fields and each projected quad axis' layout units per screen pixel.
    pub(crate) fn vertex_text_margin(
        self,
        packed_path: PackedPathRecord,
        projected_units_per_pixel: Vec2,
    ) -> VertexTextMargin {
        const LINE_AA_EPSILON: f32 = 0.000_001;
        const TEXT_AA_MARGIN_PX: f32 = 0.5;
        const TEXT_LOW_END_PX_PER_EM: f32 = 2.9;
        const TEXT_LOW_FADE_SPAN_PX_PER_EM: f32 = 2.0;
        let units_per_pixel = projected_units_per_pixel.x.max(projected_units_per_pixel.y);
        let design_span_x = self.uv_size.x * packed_path.bounds_min_size.z;
        let layout_em =
            packed_path.text_em_size * self.rect_size.x / design_span_x.max(LINE_AA_EPSILON);
        let pixels_per_em = layout_em / units_per_pixel.max(LINE_AA_EPSILON);
        let size_strength = ((pixels_per_em - TEXT_LOW_END_PX_PER_EM)
            / TEXT_LOW_FADE_SPAN_PX_PER_EM)
            .clamp(0.0, 1.0);
        let margin_px = TEXT_AA_MARGIN_PX * size_strength;
        VertexTextMargin {
            pixels_per_em,
            margin_px,
        }
    }
}

const GLYPH_PADDING_DESIGN_UNITS: f32 = 16.0;

const CLIP_LEFT: u32 = 1;
const CLIP_RIGHT: u32 = 2;
const CLIP_BOTTOM: u32 = 4;
const CLIP_TOP: u32 = 8;

/// Padded, optionally clipped quad rect and UVs for one glyph instance — the
/// extents `build_glyph_records` packs into `PathQuadRecord`s.
pub(crate) struct GlyphQuadExtents {
    pub(crate) left:       f32,
    pub(crate) right:      f32,
    pub(crate) bottom:     f32,
    pub(crate) top:        f32,
    source_left:           f32,
    source_right:          f32,
    source_bottom:         f32,
    source_top:            f32,
    pub(crate) uv_left:    f32,
    pub(crate) uv_right:   f32,
    pub(crate) uv_top:     f32,
    pub(crate) uv_bottom:  f32,
    pub(crate) clip_edges: u32,
}

impl GlyphQuadExtents {
    fn new(left: f32, right: f32, bottom: f32, top: f32, padding_x: f32, padding_y: f32) -> Self {
        let width = (right - left).max(f32::EPSILON);
        let height = (top - bottom).max(f32::EPSILON);
        let uv_padding_x = padding_x / width;
        let uv_padding_y = padding_y / height;

        Self {
            left:          left - padding_x,
            right:         right + padding_x,
            bottom:        bottom - padding_y,
            top:           top + padding_y,
            source_left:   left,
            source_right:  right,
            source_bottom: bottom,
            source_top:    top,
            uv_left:       -uv_padding_x,
            uv_right:      1.0 + uv_padding_x,
            uv_top:        -uv_padding_y,
            uv_bottom:     1.0 + uv_padding_y,
            clip_edges:    0,
        }
    }

    fn clipped(mut self, clip_rect: Option<[f32; 4]>) -> Option<Self> {
        let Some([clip_left, clip_bottom, clip_right, clip_top]) = clip_rect else {
            return Some(self);
        };
        if self.right <= clip_left
            || self.left >= clip_right
            || self.top <= clip_bottom
            || self.bottom >= clip_top
        {
            return None;
        }

        if self.left <= clip_left {
            self.clip_edges |= CLIP_LEFT;
        }
        if self.right >= clip_right {
            self.clip_edges |= CLIP_RIGHT;
        }
        if self.bottom <= clip_bottom {
            self.clip_edges |= CLIP_BOTTOM;
        }
        if self.top >= clip_top {
            self.clip_edges |= CLIP_TOP;
        }
        self.left = self.left.max(clip_left);
        self.right = self.right.min(clip_right);
        self.bottom = self.bottom.max(clip_bottom);
        self.top = self.top.min(clip_top);

        let width = self.source_right - self.source_left;
        let height = self.source_top - self.source_bottom;
        if width <= f32::EPSILON || height <= f32::EPSILON {
            return None;
        }
        self.uv_left = (self.left - self.source_left) / width;
        self.uv_right = (self.right - self.source_left) / width;
        self.uv_top = (self.source_top - self.top) / height;
        self.uv_bottom = (self.source_top - self.bottom) / height;
        Some(self)
    }
}

/// Padded quad rect and UVs for one glyph instance, clipped to `clip_rect`.
/// Returns `None` when clipping removes the whole quad.
pub(crate) fn glyph_quad_extents(
    glyph: GlyphInstance,
    scale: f32,
    clip_rect: Option<[f32; 4]>,
) -> Option<GlyphQuadExtents> {
    let bounds = glyph.bounds();
    let bounds_scale = glyph.bounds_scale();
    let origin = glyph.origin();
    let left = bounds.min.x.mul_add(bounds_scale.x, origin.x) * scale;
    let right = bounds.max.x.mul_add(bounds_scale.x, origin.x) * scale;
    let bottom = bounds.min.y.mul_add(bounds_scale.y, origin.y) * scale;
    let top = bounds.max.y.mul_add(bounds_scale.y, origin.y) * scale;
    let padding_x = GLYPH_PADDING_DESIGN_UNITS * bounds_scale.x.abs() * scale;
    let padding_y = GLYPH_PADDING_DESIGN_UNITS * bounds_scale.y.abs() * scale;
    GlyphQuadExtents::new(left, right, bottom, top, padding_x, padding_y).clipped(clip_rect)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should fail loudly when fixture glyph data is missing"
)]
mod tests {
    use super::*;
    use crate::text::slug::runtime::BuiltTextRun;
    use crate::text::slug::support;

    const FONT_DATA: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf");

    pub(super) fn fixture_glyph_for_clip_test() -> GlyphInstance { fixture_glyph("H") }

    fn fixture_glyph(text: &str) -> GlyphInstance {
        let (preview, _) = support::fixture_run_with_cache(FONT_DATA, 7, text);
        *preview
            .run
            .glyphs()
            .first()
            .expect("fixture run should hold at least one glyph")
    }

    /// X/Y extent of the unclipped fixture glyph quad:
    /// `(min_x, max_x, min_y, max_y)`.
    fn quad_extent(preview: &BuiltTextRun) -> (f32, f32, f32, f32) {
        let glyph = preview
            .run
            .glyphs()
            .first()
            .expect("fixture run should hold at least one glyph");
        let extents =
            glyph_quad_extents(*glyph, 1.0, None).expect("unclipped glyph should produce a quad");
        (extents.left, extents.right, extents.bottom, extents.top)
    }

    #[test]
    fn clip_rect_trims_quad_and_moves_uvs_into_the_glyph() {
        let (preview, _) = support::fixture_run_with_cache(FONT_DATA, 7, "H");
        let (min_x, max_x, min_y, max_y) = quad_extent(&preview);
        let clip_x = f32::midpoint(min_x, max_x);

        let glyph = fixture_glyph("H");
        let extents = glyph_quad_extents(glyph, 1.0, Some([clip_x, min_y, max_x, max_y]))
            .expect("half-clipped glyph should keep a quad");

        assert!(
            extents.left >= clip_x,
            "clipped quad should not extend left of the clip rect"
        );
        assert!(
            extents.uv_left > 0.0,
            "left-trimmed glyph should move UVs into the glyph quad"
        );
    }

    #[test]
    fn fully_clipped_glyph_produces_no_quad() {
        let (preview, _) = support::fixture_run_with_cache(FONT_DATA, 7, "H");
        let (_, max_x, min_y, max_y) = quad_extent(&preview);

        let glyph = fixture_glyph("H");
        let extents =
            glyph_quad_extents(glyph, 1.0, Some([max_x + 1.0, min_y, max_x + 2.0, max_y]));

        assert!(
            extents.is_none(),
            "a quad entirely outside the clip rect is dropped"
        );
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixture assertions must fail loudly")]
mod vertex_margin_regressions {
    use super::*;

    #[test]
    fn subpixel_bar_quad_reaches_a_pixel_center_at_every_phase() {
        let vertex_shader =
            include_str!("../../../render/analytic_paths/analytic_path_vertex_pull.wesl");
        assert!(vertex_shader.contains("const TEXT_AA_MARGIN_PX: f32 = TEXT_BAND_OUTER_HALF_PX;"));
        assert!(vertex_shader.contains("line_aa_margin_local(axis_x, base_clip, margin_px)"));
        assert!(vertex_shader.contains("const TEXT_BAND_OUTER_HALF_PX: f32 = 0.5;"));
        assert!(vertex_shader.contains("const TEXT_LOW_FULL_PX_PER_EM: f32 = 4.9;"));
        let margin_px = 0.5;
        let cpu_padding_px = GLYPH_PADDING_DESIGN_UNITS * 6.0 / 1000.0;
        for left in [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] {
            let right = left + 0.4;
            let min = left - cpu_padding_px - margin_px;
            let max = right + cpu_padding_px + margin_px;
            assert!(
                (0.0 >= min && 0.0 <= max) || (1.0 >= min && 1.0 <= max),
                "phase {left}"
            );
        }
    }

    #[test]
    fn clipped_glyph_sides_stay_at_the_clip_boundary() {
        let shader = include_str!("../../../render/analytic_paths/analytic_path_vertex_pull.wesl");
        assert!(shader.contains("(record.clip_edges & edge_x) != 0u"));
        assert!(shader.contains("(record.clip_edges & edge_y) != 0u"));
        let glyph = super::tests::fixture_glyph_for_clip_test();
        let unclipped = glyph_quad_extents(glyph, 1.0, None).expect("glyph quad");
        let clip_left = f32::midpoint(unclipped.left, unclipped.right);
        let clipped = glyph_quad_extents(
            glyph,
            1.0,
            Some([
                clip_left,
                unclipped.bottom - 1.0,
                unclipped.right + 1.0,
                unclipped.top + 1.0,
            ]),
        )
        .expect("clipped glyph");
        assert_eq!(clipped.left, clip_left);
        assert_ne!(clipped.clip_edges & CLIP_LEFT, 0);
        assert_eq!(clipped.clip_edges & CLIP_RIGHT, 0);
    }
}
