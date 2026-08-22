// Vertex-pulling stage for batched path text.
//
// The batch mesh is inert: capacity-sized zeroed vertices whose only job is
// switching the VERTEX_UVS_A / VERTEX_UVS_B pipeline defs on. Every visible
// value is pulled from two storage tables — one PathQuadRecord per path
// quad, one PathRenderRecord per text run — indexed from the vertex index. One file
// serves the main, prepass, and shadow pipelines: `#ifdef PREPASS_PIPELINE`
// picks the entry point, and both call the same `pull_vertex` helper so the
// expansion and the depth nudge cannot drift between passes.

#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::VertexOutput
#else
#import bevy_pbr::forward_io::VertexOutput
#endif

// Mirrors `PathQuadRecord` in `path/packing.rs` (std430, 64 B stride).
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

// Mirrors `PathRenderRecord` in `path/packing.rs` (std430, 96 B stride).
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

// Mirrors the leading fields of `PackedPathRecord` in packing.rs. Only `min_feature`
// is read here, to tell a panel-line quad (min_feature > 0, screen-space
// hairline coverage) from a text path quad (min_feature 0).
struct PackedPathRecord {
    bounds_min_size: vec4<f32>,
    band_range: vec4<u32>,
    min_feature: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<storage, read> path_records: array<PackedPathRecord>;

// Clip-space depth shift per clip_depth_nudge layer unit, applied post-projection
// so coplanar runs resolve to distinct depths.
const CLIP_DEPTH_NUDGE_PER_LAYER: f32 = 0.000002;

// Screen-space room, in device pixels, added around a panel-line quad for its
// coverage ramp. That ramp is screen-space (~1px) plus any hairline dilation;
// on a foreshortened panel its WORLD width grows ~1/cos(grazing), so a
// fixed-world CPU quad pad clips it at a tilt and the line renders the quad's
// hard edge as a staircase. The vertex stage expands each line corner so this
// many pixels of ramp always fit. Over-expansion only adds transparent
// fragments that discard before OIT.
const LINE_AA_MARGIN_PX: f32 = 8.0;
// Upper bound on the per-corner expansion, as a multiple of the quad's longer
// side, so a near-edge-on panel (pixels-per-unit -> 0) cannot grow the quad
// without bound.
const LINE_AA_MARGIN_MAX_RATIO: f32 = 4.0;
// Guards the reciprocals in the expansion math.
const LINE_AA_EPSILON: f32 = 0.000001;

struct PulledVertex {
    clip_position: vec4<f32>,
    world_position: vec4<f32>,
    world_normal: vec3<f32>,
    material_uv: vec2<f32>,
    coverage_uv: vec2<f32>,
}

#ifndef PREPASS_PIPELINE
// Panel-local distance whose projection along `world_axis` spans
// LINE_AA_MARGIN_PX device pixels at the corner whose clip position is `clip`.
// Uses the projection Jacobian d(ndc)/d(world): as the axis tilts toward
// edge-on its pixels-per-unit shrinks, so the returned margin grows with the
// grazing angle. `view` is the camera view (main pass only).
fn line_aa_margin_local(world_axis: vec3<f32>, clip: vec4<f32>) -> f32 {
    let d_clip = view.clip_from_world * vec4<f32>(world_axis, 0.0);
    let inv_w = 1.0 / clip.w;
    let d_ndc = (d_clip.xy - clip.xy * (d_clip.w * inv_w)) * inv_w;
    let px_per_unit = 0.5 * length(d_ndc * view.viewport.zw);
    return LINE_AA_MARGIN_PX / max(px_per_unit, LINE_AA_EPSILON);
}
#endif

// Expands one inert-mesh vertex into its path-quad corner. Both vertex entry
// points route through here, so the corner math and the depth nudge are one
// code path for the main, prepass, and shadow passes.
fn pull_vertex(vertex_index: u32, instance_index: u32) -> PulledVertex {
    var out: PulledVertex;

    // wgpu's vertex_index includes the draw's base_vertex, and the mesh
    // allocator packs meshes into shared slabs, so the slab base must come
    // off before deriving path and corner (same correction as
    // bevy_pbr's wireframe.wgsl).
    let local_index = vertex_index - mesh[instance_index].first_vertex_index;
    let path = local_index / 4u;
    let corner = local_index % 4u;

    // The index buffer is capacity-sized while the instance buffer is
    // live-sized: collapse the capacity tail to degenerate quads so
    // robustness clamping can never re-blend the last record.
    if path >= arrayLength(&instances) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        return out;
    }

    let record = instances[path];

    // Bevy compiles shaders without bounds checks, so a render_index outside the
    // run table would read arbitrary memory and emit a quad anywhere on
    // screen. Collapse such a record to a degenerate quad.
    if record.render_index >= arrayLength(&run_records) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        return out;
    }
    let run = run_records[record.render_index];

    // Corner order matches RunMeshBuilder::push_path:
    // 0 = (left, top), 1 = (right, top), 2 = (right, bottom), 3 = (left, bottom).
    let corner_x = f32(corner == 1u || corner == 2u);
    let corner_top = f32(corner <= 1u);
    var local = record.rect_min + vec2<f32>(corner_x, corner_top) * record.rect_size;
    // coverage_uv = padded path quad UVs (uv_min sits at the top-left corner).
    var coverage_uv = record.uv_min + vec2<f32>(corner_x, 1.0 - corner_top) * record.uv_size;
    let box_corner_x = select(corner_x, 1.0 - corner_x, record.box_uv_flip_x != 0u);
    let material_uv =
        record.box_uv_min + vec2<f32>(box_corner_x, 1.0 - corner_top) * record.box_uv_size;

#ifndef PREPASS_PIPELINE
    // Panel lines (min_feature > 0) only: grow the quad in the panel plane so
    // the screen-space coverage ramp clears the quad edge at any grazing angle
    // (see LINE_AA_MARGIN_PX). The margin is per panel-local axis, so the
    // foreshortened axis expands more; uv shifts by the same affine
    // rect->uv ratio, keeping the coverage field exact. Text is untouched.
    if path_records[record.packed_path_index].min_feature > 0.0 {
        let base_clip = position_world_to_clip((run.transform * vec4<f32>(local, 0.0, 1.0)).xyz);
        let axis_x = (run.transform * vec4<f32>(1.0, 0.0, 0.0, 0.0)).xyz;
        let axis_y = (run.transform * vec4<f32>(0.0, 1.0, 0.0, 0.0)).xyz;
        let cap = max(record.rect_size.x, record.rect_size.y) * LINE_AA_MARGIN_MAX_RATIO;
        let margin_x = min(line_aa_margin_local(axis_x, base_clip), cap);
        let margin_y = min(line_aa_margin_local(axis_y, base_clip), cap);
        let sign_x = corner_x * 2.0 - 1.0;
        let sign_y = corner_top * 2.0 - 1.0;
        local += vec2<f32>(sign_x * margin_x, sign_y * margin_y);
        let inv_rect = vec2<f32>(1.0, 1.0) / max(record.rect_size, vec2<f32>(LINE_AA_EPSILON));
        coverage_uv += vec2<f32>(
            sign_x * margin_x * record.uv_size.x * inv_rect.x,
            -sign_y * margin_y * record.uv_size.y * inv_rect.y,
        );
    }
#endif

    var world = run.transform * vec4<f32>(local, 0.0, 1.0);
    world.w = f32(path);
    out.world_position = world;

    var clip = position_world_to_clip(world.xyz);
#ifndef OIT_ENABLED
    clip.z += run.clip_depth_nudge * CLIP_DEPTH_NUDGE_PER_LAYER * clip.w;
#endif
    out.clip_position = clip;

    // Records carry no normal: rotate layout-space +z by the run transform.
    out.world_normal = normalize((run.transform * vec4<f32>(0.0, 0.0, 1.0, 0.0)).xyz);

    // `uv` is the material box UV consumed by Bevy PBR texture sampling.
    // `uv_b` is the analytic path coverage UV consumed by `analytic_path.wgsl`.
    out.material_uv = material_uv;
    out.coverage_uv = coverage_uv;
    return out;
}

#ifdef PREPASS_PIPELINE
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let pulled = pull_vertex(vertex_index, instance_index);
    var out: VertexOutput;
    out.position = pulled.clip_position;
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = pulled.clip_position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = pulled.material_uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = pulled.coverage_uv;
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = pulled.world_normal;
#endif
    out.world_position = pulled.world_position;
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = instance_index;
#endif
    return out;
}
#else
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let pulled = pull_vertex(vertex_index, instance_index);
    var out: VertexOutput;
    out.position = pulled.clip_position;
    out.world_position = pulled.world_position;
    out.world_normal = pulled.world_normal;
#ifdef VERTEX_UVS_A
    out.uv = pulled.material_uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = pulled.coverage_uv;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        instance_index, mesh_functions::get_world_from_local(instance_index)[3]);
#endif
    return out;
}
#endif
