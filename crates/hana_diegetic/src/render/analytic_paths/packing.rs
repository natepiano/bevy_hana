use bevy::math::Mat4;
use bevy::math::UVec4;
use bevy::math::Vec2;
use bevy::math::Vec4;
use bevy::render::render_resource::ShaderType;
use hana_kana::ToF32;
use hana_kana::ToU32;
use hana_kana::ToUsize;

use super::Bounds;
use super::ContourRole;
use super::PathContour;
use super::PathOutline;
use super::QuadraticSegment;
use crate::render::material_table::GpuMaterialSlotId;

/// Default number of horizontal bands packed per path.
pub(crate) const DEFAULT_BAND_COUNT: usize = 96;

/// Vertex-stage rule for applying a retained panel-text fit offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum TextFitMode {
    /// Apply the offset only across the small-glyph size band.
    SmallGlyph  = 0,
    /// Apply the offset at every size to align the cap and baseline visually.
    CapBaseline = 1,
}

impl TextFitMode {
    #[must_use]
    pub(crate) const fn shader_value(self) -> u32 { self as u32 }
}

const BAND_OVERLAP_EM_UNITS: f32 = 1.0;
/// Fewest curves a band must hold. Band count is capped at
/// `ceil(curve_count / this)` so a sparse path collapses toward one band and
/// its distance scan reads every curve at any grazing angle, where the
/// on-screen footprint exceeds a thin band's overlap.
const MIN_CURVES_PER_BAND: usize = 256;
const CURVE_DEGENERATE_EPS: f32 = 0.000_000_01;
/// Bow-to-chord ratio below which a segment packs as exactly linear.
///
/// Coordinate scaling rounds start/control/end independently, so a
/// midpoint-control line segment can carry a few-ulp second difference.
/// The shader's winding root `(-b ± sqrt(b² - 4ac)) / 2a` then cancels
/// catastrophically (for a long edge, `b²`'s ulp exceeds `4ac`), losing the
/// crossing entirely. A bow this small is far below one design unit of
/// deviation, so snapping it to zero routes such segments through the exact
/// linear solve with no visible change.
const CURVE_LINEAR_SNAP_RATIO: f32 = 0.000_1;
// Two unsigned 16-bit axis extents span 0..2 em within the old four-byte word.
const ISLAND_EXTENT_UNITS_PER_EM: f32 = 32_767.5;

fn pack_island_extent(extent_em: Vec2) -> u32 {
    let x = (extent_em.x.clamp(0.0, 2.0) * ISLAND_EXTENT_UNITS_PER_EM)
        .round()
        .to_u32();
    let y = (extent_em.y.clamp(0.0, 2.0) * ISLAND_EXTENT_UNITS_PER_EM)
        .round()
        .to_u32();
    x | (y << 16)
}

/// GPU curve record for a quadratic Bezier segment.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct CurveRecord {
    /// Segment start point in `.xy`, control-minus-start in `.zw`.
    pub start_delta:      Vec4,
    /// Quadratic second-difference in `.xy`, segment end point in `.zw`.
    pub curve_end:        Vec4,
    /// Conservative control-point bounds minimum in `.xy`, maximum in `.zw`.
    pub bounds:           Vec4,
    /// Distance-solver coefficients in `.xyz`; `.w` carries the owning
    /// contour's narrowest stroke in design units (per-curve hairline
    /// dilation), 0.0 for undilated contours (text paths).
    pub solver:           Vec4,
    /// Owning contour's resolved hairline fade exponent
    /// (`PathContour::fade_exponent`). Each coverage evaluation fades by the
    /// winning (nearest) curve's exponent, so one merged path can mix fading
    /// and non-fading contours. `0.0` disables fade for this curve.
    pub fade_exponent:    f32,
    /// Isolated contour x/y extents in em, quantized to two u16 values.
    pub island_extent_xy: u32,
    /// Outward normal of this segment's edge line, oriented by the owning
    /// contour's winding. The line branch's convex-corner clip reads it as the
    /// edge half-plane direction; the radial `normalize(point - closest)`
    /// degenerates to the vertex direction past a corner, so it cannot stand in
    /// here. Text counter edges use length 2 to suppress the analytic band
    /// floor inside letter holes; other nondegenerate edges use length 1.
    /// `Vec2::ZERO` routes the shader back to the radial normal.
    pub edge_normal:      Vec2,
}

impl From<&QuadraticSegment> for CurveRecord {
    fn from(segment: &QuadraticSegment) -> Self {
        let control_delta = segment.control - segment.start;
        let mut curve_delta = segment.end - 2.0 * segment.control + segment.start;
        let chord_sq = (segment.end - segment.start).length_squared();
        if curve_delta.length_squared()
            < chord_sq * (CURVE_LINEAR_SNAP_RATIO * CURVE_LINEAR_SNAP_RATIO)
        {
            curve_delta = Vec2::ZERO;
        }
        let curve_norm_sq = curve_delta.length_squared();
        let inverse_curve_norm_sq = if curve_norm_sq >= CURVE_DEGENERATE_EPS {
            curve_norm_sq.recip()
        } else {
            0.0
        };
        Self {
            start_delta:      Vec4::new(
                segment.start.x,
                segment.start.y,
                control_delta.x,
                control_delta.y,
            ),
            curve_end:        Vec4::new(curve_delta.x, curve_delta.y, segment.end.x, segment.end.y),
            bounds:           Vec4::new(
                segment.start.x.min(segment.control.x).min(segment.end.x),
                segment.start.y.min(segment.control.y).min(segment.end.y),
                segment.start.x.max(segment.control.x).max(segment.end.x),
                segment.start.y.max(segment.control.y).max(segment.end.y),
            ),
            solver:           Vec4::new(
                3.0 * control_delta.dot(curve_delta) * inverse_curve_norm_sq,
                2.0 * control_delta.length_squared() * inverse_curve_norm_sq,
                inverse_curve_norm_sq,
                0.0,
            ),
            fade_exponent:    0.0,
            island_extent_xy: 0,
            // Winding is a per-contour property; the packer overwrites this from
            // the owning contour's orientation. A lone segment has no winding, so
            // it keeps the radial-fallback sentinel.
            edge_normal:      Vec2::ZERO,
        }
    }
}

/// GPU band record pointing at a contiguous curve range.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct BandRecord {
    /// First curve record for this band.
    pub start:     u32,
    /// Number of curve records for this band.
    pub count:     u32,
    /// Lower band edge in font design-space units on the banded axis.
    pub range_min: f32,
    /// Upper band edge in font design-space units on the banded axis.
    pub range_max: f32,
}

/// GPU path record for one packed analytic path's bounds and curve-band ranges.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct PackedPathRecord {
    /// Bounds minimum in `.xy`, bounds size in `.zw`, in font design-space units.
    pub bounds_min_size:      Vec4,
    /// Horizontal band start/count in `.xy`, vertical band start/count in `.zw`.
    pub band_range:           UVec4,
    /// Narrowest stroke of the path in design-space units; the shader dilates
    /// the silhouette so that stroke covers at least one screen pixel
    /// (hairline rendering). `0.0` disables — text paths stay undilated.
    pub minimum_feature_size: f32,
    /// Font em in this path's design units; zero for shapes and lines.
    pub text_em_size:         f32,
    /// Midpoint of the figure eight's crossbar, in font design units.
    pub figure_crossbar_mid:  f32,
    /// Nonzero when at least one curve belongs to an isolated ink contour.
    pub has_island:           u32,
}

impl PackedPathRecord {
    /// Creates a packed path record that points into the combined band buffer.
    #[must_use]
    pub(crate) fn new(
        bounds: Bounds,
        horizontal_band_start: u32,
        horizontal_band_count: u32,
        vertical_band_start: u32,
        vertical_band_count: u32,
        minimum_feature_size: f32,
    ) -> Self {
        Self {
            bounds_min_size: Vec4::new(bounds.min.x, bounds.min.y, bounds.width(), bounds.height()),
            band_range: UVec4::new(
                horizontal_band_start,
                horizontal_band_count,
                vertical_band_start,
                vertical_band_count,
            ),
            minimum_feature_size,
            text_em_size: 0.0,
            figure_crossbar_mid: 0.0,
            has_island: 0,
        }
    }

    #[must_use]
    pub(crate) const fn with_text_em_size(mut self, text_em_size: f32) -> Self {
        self.text_em_size = text_em_size;
        self
    }

    #[must_use]
    pub(crate) const fn with_figure_crossbar_mid(mut self, figure_crossbar_mid: f32) -> Self {
        self.figure_crossbar_mid = figure_crossbar_mid;
        self
    }

    #[must_use]
    pub(crate) const fn with_has_island(mut self, has_island: bool) -> Self {
        self.has_island = has_island as u32;
        self
    }
}

/// GPU record for one batched path quad. The vertex-pulling shader expands
/// each record into four corners: positions from `rect_min`/`rect_size` in run
/// layout space, padded path UVs from `uv_min`/`uv_size`, material-sampling UVs
/// from `box_uv_min`/`box_uv_size`, the shared-atlas slot through
/// `packed_path_index`, and the owning run through `render_index`.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct PathQuadRecord {
    /// Quad minimum corner (left, bottom) in run layout space, clipped.
    pub rect_min:           Vec2,
    /// Quad size (width, height) in run layout space.
    pub rect_size:          Vec2,
    /// Padded path UV at the quad's (left, top) corner.
    pub uv_min:             Vec2,
    /// Padded path UV extent from `uv_min` toward (right, bottom).
    pub uv_size:            Vec2,
    /// Material box UV at the quad's top-left corner.
    pub box_uv_min:         Vec2,
    /// Material box UV extent from `box_uv_min` toward (right, bottom).
    pub box_uv_size:        Vec2,
    /// Shared-atlas [`PackedPathRecord`] index.
    pub packed_path_index:  u32,
    /// [`PathRenderRecord`] index within the same batch.
    pub render_index:       u32,
    /// Non-zero flips material box UVs horizontally for this quad.
    pub box_uv_flip_x:      u32,
    /// Sides trimmed by the text clip rect; suppress vertex expansion there.
    pub clip_edges:         u32,
    /// Pixel-fit rule applied by the vertex stage.
    pub text_fit_mode:      u32,
    /// Explicit WGSL alignment before the two-component fit offset.
    pub padding_before_fit: u32,
    /// Retained local-space text fit, held while its view moves.
    pub text_fit_offset:    Vec2,
}

/// GPU record for one text run or panel-shape primitive inside a path batch.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(crate) struct PathRenderRecord {
    /// Shared physical panel plane for stable coplanar OIT ordering.
    pub panel_plane:            Vec4,
    /// Label world matrix (run layout space → world).
    pub transform:              Mat4,
    /// Frame-local material-table row selected by this rendered path record.
    pub material:               GpuMaterialSlotId,
    /// Visible render mode (`RenderMode` as `u32`).
    pub render_mode:            u32,
    /// Clip-space depth nudge in layer units for non-OIT views.
    pub clip_depth_nudge:       f32,
    /// Normalized local paint order used only at equal physical OIT depth.
    pub oit_paint_order:        f32,
    /// Resolved anti-alias mode bits (`AntiAlias::aa_flags`:
    /// `AA_FLAG_SUPERSAMPLE` | `AA_FLAG_BAND`). Per-record so an element-level
    /// AA override never splits a batch or material.
    pub aa_flags:               u32,
    /// Signed coverage transfer for text paths. `0.0` leaves the glyph's
    /// geometric coverage unchanged; positive values make fractional edge
    /// pixels more opaque, and negative values make them thinner. Ignored by
    /// line and panel-shape paths.
    pub text_coverage_bias:     f32,
    /// Vertex-stage reverse-Z band for world tooltip text, before opaque
    /// prepass occlusion. `0` for other paths, `1` for tooltip text.
    pub tooltip_depth_priority: u32,
    /// Explicit WGSL array-stride padding.
    pub padding:                u32,
}

/// One analytic path's packed curve and band data for the shader.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PackedPath {
    bounds:                Bounds,
    curves:                Vec<CurveRecord>,
    bands:                 Vec<BandRecord>,
    horizontal_band_count: u32,
    vertical_band_count:   u32,
}

impl PackedPath {
    /// Path bounds in local design-space units.
    #[must_use]
    pub(crate) const fn bounds(&self) -> Bounds { self.bounds }

    /// Band-packed curve records.
    #[must_use]
    pub(crate) fn curves(&self) -> &[CurveRecord] { &self.curves }

    /// Band records.
    #[must_use]
    pub(crate) fn bands(&self) -> &[BandRecord] { &self.bands }

    /// Number of horizontal bands (first in the band list).
    #[must_use]
    pub(crate) const fn horizontal_band_count(&self) -> u32 { self.horizontal_band_count }

    /// Number of vertical bands (after the horizontal bands).
    #[must_use]
    pub(crate) const fn vertical_band_count(&self) -> u32 { self.vertical_band_count }
}

/// Per-axis band counts and inclusion overlap for packing one path.
#[derive(Clone, Copy, Debug)]
pub(super) struct BandLayout {
    /// Horizontal band count (splits the y extent).
    pub horizontal_band_count: usize,
    /// Vertical band count (splits the x extent).
    pub vertical_band_count:   usize,
    /// Design-unit margin around each band; curves within it are included so
    /// the distance scan near a band edge still reads them.
    pub overlap:               f32,
}

impl BandLayout {
    /// Equal band counts on both axes with the text overlap margin.
    #[must_use]
    pub(super) const fn uniform(band_count: usize) -> Self {
        Self {
            horizontal_band_count: band_count,
            vertical_band_count:   band_count,
            overlap:               BAND_OVERLAP_EM_UNITS,
        }
    }

    /// Per-axis counts sized so each band spans about `target_extent` design
    /// units, with a half-band overlap. Small paths keep one exact band (the
    /// distance scan reads every curve at any zoom); large merged paths split
    /// so the per-fragment curve loop stays short.
    #[must_use]
    pub(super) fn for_extents(bounds: Bounds, target_extent: f32, curve_count: usize) -> Self {
        let curve_cap = band_cap_for_curves(curve_count);
        Self {
            horizontal_band_count: band_count_for_extent(bounds.height(), target_extent)
                .min(curve_cap),
            vertical_band_count:   band_count_for_extent(bounds.width(), target_extent)
                .min(curve_cap),
            overlap:               target_extent * 0.5,
        }
    }
}

fn band_count_for_extent(extent: f32, target_extent: f32) -> usize {
    if target_extent <= 0.0 || !(extent / target_extent).is_finite() {
        return 1;
    }
    (extent / target_extent)
        .ceil()
        .to_usize()
        .clamp(1, DEFAULT_BAND_COUNT)
}

/// Largest band count a path of `curve_count` segments justifies, holding at
/// least [`MIN_CURVES_PER_BAND`] curves per band.
fn band_cap_for_curves(curve_count: usize) -> usize {
    curve_count
        .div_ceil(MIN_CURVES_PER_BAND)
        .clamp(1, DEFAULT_BAND_COUNT)
}

/// Builds along-Y and along-X band data for one quadratic path outline
/// with equal per-axis band counts (the text path mode).
#[must_use]
pub(crate) fn build_packed_path(path: PathOutline, band_count: usize) -> PackedPath {
    build_packed_path_with_layout(path, BandLayout::uniform(band_count))
}

/// Builds along-Y and along-X band data for one quadratic path outline.
#[must_use]
pub(super) fn build_packed_path_with_layout(path: PathOutline, layout: BandLayout) -> PackedPath {
    let horizontal_band_count = layout.horizontal_band_count.max(1);
    let vertical_band_count = layout.vertical_band_count.max(1);
    let mut curves = Vec::new();
    let mut bands = Vec::with_capacity(horizontal_band_count + vertical_band_count);
    let bounds = path.bounds;
    let outer_sign = path
        .contours
        .iter()
        .map(contour_signed_area)
        .max_by(|left, right| left.abs().total_cmp(&right.abs()))
        .unwrap_or(0.0)
        .signum();
    let oriented_segments: Vec<PreparedBandCurve> = path
        .contours
        .iter()
        .flat_map(|contour| {
            let chirality = contour_signed_area(contour).signum();
            contour.segments.iter().map(move |segment| {
                let mut record = CurveRecord::from(segment);
                record.solver.w = contour.minimum_feature_size;
                record.fade_exponent = contour.fade_exponent;
                record.island_extent_xy = match contour.role {
                    ContourRole::Ink => 0,
                    ContourRole::Island { extent_em } => pack_island_extent(extent_em),
                };
                record.edge_normal = segment_outward_normal(segment, chirality)
                    * if contour.minimum_feature_size == 0.0 && chirality * outer_sign < 0.0 {
                        2.0
                    } else {
                        1.0
                    };
                PreparedBandCurve {
                    segment: *segment,
                    record,
                }
            })
        })
        .collect();

    append_bands(
        &oriented_segments,
        bounds.min.y,
        bounds.height(),
        horizontal_band_count,
        layout.overlap,
        BandAxis::AlongY,
        &mut curves,
        &mut bands,
    );
    append_bands(
        &oriented_segments,
        bounds.min.x,
        bounds.width(),
        vertical_band_count,
        layout.overlap,
        BandAxis::AlongX,
        &mut curves,
        &mut bands,
    );

    PackedPath {
        bounds,
        curves,
        bands,
        horizontal_band_count: horizontal_band_count.to_u32(),
        vertical_band_count: vertical_band_count.to_u32(),
    }
}

/// Whether every contour of `path` is a straight-edge polygon (each segment
/// snaps to exactly linear under the same bow-to-chord test packing applies).
/// Straight-edge convex marks (Segment rectangles, Triangle/Square/Diamond
/// caps) render through the half-plane product path; a curved contour (a Circle
/// cap's ellipse arcs) keeps the curve-distance path.
#[must_use]
pub(super) fn path_is_straight_polygon(path: &PathOutline) -> bool {
    path.contours
        .iter()
        .flat_map(|contour| &contour.segments)
        .all(segment_is_linear)
}

fn segment_is_linear(segment: &QuadraticSegment) -> bool {
    let curve_delta = segment.end - 2.0 * segment.control + segment.start;
    let chord_sq = (segment.end - segment.start).length_squared();
    curve_delta.length_squared() < chord_sq * (CURVE_LINEAR_SNAP_RATIO * CURVE_LINEAR_SNAP_RATIO)
}

/// Packs each contour as one convex polygon for the half-plane product path:
/// one band per contour holding its edges in contour order (no axis split, no
/// sort, no overlap margin), each edge carrying its outward half-plane
/// [`CurveRecord::edge_normal`], start point, contour stroke (`solver.w`), and
/// fade exponent. `vertical_band_count` is `0` — the shader reads that as the
/// polygon-mode sentinel and evaluates `analytic_polygon_coverage` instead of
/// the curve-distance scan.
#[must_use]
pub(super) fn build_packed_polygons(path: PathOutline) -> PackedPath {
    let bounds = path.bounds;
    let mut curves = Vec::new();
    let mut bands = Vec::with_capacity(path.contours.len());
    for contour in &path.contours {
        let chirality = contour_signed_area(contour).signum();
        let start = curves.len().to_u32();
        for segment in &contour.segments {
            let normal = segment_outward_normal(segment, chirality);
            let mut record = CurveRecord::from(segment);
            // Per-edge hairline-floor dimension = the contour's perpendicular
            // extent in this edge's outward normal. A stroke's long edge gets
            // the (thin) width, so the floor pads it to one screen pixel; its
            // cap gets the (long) length, so the floor never inflates the line
            // past its ends (a foreshortened cap floor would paint a stray
            // ~1px dot off the receding end).
            record.solver.w = edge_slab_width(contour, segment, normal);
            record.fade_exponent = contour.fade_exponent;
            record.island_extent_xy = match contour.role {
                ContourRole::Ink => 0,
                ContourRole::Island { extent_em } => pack_island_extent(extent_em),
            };
            record.edge_normal = normal;
            curves.push(record);
        }
        let count = curves.len().to_u32() - start;
        if count == 0 {
            continue;
        }
        bands.push(BandRecord {
            start,
            count,
            range_min: 0.0,
            range_max: 0.0,
        });
    }
    PackedPath {
        bounds,
        horizontal_band_count: bands.len().to_u32(),
        vertical_band_count: 0,
        curves,
        bands,
    }
}

#[derive(Clone, Copy)]
enum BandAxis {
    AlongY,
    AlongX,
}

/// One curve prepared once for reuse by every band that contains it.
#[derive(Clone, Copy)]
struct PreparedBandCurve {
    segment: QuadraticSegment,
    record:  CurveRecord,
}

fn append_bands(
    oriented_segments: &[PreparedBandCurve],
    start_position: f32,
    extent: f32,
    band_count: usize,
    overlap: f32,
    band_axis: BandAxis,
    curves: &mut Vec<CurveRecord>,
    bands: &mut Vec<BandRecord>,
) {
    // Filtering a stable axis ordering preserves the previous per-band sort,
    // including the original contour order for equal sort values.
    let mut sorted = oriented_segments.to_vec();
    sorted.sort_by(|left, right| {
        descending_band_sort_value(&right.segment, band_axis)
            .total_cmp(&descending_band_sort_value(&left.segment, band_axis))
    });
    let band_size = extent.max(1.0) / band_count.to_f32();

    for band_index in 0..band_count {
        let band_min = band_size.mul_add(band_index.to_f32(), start_position);
        let band_max = if band_index + 1 == band_count {
            start_position + extent
        } else {
            band_min + band_size
        };
        let start = curves.len().to_u32();
        append_band_curves(
            &sorted,
            band_min - overlap,
            band_max + overlap,
            band_axis,
            curves,
        );
        bands.push(BandRecord {
            start,
            count: curves.len().to_u32() - start,
            range_min: band_min,
            range_max: band_max,
        });
    }
}

fn append_band_curves(
    oriented_segments: &[PreparedBandCurve],
    band_min: f32,
    band_max: f32,
    band_axis: BandAxis,
    curves: &mut Vec<CurveRecord>,
) {
    // Every edge can be the nearest silhouette just outside its y band.
    // The x bands must include horizontal edges too: a thin H crossbar
    // disappears at some pixel phases if only vertical edges are scanned.
    curves.extend(
        oriented_segments
            .iter()
            .filter(|banded| overlaps_band(&banded.segment, band_min, band_max, band_axis))
            .map(|banded| banded.record),
    );
}

/// Signed area of the contour's start-point polygon. The sign gives the
/// winding orientation (positive = counter-clockwise) that orients each
/// segment's outward edge normal.
fn contour_signed_area(contour: &PathContour) -> f32 {
    let segments = &contour.segments;
    let mut area = 0.0;
    for index in 0..segments.len() {
        let current = segments[index].start;
        let next = segments[(index + 1) % segments.len()].start;
        area += current.x.mul_add(next.y, -(next.x * current.y));
    }
    area * 0.5
}

/// Outward unit normal of a segment's edge line given the contour's winding
/// chirality (`+1` counter-clockwise, `-1` clockwise). A counter-clockwise
/// contour's outward direction is the chord's right perpendicular. Returns
/// `Vec2::ZERO` for a degenerate (zero-length) chord so the shader keeps the
/// radial fallback.
fn segment_outward_normal(segment: &QuadraticSegment, chirality: f32) -> Vec2 {
    let chord = segment.end - segment.start;
    let length = chord.length();
    if length <= CURVE_DEGENERATE_EPS {
        return Vec2::ZERO;
    }
    Vec2::new(chord.y, -chord.x) / length * chirality
}

/// Perpendicular extent of `contour` in `normal`'s direction: the dimension the
/// hairline floor pads for the edge with that outward normal. For a convex
/// contour the edge's start sits on the supporting face, so this is the full
/// slab thickness between that edge's line and the opposite vertex. Returns the
/// contour stroke for a degenerate (zero-normal) edge.
fn edge_slab_width(contour: &PathContour, segment: &QuadraticSegment, normal: Vec2) -> f32 {
    if normal == Vec2::ZERO {
        return contour.minimum_feature_size;
    }
    let edge_support = normal.dot(segment.start);
    let min_support = contour
        .segments
        .iter()
        .map(|other| normal.dot(other.start))
        .fold(f32::INFINITY, f32::min);
    (edge_support - min_support).max(0.0)
}

fn overlaps_band(segment: &QuadraticSegment, band_min: f32, band_max: f32, axis: BandAxis) -> bool {
    // Axis-parallel edges (e.g. a horizontal edge in an along-Y band) are kept:
    // they add 0 to winding (`curve_winding` returns 0 when the scanline is parallel)
    // but DO carry distance, which the signed-distance field needs. Dropping them
    // leaves the field with no distance to those edges, so it saturates to
    // ±edge_width near them and the screen-space AA band balloons at grazing angles.
    let segment_min = segment_axis_min(segment, axis);
    let segment_max = segment_axis_max(segment, axis);
    segment_min <= band_max && segment_max >= band_min
}

const fn segment_axis_min(segment: &QuadraticSegment, axis: BandAxis) -> f32 {
    match axis {
        BandAxis::AlongY => segment.start.y.min(segment.control.y).min(segment.end.y),
        BandAxis::AlongX => segment.start.x.min(segment.control.x).min(segment.end.x),
    }
}

const fn segment_axis_max(segment: &QuadraticSegment, axis: BandAxis) -> f32 {
    match axis {
        BandAxis::AlongY => segment.start.y.max(segment.control.y).max(segment.end.y),
        BandAxis::AlongX => segment.start.x.max(segment.control.x).max(segment.end.x),
    }
}

const fn descending_band_sort_value(segment: &QuadraticSegment, axis: BandAxis) -> f32 {
    match axis {
        BandAxis::AlongY => segment.start.x.max(segment.control.x).max(segment.end.x),
        BandAxis::AlongX => segment.start.y.max(segment.control.y).max(segment.end.y),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should fail loudly when encase encoding breaks"
)]
mod tests {
    use bevy::render::render_resource::ShaderSize;
    use bevy::render::render_resource::encase::StorageBuffer;

    // GPU-layout assertions against the std430 sizes the shaders index by — the
    // WGSL mirror structs in `analytic_path_vertex_pull.wesl` are declared at these
    // strides.
    // `ShaderSize` measures the encase layout, not the Rust layout.
    const _: () = assert!(CurveRecord::SHADER_SIZE.get() == 80);
    const _: () = assert!(PackedPathRecord::SHADER_SIZE.get() == 48);
    const _: () = assert!(PathQuadRecord::SHADER_SIZE.get() == 80);
    const _: () = assert!(PathRenderRecord::SHADER_SIZE.get() == 112);

    use super::*;
    use crate::render::material_table::MaterialSlotId;

    fn glyph_record(seed: f32, packed_path_index: u32, render_index: u32) -> PathQuadRecord {
        PathQuadRecord {
            rect_min: Vec2::new(seed, seed + 0.5),
            rect_size: Vec2::new(seed + 1.0, seed + 1.5),
            uv_min: Vec2::new(-0.0625, -0.0625),
            uv_size: Vec2::new(1.125, 1.125),
            box_uv_min: Vec2::ZERO,
            box_uv_size: Vec2::ONE,
            packed_path_index,
            render_index,
            box_uv_flip_x: 0,
            clip_edges: 0,
            text_fit_mode: TextFitMode::SmallGlyph.shader_value(),
            padding_before_fit: 0,
            text_fit_offset: Vec2::ZERO,
        }
    }

    fn run_record(seed: f32) -> PathRenderRecord {
        PathRenderRecord {
            panel_plane:            Vec4::new(seed, 1.0, -2.0, 3.0),
            transform:              Mat4::from_translation(bevy::math::Vec3::new(
                seed,
                -seed,
                seed * 2.0,
            )),
            material:               GpuMaterialSlotId::from(
                MaterialSlotId::try_from(seed.to_bits()).expect("seed bits are valid slot ids"),
            ),
            render_mode:            1,
            clip_depth_nudge:       seed,
            oit_paint_order:        -seed,
            aa_flags:               3,
            text_coverage_bias:     seed * 0.25,
            padding:                0,
            tooltip_depth_priority: 1,
        }
    }

    #[test]
    fn glyph_instance_records_round_trip_through_encase_at_80_byte_stride() {
        let records = vec![glyph_record(1.0, 7, 0), glyph_record(-2.0, 11, 1)];
        let mut encoded = StorageBuffer::new(Vec::<u8>::new());
        encoded.write(&records).expect("records should encode");
        assert_eq!(
            encoded.as_ref().len(),
            160,
            "two records at an 80-byte stride"
        );

        assert_eq!(
            bytemuck::cast_slice::<PathQuadRecord, u8>(&records),
            encoded.as_ref()
        );
        assert_eq!(std::mem::offset_of!(PathQuadRecord, text_fit_offset), 72);
        assert_storage_layout(&records[0], 80);
        let mut decoded: Vec<PathQuadRecord> = Vec::new();
        StorageBuffer::new(encoded.as_ref().clone())
            .read(&mut decoded)
            .expect("records should decode");
        assert_eq!(decoded, records);
    }

    #[test]
    fn run_records_round_trip_through_encase_at_112_byte_stride() {
        let records = vec![run_record(0.0), run_record(3.5)];
        let mut encoded = StorageBuffer::new(Vec::<u8>::new());
        encoded.write(&records).expect("records should encode");
        assert_eq!(
            encoded.as_ref().len(),
            224,
            "two records at a 112-byte stride"
        );

        assert_eq!(
            bytemuck::cast_slice::<PathRenderRecord, u8>(&records),
            encoded.as_ref()
        );
        assert_eq!(std::mem::offset_of!(PathRenderRecord, material), 80);
        assert_storage_layout(&records[1], 112);
        let mut decoded: Vec<PathRenderRecord> = Vec::new();
        StorageBuffer::new(encoded.as_ref().clone())
            .read(&mut decoded)
            .expect("records should decode");
        assert_eq!(decoded, records);
    }
    #[test]
    fn curve_record_direct_bytes_match_encase_storage() {
        let mut record = CurveRecord::from(&QuadraticSegment {
            start:   Vec2::new(-3.0, 1.0),
            control: Vec2::new(2.0, 8.0),
            end:     Vec2::new(9.0, -2.0),
        });
        record.solver.w = 0.25;
        record.fade_exponent = 0.75;
        record.island_extent_xy = 0x1234_5678;
        record.edge_normal = Vec2::new(-0.5, 0.25);
        assert_storage_layout(&record, 80);
    }

    #[test]
    fn band_record_direct_bytes_match_encase_storage() {
        assert_storage_layout(
            &BandRecord {
                start:     7,
                count:     13,
                range_min: -2.0,
                range_max: 12.5,
            },
            16,
        );
    }

    #[test]
    fn packed_path_record_direct_bytes_match_encase_storage() {
        let record = PackedPathRecord::new(
            Bounds {
                min: Vec2::new(-1.0, -2.0),
                max: Vec2::new(11.0, 17.0),
            },
            3,
            7,
            10,
            5,
            0.25,
        )
        .with_text_em_size(1000.0)
        .with_figure_crossbar_mid(450.0)
        .with_has_island(true);
        assert_storage_layout(&record, 48);
    }

    fn assert_storage_layout<
        T: bytemuck::Pod + ShaderType + bevy::render::render_resource::encase::internal::WriteInto,
    >(
        record: &T,
        stride: usize,
    ) {
        let encoded =
            crate::render::encase_storage_bytes(record).expect("canonical record encodes");
        assert_eq!(std::mem::size_of::<T>(), stride);
        assert_eq!(bytemuck::bytes_of(record), encoded.as_slice());
    }

    #[test]
    fn prepared_curve_bands_match_per_band_conversion_and_stable_sort() {
        let segments = vec![
            QuadraticSegment {
                start:   Vec2::ZERO,
                control: Vec2::new(4.0, 12.0),
                end:     Vec2::new(8.0, 0.0),
            },
            QuadraticSegment {
                start:   Vec2::new(8.0, 0.0),
                control: Vec2::new(4.0, 0.0),
                end:     Vec2::ZERO,
            },
            QuadraticSegment {
                start:   Vec2::new(8.0, 4.0),
                control: Vec2::new(8.0, 7.0),
                end:     Vec2::new(8.0, 10.0),
            },
        ];
        let path = PathOutline {
            bounds:   Bounds {
                min: Vec2::ZERO,
                max: Vec2::new(8.0, 12.0),
            },
            contours: vec![PathContour {
                segments,
                minimum_feature_size: 0.3,
                fade_exponent: 0.7,
                role: ContourRole::Island {
                    extent_em: Vec2::new(0.2, 0.4),
                },
            }],
        };
        for count in [0, 1, 8, 96] {
            let packed = build_packed_path(path.clone(), count);
            let count = count.max(1);
            let contour = &path.contours[0];
            let chirality = contour_signed_area(contour).signum();
            let mut expected_curves = Vec::new();
            let mut expected_bands = Vec::new();
            for (axis, start, extent) in
                [(BandAxis::AlongY, 0.0, 12.0), (BandAxis::AlongX, 0.0, 8.0)]
            {
                let size = extent / count.to_f32();
                for index in 0..count {
                    let min = size.mul_add(index.to_f32(), start);
                    let max = if index + 1 == count {
                        start + extent
                    } else {
                        min + size
                    };
                    let first = expected_curves.len().to_u32();
                    let mut segments: Vec<_> = contour
                        .segments
                        .iter()
                        .filter(|segment| {
                            overlaps_band(
                                segment,
                                min - BAND_OVERLAP_EM_UNITS,
                                max + BAND_OVERLAP_EM_UNITS,
                                axis,
                            )
                        })
                        .collect();
                    segments.sort_by(|left, right| {
                        descending_band_sort_value(right, axis)
                            .total_cmp(&descending_band_sort_value(left, axis))
                    });
                    expected_curves.extend(segments.into_iter().map(|segment| {
                        let mut record = CurveRecord::from(segment);
                        record.solver.w = contour.minimum_feature_size;
                        record.fade_exponent = contour.fade_exponent;
                        record.island_extent_xy = pack_island_extent(Vec2::new(0.2, 0.4));
                        record.edge_normal = segment_outward_normal(segment, chirality);
                        record
                    }));
                    expected_bands.push(BandRecord {
                        start:     first,
                        count:     expected_curves.len().to_u32() - first,
                        range_min: min,
                        range_max: max,
                    });
                }
            }
            assert_eq!(packed.curves(), expected_curves);
            assert_eq!(packed.bands(), expected_bands);
        }
    }
}
