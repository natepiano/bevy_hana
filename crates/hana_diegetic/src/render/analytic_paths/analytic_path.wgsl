// Analytic path fragment shader: exact coverage for quadratic-Bezier
// outlines, shared by slug text runs and merged panel-line paths. A
// "path" here is one packed path — a font path or a whole merge group of
// panel lines (ticks + spine in one record).
//
// Inputs (packed by render/analytic_paths/packing.rs, structs mirrored
// below):
//   curves  — CurveRecord quadratic segments with precomputed
//             distance-solver coefficients, per-contour hairline stroke
//             width (solver.w), and per-contour fade exponent.
//   bands   — BandRecord (start, count) windows into a band-ordered curve
//             table. Each path has along-Y bands (y-slabs: every curve
//             a +x ray from a point in the slab can cross) and along-X
//             bands (x-slabs), so winding and nearest-distance scans touch
//             a handful of curves, not the whole path.
//   path_records  — PackedPathRecord per packed path: design-space bounds, band table
//             ranges, narrowest dilating stroke.
//   uv_a    — material box UV under vertex pulling, else fractional position
//             inside the path bounds quad.
//   uv_b    — path-coverage UV under vertex pulling, else (path index, run
//             index) as floats, constant per quad.
//   world.w — PathQuadRecord index under vertex pulling.
//   per-run — material slot / render mode / OIT offset / AA flags from the
//             PathRenderRecord table under FRAGMENT_DATA_FROM_BATCHED_PATHS.
//             Non-vertex-pulled fallback fragments carry no material row and
//             resolve to transparent output.
//
// Evaluation: non-zero winding (inside/outside) + distance to the nearest
// curve (the AA ramp), both banded. Strokes thinner than
// PathUniform.hairline_min_px on screen are dilated per curve to that
// floor and faded back by each contour's fade exponent; fading and exempt
// contours coexist in one path via the two-lane CoverageTerms evaluation.
// Coverage then feeds PBR lighting and (when enabled) OIT.
//
// For coverage debugging, text/slug/glyph/coverage_probe.rs contains a CPU
// model of the key distance, band, anisotropic sampling, convex-corner, and
// hairline-fade logic below. Use it when changing coverage math or when GPU
// visual debugging is not converging; shader plumbing changes do not need to
// keep that probe in lockstep.

#import bevy_pbr::{
    pbr_functions::alpha_discard,
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#import hana_diegetic::material_table::INVALID_GPU_MATERIAL_SLOT
#import hana_diegetic::sdf_material_table::pbr_input_from_material_table

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::VertexOutput
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#import bevy_pbr::pbr_types::{
    STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS,
    STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE,
}

// Floor for the offset fragment depth handed to oit_draw.
// pack_24bit_depth_8bit_alpha saturates depth to [0, 1] and bevy's OIT
// resolve compares packed (depth << 8 | alpha) values against the cleared
// background (depth 0, alpha 1): a fragment whose offset z reaches 0 packs
// below the background whenever its alpha < 1.0 and is silently dropped.
// This tracks 3 × OIT_DEPTH_STEP from render/constants.rs.
const OIT_MIN_DEPTH: f32 = 3e-6;
#endif

const ROOT_EPSILON: f32 = 0.00001;
const DEGENERATE_EPS: f32 = 0.00000001;
const SQRT_3_OVER_2: f32 = 0.8660254037844386;
const DISCARD_ALPHA: f32 = 0.02;
const EDGE_FILTER_WIDTH: f32 = 1.2;
// Small analytic text can look lighter than its geometric coverage suggests:
// most of each stroke lands in fractional edge pixels, while few fully covered
// pixels remain to carry its intended optical weight. Fade in a bounded
// coverage transfer as the projected em shrinks. These thresholds are in
// physical device pixels because derivatives operate in device-pixel space.
const AUTO_TEXT_BIAS_FULL_BELOW_PX: f32 = 8.0;
const AUTO_TEXT_BIAS_NONE_ABOVE_PX: f32 = 22.0;
const AUTO_TEXT_BIAS_MAX: f32 = 0.85;
// Anisotropic sub-sample ceiling for the line/circle path
// (analytic_line_coverage). The sample count tracks footprint anisotropy and
// collapses to 1 head-on. The line path's convex-corner wing is handled by the
// polygon corner correction, so this stays at the lower validated bound.
const MAX_ANISO_SAMPLES: f32 = 16.0;
// Anisotropic sub-sample ceiling for the text band path (aniso_band_coverage).
// The sample count tracks footprint anisotropy and collapses to 1 head-on, so
// the ceiling binds only past 64:1 grazing. At 64 the convex-corner over-coverage
// on text (the grazing wing) stays suppressed up to that ratio; beyond it the
// wing reappears, shrinking as the ceiling rises. A ratio-independent fix makes
// the sample count adaptive at detected convex corners instead of a fixed
// ceiling; deferred — the fixed ceiling holds for real viewing angles.
const MAX_ANISO_SAMPLES_TEXT: f32 = 64.0;
// Diagnostic: when true the line branch returns a firing mask (dim line + bright
// where the corner correction clipped coverage) instead of the corrected
// coverage, so the gates can be verified to light only exterior corners. Set
// false to render the actual corrected coverage.
const CORNER_DEBUG_MASK: bool = false;
// Diagnostic: paint line fragments (min_feature > 0) with an upstream coverage
// input instead of coverage, to locate the grazing wave. 0 = off (real render);
// 1 = union_normal as RGB (flat color on a straight edge; shimmer => directional
// round-off in normalize(point - closest_point)); 2 = band |Jᵀn| as fract
// contours (tests normal AND screen Jacobian); 3 = signed distance as fract
// iso-distance contours (tests the closest-point solve).
const LINE_DEBUG_MODE: i32 = 0;
const LINE_DEBUG_SCALE: f32 = 8.0;
const RENDER_MODE_TEXT: u32 = 1u;
const RENDER_MODE_PUNCH_OUT: u32 = 2u;
// Mirrors AA_FLAG_SUPERSAMPLE / AA_FLAG_BAND in render/mod.rs — the
// PathRenderRecord.aa_flags bit encoding written by AntiAlias::aa_flags().
const AA_FLAG_SUPERSAMPLE: u32 = 1u;
const AA_FLAG_BAND: u32 = 2u;

struct PathUniform {
    render_mode: u32,
    oit_depth_offset: f32,
    supersample: u32,
    aa_band: u32,
    // Minimum on-screen stroke width in device pixels for hairline-dilated
    // paths. Synced from the HairlineWidth resource (logical px × window
    // scale factor, floored): a stroke dilated to under ~1.5px renders as
    // either one solid column or two half-bright columns depending on pixel
    // phase, so a near-vertical line stairsteps a full column per crossover.
    hairline_min_px: f32,
}

// One quadratic Bezier segment, mirroring `CurveRecord` in packing.rs. The
// segment is evaluated as B(t) = start + 2t·control_delta + t²·curve_delta,
// so winding and distance never need the raw control point.
struct CurveRecord {
    // Segment start point in .xy; control-minus-start in .zw.
    start_delta: vec4<f32>,
    // Quadratic second difference (end − 2·control + start) in .xy — zero
    // for segments the packer snapped to exactly linear; segment end point
    // in .zw.
    curve_end: vec4<f32>,
    // Control-point AABB, min in .xy / max in .zw — conservative cull for
    // the distance scan.
    bounds: vec4<f32>,
    // .xyz: precomputed closest-point cubic coefficients — .x and .y are the
    // point-independent terms of the normalized cubic
    // exact_quadratic_distance solves, .z is 1/|curve_delta|² (0 routes
    // the segment through the exact linear solve). .w: the owning contour's
    // narrowest stroke in design units (per-curve hairline dilation), 0.0
    // for undilated contours (text paths).
    solver: vec4<f32>,
    // Owning contour's resolved hairline fade exponent; each coverage
    // evaluation fades by the winning (nearest) curve's exponent, so one
    // merged path can mix fading and non-fading contours. 0.0 disables fade
    // for this curve.
    fade_exponent: f32,
    // Outward unit edge normal from the contour winding (packing.rs). The line
    // branch's convex-corner clip reads it as the edge half-plane direction;
    // the radial normalize(point - closest) goes to the vertex direction past a
    // corner, so it can't stand in. (0,0) routes the clip back to the radial
    // normal (text paths, degenerate edges).
    edge_normal: vec2<f32>,
}

// One band's window into the band-ordered curve table: an along-Y band
// holds every curve whose y-extent overlaps that y-slab of the path (the
// complete crossing set for a +x winding ray from inside the slab), an
// along-X band the same by x. Band lookup derives the index from the
// point's normalized coordinate; range_min/range_max are the band edges in design
// units.
struct BandRecord {
    start: u32,
    count: u32,
    range_min: f32,
    range_max: f32,
}

// One packed path (font path or merged panel-line group).
struct PackedPathRecord {
    // Design-space bounds: min in .xy, size in .zw. uv_a maps onto this
    // rectangle (see design_position).
    bounds_min_size: vec4<f32>,
    // Band-table ranges: along-Y band start/count in .xy, along-X band
    // start/count in .zw.
    band_range: vec4<u32>,
    // Narrowest dilating stroke across the path's contours, in design units;
    // > 0 enables hairline dilation and sizes the distance scan for the
    // largest dilation in the path. 0 for text. Per-contour widths arrive in
    // each curve's solver.w.
    min_feature: f32,
}

struct LaneTerms {
    winding: i32,
    // Min over this lane's scanned curves of (distance - that curve's
    // dilation): the distance to the lane's nearest per-contour-dilated
    // silhouette.
    adjusted: f32,
    // Dilation of the curve that won `adjusted`.
    dilation: f32,
    // Design-space unit field gradient of the curve that won `adjusted`
    // (direction from its closest point to `point`). The analytic AA band
    // projects the screen footprint onto it; sign-independent for the band
    // (|Jᵀn| == |Jᵀ(-n)|), so the medial-axis flip between a slab's two edges
    // does not matter.
    normal: vec2<f32>,
    // Second-nearest silhouette in this lane (raw adjusted distance + its field
    // gradient), for the line branch's convex-corner correction: a fragment
    // outside a convex corner is outside both meeting edges, so the second
    // edge's half-plane coverage clips the nearest edge's straight-edge
    // over-cover. Sentinel 1e6 / (1,0) means no second edge was scanned. Text
    // ignores these (it reads only the nearest).
    adjusted2: f32,
    normal2: vec2<f32>,
    // Outward edge-line normal (from the contour winding, CurveRecord.edge_normal)
    // and signed perpendicular distance to that edge's line (outward positive) for
    // the nearest and second-nearest curves. The convex-corner clip uses the
    // SECOND edge's half-plane (edge_normal2/edge_perp2); the nearest pair exists
    // so a new nearest can demote into the second slot. Distinct from normal/normal2
    // (the radial field gradient the band and floor still use): past a convex vertex
    // the radial normal goes to the vertex direction, but the edge normal stays the
    // true edge half-plane.
    edge_normal: vec2<f32>,
    edge_perp: f32,
    edge_normal2: vec2<f32>,
    edge_perp2: f32,
}

// Two-lane coverage accumulator. Curves split by fade policy: the exempt
// lane holds fade_exponent == 0 curves (never-fading contours, and every
// text path), the faded lane holds fade_exponent > 0 curves. Contours are
// wholly one lane, so each lane's winding is a valid winding number of its
// own sub-geometry, and the lanes' windings sum to the whole path's. Final
// alpha takes mix(exempt_coverage, union_coverage, fade_factor): at fade
// factor 1 the path renders exactly as an unfaded single-winding union (an
// exempt/faded abutment is union-interior, so no junction line), at factor 0
// only the exempt sub-geometry remains, and a never-fading ruler spine keeps
// full alpha (mix(1, 1, f) = 1) even where a thinner (more-dilated) fading
// tick's curves are nearer in adjusted-distance terms.
struct CoverageTerms {
    exempt: LaneTerms,
    faded: LaneTerms,
    // Fade exponent of the faded lane's winning curve.
    fade_exponent: f32,
}

fn empty_coverage_terms() -> CoverageTerms {
    return CoverageTerms(
        LaneTerms(
            0, 1000000.0, 0.0, vec2<f32>(1.0, 0.0), 1000000.0, vec2<f32>(1.0, 0.0),
            vec2<f32>(1.0, 0.0), -1000000.0, vec2<f32>(1.0, 0.0), -1000000.0,
        ),
        LaneTerms(
            0, 1000000.0, 0.0, vec2<f32>(1.0, 0.0), 1000000.0, vec2<f32>(1.0, 0.0),
            vec2<f32>(1.0, 0.0), -1000000.0, vec2<f32>(1.0, 0.0), -1000000.0,
        ),
        0.0,
    );
}

// The whole path's terms, rebuilt from the two lanes: windings add (the
// lanes partition the path's contours), and the nearest-silhouette race is
// the cross-lane min with the winner's dilation.
fn union_lane(terms: CoverageTerms) -> LaneTerms {
    let faded_wins = terms.faded.adjusted < terms.exempt.adjusted;
    let first_adjusted = select(terms.exempt.adjusted, terms.faded.adjusted, faded_wins);
    let first_dilation = select(terms.exempt.dilation, terms.faded.dilation, faded_wins);
    let first_normal = select(terms.exempt.normal, terms.faded.normal, faded_wins);
    // Union second-nearest = second smallest across the two lanes' (first, second)
    // candidates: the losing lane's first vs the winning lane's own second.
    let first_edge_normal = select(terms.exempt.edge_normal, terms.faded.edge_normal, faded_wins);
    let first_edge_perp = select(terms.exempt.edge_perp, terms.faded.edge_perp, faded_wins);
    let other_first_adjusted = select(terms.faded.adjusted, terms.exempt.adjusted, faded_wins);
    let other_first_normal = select(terms.faded.normal, terms.exempt.normal, faded_wins);
    let other_first_edge_normal = select(terms.faded.edge_normal, terms.exempt.edge_normal, faded_wins);
    let other_first_edge_perp = select(terms.faded.edge_perp, terms.exempt.edge_perp, faded_wins);
    let win_second_adjusted = select(terms.exempt.adjusted2, terms.faded.adjusted2, faded_wins);
    let win_second_normal = select(terms.exempt.normal2, terms.faded.normal2, faded_wins);
    let win_second_edge_normal = select(terms.exempt.edge_normal2, terms.faded.edge_normal2, faded_wins);
    let win_second_edge_perp = select(terms.exempt.edge_perp2, terms.faded.edge_perp2, faded_wins);
    let other_is_second = other_first_adjusted < win_second_adjusted;
    let second_adjusted = select(win_second_adjusted, other_first_adjusted, other_is_second);
    let second_normal = select(win_second_normal, other_first_normal, other_is_second);
    let second_edge_normal = select(win_second_edge_normal, other_first_edge_normal, other_is_second);
    let second_edge_perp = select(win_second_edge_perp, other_first_edge_perp, other_is_second);
    return LaneTerms(
        terms.exempt.winding + terms.faded.winding,
        first_adjusted,
        first_dilation,
        first_normal,
        second_adjusted,
        second_normal,
        first_edge_normal,
        first_edge_perp,
        second_edge_normal,
        second_edge_perp,
    );
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> uniforms: PathUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> curves: array<CurveRecord>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<storage, read> bands: array<BandRecord>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<storage, read> path_records: array<PackedPathRecord>;

#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
// Mirrors `PathQuadRecord` in `path/packing.rs` (std430, 64 B stride). The
// fragment stage reads it so coverage UV and material box UV can diverge.
struct PathQuadRecord {
    rect_min: vec2<f32>,
    rect_size: vec2<f32>,
    uv_min: vec2<f32>,
    uv_size: vec2<f32>,
    box_uv_min: vec2<f32>,
    box_uv_size: vec2<f32>,
    packed_path_index: u32,
    render_index: u32,
    box_uv_flip_x: u32,
}

// Mirrors `PathRenderRecord` in `path/packing.rs` (std430, 96 B stride). Under the
// vertex-pulling route a batch holds many runs, so the per-run values move
// out of the material uniform into this table, indexed by the run index the
// vertex stage reaches through `PathQuadRecord::render_index`.
struct PathRenderRecord {
    transform: mat4x4<f32>,
    material: u32,
    render_mode: u32,
    clip_depth_nudge: f32,
    oit_depth_offset: f32,
    aa_flags: u32,
    text_coverage_bias: f32,
    text_em_height: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(104) var<storage, read> instances: array<PathQuadRecord>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var<storage, read> run_records: array<PathRenderRecord>;

// The index arrives in an interpolated varying that is constant across the
// quad, but perspective-corrected interpolation can land a hair below the
// integer (478.9999 for 479.0) on long sliver quads, so recovery must round —
// a floor() here reads the previous record and the quad renders another
// path's coverage.
fn instance_index(in: VertexOutput) -> u32 {
    return u32(in.world_position.w + 0.5);
}

fn render_index(in: VertexOutput) -> u32 {
    return instances[instance_index(in)].render_index;
}
#endif

// Coverage UVs are separate from material sampling UVs under vertex pulling.
fn coverage_uv(in: VertexOutput) -> vec2<f32> {
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    return in.uv_b;
#else
    return in.uv;
#endif
}

fn path_index(in: VertexOutput) -> u32 {
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    return instances[instance_index(in)].packed_path_index;
#else
    // uv_b.x carries the path table index; same rounding recovery as
    // render_index above.
    return u32(in.uv_b.x + 0.5);
#endif
}

// Per-run material-table slot: the run table under vertex pulling, invalid on
// the per-run path.
fn run_material_id(in: VertexOutput) -> u32 {
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    return run_records[render_index(in)].material;
#else
    return INVALID_GPU_MATERIAL_SLOT;
#endif
}

// Per-run render mode (Text / PunchOut), sourced like `run_material_id`.
fn run_render_mode(in: VertexOutput) -> u32 {
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    return run_records[render_index(in)].render_mode;
#else
    return uniforms.render_mode;
#endif
}

// Per-run OIT z offset: the run table under vertex pulling, the material
// uniform on the per-run path.
fn run_oit_depth_offset(in: VertexOutput) -> f32 {
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    return run_records[render_index(in)].oit_depth_offset;
#else
    return uniforms.oit_depth_offset;
#endif
}

// Per-run anti-alias mode bits (AA_FLAG_SUPERSAMPLE | AA_FLAG_BAND), sourced
// like `run_material_id`.
fn run_aa_flags(in: VertexOutput) -> u32 {
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    return run_records[render_index(in)].aa_flags;
#else
    var flags = 0u;
    if uniforms.supersample != 0u {
        flags |= AA_FLAG_SUPERSAMPLE;
    }
    if uniforms.aa_band != 0u {
        flags |= AA_FLAG_BAND;
    }
    return flags;
#endif
}

// Per-run signed text coverage transfer. Only text paths consume this; line
// and panel-shape paths ignore it structurally because `path.min_feature > 0`.
fn run_text_coverage_bias(in: VertexOutput) -> f32 {
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    return run_records[render_index(in)].text_coverage_bias;
#else
    return 0.0;
#endif
}

fn path_bounds_min(path: PackedPathRecord) -> vec2<f32> {
    return path.bounds_min_size.xy;
}

fn path_bounds_size(path: PackedPathRecord) -> vec2<f32> {
    return path.bounds_min_size.zw;
}

// Converts the run's local em height back into this glyph's design units.
// rect_size / (uv_size * path_bounds_size) is the glyph's affine
// design-to-layout scale and remains valid when the quad has been clipped.
fn text_em_design_height(in: VertexOutput, path: PackedPathRecord) -> f32 {
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    let instance = instances[instance_index(in)];
    let run = run_records[instance.render_index];
    let bounds_height = max(abs(path_bounds_size(path).y), ROOT_EPSILON);
    let uv_height = max(abs(instance.uv_size.y), ROOT_EPSILON);
    let rect_height = max(abs(instance.rect_size.y), ROOT_EPSILON);
    let layout_per_design_y = rect_height / (uv_height * bounds_height);
    return run.text_em_height / max(layout_per_design_y, ROOT_EPSILON);
#else
    return 0.0;
#endif
}

// uv_a → design-space point inside the path bounds. v flips: uv origin is
// top-left, design space is y-up.
fn design_position(uv: vec2<f32>, path: PackedPathRecord) -> vec2<f32> {
    let bounds_min = path_bounds_min(path);
    let bounds_size = path_bounds_size(path);
    return bounds_min + vec2<f32>(
        uv.x * bounds_size.x,
        (1.0 - uv.y) * bounds_size.y,
    );
}

fn along_y_band_index(point: vec2<f32>, path: PackedPathRecord) -> u32 {
    let bounds_min = path_bounds_min(path);
    let bounds_size = path_bounds_size(path);
    let band_count = path.band_range.y;
    let normalized_y = clamp(
        (point.y - bounds_min.y) / max(bounds_size.y, ROOT_EPSILON),
        0.0,
        0.999999,
    );
    return min(u32(normalized_y * f32(band_count)), band_count - 1u);
}

fn along_x_band_index(point: vec2<f32>, path: PackedPathRecord) -> u32 {
    let bounds_min = path_bounds_min(path);
    let bounds_size = path_bounds_size(path);
    let band_count = path.band_range.w;
    let normalized_x = clamp(
        (point.x - bounds_min.x) / max(bounds_size.x, ROOT_EPSILON),
        0.0,
        0.999999,
    );
    return min(u32(normalized_x * f32(band_count)), band_count - 1u);
}

// Sign-preserving cube root for the depressed-cubic solver below.
fn cbrt_signed(x: f32) -> f32 {
    if x < 0.0 {
        return -pow(-x, 1.0 / 3.0);
    }
    return pow(x, 1.0 / 3.0);
}

// Real roots of t³ + a·t² + b·t + c (trigonometric branch for three roots,
// Cardano for one). Writes them to `roots`, returns the count.
fn solve_cubic_normed(a: f32, b: f32, c: f32, roots: ptr<function, array<f32, 3>>) -> u32 {
    let a2 = a * a;
    let q = (1.0 / 9.0) * (a2 - 3.0 * b);
    let r = (1.0 / 54.0) * (a * (2.0 * a2 - 9.0 * b) + 27.0 * c);
    let r2 = r * r;
    let q3 = q * q * q;
    let a_third = a * (1.0 / 3.0);
    if r2 < q3 {
        let t_norm = clamp(r / sqrt(q3), -1.0, 1.0);
        let theta = acos(t_norm);
        let q_pre = -2.0 * sqrt(q);
        let cos_t3 = cos(theta / 3.0);
        let sin_t3 = sin(theta / 3.0);
        (*roots)[0] = q_pre * cos_t3 - a_third;
        (*roots)[1] = q_pre * (-0.5 * cos_t3 - SQRT_3_OVER_2 * sin_t3) - a_third;
        (*roots)[2] = q_pre * (-0.5 * cos_t3 + SQRT_3_OVER_2 * sin_t3) - a_third;
        return 3u;
    }
    let sgn = select(-1.0, 1.0, r < 0.0);
    let u = sgn * cbrt_signed(abs(r) + sqrt(r2 - q3));
    let v = select(q / u, 0.0, u == 0.0);
    (*roots)[0] = (u + v) - a_third;
    return 1u;
}

// Squared distance from `point` to the segment plus the on-curve closest
// point (the field gradient is the direction from it to `point`). The
// closest-point parameters are the roots of the cubic (B(t) − point)·B'(t) =
// 0; its two point-independent coefficients and the 1/|curve_delta|²
// normalizer arrive precomputed in curve.solver.xyz, the point-dependent terms
// are filled in here. Endpoints compete with the in-range (t ∈ [0, 1]) roots;
// degenerate curvature (solver.z == 0) falls back to the exact point–segment
// closest point.
struct CurveDistance {
    dist_sq: f32,
    closest: vec2<f32>,
}

fn exact_quadratic_distance(
    curve: CurveRecord,
    point: vec2<f32>,
    start: vec2<f32>,
    control_delta: vec2<f32>,
    curve_delta: vec2<f32>,
    end: vec2<f32>,
) -> CurveDistance {
    let pv = point - start;
    var best_sq = dot(pv, pv);
    var best_closest = start;

    let end_diff = end - point;
    let end_sq = dot(end_diff, end_diff);
    if end_sq < best_sq {
        best_sq = end_sq;
        best_closest = end;
    }

    let inverse_curve_norm_sq = curve.solver.z;
    if inverse_curve_norm_sq > 0.0 {
        var roots: array<f32, 3>;
        let root_count = solve_cubic_normed(
            curve.solver.x,
            curve.solver.y - dot(curve_delta, pv) * inverse_curve_norm_sq,
            -dot(control_delta, pv) * inverse_curve_norm_sq,
            &roots,
        );
        for (var index = 0u; index < root_count; index += 1u) {
            let t = roots[index];
            if t >= 0.0 && t <= 1.0 {
                let closest = start + control_delta * (2.0 * t) + curve_delta * (t * t);
                let diff = closest - point;
                let dist_sq = dot(diff, diff);
                if dist_sq < best_sq {
                    best_sq = dist_sq;
                    best_closest = closest;
                }
            }
        }
    } else {
        let foot = point_line_closest(point, start, end);
        let diff = foot - point;
        let dist_sq = dot(diff, diff);
        if dist_sq < best_sq {
            best_sq = dist_sq;
            best_closest = foot;
        }
    }

    return CurveDistance(best_sq, best_closest);
}

fn winding_for_t(curve: CurveRecord, point: vec2<f32>, t: f32) -> i32 {
    let dy = 2.0 * (curve.start_delta.w + curve.curve_end.y * t);
    if abs(dy) < ROOT_EPSILON {
        return 0;
    }
    // Half-open in y, not t: an upward crossing counts on t ∈ [0, 1), a
    // downward crossing on t ∈ (0, 1], so each segment's y-interval includes
    // its lower endpoint and excludes its upper one. A ray exactly through a
    // join (e.g. a rectangle cap's corner, where the horizontal edge
    // contributes nothing) then sees both adjoining segments agree — counting
    // half-open in t lets one cap count its t=0 corner while the other
    // excludes its t=1 corner, flipping a whole row of fragments inside.
    let upward = dy > 0.0;
    if upward && (t < 0.0 || t >= 1.0) {
        return 0;
    }
    if !upward && (t <= 0.0 || t > 1.0) {
        return 0;
    }

    let curve_x = curve.start_delta.x +
        2.0 * curve.start_delta.z * t +
        curve.curve_end.x * t * t;
    if curve_x <= point.x {
        return 0;
    }
    return select(-1, 1, upward);
}

// Signed crossing count this segment contributes to a +x ray from `point`:
// solve the quadratic y(t) = point.y, score each in-range root by crossing
// direction (winding_for_t). Sums to the non-zero winding number over a
// whole contour.
fn curve_winding(curve: CurveRecord, point: vec2<f32>) -> i32 {
    let a = curve.curve_end.y;
    let b = 2.0 * curve.start_delta.w;
    let c = curve.start_delta.y - point.y;

    if abs(a) < ROOT_EPSILON {
        if abs(b) < ROOT_EPSILON {
            return 0;
        }
        return winding_for_t(curve, point, -c / b);
    }

    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return 0;
    }

    let root = sqrt(discriminant);
    return winding_for_t(curve, point, (-b - root) / (2.0 * a)) +
        winding_for_t(curve, point, (-b + root) / (2.0 * a));
}

fn outside_path_bounds(point: vec2<f32>, path: PackedPathRecord) -> bool {
    // Line paths (min_feature > 0) render with the vertex-stage quad expansion
    // (analytic_path_vertex_pull.wgsl LINE_AA_MARGIN_PX) so the grazing AA ramp
    // clears the quad edge. The packed bounds are NOT expanded, so clipping
    // winding here would zero the outward ramp and the grazing strided samples
    // of a line that sits at a bounds edge (a box's top/bottom edge). The +x
    // winding-ray math is valid outside the packed bounds, so do not early-out
    // there for lines. DIAGNOSTIC: full bypass to confirm the bounds clip is the
    // grazing dash/wing source; the keep version inflates by the scan margin.
    if path.min_feature > 0.0 {
        return false;
    }
    let bounds_min = path_bounds_min(path);
    let bounds_max = bounds_min + path_bounds_size(path);
    return point.x < bounds_min.x ||
        point.x > bounds_max.x ||
        point.y < bounds_min.y ||
        point.y > bounds_max.y;
}

// Hairline fade factor for one coverage evaluation: the alpha scale that
// makes a dilated sub-floor stroke fade toward its natural coverage instead
// of rendering at full alpha. Both inputs come from the evaluation's winning
// curve — its dilation (natural = hairline_target − 2 × dilation) and its
// contour's fade_exponent — so contours with different fade policies coexist
// in one merged path. dilation 0 (at-floor strokes, and text paths whose
// curves carry solver.w = 0) gives factor 1; fade_exponent 0 disables (Full
// policy).
fn hairline_fade_factor(dilation: f32, hairline_target: f32, fade_exponent: f32) -> f32 {
    if fade_exponent <= 0.0 || dilation <= 0.0 || hairline_target <= 0.0 {
        return 1.0;
    }
    let natural = max(hairline_target - 2.0 * dilation, 0.0);
    return pow(natural / hairline_target, fade_exponent);
}

// Per-curve hairline dilation. hairline_target is the minimum on-screen
// stroke width converted to design units (pixel_design_units ×
// uniforms.hairline_min_px); a curve whose contour stroke (solver.w) falls
// below it dilates by half the deficit. 0 for undilated contours.
fn curve_dilation(curve: CurveRecord, hairline_target: f32) -> f32 {
    if curve.solver.w <= 0.0 {
        return 0.0;
    }
    return max(0.0, (hairline_target - curve.solver.w) * 0.5);
}

// Run one curve in the nearest-silhouette race for its lane: adjusted =
// distance − that curve's dilation, so a more-dilated (thinner-stroke)
// curve's silhouette wins at the same raw distance. The winner also records
// the field gradient (normalized direction from its closest point to `point`)
// for the analytic AA band, and the faded lane records the winner's fade
// exponent.
// A nearer curve demotes the lane's current nearest to second; an intermediate
// curve replaces only the second. Dilation and the per-curve fade tracking stay
// tied to the nearest (the second is used for the line branch's corner band and
// the floor cap). The two lanes are inlined rather than routed through a
// pointer-to-member helper to keep to the member-assignment form.
fn accumulate_nearest(
    terms: ptr<function, CoverageTerms>,
    point: vec2<f32>,
    curve: CurveRecord,
    hairline_target: f32,
) {
    let dilation = curve_dilation(curve, hairline_target);
    let distance = curve_distance(point, curve);
    let adjusted = sqrt(distance.dist_sq) - dilation;
    let to_point = point - distance.closest;
    let to_point_len = length(to_point);
    let normal = select(vec2<f32>(1.0, 0.0), to_point / to_point_len, to_point_len > ROOT_EPSILON);
    // Outward edge-line normal from the contour winding, with the radial normal as
    // fallback when none was packed (text paths). edge_perp is the signed
    // perpendicular distance to that edge's line (outward positive): negative
    // inside the half-plane, so a straight run's far stroke side reads negative and
    // the corner clip stays a no-op there.
    let has_edge = dot(curve.edge_normal, curve.edge_normal) > 0.25;
    let edge_n = select(normal, curve.edge_normal, has_edge);
    let edge_p = dot(to_point, edge_n);
    if curve.fade_exponent > 0.0 {
        if adjusted < (*terms).faded.adjusted {
            (*terms).faded.adjusted2 = (*terms).faded.adjusted;
            (*terms).faded.normal2 = (*terms).faded.normal;
            (*terms).faded.edge_normal2 = (*terms).faded.edge_normal;
            (*terms).faded.edge_perp2 = (*terms).faded.edge_perp;
            (*terms).faded.adjusted = adjusted;
            (*terms).faded.dilation = dilation;
            (*terms).faded.normal = normal;
            (*terms).faded.edge_normal = edge_n;
            (*terms).faded.edge_perp = edge_p;
            (*terms).fade_exponent = curve.fade_exponent;
        } else if adjusted < (*terms).faded.adjusted2 {
            (*terms).faded.adjusted2 = adjusted;
            (*terms).faded.normal2 = normal;
            (*terms).faded.edge_normal2 = edge_n;
            (*terms).faded.edge_perp2 = edge_p;
        }
    } else {
        if adjusted < (*terms).exempt.adjusted {
            (*terms).exempt.adjusted2 = (*terms).exempt.adjusted;
            (*terms).exempt.normal2 = (*terms).exempt.normal;
            (*terms).exempt.edge_normal2 = (*terms).exempt.edge_normal;
            (*terms).exempt.edge_perp2 = (*terms).exempt.edge_perp;
            (*terms).exempt.adjusted = adjusted;
            (*terms).exempt.dilation = dilation;
            (*terms).exempt.normal = normal;
            (*terms).exempt.edge_normal = edge_n;
            (*terms).exempt.edge_perp = edge_p;
        } else if adjusted < (*terms).exempt.adjusted2 {
            (*terms).exempt.adjusted2 = adjusted;
            (*terms).exempt.normal2 = normal;
            (*terms).exempt.edge_normal2 = edge_n;
            (*terms).exempt.edge_perp2 = edge_p;
        }
    }
}

// One pass over the point's along-Y band: per-lane winding (the y-slab
// band holds every curve a +x ray can cross, so the winding is complete
// after this pass) plus nearest-distance candidates for curves whose AABB
// is within scan_width.
fn along_y_coverage_terms(
    point: vec2<f32>,
    scan_width: f32,
    scan_width_sq: f32,
    hairline_target: f32,
    path: PackedPathRecord,
) -> CoverageTerms {
    let include_winding = !outside_path_bounds(point, path);
    let along_y_band = bands[path.band_range.x + along_y_band_index(point, path)];
    var terms = empty_coverage_terms();
    for (var offset = 0u; offset < along_y_band.count; offset += 1u) {
        let curve = curves[along_y_band.start + offset];
        // Along-Y bands are sorted by descending max-x. Once the whole curve is
        // left of the AA scan window, all remaining curves are too.
        if curve.bounds.z < point.x - scan_width {
            break;
        }
        if include_winding {
            let winding = curve_winding(curve, point);
            if curve.fade_exponent > 0.0 {
                terms.faded.winding += winding;
            } else {
                terms.exempt.winding += winding;
            }
        }
        if curve_bounds_distance_sq(point, curve) <= scan_width_sq {
            accumulate_nearest(&terms, point, curve, hairline_target);
        }
    }
    return terms;
}

fn point_line_closest(point: vec2<f32>, start: vec2<f32>, end: vec2<f32>) -> vec2<f32> {
    let edge = end - start;
    let edge_length_squared = max(dot(edge, edge), ROOT_EPSILON);
    let t = clamp(dot(point - start, edge) / edge_length_squared, 0.0, 1.0);
    return start + edge * t;
}

fn curve_distance(point: vec2<f32>, curve: CurveRecord) -> CurveDistance {
    return exact_quadratic_distance(
        curve,
        point,
        curve.start_delta.xy,
        curve.start_delta.zw,
        curve.curve_end.xy,
        curve.curve_end.zw,
    );
}

fn curve_bounds_distance_sq(point: vec2<f32>, curve: CurveRecord) -> f32 {
    let nearest = clamp(point, curve.bounds.xy, curve.bounds.zw);
    let diff = point - nearest;
    return dot(diff, diff);
}

// Distance-only second pass over the point's along-X (x-slab) band: a
// curve above or below the point can sit outside its along-Y band yet
// inside the AA scan radius. Winding is already complete from the
// along-Y pass.
fn nearest_along_x_curve(
    point: vec2<f32>,
    scan_width: f32,
    scan_width_sq: f32,
    hairline_target: f32,
    path: PackedPathRecord,
    initial: CoverageTerms,
) -> CoverageTerms {
    let along_x_band = bands[path.band_range.z + along_x_band_index(point, path)];
    var terms = initial;
    for (var offset = 0u; offset < along_x_band.count; offset += 1u) {
        let curve = curves[along_x_band.start + offset];
        // Along-X bands are sorted by descending max-y. Once the whole curve is
        // below the AA scan window, all remaining curves are too.
        if curve.bounds.w < point.y - scan_width {
            break;
        }
        if curve_bounds_distance_sq(point, curve) <= scan_width_sq {
            accumulate_nearest(&terms, point, curve, hairline_target);
        }
    }
    return terms;
}

// Non-zero winding number at `point` over ALL curves (both lanes), using the
// point's along-Y band. Returns 0 outside the path bounds. The prepass
// silhouette test wants the union fill, not a per-lane one.
fn winding_at(point: vec2<f32>, path: PackedPathRecord) -> i32 {
    if outside_path_bounds(point, path) {
        return 0;
    }
    if is_polygon_mode(path) {
        // Polygon mode has no banded crossing set; the silhouette is the union
        // of the convex contours. Inside if the point lies inside every
        // half-plane of any one contour.
        let poly_start = path.band_range.x;
        let poly_count = path.band_range.y;
        for (var poly = 0u; poly < poly_count; poly += 1u) {
            let group = bands[poly_start + poly];
            var inside = group.count > 0u;
            for (var offset = 0u; offset < group.count; offset += 1u) {
                let edge = curves[group.start + offset];
                if dot(edge.edge_normal, edge.edge_normal) < 0.25 {
                    continue;
                }
                if dot(edge.edge_normal, point - edge.start_delta.xy) > 0.0 {
                    inside = false;
                    break;
                }
            }
            if inside {
                return 1;
            }
        }
        return 0;
    }
    let band = bands[path.band_range.x + along_y_band_index(point, path)];
    var winding = 0;
    for (var offset = 0u; offset < band.count; offset += 1u) {
        let curve = curves[band.start + offset];
        if curve.bounds.z < point.x {
            break;
        }
        winding += curve_winding(curve, point);
    }
    return winding;
}

// Per-lane winding number at `point` (x = exempt lane, y = faded lane);
// zero outside the path bounds.
fn lane_winding_at(point: vec2<f32>, path: PackedPathRecord) -> vec2<i32> {
    if outside_path_bounds(point, path) {
        return vec2<i32>(0, 0);
    }
    let band = bands[path.band_range.x + along_y_band_index(point, path)];
    var winding = vec2<i32>(0, 0);
    for (var offset = 0u; offset < band.count; offset += 1u) {
        let curve = curves[band.start + offset];
        if curve.bounds.z < point.x {
            break;
        }
        let curve_winding_value = curve_winding(curve, point);
        if curve.fade_exponent > 0.0 {
            winding.y += curve_winding_value;
        } else {
            winding.x += curve_winding_value;
        }
    }
    return winding;
}

// Whether any neighbor one filter-width away is outside the fill: x for the
// exempt lane, y for the whole-path union (a lane's winding components sum
// to the union's). A true outer silhouette has at least one outside
// neighbor; an interior edge of a self-intersecting / overlapping path
// (e.g. the EB Garamond `g` neck) is filled on both sides, so all neighbors
// stay inside.
fn lanes_any_outside_neighbor(point: vec2<f32>, edge_width: f32, path: PackedPathRecord) -> vec2<bool> {
    let right = lane_winding_at(point + vec2<f32>(edge_width, 0.0), path);
    let left = lane_winding_at(point - vec2<f32>(edge_width, 0.0), path);
    let up = lane_winding_at(point + vec2<f32>(0.0, edge_width), path);
    let down = lane_winding_at(point - vec2<f32>(0.0, edge_width), path);
    return vec2<bool>(
        right.x == 0 || left.x == 0 || up.x == 0 || down.x == 0,
        right.x + right.y == 0 || left.x + left.y == 0
            || up.x + up.y == 0 || down.x + down.y == 0,
    );
}

// Whether a lane's inside fragment sits within `edge_width` of one of its
// curves — the only case where the interior-edge suppression (the neighbor
// walk) matters.
fn lane_needs_neighbor_test(lane: LaneTerms, edge_width: f32) -> bool {
    return lane.winding != 0 && lane.adjusted <= edge_width;
}

// One lane's inside-POSITIVE smoothstep coverage. `no_outside` is the lane's
// interior-edge suppression: an inside fragment within edge_width of a curve
// sits either near the true outer silhouette (apply the AA ramp) or near an
// interior edge where two filled regions overlap (keep solid). The overlap
// case has no outside neighbor, so the down-ramp toward the submerged edge
// must be suppressed.
fn lane_coverage(lane: LaneTerms, edge_width: f32, no_outside: bool) -> f32 {
    let inside = lane.winding != 0;
    if lane.adjusted > edge_width {
        return select(0.0, 1.0, inside);
    }
    if inside && no_outside {
        return 1.0;
    }
    // This smoothstep ramp is inside-POSITIVE (unlike lane_signed_distance,
    // whose band_coverage consumer is inside-negative). Outside, -adjusted
    // = dilation - distance goes positive within the dilated halo; inside,
    // the dilated edge is farther out than the raw edge:
    // distance + dilation = adjusted + 2 * dilation.
    let signed_distance = select(-lane.adjusted, lane.adjusted + 2.0 * lane.dilation, inside);
    return smoothstep(-edge_width, edge_width, signed_distance);
}

// Interior-edge suppression flags (x = exempt lane, y = whole-path union),
// walking the neighbor windings once for both and only when some evaluation
// needs the test.
fn lanes_no_outside_neighbor(
    terms: CoverageTerms,
    union_terms: LaneTerms,
    point: vec2<f32>,
    edge_width: f32,
    path: PackedPathRecord,
) -> vec2<bool> {
    if lane_needs_neighbor_test(terms.exempt, edge_width)
        || lane_needs_neighbor_test(union_terms, edge_width) {
        let any_outside = lanes_any_outside_neighbor(point, edge_width, path);
        return vec2<bool>(!any_outside.x, !any_outside.y);
    }
    return vec2<bool>(false, false);
}

fn distance_coverage(
    point: vec2<f32>,
    pixel: vec2<f32>,
    dilation_max: f32,
    hairline_target: f32,
    path: PackedPathRecord,
) -> f32 {
    let edge_width = max(max(pixel.x, pixel.y) * EDGE_FILTER_WIDTH, ROOT_EPSILON);
    // The distance scan must reach the most-dilated silhouette plus the AA ramp.
    let scan_width = edge_width + dilation_max;
    let scan_width_sq = scan_width * scan_width;
    var terms = along_y_coverage_terms(point, scan_width, scan_width_sq, hairline_target, path);
    terms = nearest_along_x_curve(point, scan_width, scan_width_sq, hairline_target, path, terms);
    let union_terms = union_lane(terms);
    let no_outside = lanes_no_outside_neighbor(terms, union_terms, point, edge_width, path);

    // The fade factor interpolates between only-exempt-visible (factor 0)
    // and the whole unfaded path (factor 1, the pre-fade single-winding
    // evaluation). The junction where an exempt contour abuts a fading one
    // is union-interior, so no per-lane AA ramps meet there at half alpha.
    let exempt = lane_coverage(terms.exempt, edge_width, no_outside.x);
    let union_coverage = lane_coverage(union_terms, edge_width, no_outside.y);
    let fade = hairline_fade_factor(terms.faded.dilation, hairline_target, terms.fade_exponent);
    return mix(exempt, union_coverage, fade);
}

// One signed-distance evaluation plus the faded lane's winning dilation and
// fade exponent, so the aa_band consumers can apply the per-evaluation fade
// factor. sd.x is the exempt lane, sd.y the whole-path union. The two normals
// are each lane's winning field gradient, for the analytic line band.
struct SdSample {
    sd: vec2<f32>,
    dilation: f32,
    fade_exponent: f32,
    exempt_normal: vec2<f32>,
    union_normal: vec2<f32>,
    // Second-nearest silhouette per lane (raw adjusted distance + field gradient),
    // for the line branch's convex-corner correction. Sentinel 1e6 means none.
    exempt_adjusted2: f32,
    exempt_normal2: vec2<f32>,
    union_adjusted2: f32,
    union_normal2: vec2<f32>,
    // Second-nearest edge half-plane per lane (outward edge normal + signed
    // perpendicular distance), for the continuous convex-corner clip. Unlike the
    // radial normal2, this stays the true edge direction past a vertex.
    exempt_edge_normal2: vec2<f32>,
    exempt_edge_perp2: f32,
    union_edge_normal2: vec2<f32>,
    union_edge_perp2: f32,
}

// One lane's signed design-space distance to its per-curve-dilated
// silhouette: negative inside, positive outside, saturated to ±scan_width
// beyond the scan range. Interior overlaps are forced solidly negative (same
// case lane_coverage suppresses).
fn lane_signed_distance(lane: LaneTerms, scan_width: f32, no_outside: bool) -> f32 {
    let inside = lane.winding != 0;
    if lane.adjusted > scan_width {
        return select(scan_width, -scan_width, inside);
    }
    if inside && no_outside {
        return -scan_width;
    }
    return select(lane.adjusted, -(lane.adjusted + 2.0 * lane.dilation), inside);
}

// Exempt-lane and whole-path-union signed distances feeding the screen-space
// AA band used by aa_band mode.
fn signed_distance_sample(
    point: vec2<f32>,
    scan_width_sq: f32,
    hairline_target: f32,
    path: PackedPathRecord,
) -> SdSample {
    let scan_width = sqrt(scan_width_sq);
    var terms = along_y_coverage_terms(point, scan_width, scan_width_sq, hairline_target, path);
    terms = nearest_along_x_curve(point, scan_width, scan_width_sq, hairline_target, path, terms);
    let union_terms = union_lane(terms);
    let no_outside = lanes_no_outside_neighbor(terms, union_terms, point, scan_width, path);
    return SdSample(
        vec2<f32>(
            lane_signed_distance(terms.exempt, scan_width, no_outside.x),
            lane_signed_distance(union_terms, scan_width, no_outside.y),
        ),
        terms.faded.dilation,
        terms.fade_exponent,
        terms.exempt.normal,
        union_terms.normal,
        terms.exempt.adjusted2,
        terms.exempt.normal2,
        union_terms.adjusted2,
        union_terms.normal2,
        terms.exempt.edge_normal2,
        terms.exempt.edge_perp2,
        union_terms.edge_normal2,
        union_terms.edge_perp2,
    );
}

// Exempt/union signed distances for callers that need only the field values
// (ramp width finite differences).
fn signed_distance(
    point: vec2<f32>,
    scan_width_sq: f32,
    hairline_target: f32,
    path: PackedPathRecord,
) -> vec2<f32> {
    return signed_distance_sample(point, scan_width_sq, hairline_target, path).sd;
}

// Coverage from a signed distance and a screen-space band width: a 1px box ramp
// centered on the silhouette (sd 0). Negative sd (inside) → 1, positive → 0.
fn band_coverage(sd: f32, band: f32) -> f32 {
    return clamp(0.5 - sd / band, 0.0, 1.0);
}

// Screen-space gradient magnitude |Jᵀn| of the signed distance for a KNOWN
// field normal n (J = [dx | dy], the screen→design footprint Jacobian):
// project the two footprint vectors onto n and take the length. Because n is
// the analytic field gradient (from the nearest curve), not a finite
// difference, this is exact at any view angle — no large along-edge step to
// cross the medial-axis crease or overshoot the scan, so a straight thin line's
// AA ramp stays a true 1px box filter from head-on to edge-on.
fn analytic_band(normal: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> f32 {
    return max(length(vec2<f32>(dot(normal, dx), dot(normal, dy))), ROOT_EPSILON);
}

// Anisotropic supersample of the band coverage. A single band sample models the
// silhouette as a straight edge through the nearest point; at a convex corner the
// fill is a thin wedge, so under a foreshortened footprint the straight-edge model
// over-covers (the grazing-angle ghost wing). Stride N ~= the footprint anisotropy
// samples along the longer footprint axis to integrate across the corner; the
// well-resolved short axis stays a single sample.
//
// The per-sample ramp width is rebuilt from directional differences of the signed
// distance along the two footprint axes: the major contribution shrinks with N
// (the sub-sample spacing), the minor contribution stays full so edges whose
// normal is the well-resolved axis keep a ~1px ramp. signed_distance is
// 1-Lipschitz, so each difference is clamped to its step length; that rejects the
// spike when a finite-difference sample lands in a band where an axis-parallel
// edge isn't visible. Head-on the footprint is isotropic, N collapses to 1.
fn aniso_band_coverage(
    point: vec2<f32>,
    dx: vec2<f32>,
    dy: vec2<f32>,
    scan_width_sq: f32,
    hairline_target: f32,
    path: PackedPathRecord,
) -> f32 {
    let sd_center = signed_distance(point, scan_width_sq, hairline_target, path);
    let len_dx = length(dx);
    let len_dy = length(dy);
    let major = select(dy, dx, len_dx >= len_dy);
    let minor = select(dx, dy, len_dx >= len_dy);
    let major_len = max(len_dx, len_dy);
    let minor_len = max(min(len_dx, len_dy), ROOT_EPSILON);
    let sample_count = clamp(ceil(major_len / minor_len), 1.0, MAX_ANISO_SAMPLES_TEXT);
    let inv_count = 1.0 / sample_count;

    // Per-evaluation (exempt / union) band widths from signed-distance
    // differences.
    let d_major = min(abs(signed_distance(point + major, scan_width_sq, hairline_target, path) - sd_center), vec2<f32>(major_len));
    let d_minor = min(abs(signed_distance(point + minor, scan_width_sq, hairline_target, path) - sd_center), vec2<f32>(minor_len));
    let per_band = max(d_minor + d_major * inv_count, vec2<f32>(ROOT_EPSILON));

    // Fade applies per stride sample from that sample's faded-lane winning
    // curve, so adjacent samples that select different winning curves each
    // fade by their own curve's deficit and exponent.
    let count = u32(sample_count);
    var sum = 0.0;
    for (var index = 0u; index < count; index += 1u) {
        let stride = (f32(index) + 0.5) * inv_count - 0.5;
        let sample = signed_distance_sample(point + stride * major, scan_width_sq, hairline_target, path);
        let exempt = band_coverage(sample.sd.x, per_band.x);
        let union_coverage = band_coverage(sample.sd.y, per_band.y);
        let fade = hairline_fade_factor(sample.dilation, hairline_target, sample.fade_exponent);
        sum += mix(exempt, union_coverage, fade);
    }
    return sum * inv_count;
}

// Convex-corner correction for one strided line sample, one lane. The nearest
// edge's straight-edge band over-covers past a convex corner along that edge's
// extension (the grazing wing); the corner exterior is outside the second meeting
// edge too, so that edge's half-plane coverage clips the over-cover via min. The
// clip is CONTINUOUS in the second edge's signed half-plane distance: it
// saturates to 1 (no-op) where the point sits inside that edge (negative
// edge_perp2 — every straight run, where the second-nearest curve is the
// antiparallel far stroke side) and falls toward 0 only past the corner. The
// second edge's outward normal (edge_normal2) comes from the contour winding, not
// the radial normalize(point - vertex): past a convex vertex both edges' nearest
// point is the shared corner, so their radial normals coincide and a normal-angle
// gate would bail exactly along the wing. Gates that remain:
//   - outside only (sd1 > 0): inside the fill, a concave junction (tick meeting a
//     spine) is union-interior and must keep full coverage.
//   - a real second edge was scanned (adjusted2 below the 1e6 sentinel).
//   - the second edge sits near the same point (adjusted2 within ~2x sd1 plus the
//     well-resolved footprint): both edges' nearest point is the shared corner
//     vertex, so their distances track; an unrelated far edge is rejected.
// The second edge's band is its own full |Jᵀ edge_normal2| and the clip subtracts
// the same per-sample floor as the nearest edge. Returns the corrected coverage.
fn corner_coverage(
    base_cov: f32,
    sd1: f32,
    adjusted2: f32,
    edge_normal2: vec2<f32>,
    edge_perp2: f32,
    dilation: f32,
    dx: vec2<f32>,
    dy: vec2<f32>,
    norm_extent: f32,
) -> f32 {
    if sd1 <= 0.0 || adjusted2 > 100000.0 {
        return base_cov;
    }
    if adjusted2 > 2.0 * sd1 + norm_extent {
        return base_cov;
    }
    let band2 = analytic_band(edge_normal2, dx, dy);
    let cov2 = band_coverage(edge_perp2 - dilation, band2);
    return min(base_cov, cov2);
}

// Hairline floor as a signed-distance offset for one line sample, one lane. The
// floor pads a sub-pixel stroke to hairline_min_px screen px so a foreshortened
// receding line stays a continuous hairline instead of aliasing into dots. It is
// sized from the EDGE cross-stroke normal's full screen-space band |Jᵀn|: on a
// straight edge that band is the true cross-stroke screen width, so the floor
// keeps the line continuous without rounding it. The hazard is the convex vertex,
// where the nearest point is the shared corner so the field normal goes RADIAL
// (aligned with the foreshortened major axis), |Jᵀn| blows up, and the isotropic
// SD offset rounds the corner outward into a wing. When a second edge is close,
// the relation of the two field normals says which case it is:
//   - antiparallel (dot ~ -1): the two sides of a genuine thin stroke -> keep the
//     full floor (this is exactly where the line needs it).
//   - separated (|cross| high, dot ~ 0): a real corner of two distinct edges ->
//     cap with the smaller edge's floor (the perpendicular edge is well-resolved,
//     so its floor is small) -> the convex cap stays ~hairline_min_px.
//   - parallel (dot ~ +1): the vertex wedge, both edges report the same radial
//     normal -> cap to the well-resolved minor-axis floor so the radial |Jᵀn|
//     can't balloon the cap. |cross| alone can't see this case (it is ~0 for both
//     antiparallel stroke sides and parallel wedge normals); the dot sign does.
// All gates are smoothed and weighted by the second edge's closeness (w_dist) so
// nothing pops as fragments cross the neighborhood.
fn line_floor(
    nearest_normal: vec2<f32>,
    adjusted1: f32,
    second_normal: vec2<f32>,
    adjusted2: f32,
    min_feature: f32,
    ramp_band: f32,
    minor_len: f32,
    dx: vec2<f32>,
    dy: vec2<f32>,
) -> f32 {
    let b1 = analytic_band(nearest_normal, dx, dy);
    let d1 = max(0.0, (uniforms.hairline_min_px * b1 - min_feature) * 0.5);
    if adjusted2 > 100000.0 {
        return d1;
    }
    let b2 = analytic_band(second_normal, dx, dy);
    let d2 = max(0.0, (uniforms.hairline_min_px * b2 - min_feature) * 0.5);
    let d_minor = max(0.0, (uniforms.hairline_min_px * minor_len - min_feature) * 0.5);
    let r = max(ramp_band, max(d1, d2));
    let w_dist = 1.0 - smoothstep(r, 2.0 * r, adjusted2 - adjusted1);
    let separation = abs(nearest_normal.x * second_normal.y - nearest_normal.y * second_normal.x);
    let alignment = dot(nearest_normal, second_normal);
    let w_corner = w_dist * smoothstep(0.20, 0.45, separation);
    let w_vertex = w_dist * smoothstep(0.30, 0.70, alignment);
    var d = mix(d1, min(d1, d2), w_corner);
    d = mix(d, min(d, d_minor), w_vertex);
    return d;
}

// Polygon-mode sentinel: a line path (min_feature > 0) packed with no along-X
// band is a set of convex half-plane polygons (packing::build_packed_polygons),
// not a banded curve scan.
fn is_polygon_mode(path: PackedPathRecord) -> bool {
    return path.min_feature > 0.0 && path.band_range.w == 0u;
}

// Convex-polygon coverage for straight-edge line marks (Segment rectangles,
// Triangle arrowheads, Square/Diamond caps). Each contour is one convex polygon
// = the intersection of its edges' outward half-planes; coverage is the PRODUCT
// of per-edge 1px band ramps (band_coverage of the signed half-plane distance),
// unioned (max) across the path's contours. No radial point-distance field
// exists, so there is no rounded cap to bulge past a convex vertex — the
// grazing-corner wing the nearest-curve SDF produced is gone. Past a corner the
// point is outside two half-planes and the product collapses to the exact sharp
// corner; on a straight run every edge but the nearest saturates to 1, so the
// result is the single near-edge ramp (the wave-free tangent-stride look). Each
// edge keeps its own full |Jᵀn| band (analytic_band), so the no-band-clamp
// constraint holds. The exempt and faded fade lanes are unioned separately and
// mixed by the winning faded polygon's hairline fade factor (sized from that
// polygon's nearest, i.e. cross-stroke, edge), matching the curve path's fade
// cascade; an all-Full path has fade_exponent 0 everywhere, so fade is 1 and the
// result is the plain union.
fn analytic_polygon_coverage(point: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, path: PackedPathRecord) -> f32 {
    let poly_start = path.band_range.x;
    let poly_count = path.band_range.y;
    var exempt_cov = 0.0;
    var union_cov = 0.0;
    var faded_best = 0.0;
    var faded_floor = 0.0;
    var faded_band = ROOT_EPSILON;
    var faded_exponent = 0.0;
    for (var poly = 0u; poly < poly_count; poly += 1u) {
        let group = bands[poly_start + poly];
        if group.count == 0u {
            continue;
        }
        let fade_exponent = curves[group.start].fade_exponent;
        var poly_cov = 1.0;
        // Track the nearest edge (largest signed distance — least interior) for
        // the fade representative: for a fragment inside a thin stroke that is
        // the cross-stroke edge, whose band sizes the hairline fade target.
        var near_d = -1000000.0;
        var near_floor = 0.0;
        var near_band = ROOT_EPSILON;
        for (var offset = 0u; offset < group.count; offset += 1u) {
            let edge = curves[group.start + offset];
            let edge_normal = edge.edge_normal;
            // A degenerate (zero-length) edge packs a zero normal; skip it so it
            // does not multiply in a spurious 0.5 ramp.
            if dot(edge_normal, edge_normal) < 0.25 {
                continue;
            }
            let edge_point = edge.start_delta.xy;
            let band = analytic_band(edge_normal, dx, dy);
            // solver.w is this edge's perpendicular slab width (packing): a long
            // edge's stroke width (floored to 1px), a cap's stroke length (never
            // inflated), so the floor pads only the genuinely thin dimension.
            let floor = max(0.0, (uniforms.hairline_min_px * band - edge.solver.w) * 0.5);
            let signed = dot(edge_normal, point - edge_point);
            poly_cov *= band_coverage(signed - floor, band);
            if signed > near_d {
                near_d = signed;
                near_floor = floor;
                near_band = band;
            }
        }
        union_cov = max(union_cov, poly_cov);
        if fade_exponent > 0.0 {
            if poly_cov > faded_best {
                faded_best = poly_cov;
                faded_floor = near_floor;
                faded_band = near_band;
                faded_exponent = fade_exponent;
            }
        } else {
            exempt_cov = max(exempt_cov, poly_cov);
        }
    }
    let fade = hairline_fade_factor(faded_floor, uniforms.hairline_min_px * faded_band, faded_exponent);
    return mix(exempt_cov, union_cov, fade);
}

// Anisotropically supersampled analytic line coverage. A single analytic-band
// evaluation is exact for a straight edge but over-covers a convex corner (the
// grazing-angle ghost wing) and, on a hairline floored to a single screen pixel,
// samples the AA ramp off the sub-pixel line center so coverage oscillates along
// the foreshortened length.
//
// The stride walks the EDGE TANGENT, not the screen-major footprint axis. The
// center fragment's field normal fixes the edge frame; the tangent is its
// perpendicular. Walking the major axis slides each sample sideways off a tilted
// edge by sin(tilt) * step, so the nearest-curve solve jumps along the length and
// coverage waves at grazing. Walking the tangent keeps every sample ON the edge:
// a straight edge's signed distance is constant along it, so the N samples agree
// and the average is the single-sample band — no wave. At a convex corner the
// samples sweep toward the corner and corner_coverage clips the second edge.
//
// Because the stride carries no normal offset, the per-sample ramp band is the
// FULL cross-stroke |Jᵀn| (analytic_band), not narrowed to the stride spacing —
// the cross-stroke AA is carried entirely by that band width. The hairline floor
// is sized per sample from the edge cross-stroke normal's full band (line_floor),
// not the radial union normal that balloons at a convex vertex, and capped
// against the second edge at a corner, so a foreshortened straight edge stays a
// continuous sub-pixel hairline while the convex cap stays bounded to
// ~hairline_min_px. Head-on the footprint is isotropic and N collapses to one.
fn analytic_line_coverage(point: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, path: PackedPathRecord) -> f32 {
    let footprint_max = max(length(dx), length(dy));
    let scan_width = footprint_max * (EDGE_FILTER_WIDTH + uniforms.hairline_min_px) + ROOT_EPSILON;
    let scan_width_sq = scan_width * scan_width;

    // Edge frame from the center fragment's field normal: the tangent is the
    // stride direction, the normal sets the band width and the sub-sample count.
    let center = signed_distance_sample(point, scan_width_sq, 0.0, path);
    let normal = center.union_normal;
    let tangent = vec2<f32>(-normal.y, normal.x);
    // Footprint parallelogram support widths in that frame: extent along the
    // tangent (what the stride tiles) and across the normal (|Jᵀn|, the
    // cross-stroke band and the sub-sample spacing).
    let tan_extent = abs(dot(dx, tangent)) + abs(dot(dy, tangent));
    let norm_extent = analytic_band(normal, dx, dy);
    let sample_count = clamp(ceil(tan_extent / norm_extent), 1.0, MAX_ANISO_SAMPLES);
    let inv_count = 1.0 / sample_count;
    let count = u32(sample_count);
    let tangent_step = tangent * tan_extent;

    // Hairline floor sized ONCE from the center fragment, reused across the
    // stride. Per-sample sizing lets each sample's rotating radial normal at a
    // convex vertex re-balloon the floor, and the union over the stride paints
    // that into a multi-pixel wing; sizing it once from the center bounds the
    // wing to ~hairline_min_px (coverage_probe::center_dilation_bounds_corner_wing,
    // mode 2 vs mode 1). The straight-edge case is unaffected — along an edge the
    // center normal IS the edge normal, so the floor matches the per-sample value.
    let exempt_floor = line_floor(
        center.exempt_normal, abs(center.sd.x), center.exempt_normal2, center.exempt_adjusted2,
        path.min_feature, analytic_band(center.exempt_normal, dx, dy), norm_extent, dx, dy,
    );
    let union_floor = line_floor(
        center.union_normal, abs(center.sd.y), center.union_normal2, center.union_adjusted2,
        path.min_feature, analytic_band(center.union_normal, dx, dy), norm_extent, dx, dy,
    );

    var sum = 0.0;
    var fired = 0.0;
    for (var index = 0u; index < count; index += 1u) {
        let stride = (f32(index) + 0.5) * inv_count - 0.5;
        let raw = signed_distance_sample(point + stride * tangent_step, scan_width_sq, 0.0, path);
        // Full cross-stroke band per sample: the stride is pure tangent, so the
        // samples carry no normal offset to tile against and the band stays the
        // true 1px |Jᵀn| ramp width.
        let exempt_band = analytic_band(raw.exempt_normal, dx, dy);
        let union_band = analytic_band(raw.union_normal, dx, dy);
        let exempt_raw = band_coverage(raw.sd.x - exempt_floor, exempt_band);
        let union_raw = band_coverage(raw.sd.y - union_floor, union_band);
        // Clip each lane's straight-edge over-cover against its second-nearest edge
        // half-plane at a convex corner (gated inside corner_coverage).
        let exempt = corner_coverage(
            exempt_raw, raw.sd.x, raw.exempt_adjusted2, raw.exempt_edge_normal2,
            raw.exempt_edge_perp2, exempt_floor, dx, dy, norm_extent,
        );
        let union_coverage = corner_coverage(
            union_raw, raw.sd.y, raw.union_adjusted2, raw.union_edge_normal2,
            raw.union_edge_perp2, union_floor, dx, dy, norm_extent,
        );
        fired = max(fired, max(exempt_raw - exempt, union_raw - union_coverage));
        let fade_target = uniforms.hairline_min_px * analytic_band(raw.union_normal, dx, dy);
        let fade = hairline_fade_factor(union_floor, fade_target, raw.fade_exponent);
        sum += mix(exempt, union_coverage, fade);
    }
    let coverage = sum * inv_count;
    if CORNER_DEBUG_MASK {
        // Dim line everywhere, bright where the corner clip removed coverage, so
        // the firing region can be checked to be exterior corners only.
        return clamp(coverage * 0.2 + fired * 4.0, 0.0, 1.0);
    }
    return coverage;
}

fn apply_text_coverage_bias(coverage: f32, bias: f32) -> f32 {
    let clamped = clamp(coverage, 0.0, 1.0);
    if bias > 0.0 {
        return 1.0 - pow(1.0 - clamped, 1.0 + bias);
    }
    if bias < 0.0 {
        return pow(clamped, 1.0 - bias);
    }
    return clamped;
}

// Length in screen pixels of a design-space vector along +Y. dx/dy form the
// Jacobian from screen pixels to design space; its inverse maps the requested
// design-space em vector back into screen space. This captures perspective,
// tilt, non-uniform scale, and window scale factor without CPU camera state.
fn projected_design_y_px(design_height: f32, dx: vec2<f32>, dy: vec2<f32>) -> f32 {
    let determinant = dx.x * dy.y - dy.x * dx.y;
    let inverse_y_column = vec2<f32>(-dy.x, dx.x) / max(abs(determinant), ROOT_EPSILON);
    return abs(design_height) * length(inverse_y_column);
}

fn automatic_text_coverage_bias(projected_em_px: f32) -> f32 {
    if projected_em_px <= 0.0 {
        return 0.0;
    }
    let resolved = smoothstep(
        AUTO_TEXT_BIAS_FULL_BELOW_PX,
        AUTO_TEXT_BIAS_NONE_ABOVE_PX,
        projected_em_px,
    );
    return (1.0 - resolved) * AUTO_TEXT_BIAS_MAX;
}

fn render_coverage(
    uv: vec2<f32>,
    path: PackedPathRecord,
    render_mode: u32,
    aa_flags: u32,
    text_coverage_bias: f32,
    text_em_design_height: f32,
) -> f32 {
    // Derivatives stay at the top, BEFORE any branch: aa_flags is per-run data
    // recovered from an interpolated varying, so the branches below are
    // non-uniform control flow where fwidth/dpdx/dpdy are undefined in WGSL.
    // Every derivative this function needs is computed here; in-branch ramp
    // widths are rebuilt from finite differences along dx/dy instead.
    let point = design_position(uv, path);
    let pixel = max(abs(fwidth(point)), vec2<f32>(ROOT_EPSILON));
    let dx = dpdx(point);
    let dy = dpdy(point);

    // The finite-difference branches below only run for text (min_feature == 0),
    // which never dilates, so the hairline floor is zero here. Lines
    // (min_feature > 0) take the analytic branch, which sizes its own dilation
    // from the analytic band.
    let dilation_max = 0.0;
    let hairline_target = 0.0;

    // Lines (min_feature > 0) use the analytic-gradient band (|Jᵀn|) with an
    // analytic hairline floor, independent of the aa_flags AA mode, and are
    // anisotropically supersampled across the footprint (the equivalent of the
    // text Both mode, always on for lines): stride samples along the longer
    // footprint axis integrate the sub-pixel hairline and the convex corner that
    // a single center sample misses. Text (min_feature == 0) takes the
    // aa_flags-selected finite-difference path below.
    var coverage: f32;
    if path.min_feature > 0.0 {
        if is_polygon_mode(path) {
            // Straight-edge convex marks: per-contour half-plane product, no
            // radial field, no grazing-corner wing.
            coverage = analytic_polygon_coverage(point, dx, dy, path);
        } else {
            // Curved line contour (a Circle cap's ellipse): the banded
            // curve-distance scan, which has no sharp convex corner to wing.
            coverage = analytic_line_coverage(point, dx, dy, path);
        }
    } else if (aa_flags & AA_FLAG_BAND) != 0u {
        let edge_width = max(max(pixel.x, pixel.y) * EDGE_FILTER_WIDTH, ROOT_EPSILON);
        let scan_width = edge_width + dilation_max;
        let scan_width_sq = scan_width * scan_width;
        if (aa_flags & AA_FLAG_SUPERSAMPLE) != 0u {
            // Supersampled screen-space band: the band sets the cross-stroke
            // edge width (full interior coverage, so the stroke stays bright)
            // and the stride samples integrate along the foreshortened axis at
            // grazing. Text reaches here as the Both AA mode.
            coverage = aniso_band_coverage(point, dx, dy, scan_width_sq, hairline_target, path);
        } else {
            // Single sample: one full-footprint exempt/union band from the
            // center sample's screen-space distance change. Forward
            // differences along dx/dy stand in for fwidth(sd), which is
            // unavailable here — this branch is non-uniform flow (aa_flags is
            // per-run).
            let center = signed_distance_sample(point, scan_width_sq, hairline_target, path);
            let band = max(
                abs(signed_distance(point + dx, scan_width_sq, hairline_target, path) - center.sd)
                    + abs(signed_distance(point + dy, scan_width_sq, hairline_target, path) - center.sd),
                vec2<f32>(ROOT_EPSILON),
            );
            let exempt = band_coverage(center.sd.x, band.x);
            let union_coverage = band_coverage(center.sd.y, band.y);
            let fade = hairline_fade_factor(center.dilation, hairline_target, center.fade_exponent);
            coverage = mix(exempt, union_coverage, fade);
        }
    } else if (aa_flags & AA_FLAG_SUPERSAMPLE) != 0u {
        // Scalar band, four rotated-grid footprint samples (the original path). At
        // grazing angles dx/dy stretch along the foreshortened axis, so the
        // samples integrate the coverage strip a single sample cannot capture.
        var sum = 0.0;
        sum += distance_coverage(point + 0.375 * dx + 0.125 * dy, pixel, dilation_max, hairline_target, path);
        sum += distance_coverage(point - 0.125 * dx + 0.375 * dy, pixel, dilation_max, hairline_target, path);
        sum += distance_coverage(point - 0.375 * dx - 0.125 * dy, pixel, dilation_max, hairline_target, path);
        sum += distance_coverage(point + 0.125 * dx - 0.375 * dy, pixel, dilation_max, hairline_target, path);
        coverage = sum * 0.25;
    } else {
        coverage = distance_coverage(point, pixel, dilation_max, hairline_target, path);
    }

    if path.min_feature == 0.0 {
        let projected_em_px = projected_design_y_px(text_em_design_height, dx, dy);
        let effective_bias = text_coverage_bias + automatic_text_coverage_bias(projected_em_px);
        coverage = apply_text_coverage_bias(coverage, effective_bias);
    }

    if render_mode == RENDER_MODE_PUNCH_OUT {
        return 1.0 - coverage;
    }
    return coverage;
}

#ifdef PREPASS_PIPELINE
@fragment
fn fragment(in: VertexOutput) {
#ifdef VERTEX_UVS_A
#ifdef VERTEX_UVS_B
    // The shadow map stores a binary silhouette, so one winding test answers
    // it: keep fragments inside the outline (punch-out runs invert the test).
    let path = path_records[path_index(in)];
    let point = design_position(coverage_uv(in), path);
    let inside = winding_at(point, path) != 0;
    if inside == (run_render_mode(in) == RENDER_MODE_PUNCH_OUT) {
        discard;
    }
#else
    discard;
#endif
#else
    discard;
#endif
}
#else
@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
#ifndef VERTEX_UVS_A
    discard;
#endif
#ifndef VERTEX_UVS_B
    discard;
#endif

    let path = path_records[path_index(in)];
    let path_uv = coverage_uv(in);
    // Diagnostic: paint line fragments with an upstream coverage input. The
    // derivatives are taken under the const-only gate (uniform control flow)
    // before the per-fragment min_feature branch.
    if LINE_DEBUG_MODE != 0 {
        let dbg_point = design_position(path_uv, path);
        let dbg_dx = dpdx(dbg_point);
        let dbg_dy = dpdy(dbg_point);
        if path.min_feature > 0.0 {
            let dbg_scan = max(length(dbg_dx), length(dbg_dy))
                * (EDGE_FILTER_WIDTH + uniforms.hairline_min_px) + ROOT_EPSILON;
            let dbg = signed_distance_sample(dbg_point, dbg_scan * dbg_scan, 0.0, path);
            var dbg_rgb: vec3<f32>;
            if LINE_DEBUG_MODE == 1 {
                dbg_rgb = vec3<f32>(dbg.union_normal * 0.5 + vec2<f32>(0.5), 0.5);
            } else if LINE_DEBUG_MODE == 2 {
                let dbg_band = analytic_band(dbg.union_normal, dbg_dx, dbg_dy);
                dbg_rgb = vec3<f32>(fract(dbg_band * LINE_DEBUG_SCALE));
            } else {
                dbg_rgb = vec3<f32>(fract(dbg.sd.y * LINE_DEBUG_SCALE));
            }
            var dbg_out: FragmentOutput;
            dbg_out.color = vec4<f32>(dbg_rgb, 1.0);
#ifdef OIT_ENABLED
            var dbg_pos = in.position;
            dbg_pos.z = max(dbg_pos.z + run_oit_depth_offset(in), OIT_MIN_DEPTH);
            oit_draw(dbg_pos, dbg_out.color);
            discard;
#endif
            return dbg_out;
        }
    }
    let coverage = render_coverage(
        path_uv,
        path,
        run_render_mode(in),
        run_aa_flags(in),
        run_text_coverage_bias(in),
        text_em_design_height(in, path),
    );
    let material_id = run_material_id(in);
    var pbr_input = pbr_input_from_material_table(
        in,
        is_front,
        material_id != INVALID_GPU_MATERIAL_SLOT,
        material_id,
    );
    // The vertex stage parks this quad's record index in `world_position.w`;
    // restore the homogeneous 1.0 so shadow sampling (which multiplies the full
    // vec4 by the light's `clip_from_world`) lands at the fragment's world
    // position instead of a per-record-displaced point.
#ifdef FRAGMENT_DATA_FROM_BATCHED_PATHS
    pbr_input.world_position.w = 1.0;
#endif
    let final_alpha = coverage * pbr_input.material.base_color.a;
    // This discard precedes oit_draw below, so faded near-zero fragments never
    // occupy OIT fragment-pool slots.
    if final_alpha < DISCARD_ALPHA {
        discard;
    }

    pbr_input.material.base_color.a = final_alpha;
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material,
        pbr_input.material.base_color,
    );

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);

#ifdef OIT_ENABLED
    let alpha_mode = pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    if alpha_mode != STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE {
        // Offset position.z so coplanar layers get distinct depths in the OIT
        // linked list; pipeline depth_bias does not affect in.position.z.
        var oit_pos = in.position;
        oit_pos.z = max(oit_pos.z + run_oit_depth_offset(in), OIT_MIN_DEPTH);
        oit_draw(oit_pos, out.color);
        discard;
    }
#endif

    return out;
}
#endif
