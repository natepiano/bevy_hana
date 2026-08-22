//! Coverage-model tests for the slug text anti-aliasing path.
//!
//! These tests do NOT run the real shader — WGSL executes on the GPU and cannot
//! be called from a Rust test. They exercise three things instead:
//!
//! - the real glyph **packer** (`build_packed_glyph`, the band/curve records, `DEFAULT_BAND_COUNT`)
//!   — production code; if it changes, these numbers move,
//! - a Rust **debugging model of the `analytic_path.wgsl` coverage math** (`Probe`) — both the
//!   `aa_band` path (`signed_distance` / `band_coverage`, inside-negative) and the Off/Supersample
//!   `distance_coverage` path (inside-positive smoothstep), including per-curve hairline dilation
//!   and the hairline fade factor. The `distance_coverage` model covers the Off/Supersample sign
//!   convention, which an `aa_band`-only model would miss,
//! - an independent brute-force **ground truth** (`GroundTruth`) that counts inside/outside over
//!   the footprint with no bands at all.
//!
//! Each test compares the band model against the brute-force ground truth and
//! locks in a fact the shader design rests on:
//! 1. a single coverage sample over-covers a sharp convex corner under a grazing footprint (the
//!    "wing"), and the over-coverage is inherent to the single-sample model — it persists even with
//!    exact distance and band,
//! 2. the anisotropic stride (`aniso_shader_fix`, `per_band = d_minor + d_major/N`) brings corner
//!    coverage back to ground truth, and beats a fixed 4-tap supersample — which is why the fix
//!    strides along the foreshortened axis rather than sampling a fixed grid,
//! 3. the stride does not alias a straight edge.
//!
//! `Probe` is a lab bench for coverage problems, not a lockstep checksum for
//! every shader edit. Update it when changing coverage math or when visual
//! debugging is not converging on the GPU. It may intentionally lag shader
//! plumbing changes such as imports, comments, entry-point gates, or data-source
//! defines.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    clippy::imprecise_flops,
    reason = "line-for-line CPU mirror of the analytic_path.wgsl coverage math: the int/f32/u32 casts \
              reproduce the shader's f32()/u32() conversions, and mul_add/cbrt are deliberately \
              avoided so the Rust results match the shader's operation order exactly"
)]

use bevy::math::Vec2;
use bevy::math::Vec4;

use super::outline::Contour;
use super::outline::Glyph;
use crate::render::BandRecord;
use crate::render::Bounds;
use crate::render::CurveRecord;
use crate::render::DEFAULT_BAND_COUNT;
use crate::render::PackedPath;
use crate::render::PackedPathRecord;
use crate::render::QuadraticSegment;

const ROOT_EPSILON: f32 = 0.000_01;
const EDGE_FILTER_WIDTH: f32 = 1.2;
const SQRT_3_OVER_2: f32 = 0.866_025_4;

// Test configuration: a ~40px-tall glyph viewed at a fixed extreme anisotropy.
const PX: f32 = 700.0 / 40.0;
const ANISO: f32 = 12.0;
const GT_SAMPLES: u32 = 24;
// Mirror the shader caps in analytic_path.wgsl: the line/circle path
// (corner_wing_reach) and the text band path (aniso_shader_fix) move with their
// shader counterparts.
const MAX_ANISO_SAMPLES: u32 = 16;
const MAX_ANISO_SAMPLES_TEXT: u32 = 64;
/// Footprint orientations whose worst over-coverage point lands on the apex
/// (the acute convex corner). 90° is excluded — there the grazing footprint
/// hits a base corner, a different case.
const GRAZING_ANGLES: [f32; 5] = [0.0, 30.0, 60.0, 120.0, 150.0];

// Golden thresholds (the slack, not the answer — the answer is measured each run
// by the brute-force ground truth). Chosen with margin over the observed values.
const WING_MIN_OVERCOVERAGE: f32 = 0.40;
const MODEL_CAUSE_MIN_OVERCOVERAGE: f32 = 0.35;
const FIX_MAX_ERROR: f32 = 0.08;
const SUPER4_MIN_OVERCOVERAGE: f32 = 0.20;
const FIX_IMPROVEMENT_OVER_SUPER4: f32 = 0.15;
const EDGE_FIX_MAX_ERROR: f32 = 0.06;
// Center-fragment dilation holds the convex-corner cap near the hairline radius
// (~0.9px observed) at any grazing angle; per-sample dilation balloons to 3-8px.
const CORNER_WING_MAX_REACH: f32 = 1.5;

// ---- shader-math debugging model -------------------------------------------

struct Probe {
    record: PackedPathRecord,
    curves: Vec<CurveRecord>,
    bands:  Vec<BandRecord>,
}

fn xy(v: Vec4) -> Vec2 { Vec2::new(v.x, v.y) }
fn zw(v: Vec4) -> Vec2 { Vec2::new(v.z, v.w) }

impl Probe {
    fn bounds_min(&self) -> Vec2 { xy(self.record.bounds_min_size) }
    fn bounds_size(&self) -> Vec2 { zw(self.record.bounds_min_size) }

    fn along_y_band_index(&self, point: Vec2) -> u32 {
        let bmin = self.bounds_min();
        let bsize = self.bounds_size();
        let band_count = self.record.band_range.y;
        let normalized_y = ((point.y - bmin.y) / bsize.y.max(ROOT_EPSILON)).clamp(0.0, 0.999_999);
        ((normalized_y * band_count as f32) as u32).min(band_count - 1)
    }

    fn along_x_band_index(&self, point: Vec2) -> u32 {
        let bmin = self.bounds_min();
        let bsize = self.bounds_size();
        let band_count = self.record.band_range.w;
        let normalized_x = ((point.x - bmin.x) / bsize.x.max(ROOT_EPSILON)).clamp(0.0, 0.999_999);
        ((normalized_x * band_count as f32) as u32).min(band_count - 1)
    }

    fn outside_bounds(&self, point: Vec2) -> bool {
        let bmin = self.bounds_min();
        let bmax = bmin + self.bounds_size();
        point.x < bmin.x || point.x > bmax.x || point.y < bmin.y || point.y > bmax.y
    }

    /// Per-lane winding at `point` (`.0` = exempt lane, `.1` = faded lane);
    /// mirrors the shader's `lane_winding_at`.
    fn lane_winding_at(&self, point: Vec2) -> (i32, i32) {
        if self.outside_bounds(point) {
            return (0, 0);
        }
        let band =
            &self.bands[(self.record.band_range.x + self.along_y_band_index(point)) as usize];
        let mut winding = (0, 0);
        for offset in 0..band.count {
            let curve = self.curves[(band.start + offset) as usize];
            let value = curve_winding(curve, point);
            if curve.fade_exponent > 0.0 {
                winding.1 += value;
            } else {
                winding.0 += value;
            }
        }
        winding
    }

    /// Mirrors the shader's `lanes_any_outside_neighbor` (`.0` = exempt lane,
    /// `.1` = whole-path union; the lane windings sum to the union's).
    fn lanes_any_outside_neighbor(&self, point: Vec2, edge_width: f32) -> (bool, bool) {
        let right = self.lane_winding_at(point + Vec2::new(edge_width, 0.0));
        let left = self.lane_winding_at(point - Vec2::new(edge_width, 0.0));
        let up = self.lane_winding_at(point + Vec2::new(0.0, edge_width));
        let down = self.lane_winding_at(point - Vec2::new(0.0, edge_width));
        (
            right.0 == 0 || left.0 == 0 || up.0 == 0 || down.0 == 0,
            right.0 + right.1 == 0
                || left.0 + left.1 == 0
                || up.0 + up.1 == 0
                || down.0 + down.1 == 0,
        )
    }

    /// Mirrors the shader's `lanes_no_outside_neighbor` (`.0` = exempt lane,
    /// `.1` = whole-path union).
    fn lanes_no_outside_neighbor(
        &self,
        terms: &CoverageTerms,
        union_terms: &LaneTerms,
        point: Vec2,
        edge_width: f32,
    ) -> (bool, bool) {
        if lane_needs_neighbor_test(&terms.exempt, edge_width)
            || lane_needs_neighbor_test(union_terms, edge_width)
        {
            let any_outside = self.lanes_any_outside_neighbor(point, edge_width);
            return (!any_outside.0, !any_outside.1);
        }
        (false, false)
    }

    /// `along_y_coverage_terms` + `nearest_along_x_curve` fused: per-lane
    /// winding, dilation-adjusted distance to the lane's nearest silhouette,
    /// and the winning dilation/exponent (mirrors the shader's
    /// `CoverageTerms`).
    fn coverage_terms(
        &self,
        point: Vec2,
        scan_width_sq: f32,
        hairline_target: f32,
    ) -> CoverageTerms {
        let include_winding = !self.outside_bounds(point);
        let mut terms = CoverageTerms::default();

        let hband =
            &self.bands[(self.record.band_range.x + self.along_y_band_index(point)) as usize];
        for offset in 0..hband.count {
            let curve = self.curves[(hband.start + offset) as usize];
            if include_winding {
                let value = curve_winding(curve, point);
                if curve.fade_exponent > 0.0 {
                    terms.faded.winding += value;
                } else {
                    terms.exempt.winding += value;
                }
            }
            if curve_bounds_distance_sq(point, curve) <= scan_width_sq {
                accumulate_nearest(&mut terms, point, curve, hairline_target);
            }
        }

        let vband =
            &self.bands[(self.record.band_range.z + self.along_x_band_index(point)) as usize];
        for offset in 0..vband.count {
            let curve = self.curves[(vband.start + offset) as usize];
            if curve_bounds_distance_sq(point, curve) <= scan_width_sq {
                accumulate_nearest(&mut terms, point, curve, hairline_target);
            }
        }
        terms
    }

    /// Verbatim port of `signed_distance_sample` (the `aa_band` feeder):
    /// signed distances (`.x` exempt lane, `.y` whole-path union) plus the
    /// faded lane's winning dilation and fade exponent.
    fn signed_distance_sample(
        &self,
        point: Vec2,
        scan_width_sq: f32,
        hairline_target: f32,
    ) -> (Vec2, f32, f32) {
        let scan_width = scan_width_sq.sqrt();
        let terms = self.coverage_terms(point, scan_width_sq, hairline_target);
        let union_terms = union_lane(&terms);
        let no_outside = self.lanes_no_outside_neighbor(&terms, &union_terms, point, scan_width);
        (
            Vec2::new(
                lane_signed_distance(&terms.exempt, scan_width, no_outside.0),
                lane_signed_distance(&union_terms, scan_width, no_outside.1),
            ),
            terms.faded.dilation,
            terms.fade_exponent,
        )
    }

    fn signed_distance(&self, point: Vec2, scan_width_sq: f32, hairline_target: f32) -> Vec2 {
        self.signed_distance_sample(point, scan_width_sq, hairline_target)
            .0
    }

    /// Verbatim port of `distance_coverage` (the Off/Supersample path):
    /// inside-POSITIVE smoothstep ramps with per-curve dilation, combined as
    /// mix(exempt, union, fade factor) — at factor 1 the path renders as the
    /// unfaded single-winding union, at factor 0 only the exempt
    /// sub-geometry remains.
    fn distance_coverage(
        &self,
        point: Vec2,
        pixel: Vec2,
        dilation_max: f32,
        hairline_target: f32,
    ) -> f32 {
        let edge_width = (pixel.x.max(pixel.y) * EDGE_FILTER_WIDTH).max(ROOT_EPSILON);
        let scan_width = edge_width + dilation_max;
        let scan_width_sq = scan_width * scan_width;
        let terms = self.coverage_terms(point, scan_width_sq, hairline_target);
        let union_terms = union_lane(&terms);
        let no_outside = self.lanes_no_outside_neighbor(&terms, &union_terms, point, edge_width);

        let exempt = lane_coverage(&terms.exempt, edge_width, no_outside.0);
        let union_coverage = lane_coverage(&union_terms, edge_width, no_outside.1);
        let fade = hairline_fade_factor(terms.faded.dilation, hairline_target, terms.fade_exponent);
        exempt + (union_coverage - exempt) * fade
    }

    /// Full `aa_band` `render_coverage` for a chosen point + footprint (dx, dy).
    /// Returns (`single_sample`, supersampled). Exempt/union bands and signed
    /// distances combine per sample as mix(exempt, union, fade); these probes
    /// carry no fade exponent on their curves, so the fade factor is 1.
    fn aa_band_coverage(&self, point: Vec2, dx: Vec2, dy: Vec2) -> (f32, f32) {
        let pixel = Vec2::new(
            (dx.x.abs() + dy.x.abs()).max(ROOT_EPSILON),
            (dx.y.abs() + dy.y.abs()).max(ROOT_EPSILON),
        );
        let edge_width = (pixel.x.max(pixel.y) * EDGE_FILTER_WIDTH).max(ROOT_EPSILON);
        let edge_width_sq = edge_width * edge_width;

        // band = fwidth(signed_distance) modeled as forward differences across
        // the 2x2 quad, per lane.
        let sd_center = self.signed_distance(point, edge_width_sq, 0.0);
        let band = ((self.signed_distance(point + dx, edge_width_sq, 0.0) - sd_center).abs()
            + (self.signed_distance(point + dy, edge_width_sq, 0.0) - sd_center).abs())
        .max(Vec2::splat(ROOT_EPSILON));

        let single = lanes_band_coverage(sd_center, band);

        let mut sum = 0.0;
        for (a, b) in [
            (0.375, 0.125),
            (-0.125, 0.375),
            (-0.375, -0.125),
            (0.125, -0.375),
        ] {
            sum += lanes_band_coverage(
                self.signed_distance(point + a * dx + b * dy, edge_width_sq, 0.0),
                band,
            );
        }
        (single, sum * 0.25)
    }

    /// Mirrors the shader's anisotropic stride: stride N samples along the longer
    /// footprint axis, `per_band = d_minor + d_major/N`. The Lipschitz clamp on
    /// the finite differences removes the spike when a sample crosses into a band
    /// where a horizontal/vertical edge isn't visible (returns a far curve).
    fn aniso_shader_fix(&self, point: Vec2, dx: Vec2, dy: Vec2, max_n: u32) -> f32 {
        let pixel = Vec2::new(
            (dx.x.abs() + dy.x.abs()).max(ROOT_EPSILON),
            (dx.y.abs() + dy.y.abs()).max(ROOT_EPSILON),
        );
        let edge_width = (pixel.x.max(pixel.y) * EDGE_FILTER_WIDTH).max(ROOT_EPSILON);
        let edge_width_sq = edge_width * edge_width;
        let sd_c = self.signed_distance(point, edge_width_sq, 0.0);

        let (major, minor, major_len, minor_len) = if dx.length() >= dy.length() {
            (dx, dy, dx.length(), dy.length())
        } else {
            (dy, dx, dy.length(), dx.length())
        };
        let n = ((major_len / minor_len.max(ROOT_EPSILON)).ceil() as u32).clamp(1, max_n);
        // sd is 1-Lipschitz, so |Δsd| over a step can't exceed the step length.
        let d_major = (self.signed_distance(point + major, edge_width_sq, 0.0) - sd_c)
            .abs()
            .min(Vec2::splat(major_len));
        let d_minor = (self.signed_distance(point + minor, edge_width_sq, 0.0) - sd_c)
            .abs()
            .min(Vec2::splat(minor_len));
        let per_band = (d_minor + d_major / n as f32).max(Vec2::splat(ROOT_EPSILON));

        let mut sum = 0.0;
        for i in 0..n {
            let s = (i as f32 + 0.5) / n as f32 - 0.5;
            sum += lanes_band_coverage(
                self.signed_distance(point + s * major, edge_width_sq, 0.0),
                per_band,
            );
        }
        sum / n as f32
    }
}

fn band_coverage(sd: f32, band: f32) -> f32 { (0.5 - sd / band).clamp(0.0, 1.0) }

/// The shader's mix(exempt, union, fade) band combine at fade factor 1 —
/// these probes carry no fade exponent, so the result is the union band
/// coverage (`sd.y`).
fn lanes_band_coverage(sd: Vec2, band: Vec2) -> f32 { band_coverage(sd.y, band.y) }

/// Mirror of the shader's `LaneTerms`.
struct LaneTerms {
    winding:  i32,
    adjusted: f32,
    dilation: f32,
}

impl Default for LaneTerms {
    fn default() -> Self {
        Self {
            winding:  0,
            adjusted: 1_000_000.0,
            dilation: 0.0,
        }
    }
}

/// Mirror of the shader's two-lane `CoverageTerms`: exempt lane =
/// `fade_exponent == 0` curves, faded lane = `fade_exponent > 0` curves.
#[derive(Default)]
struct CoverageTerms {
    exempt:        LaneTerms,
    faded:         LaneTerms,
    fade_exponent: f32,
}

/// Mirror of the shader's `lane_needs_neighbor_test`.
fn lane_needs_neighbor_test(lane: &LaneTerms, edge_width: f32) -> bool {
    lane.winding != 0 && lane.adjusted <= edge_width
}

/// Mirror of the shader's `union_lane`: the whole path's terms rebuilt from
/// the two lanes — windings add (the lanes partition the path's contours),
/// the nearest-silhouette race is the cross-lane min with the winner's
/// dilation.
fn union_lane(terms: &CoverageTerms) -> LaneTerms {
    let faded_wins = terms.faded.adjusted < terms.exempt.adjusted;
    LaneTerms {
        winding:  terms.exempt.winding + terms.faded.winding,
        adjusted: terms.exempt.adjusted.min(terms.faded.adjusted),
        dilation: if faded_wins {
            terms.faded.dilation
        } else {
            terms.exempt.dilation
        },
    }
}

/// Mirror of the shader's `lane_coverage`.
fn lane_coverage(lane: &LaneTerms, edge_width: f32, no_outside: bool) -> f32 {
    let inside = lane.winding != 0;
    if lane.adjusted > edge_width {
        return if inside { 1.0 } else { 0.0 };
    }
    if inside && no_outside {
        return 1.0;
    }
    let signed_distance = if inside {
        lane.adjusted + 2.0 * lane.dilation
    } else {
        -lane.adjusted
    };
    smoothstep(-edge_width, edge_width, signed_distance)
}

/// Mirror of the shader's `lane_signed_distance`.
fn lane_signed_distance(lane: &LaneTerms, scan_width: f32, no_outside: bool) -> f32 {
    let inside = lane.winding != 0;
    if lane.adjusted > scan_width {
        return if inside { -scan_width } else { scan_width };
    }
    if inside && no_outside {
        return -scan_width;
    }
    if inside {
        -(lane.adjusted + 2.0 * lane.dilation)
    } else {
        lane.adjusted
    }
}

/// Mirror of the shader's `curve_dilation`.
fn curve_dilation(curve: CurveRecord, hairline_target: f32) -> f32 {
    if curve.solver.w <= 0.0 {
        return 0.0;
    }
    ((hairline_target - curve.solver.w) * 0.5).max(0.0)
}

/// Mirror of the shader's `accumulate_nearest`.
fn accumulate_nearest(
    terms: &mut CoverageTerms,
    point: Vec2,
    curve: CurveRecord,
    hairline_target: f32,
) {
    let dilation = curve_dilation(curve, hairline_target);
    let adjusted = curve_distance_sq(point, curve).sqrt() - dilation;
    if curve.fade_exponent > 0.0 {
        if adjusted < terms.faded.adjusted {
            terms.faded.adjusted = adjusted;
            terms.faded.dilation = dilation;
            terms.fade_exponent = curve.fade_exponent;
        }
    } else if adjusted < terms.exempt.adjusted {
        terms.exempt.adjusted = adjusted;
        terms.exempt.dilation = dilation;
    }
}

/// Mirror of the shader's `hairline_fade_factor`.
fn hairline_fade_factor(dilation: f32, hairline_target: f32, fade_exponent: f32) -> f32 {
    if fade_exponent <= 0.0 || dilation <= 0.0 || hairline_target <= 0.0 {
        return 1.0;
    }
    let natural = (hairline_target - 2.0 * dilation).max(0.0);
    (natural / hairline_target).powf(fade_exponent)
}

/// WGSL `smoothstep`.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn curve_distance_sq(point: Vec2, curve: CurveRecord) -> f32 {
    exact_quadratic_distance_sq(
        curve,
        point,
        xy(curve.start_delta),
        zw(curve.start_delta),
        xy(curve.curve_end),
        zw(curve.curve_end),
    )
}

fn curve_bounds_distance_sq(point: Vec2, curve: CurveRecord) -> f32 {
    let nearest = point.clamp(xy(curve.bounds), zw(curve.bounds));
    (point - nearest).length_squared()
}

fn point_line_distance_sq(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let edge = end - start;
    let len_sq = edge.length_squared().max(ROOT_EPSILON);
    let t = ((point - start).dot(edge) / len_sq).clamp(0.0, 1.0);
    (point - (start + edge * t)).length_squared()
}

fn exact_quadratic_distance_sq(
    curve: CurveRecord,
    point: Vec2,
    start: Vec2,
    control_delta: Vec2,
    curve_delta: Vec2,
    end: Vec2,
) -> f32 {
    let pv = point - start;
    let mut best_sq = pv.length_squared();
    best_sq = best_sq.min((end - point).length_squared());

    let inv = curve.solver.z;
    if inv > 0.0 {
        let roots = solve_cubic_normed(
            curve.solver.x,
            curve.solver.y - curve_delta.dot(pv) * inv,
            -control_delta.dot(pv) * inv,
        );
        for &t in &roots {
            if (0.0..=1.0).contains(&t) {
                let closest = start + control_delta * (2.0 * t) + curve_delta * (t * t);
                best_sq = best_sq.min((closest - point).length_squared());
            }
        }
        best_sq
    } else {
        best_sq.min(point_line_distance_sq(point, start, end))
    }
}

fn cbrt_signed(x: f32) -> f32 {
    if x < 0.0 {
        -(-x).powf(1.0 / 3.0)
    } else {
        x.powf(1.0 / 3.0)
    }
}

/// Returns 1 or 3 valid roots; unused slots are NaN (filtered by the [0,1] gate).
/// Solves the monic cubic `t³ + quadratic·t² + linear·t + constant = 0`.
fn solve_cubic_normed(quadratic: f32, linear: f32, constant: f32) -> [f32; 3] {
    let quadratic_sq = quadratic * quadratic;
    let quad_reduced = (1.0 / 9.0) * (quadratic_sq - 3.0 * linear);
    let cubic_reduced =
        (1.0 / 54.0) * (quadratic * (2.0 * quadratic_sq - 9.0 * linear) + 27.0 * constant);
    let cubic_reduced_sq = cubic_reduced * cubic_reduced;
    let quad_reduced_cubed = quad_reduced * quad_reduced * quad_reduced;
    let root_shift = quadratic * (1.0 / 3.0);
    if cubic_reduced_sq < quad_reduced_cubed {
        let cos_argument = (cubic_reduced / quad_reduced_cubed.sqrt()).clamp(-1.0, 1.0);
        let theta = cos_argument.acos();
        let trig_scale = -2.0 * quad_reduced.sqrt();
        let cos_t3 = (theta / 3.0).cos();
        let sin_t3 = (theta / 3.0).sin();
        [
            trig_scale * cos_t3 - root_shift,
            trig_scale * (-0.5 * cos_t3 - SQRT_3_OVER_2 * sin_t3) - root_shift,
            trig_scale * (-0.5 * cos_t3 + SQRT_3_OVER_2 * sin_t3) - root_shift,
        ]
    } else {
        let sign = if cubic_reduced < 0.0 { -1.0 } else { 1.0 };
        let cube_root = sign
            * cbrt_signed(cubic_reduced.abs() + (cubic_reduced_sq - quad_reduced_cubed).sqrt());
        let paired_root = if cube_root == 0.0 {
            0.0
        } else {
            quad_reduced / cube_root
        };
        [(cube_root + paired_root) - root_shift, f32::NAN, f32::NAN]
    }
}

fn winding_for_t(curve: CurveRecord, point: Vec2, t: f32) -> i32 {
    let dy = 2.0 * (curve.start_delta.w + curve.curve_end.y * t);
    if dy.abs() < ROOT_EPSILON {
        return 0;
    }
    // Half-open in y, not t: upward crossings count on t ∈ [0, 1), downward
    // on t ∈ (0, 1], so a ray exactly through a segment join sees both
    // adjoining segments agree (the shader's grazing-row parity rule).
    let upward = dy > 0.0;
    if upward && !(0.0..1.0).contains(&t) {
        return 0;
    }
    if !upward && (t <= 0.0 || t > 1.0) {
        return 0;
    }
    let curve_x = curve.start_delta.x + 2.0 * curve.start_delta.z * t + curve.curve_end.x * t * t;
    if curve_x <= point.x {
        return 0;
    }
    if upward { 1 } else { -1 }
}

fn curve_winding(curve: CurveRecord, point: Vec2) -> i32 {
    let a = curve.curve_end.y;
    let b = 2.0 * curve.start_delta.w;
    let c = curve.start_delta.y - point.y;
    if a.abs() < ROOT_EPSILON {
        if b.abs() < ROOT_EPSILON {
            return 0;
        }
        return winding_for_t(curve, point, -c / b);
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return 0;
    }
    let root = discriminant.sqrt();
    winding_for_t(curve, point, (-b - root) / (2.0 * a))
        + winding_for_t(curve, point, (-b + root) / (2.0 * a))
}

// ---- ground truth (independent of bands; full segment scan) ----------------

struct GroundTruth {
    segments: Vec<QuadraticSegment>,
}

impl GroundTruth {
    /// Global non-zero winding via the same crossing rule, over all segments.
    fn inside(&self, point: Vec2) -> bool {
        let mut winding = 0;
        for segment in &self.segments {
            winding += curve_winding(CurveRecord::from(segment), point);
        }
        winding != 0
    }

    /// True signed distance + outward unit normal at the nearest point.
    /// Synthetic glyph is all line segments, so nearest-point is exact here.
    fn signed_distance_and_normal(&self, point: Vec2) -> (f32, Vec2) {
        let mut best_sq = f32::MAX;
        let mut nearest = point;
        for segment in &self.segments {
            let edge = segment.end - segment.start;
            let len_sq = edge.length_squared().max(ROOT_EPSILON);
            let t = ((point - segment.start).dot(edge) / len_sq).clamp(0.0, 1.0);
            let foot = segment.start + edge * t;
            let d_sq = (point - foot).length_squared();
            if d_sq < best_sq {
                best_sq = d_sq;
                nearest = foot;
            }
        }
        let distance = best_sq.sqrt();
        let inside = self.inside(point);
        let outward = point - nearest;
        let normal = if outward.length() > ROOT_EPSILON {
            outward.normalize()
        } else {
            Vec2::new(1.0, 0.0)
        };
        (if inside { -distance } else { distance }, normal)
    }

    /// Reference coverage: fraction of the footprint parallelogram inside the
    /// fill, by stratified supersampling. `point + s*dx + t*dy`, s,t in [-.5,.5].
    fn coverage(&self, point: Vec2, dx: Vec2, dy: Vec2, n: u32) -> f32 {
        let mut inside = 0u32;
        for i in 0..n {
            for j in 0..n {
                let s = (i as f32 + 0.5) / n as f32 - 0.5;
                let t = (j as f32 + 0.5) / n as f32 - 0.5;
                if self.inside(point + s * dx + t * dy) {
                    inside += 1;
                }
            }
        }
        inside as f32 / (n * n) as f32
    }

    fn signed_distance(&self, point: Vec2) -> f32 { self.signed_distance_and_normal(point).0 }

    /// Footprint extent along the silhouette normal: the band a straight-edge
    /// model SHOULD use (L1 projection of the two footprint basis vectors).
    fn exact_band(&self, point: Vec2, dx: Vec2, dy: Vec2) -> f32 {
        let (_, normal) = self.signed_distance_and_normal(point);
        (normal.dot(dx)).abs() + (normal.dot(dy)).abs()
    }
}

// ---- synthetic glyph -------------------------------------------------------

fn line(start: Vec2, end: Vec2) -> QuadraticSegment {
    QuadraticSegment {
        start,
        control: start.midpoint(end),
        end,
    }
}

/// A sharp upward triangle: one acute convex corner at the apex.
fn sharp_corner_glyph() -> (Glyph, Vec<QuadraticSegment>) {
    let apex = Vec2::new(500.0, 700.0);
    let right = Vec2::new(580.0, 0.0);
    let left = Vec2::new(420.0, 0.0);
    let segments = vec![line(apex, right), line(right, left), line(left, apex)];
    let glyph = Glyph {
        character: '^',
        id:        0,
        bounds:    Bounds {
            min: Vec2::new(420.0, 0.0),
            max: Vec2::new(580.0, 700.0),
        },
        contours:  vec![Contour {
            segments:      segments.clone(),
            min_feature:   0.0,
            fade_exponent: 0.0,
        }],
    };
    (glyph, segments)
}

/// Anisotropic grazing footprint: long axis at `angle_deg`, ratio `aniso`,
/// base scale `px` design-units per head-on screen pixel.
fn footprint(angle_deg: f32, aniso: f32, px: f32) -> (Vec2, Vec2) {
    let a = angle_deg.to_radians();
    let long = aniso * px * Vec2::new(a.cos(), a.sin());
    let short = px * Vec2::new(-a.sin(), a.cos());
    (long, short)
}

// ---- test harness ----------------------------------------------------------

fn build() -> (Probe, GroundTruth) {
    let (glyph, segments) = sharp_corner_glyph();
    let packed = super::build_packed_glyph(glyph, DEFAULT_BAND_COUNT);
    let band_count = (packed.bands().len() / 2) as u32;
    let probe = Probe {
        record: PackedPathRecord::new(packed.bounds(), 0, band_count, band_count, band_count, 0.0),
        curves: packed.curves().to_vec(),
        bands:  packed.bands().to_vec(),
    };
    let gt = GroundTruth { segments };
    (probe, gt)
}

struct CornerSample {
    gt:          f32,
    single:      f32,
    super4:      f32,
    fix_b:       f32,
    model_cause: f32,
}

/// Scans the glyph bounds (plus footprint margin) and returns the metrics at the
/// pixel where the single-sample band over-covers ground truth the most — the
/// corner where the wing forms.
fn worst_overcoverage(probe: &Probe, gt: &GroundTruth, dx: Vec2, dy: Vec2) -> CornerSample {
    let bmin = probe.bounds_min();
    let bmax = bmin + probe.bounds_size();
    let margin = 2.0 * (dx.length() + dy.length());
    let x0 = bmin.x - margin;
    let x1 = bmax.x + margin;
    let y0 = bmin.y - margin;
    let y1 = bmax.y + margin;
    let cols = 80usize;
    let rows = 40usize;

    let mut best = CornerSample {
        gt:          0.0,
        single:      0.0,
        super4:      0.0,
        fix_b:       0.0,
        model_cause: 0.0,
    };
    let mut max_over = f32::MIN;
    for row in 0..rows {
        let y = y1 + (y0 - y1) * (row as f32 + 0.5) / rows as f32;
        for col in 0..cols {
            let x = x0 + (x1 - x0) * (col as f32 + 0.5) / cols as f32;
            let point = Vec2::new(x, y);
            let reference = gt.coverage(point, dx, dy, GT_SAMPLES);
            let (single, super4) = probe.aa_band_coverage(point, dx, dy);
            if single - reference > max_over {
                max_over = single - reference;
                let sd_true = gt.signed_distance(point);
                let band_true = gt.exact_band(point, dx, dy);
                best = CornerSample {
                    gt: reference,
                    single,
                    super4,
                    fix_b: probe.aniso_shader_fix(point, dx, dy, MAX_ANISO_SAMPLES_TEXT),
                    model_cause: band_coverage(sd_true, band_true),
                };
            }
        }
    }
    best
}

/// A single coverage sample over-covers the sharp convex corner under a grazing
/// footprint, and the over-coverage is inherent to the single-sample model: it
/// persists even when fed exact distance and exact band.
#[test]
fn single_sample_overcovers_convex_corner() {
    let (probe, gt) = build();
    for angle in GRAZING_ANGLES {
        let (dx, dy) = footprint(angle, ANISO, PX);
        let s = worst_overcoverage(&probe, &gt, dx, dy);
        assert!(
            s.single - s.gt >= WING_MIN_OVERCOVERAGE,
            "angle {angle}: single-sample over-coverage {:.3} (single {:.3}, truth {:.3}) below {WING_MIN_OVERCOVERAGE}",
            s.single - s.gt,
            s.single,
            s.gt,
        );
        assert!(
            s.model_cause - s.gt >= MODEL_CAUSE_MIN_OVERCOVERAGE,
            "angle {angle}: even exact distance+band over-covers by {:.3}; the single-sample model is the cause",
            s.model_cause - s.gt,
        );
    }
}

/// The anisotropic stride brings corner coverage back to ground truth, and beats
/// a fixed 4-tap supersample — the data behind striding along the foreshortened
/// axis rather than sampling a fixed sub-pixel grid.
#[test]
fn anisotropic_stride_beats_supersample_at_corner() {
    let (probe, gt) = build();
    for angle in GRAZING_ANGLES {
        let (dx, dy) = footprint(angle, ANISO, PX);
        let s = worst_overcoverage(&probe, &gt, dx, dy);
        let fix_error = (s.fix_b - s.gt).abs();
        assert!(
            fix_error <= FIX_MAX_ERROR,
            "angle {angle}: stride fix off ground truth by {fix_error:.3} (fix {:.3}, truth {:.3})",
            s.fix_b,
            s.gt,
        );
        let super4_over = s.super4 - s.gt;
        assert!(
            super4_over >= SUPER4_MIN_OVERCOVERAGE,
            "angle {angle}: fixed 4-tap supersample over-covers only {super4_over:.3}; expected it to stay well over ground truth at the corner",
        );
        assert!(
            super4_over - fix_error >= FIX_IMPROVEMENT_OVER_SUPER4,
            "angle {angle}: stride fix should beat 4-tap by >= {FIX_IMPROVEMENT_OVER_SUPER4} (4-tap over {super4_over:.3}, fix error {fix_error:.3})",
        );
    }
}

/// The stride tracks ground truth across a straight edge — it does not steepen
/// the ramp into aliasing the way a naive `full_band / N` stride would.
#[test]
fn stride_does_not_alias_straight_edge() {
    let (probe, gt) = build();
    // Horizontal grazing across the triangle's base edge; the monotonic ramp
    // region only (the band-boundary row has a known signed-distance reach spike).
    let (dx, dy) = footprint(0.0, 4.0, PX);
    let x = 500.0;
    for k in 0..=12 {
        let y = -7.5 + 12.5 * (k as f32) / 12.0;
        let point = Vec2::new(x, y);
        let reference = gt.coverage(point, dx, dy, GT_SAMPLES);
        let fix = probe.aniso_shader_fix(point, dx, dy, MAX_ANISO_SAMPLES_TEXT);
        assert!(
            (fix - reference).abs() <= EDGE_FIX_MAX_ERROR,
            "y {y:.1}: stride fix {fix:.3} should track straight-edge ground truth {reference:.3} within {EDGE_FIX_MAX_ERROR}",
        );
    }
}

/// CPU mirror of the line branch (single lane). `dil_mode`: 0 = no dilation,
/// 1 = per-strided-sample dilation (attempt 1, the shader as-is), 2 = dilation
/// sized ONCE from the center fragment's normal and reused across the stride
/// (proposed fix). Returns coverage.
fn line_cov(
    gt: &GroundTruth,
    point: Vec2,
    dx: Vec2,
    dy: Vec2,
    min_feature: f32,
    hairline_min_px: f32,
    dil_mode: u32,
) -> f32 {
    let x_span_len = dx.length();
    let y_span_len = dy.length();
    let (major, minor, major_len, minor_len) = if x_span_len >= y_span_len {
        (dx, dy, x_span_len, y_span_len.max(ROOT_EPSILON))
    } else {
        (dy, dx, y_span_len, x_span_len.max(ROOT_EPSILON))
    };
    let n_samp = (major_len / minor_len)
        .ceil()
        .clamp(1.0, MAX_ANISO_SAMPLES as f32);
    let inv = 1.0 / n_samp;
    let count = n_samp as u32;
    let dilation_of = |normal: Vec2| {
        let band_full = Vec2::new(normal.dot(dx), normal.dot(dy))
            .length()
            .max(ROOT_EPSILON);
        ((hairline_min_px * band_full - min_feature) * 0.5).max(0.0)
    };
    let center_dilation = dilation_of(gt.signed_distance_and_normal(point).1);
    let mut sum = 0.0;
    for i in 0..count {
        let s = (i as f32 + 0.5) * inv - 0.5;
        let (sd, normal) = gt.signed_distance_and_normal(point + s * major);
        let band_strided = Vec2::new(normal.dot(minor), normal.dot(major) * inv)
            .length()
            .max(ROOT_EPSILON);
        let dilation = match dil_mode {
            1 => dilation_of(normal),
            2 => center_dilation,
            _ => 0.0,
        };
        sum += (0.5 - (sd - dilation) / band_strided).clamp(0.0, 1.0);
    }
    sum * inv
}

/// Wing reach: farthest exterior fragment (ground-truth coverage ~0) still
/// covered above 0.3, in screen pixels from the silhouette, for the given
/// dilation mode.
fn corner_wing_reach(gt: &GroundTruth, dx: Vec2, dy: Vec2, mode: u32) -> f32 {
    let hairline_min_px = 1.5f32;
    let min_feature = 8.0f32;
    let margin = 6.0 * (dx.length() + dy.length());
    let bmin = Vec2::new(420.0 - margin, -margin);
    let bmax = Vec2::new(580.0 + margin, 700.0 + margin);
    let (cols, rows) = (120usize, 120usize);
    let mut max_reach = 0.0f32;
    for r in 0..rows {
        let y = bmin.y + (bmax.y - bmin.y) * (r as f32 + 0.5) / rows as f32;
        for c in 0..cols {
            let x = bmin.x + (bmax.x - bmin.x) * (c as f32 + 0.5) / cols as f32;
            let p = Vec2::new(x, y);
            if gt.coverage(p, dx, dy, GT_SAMPLES) > 0.05 {
                continue;
            }
            if line_cov(gt, p, dx, dy, min_feature, hairline_min_px, mode) > 0.3 {
                let (sd, normal) = gt.signed_distance_and_normal(p);
                let band_full = Vec2::new(normal.dot(dx), normal.dot(dy))
                    .length()
                    .max(ROOT_EPSILON);
                max_reach = max_reach.max(sd.abs() / band_full);
            }
        }
    }
    max_reach
}

/// The convex-corner wing on the line branch is the hairline dilation, and
/// sizing it once from the center fragment (the shipping path, mode 2) bounds it
/// to the hairline corner cap instead of letting the stride smear a
/// grazing-inflated per-sample cap (mode 1) into a multi-pixel wing that grows
/// with grazing. Mode 0 (no dilation) confirms the band model carries no
/// over-cover of its own.
#[test]
#[ignore = "slow coverage diagnostic; run when changing analytic line hairline/corner math"]
fn center_dilation_bounds_corner_wing() {
    let (_, segments) = sharp_corner_glyph();
    let gt = GroundTruth { segments };
    for &aniso in &[12.0f32, 24.0, 40.0] {
        for &angle in &[75.0f32, 85.0] {
            let (dx, dy) = footprint(angle, aniso, PX);
            let per_sample = corner_wing_reach(&gt, dx, dy, 1);
            let center = corner_wing_reach(&gt, dx, dy, 2);
            let none = corner_wing_reach(&gt, dx, dy, 0);
            assert!(
                none < 0.05,
                "aniso {aniso} angle {angle}: band model over-covers without dilation by {none:.2}px",
            );
            assert!(
                center <= CORNER_WING_MAX_REACH,
                "aniso {aniso} angle {angle}: center-dilation wing reach {center:.2}px exceeds {CORNER_WING_MAX_REACH}px (per-sample was {per_sample:.2}px)",
            );
            assert!(
                per_sample > center + 0.5,
                "aniso {aniso} angle {angle}: per-sample reach {per_sample:.2}px should exceed center {center:.2}px (the wing the fix removes)",
            );
        }
    }
}

/// Tripwire: hashes `analytic_path.wgsl` and fails when it changes. The [`Probe`]
/// above mirrors the shader's coverage math by hand; this flags that the shader
/// changed so the mirror gets re-checked. It cannot tell whether the change was
/// correct — the shader runs on the GPU. On failure, re-verify [`Probe`], then
/// set `EXPECTED_SHADER_FNV1A` to the printed value.
#[test]
fn shader_mirror_matches_wgsl() {
    const SHADER: &str = include_str!("../../../render/analytic_paths/analytic_path.wgsl");
    const EXPECTED_SHADER_FNV1A: u64 = 0xb7a2_b3f0_8884_b007;
    let actual = fnv1a_64(SHADER.as_bytes());
    assert_eq!(
        actual, EXPECTED_SHADER_FNV1A,
        "shader has changed, make sure to update this test to match the new logic (new hash {actual:#018x})",
    );
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// ---- panel-line tick scale probe -------------------------------------------

/// Builds a probe for an axis-aligned rectangle outline with the given design
/// size, packed exactly like a panel-line tick. `min_feature > 0` marks the
/// rectangle as a hairline-dilating stroke of that design width (the panel
/// line path); `0.0` packs it like a text glyph.
fn rectangle_probe(size: Vec2, band_count: usize, min_feature: f32) -> Probe {
    rectangle_probe_with_fade(size, band_count, min_feature, 0.0)
}

/// `rectangle_probe` with a per-contour fade exponent (the panel-line fade
/// path: every curve carries its contour's resolved exponent).
fn rectangle_probe_with_fade(
    size: Vec2,
    band_count: usize,
    min_feature: f32,
    fade_exponent: f32,
) -> Probe {
    let glyph = Glyph {
        character: '-',
        id:        0,
        bounds:    Bounds {
            min: Vec2::ZERO,
            max: size,
        },
        contours:  vec![rectangle_contour(
            Vec2::ZERO,
            size,
            min_feature,
            fade_exponent,
        )],
    };
    let packed = super::build_packed_glyph(glyph, band_count);
    let band_count = (packed.bands().len() / 2) as u32;
    Probe {
        record: PackedPathRecord::new(
            packed.bounds(),
            0,
            band_count,
            band_count,
            band_count,
            min_feature,
        ),
        curves: packed.curves().to_vec(),
        bands:  packed.bands().to_vec(),
    }
}

/// An axis-aligned rectangle contour from `min` to `min + size` with the
/// given per-contour packing inputs.
fn rectangle_contour(min: Vec2, size: Vec2, min_feature: f32, fade_exponent: f32) -> Contour {
    let corners = [
        min,
        min + Vec2::new(size.x, 0.0),
        min + size,
        min + Vec2::new(0.0, size.y),
    ];
    Contour {
        segments: corners
            .iter()
            .copied()
            .zip(corners.iter().copied().cycle().skip(1))
            .take(corners.len())
            .map(|(start, end)| line(start, end))
            .collect(),
        min_feature,
        fade_exponent,
    }
}

/// A ruler tick packs to ~709x106 design units. With ~18 design units per
/// screen pixel, scan coverage across the long (top) edge and report the AA
/// ramp the shader would produce.
#[test]
fn tick_edge_coverage_ramps_instead_of_stepping() {
    let size = Vec2::new(709.0, 106.0);
    for band_count in [DEFAULT_BAND_COUNT, 1] {
        run_tick_scan(rectangle_probe(size, band_count, 0.0), size, band_count);
    }
}

fn run_tick_scan(probe: Probe, size: Vec2, band_count: usize) {
    let du_per_px = 18.0;
    let dx = Vec2::new(du_per_px, 0.0);
    let dy = Vec2::new(0.0, du_per_px);

    let x = size.x * 0.5;
    let mut distinct = std::collections::BTreeSet::new();
    println!(
        "bands={band_count}: scan across top edge y={} (du/px {du_per_px})",
        size.y
    );
    for step in -3..=3 {
        let y = (step as f32).mul_add(du_per_px * 0.5, size.y);
        let point = Vec2::new(x, y);
        let (single, super4) = probe.aa_band_coverage(point, dx, dy);
        let aniso = probe.aniso_shader_fix(point, dx, dy, MAX_ANISO_SAMPLES_TEXT);
        let edge_width_sq = (du_per_px * EDGE_FILTER_WIDTH).powi(2);
        let sd = probe.signed_distance(point, edge_width_sq, 0.0);
        println!("  y={y:8.2} sd={sd:8.2} single={single:.3} super4={super4:.3} aniso={aniso:.3}");
        distinct.insert((single * 1000.0) as i32);
    }
    assert!(
        distinct.iter().any(|&value| value > 100 && value < 900),
        "coverage across the edge should pass through intermediate values, got {distinct:?}"
    );
}

/// A ray exactly level with a rectangle's horizontal edge grazes both cap
/// corners. The half-open-in-y crossing rule must count neither cap; counting
/// half-open in t counts exactly one (one cap crosses at t=0, the other at
/// t=1), flipping every fragment on that row to inside — the ruler-tick
/// sparkle row.
#[test]
fn grazing_ray_through_edge_join_has_zero_winding() {
    let size = Vec2::new(709.0, 106.0);
    let probe = rectangle_probe(size, 1, 0.0);
    for y in [0.0, size.y] {
        let winding: i32 = probe
            .curves
            .iter()
            .copied()
            .map(|curve| curve_winding(curve, Vec2::new(-50.0, y)))
            .sum();
        assert_eq!(winding, 0, "grazing ray at y={y} must stay outside");
    }
    let interior: i32 = probe
        .curves
        .iter()
        .copied()
        .map(|curve| curve_winding(curve, size * 0.5))
        .sum();
    assert_ne!(interior, 0, "interior point must still wind");
}

/// The panel-route ruler spine packs as a 96x48000 design-unit rectangle
/// (0.2mm x 100mm at 480000 du/m); the probe route packs the same world
/// geometry as 64000x128 and rotates it into place. Scan coverage at the
/// stroke center along the full length of both boxes at the on-screen scale
/// where the panel spine renders broken (~49 du/px for the panel box, ~66 for
/// the probe box) and report any along-length variation.
#[test]
fn spine_coverage_uniform_along_length_for_both_orientations() {
    let cases = [
        ("panel  96x48000", Vec2::new(96.0, 48000.0), 49.0, true),
        ("probe 64000x128", Vec2::new(64000.0, 128.0), 66.0, false),
    ];
    for (label, size, du_per_px, tall) in cases {
        let probe = rectangle_probe(size, 1, 0.0);
        let dx = Vec2::new(du_per_px, 0.0);
        let dy = Vec2::new(0.0, du_per_px);
        let long_extent = if tall { size.y } else { size.x };
        let mut min_cov = f32::MAX;
        let mut max_cov = f32::MIN;
        let mut worst_at = 0.0;
        for step in 0..=400 {
            let along = long_extent * (step as f32 + 0.5) / 401.0;
            let point = if tall {
                Vec2::new(size.x * 0.5, along)
            } else {
                Vec2::new(along, size.y * 0.5)
            };
            let (single, _) = probe.aa_band_coverage(point, dx, dy);
            let aniso = probe.aniso_shader_fix(point, dx, dy, MAX_ANISO_SAMPLES_TEXT);
            let cov = aniso.min(single);
            if cov < min_cov {
                min_cov = cov;
                worst_at = along;
            }
            max_cov = max_cov.max(cov);
        }
        println!(
            "{label}: stroke-center coverage min {min_cov:.3} (at {worst_at:.0}du) max {max_cov:.3}"
        );
        assert!(
            max_cov - min_cov < 0.05,
            "{label}: coverage varies {min_cov:.3}..{max_cov:.3} along the length (worst at {worst_at:.0}du)"
        );
    }
}

/// Atlas-format check: pack several paths the way `PathAtlas::rebuild` does
/// (global curve/band arrays, per-glyph offsets) and probe a tall spine glyph
/// through the GLOBAL arrays. Catches offset/format defects the single-glyph
/// probes cannot.
#[test]
fn atlas_packed_tall_spine_covers_full_length() {
    // Mimic the analytic_line_probe example's atlas: tick paths around the
    // spines so the spine records land at non-zero offsets.
    let mut sizes: Vec<Vec2> = Vec::new();
    for _ in 0..40 {
        sizes.push(Vec2::new(1920.0, 96.0));
    }
    for spine_h in [12000.0, 24000.0, 36000.0, 48000.0] {
        sizes.push(Vec2::new(96.0, spine_h));
        for _ in 0..10 {
            sizes.push(Vec2::new(1920.0, 96.0));
        }
    }

    let mut curves = Vec::new();
    let mut bands = Vec::new();
    let mut records = Vec::new();
    for size in &sizes {
        let corners = [
            Vec2::ZERO,
            Vec2::new(size.x, 0.0),
            *size,
            Vec2::new(0.0, size.y),
        ];
        let segments: Vec<QuadraticSegment> = corners
            .iter()
            .copied()
            .zip(corners.iter().copied().cycle().skip(1))
            .take(corners.len())
            .map(|(start, end)| line(start, end))
            .collect();
        let glyph = Glyph {
            character: '-',
            id:        0,
            bounds:    Bounds {
                min: Vec2::ZERO,
                max: *size,
            },
            contours:  vec![Contour {
                segments,
                min_feature: 0.0,
                fade_exponent: 0.0,
            }],
        };
        let packed: PackedPath = super::build_packed_glyph(glyph, 1);
        let curve_start = curves.len() as u32;
        let band_start = bands.len() as u32;
        let axis_band_count = (packed.bands().len() / 2) as u32;
        curves.extend_from_slice(packed.curves());
        bands.extend(packed.bands().iter().map(|band| BandRecord {
            start: band.start + curve_start,
            ..*band
        }));
        records.push(PackedPathRecord::new(
            packed.bounds(),
            band_start,
            axis_band_count,
            band_start + axis_band_count,
            axis_band_count,
            0.0,
        ));
    }

    for (index, size) in sizes.iter().enumerate() {
        if size.y < 10000.0 {
            continue;
        }
        let probe = Probe {
            record: records[index],
            curves: curves.clone(),
            bands:  bands.clone(),
        };
        let du_per_px = 49.0;
        let dx = Vec2::new(du_per_px, 0.0);
        let dy = Vec2::new(0.0, du_per_px);
        let mut min_cov = f32::MAX;
        let mut worst = 0.0;
        let margin = du_per_px * 2.0;
        for step in 0..=400 {
            let along = margin + (size.y - 2.0 * margin) * (step as f32) / 400.0;
            let point = Vec2::new(size.x * 0.5, along);
            let (single, _) = probe.aa_band_coverage(point, dx, dy);
            if single < min_cov {
                min_cov = single;
                worst = along;
            }
        }
        println!(
            "spine {}du at record {index}: min coverage {min_cov:.3} (at {worst:.0}du)",
            size.y
        );
        assert!(
            min_cov > 0.95,
            "spine {}du record {index}: coverage drops to {min_cov:.3} at {worst:.0}du",
            size.y,
        );
    }
}

/// Reproduction with the LIVE app's inexact design size (logged from the
/// uploaded atlas: 96 x 48000.082). Clean power-friendly sizes pass; the live
/// conversion produces sizes whose midpoint-control rounding can leave the
/// quadratic winding solver with a tiny non-zero `a` and catastrophic
/// cancellation in the root.
#[test]
fn live_inexact_spine_size_covers_full_length() {
    let size = Vec2::new(96.0, 48000.082);
    let probe = rectangle_probe(size, 1, 0.0);
    let du_per_px = 49.0;
    let dx = Vec2::new(du_per_px, 0.0);
    let dy = Vec2::new(0.0, du_per_px);
    let margin = du_per_px * 2.0;
    let mut failures = Vec::new();
    for step in 0..=400 {
        let along = margin + (size.y - 2.0 * margin) * (step as f32) / 400.0;
        let point = Vec2::new(size.x * 0.5, along);
        let (single, _) = probe.aa_band_coverage(point, dx, dy);
        if single < 0.95 {
            failures.push((along, single));
        }
    }
    if !failures.is_empty() {
        println!(
            "first failure at {:.1}du, last at {:.1}du, {} of 401 samples",
            failures[0].0,
            failures[failures.len() - 1].0,
            failures.len()
        );
    }
    assert!(
        failures.is_empty(),
        "coverage lost at {} samples, first at {:.1}du",
        failures.len(),
        failures[0].0,
    );
}

/// units.rs A4 ruler reproduction: spine 96x142560 du (0.2mm x 297mm at
/// 480000 du/m) scanned at ~51 du/px, and a 1mm tick 960x96 du (at 960000
/// du/m) scanned at ~103 du/px. The live render drops the mm ticks entirely
/// and renders the spine in patches at this scale.
#[test]
fn units_ruler_sizes_cover_at_screen_scale() {
    let cases = [
        ("spine 96x142560", Vec2::new(96.0, 142_560.0), 51.0, true),
        ("tick  960x96", Vec2::new(960.0, 96.0), 103.0, false),
    ];
    for (label, size, du_per_px, tall) in cases {
        let probe = rectangle_probe(size, 1, 0.0);
        let dx = Vec2::new(du_per_px, 0.0);
        let dy = Vec2::new(0.0, du_per_px);
        let long_extent = if tall { size.y } else { size.x };
        let mut min_cov = f32::MAX;
        let mut max_cov = f32::MIN;
        let mut worst_at = 0.0;
        for step in 0..=800 {
            let along = long_extent * (step as f32 + 0.5) / 801.0;
            let point = if tall {
                Vec2::new(size.x * 0.5, along)
            } else {
                Vec2::new(along, size.y * 0.5)
            };
            let (single, _) = probe.aa_band_coverage(point, dx, dy);
            let aniso = probe.aniso_shader_fix(point, dx, dy, MAX_ANISO_SAMPLES_TEXT);
            let cov = aniso.min(single);
            if cov < min_cov {
                min_cov = cov;
                worst_at = along;
            }
            max_cov = max_cov.max(cov);
        }
        println!(
            "{label}: stroke-center coverage min {min_cov:.3} (at {worst_at:.0}du) max {max_cov:.3}"
        );
    }
}

// ---- hairline dilation + fade probes (the distance_coverage mirror) --------

/// Sub-floor stroke fixture: a 40 design-unit-wide tall stroke viewed at
/// 50 du/px with a 1.5px hairline floor, so the floor is 75 du and each curve
/// dilates by (75 − 40) / 2.
const STROKE_NATURAL_DU: f32 = 40.0;
const STROKE_DU_PER_PX: f32 = 50.0;
const HAIRLINE_MIN_PX: f32 = 1.5;
const STROKE_TARGET_DU: f32 = STROKE_DU_PER_PX * HAIRLINE_MIN_PX;
const STROKE_HEIGHT_DU: f32 = 4000.0;

fn subfloor_stroke_probe() -> Probe {
    rectangle_probe(
        Vec2::new(STROKE_NATURAL_DU, STROKE_HEIGHT_DU),
        1,
        STROKE_NATURAL_DU,
    )
}

/// The dilation contract, sign convention included: a sub-floor stroke
/// dilated to the hairline floor must render exactly like a stroke whose
/// natural width IS the floor, across the whole cross-axis profile, on the
/// `distance_coverage` (AA Off) path. The failure mode this guards: the
/// inside-positive ramp applying the dilation with the inside-negative sign,
/// eroding sub-floor strokes to hollow rims.
#[test]
fn dilated_subfloor_stroke_matches_at_floor_stroke_profile() {
    let dilated = subfloor_stroke_probe();
    let at_floor = rectangle_probe(
        Vec2::new(STROKE_TARGET_DU, STROKE_HEIGHT_DU),
        1,
        STROKE_TARGET_DU,
    );
    let pixel = Vec2::splat(STROKE_DU_PER_PX);
    let dilation_max = (STROKE_TARGET_DU - STROKE_NATURAL_DU) * 0.5;
    let y = STROKE_HEIGHT_DU * 0.5;
    // The dilated silhouettes coincide when the centers align: thin center
    // 20 du ↔ floor center 37.5 du.
    let center_offset = (STROKE_TARGET_DU - STROKE_NATURAL_DU) * 0.5;

    for step in -40..=80 {
        let x = STROKE_NATURAL_DU * 0.5 + (step as f32) * 2.0;
        let dilated_cov =
            dilated.distance_coverage(Vec2::new(x, y), pixel, dilation_max, STROKE_TARGET_DU);
        let floor_cov = at_floor.distance_coverage(
            Vec2::new(x + center_offset, y),
            pixel,
            0.0,
            STROKE_TARGET_DU,
        );
        assert!(
            (dilated_cov - floor_cov).abs() < 1.0e-3,
            "x {x:.1}: dilated sub-floor stroke covers {dilated_cov:.4} but the \
             naturally-at-floor stroke covers {floor_cov:.4}",
        );
    }
}

/// `Fade { exponent }` scales every `distance_coverage` evaluation by
/// `(natural / floor)^exponent` from that evaluation's winning curve (its
/// dilation and its contour's packed exponent) — nothing else about the
/// profile changes.
#[test]
fn fade_scales_subfloor_stroke_coverage_by_natural_ratio() {
    let full_probe = subfloor_stroke_probe();
    let pixel = Vec2::splat(STROKE_DU_PER_PX);
    let dilation_max = (STROKE_TARGET_DU - STROKE_NATURAL_DU) * 0.5;
    let y = STROKE_HEIGHT_DU * 0.5;

    for exponent in [1.0_f32, 2.0] {
        let faded_probe = rectangle_probe_with_fade(
            Vec2::new(STROKE_NATURAL_DU, STROKE_HEIGHT_DU),
            1,
            STROKE_NATURAL_DU,
            exponent,
        );
        let factor = (STROKE_NATURAL_DU / STROKE_TARGET_DU).powf(exponent);
        for step in -40..=80 {
            let point = Vec2::new(STROKE_NATURAL_DU * 0.5 + (step as f32) * 2.0, y);
            let full = full_probe.distance_coverage(point, pixel, dilation_max, STROKE_TARGET_DU);
            let faded = faded_probe.distance_coverage(point, pixel, dilation_max, STROKE_TARGET_DU);
            assert!(
                (faded - full * factor).abs() < 1.0e-4,
                "exponent {exponent}, x {:.1}: faded coverage {faded:.4} should be full \
                 {full:.4} × {factor:.4}",
                point.x,
            );
        }
    }
}

/// Text exemption is structural: a path packed with `min_feature = 0` (every
/// text glyph) has zero dilation, so its fade factor is 1 under any exponent.
#[test]
fn text_paths_are_exempt_from_fade() {
    let plain_probe = rectangle_probe(Vec2::new(STROKE_NATURAL_DU, STROKE_HEIGHT_DU), 1, 0.0);
    let faded_probe =
        rectangle_probe_with_fade(Vec2::new(STROKE_NATURAL_DU, STROKE_HEIGHT_DU), 1, 0.0, 4.0);
    let pixel = Vec2::splat(STROKE_DU_PER_PX);
    let y = STROKE_HEIGHT_DU * 0.5;

    for step in -40..=80 {
        let point = Vec2::new(STROKE_NATURAL_DU * 0.5 + (step as f32) * 2.0, y);
        let plain = plain_probe.distance_coverage(point, pixel, 0.0, STROKE_TARGET_DU);
        let faded = faded_probe.distance_coverage(point, pixel, 0.0, STROKE_TARGET_DU);
        assert!(
            (plain - faded).abs() < 1.0e-6,
            "x {:.1}: fade exponent changed an undilated path's coverage \
             ({plain:.4} vs {faded:.4})",
            point.x,
        );
    }
}

/// The `aa_band` feeder tracks the same winning dilation: the union signed
/// distance (`sd.y`) of a dilated sub-floor stroke matches the
/// naturally-at-floor stroke, and the sample's dilation feeds the same fade
/// factor as the Off path.
#[test]
fn signed_distance_sample_tracks_winning_dilation() {
    let dilated = rectangle_probe_with_fade(
        Vec2::new(STROKE_NATURAL_DU, STROKE_HEIGHT_DU),
        1,
        STROKE_NATURAL_DU,
        1.0,
    );
    let at_floor = rectangle_probe_with_fade(
        Vec2::new(STROKE_TARGET_DU, STROKE_HEIGHT_DU),
        1,
        STROKE_TARGET_DU,
        1.0,
    );
    let edge_width = STROKE_DU_PER_PX * EDGE_FILTER_WIDTH;
    let dilation_max = (STROKE_TARGET_DU - STROKE_NATURAL_DU) * 0.5;
    let scan_width = edge_width + dilation_max;
    let scan_width_sq = scan_width * scan_width;
    let y = STROKE_HEIGHT_DU * 0.5;
    let center_offset = (STROKE_TARGET_DU - STROKE_NATURAL_DU) * 0.5;

    let center = Vec2::new(STROKE_NATURAL_DU * 0.5, y);
    let (sd, dilation, _) = dilated.signed_distance_sample(center, scan_width_sq, STROKE_TARGET_DU);
    assert!(
        (dilation - dilation_max).abs() < 1.0e-4,
        "the stroke's own curves should win with dilation {dilation_max}, got {dilation}"
    );
    let (floor_sd, floor_dilation, _) = at_floor.signed_distance_sample(
        center + Vec2::new(center_offset, 0.0),
        scan_width_sq,
        STROKE_TARGET_DU,
    );
    assert!(
        (sd.y - floor_sd.y).abs() < 1.0e-3,
        "dilated faded-lane signed distance {:.4} should match the at-floor stroke {:.4}",
        sd.y,
        floor_sd.y,
    );
    assert!(
        floor_dilation.abs() < 1.0e-6,
        "an at-floor stroke needs no dilation, got {floor_dilation}"
    );
}

/// The abut-line fix: a fading minor tick and a never-fading spine merged
/// into ONE path, combined as mix(exempt, union, fade factor). The exempt
/// spine interior renders exactly as it would alone (a fading neighbor
/// cannot dim it — the dotted-spine defect of single-winning-curve fade
/// selection), the tick interior matches an isolated faded tick, no point on
/// the shared row renders darker than the same geometry with every contour
/// fading, and in the undilated regime (strokes above the hairline floor,
/// fade factor 1) the junction is union-interior at full alpha — the
/// max-combine defect put a half-alpha line there, the two lanes' AA ramps
/// each reaching ~0.5 where the contours abut.
#[test]
fn merged_mixed_fade_path_has_no_junction_dip() {
    // Spine: 40 du wide, x ∈ [0, 40], fade-exempt (exponent 0).
    // Tick: 800 du long, x ∈ [40, 840], 40 du tall, fading (exponent 1).
    let spine_size = Vec2::new(STROKE_NATURAL_DU, STROKE_HEIGHT_DU);
    let tick_min = Vec2::new(STROKE_NATURAL_DU, STROKE_HEIGHT_DU * 0.5);
    let tick_size = Vec2::new(800.0, STROKE_NATURAL_DU);
    let merged_probe = |spine_fade: f32| {
        let glyph = Glyph {
            character: '-',
            id:        0,
            bounds:    Bounds {
                min: Vec2::ZERO,
                max: Vec2::new(tick_min.x + tick_size.x, spine_size.y),
            },
            contours:  vec![
                rectangle_contour(Vec2::ZERO, spine_size, STROKE_NATURAL_DU, spine_fade),
                rectangle_contour(tick_min, tick_size, STROKE_NATURAL_DU, 1.0),
            ],
        };
        let packed = super::build_packed_glyph(glyph, 1);
        let band_count = (packed.bands().len() / 2) as u32;
        Probe {
            record: PackedPathRecord::new(
                packed.bounds(),
                0,
                band_count,
                band_count,
                band_count,
                STROKE_NATURAL_DU,
            ),
            curves: packed.curves().to_vec(),
            bands:  packed.bands().to_vec(),
        }
    };
    let mixed = merged_probe(0.0);
    let uniform = merged_probe(1.0);
    let spine_only = rectangle_probe(spine_size, 1, STROKE_NATURAL_DU);
    let tick_only = rectangle_probe_with_fade(tick_size, 1, STROKE_NATURAL_DU, 1.0);

    let pixel = Vec2::splat(STROKE_DU_PER_PX);
    let dilation_max = (STROKE_TARGET_DU - STROKE_NATURAL_DU) * 0.5;
    let row_y = tick_min.y + tick_size.y * 0.5;
    let coverage = |probe: &Probe, point: Vec2| {
        probe.distance_coverage(point, pixel, dilation_max, STROKE_TARGET_DU)
    };

    // The exempt spine renders exactly as it would alone.
    let spine_center = Vec2::new(spine_size.x * 0.5, row_y);
    let spine_in_merged = coverage(&mixed, spine_center);
    let spine_alone = coverage(&spine_only, spine_center);
    assert!(
        (spine_in_merged - spine_alone).abs() < 1.0e-4,
        "exempt spine in the merged path covers {spine_in_merged:.4}, alone {spine_alone:.4}"
    );

    // The fading tick interior matches an isolated faded tick.
    let tick_center_merged = Vec2::new(tick_min.x + tick_size.x * 0.5, row_y);
    let tick_center_alone = tick_size * 0.5;
    let tick_in_merged = coverage(&mixed, tick_center_merged);
    let tick_alone = coverage(&tick_only, tick_center_alone);
    assert!(
        (tick_in_merged - tick_alone).abs() < 1.0e-4,
        "fading tick in the merged path covers {tick_in_merged:.4}, alone {tick_alone:.4}"
    );

    // Pointwise along the shared row: exempting the spine never renders
    // darker than the all-fading merged path, and never darker than the
    // spine alone — no junction dip.
    for step in 0..=400 {
        let x = (step as f32).mul_add(0.5, spine_size.x - 60.0);
        let point = Vec2::new(x, row_y);
        let cov = coverage(&mixed, point);
        let uniform_cov = coverage(&uniform, point);
        let spine_cov = coverage(&spine_only, point);
        assert!(
            cov >= uniform_cov - 1.0e-4,
            "x={x:.1}: mixed-fade coverage {cov:.4} below all-fading {uniform_cov:.4}"
        );
        assert!(
            cov >= spine_cov - 1.0e-4,
            "x={x:.1}: mixed-fade coverage {cov:.4} below the spine alone {spine_cov:.4}"
        );
    }

    // Undilated regime (the zoomed-in view): strokes are many pixels wide,
    // every per-curve dilation is 0 and the fade factor is 1, so the mixed
    // path must render identically to the all-fading one — in particular the
    // junction at x = 40 is union-interior at full alpha. hairline_target 0
    // disables dilation and the fade, exactly as the shader does for runs
    // whose strokes sit above the floor.
    let fine_pixel = Vec2::splat(2.0);
    let fine = |probe: &Probe, point: Vec2| probe.distance_coverage(point, fine_pixel, 0.0, 0.0);
    for step in 0..=200 {
        let x = (step as f32).mul_add(0.1, spine_size.x - 10.0);
        let point = Vec2::new(x, row_y);
        let cov = fine(&mixed, point);
        let uniform_cov = fine(&uniform, point);
        assert!(
            (cov - uniform_cov).abs() < 1.0e-4,
            "x={x:.2}: undilated mixed coverage {cov:.4} differs from all-fading {uniform_cov:.4}"
        );
    }
    let junction = fine(&mixed, Vec2::new(spine_size.x, row_y));
    assert!(
        junction >= 1.0 - 1.0e-4,
        "the spine/tick junction must be union-interior at full alpha, got {junction:.4}"
    );
}
