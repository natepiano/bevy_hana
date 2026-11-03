import bevy_pbr::render::{
    mesh_bindings::mesh,
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
};
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::VertexOutput;
@if(!PREPASS_PIPELINE)
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
    pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_UNLIT_BIT},
};
@if(!PREPASS_PIPELINE && SDF_PBR_LIGHTING)
import bevy_pbr::render::pbr_functions::apply_pbr_lighting;
@if(MATERIAL_OIT_ENABLED)
import bevy_pbr::render::pbr_types::{
    STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS,
    STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE,
    STANDARD_MATERIAL_FLAGS_ALPHA_MODE_BLEND,
};
import hana_diegetic::sdf_material_table::pbr_input_from_material_table;
import hana_diegetic::material_table::INVALID_GPU_MATERIAL_SLOT;
import hana_diegetic::sdf_stroke::{
    centered_stroke_alpha,
    distance_field_band,
    inflate_subpixel_half_size,
};
@if(MATERIAL_OIT_ENABLED)
import bevy_core_pipeline::oit::draw::{oit_draw_layer, oit_plane_depth};
@if(MATERIAL_OIT_ENABLED)
import bevy_pbr::prepass::utils as prepass_utils;

@if(MATERIAL_OIT_ENABLED)
// One layer step permits a fragment on its own opaque surface.
const OIT_OCCLUSION_TOLERANCE: f32 = 0.0000001192092967;

const CLIP_DEPTH_NUDGE_PER_LAYER: f32 = 0.00000002;
const SDF_PAINT_FILL: u32 = 1u;
const SDF_PAINT_BORDER: u32 = 2u;
const TOOLTIP_DEPTH_PRIORITY_FLAG: u32 = 1u;
const SDF_PAINT_INSIDE_BORDER_FLAG: u32 = 2u;
const SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG: u32 = 4u;
const TOOLTIP_KEEP_SNAPPED_EDGES_INSIDE_FLAG: u32 = 8u;
const TOOLTIP_NDC_DEPTH_PRIORITY: f32 = 0.0001;

@if(SDF_STRIPPED_MATERIAL_GROUP) {
@if(PREPASS_PIPELINE) {
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    _ = vertex_index;
    var out: VertexOutput;
    out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = out.position.z;
}
@if(VERTEX_UVS_A) {
    out.uv = vec2<f32>(0.0);
}
@if(VERTEX_UVS_B) {
    out.uv_b = vec2<f32>(0.0);
}
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
}
    out.world_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
@if(VERTEX_OUTPUT_INSTANCE_INDEX) {
    out.instance_index = instance_index;
}
@if(!VERTEX_OUTPUT_INSTANCE_INDEX) {
    _ = instance_index;
}
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) {
    _ = in;
    _ = is_front;
    discard;
}
}
@if(!PREPASS_PIPELINE) {
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    _ = vertex_index;
    var out: VertexOutput;
    out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.world_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
@if(VERTEX_UVS_A) {
    out.uv = vec2<f32>(0.0);
}
@if(VERTEX_UVS_B) {
    out.uv_b = vec2<f32>(0.0);
}
@if(VERTEX_OUTPUT_INSTANCE_INDEX) {
    out.instance_index = instance_index;
}
@if(!VERTEX_OUTPUT_INSTANCE_INDEX) {
    _ = instance_index;
}
@if(VISIBILITY_RANGE_DITHER) {
    out.visibility_range_dither = 0;
}
    return out;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    _ = in;
    _ = is_front;
    discard;
    var out: FragmentOutput;
    out.color = vec4<f32>(0.0);
    return out;
}
}
}
@if(!SDF_STRIPPED_MATERIAL_GROUP) {

struct SdfRenderRecord {
    transform: mat4x4<f32>,
    panel_plane: vec4<f32>,
    half_size: vec2<f32>,
    mesh_half_size: vec2<f32>,
    corner_radii: vec4<f32>,
    border_widths: vec4<f32>,
    clip_rect: vec4<f32>,
    fill_material: u32,
    border_material: u32,
    paint_mask: u32,
    flags: u32,
    clip_depth_nudge: f32,
    oit_paint_order: f32,
    // Tooltip shadow (blur support, downward offset) in local record space.
    tooltip_shadow: vec2<f32>,
}

struct SdfMeshRecord {
    reserved: vec4<u32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(107) var<storage, read> sdf_records: array<SdfRenderRecord>;
@group(constants::MATERIAL_BIND_GROUP) @binding(108) var<storage, read> sdf_mesh_records: array<SdfMeshRecord>;
@group(constants::MATERIAL_BIND_GROUP) @binding(109) var display_table: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(110) var display_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(111) var<uniform> device_pixels_per_logical_pixel: f32;

fn compensated_unlit_rgb(color: vec3<f32>) -> vec3<f32> {
    let size = vec3<f32>(textureDimensions(display_table));
    let linear = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let encoded = select(1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055,
        12.92 * linear, linear <= vec3<f32>(0.0031308));
    let uv = (encoded * (size - 1.0) + 0.5) / size;
    return textureSampleLevel(display_table, display_sampler, uv, 0.0).rgb;
}

struct PulledSdfVertex {
    clip_position: vec4<f32>,
    world_position: vec4<f32>,
    world_normal: vec3<f32>,
    box_uv: vec2<f32>,
    record_index: u32,
}

fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let r = select(radii.xw, radii.yz, p.x > 0.0);
    let radius = select(r.x, r.y, p.y > 0.0);
    let q = abs(p) - half_size + radius;
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - radius;
}

fn inner_half_size(record: SdfRenderRecord, border_widths: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(
        record.half_size.x - 0.5 * (border_widths.y + border_widths.w),
        record.half_size.y - 0.5 * (border_widths.x + border_widths.z),
    );
}

fn border_center_offset(border_widths: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(
        0.5 * (border_widths.w - border_widths.y),
        0.5 * (border_widths.x - border_widths.z),
    );
}

fn inner_corner_radii(record: SdfRenderRecord, border_widths: vec4<f32>) -> vec4<f32> {
    return max(
        vec4(0.0),
        vec4<f32>(
            record.corner_radii.x - min(border_widths.x, border_widths.w),
            record.corner_radii.y - min(border_widths.x, border_widths.y),
            record.corner_radii.z - min(border_widths.z, border_widths.y),
            record.corner_radii.w - min(border_widths.z, border_widths.w),
        ),
    );
}

fn aa_width(dist: f32) -> f32 {
    return distance_field_band(dist) * 0.75;
}

// Half-width of the ramp across one edge. Painting inside the border centres
// a one-pixel ramp on the edge, so an edge on the pixel grid fills the pixel
// inside it and leaves the next one alone: a 1 px frame draws in one pixel.
fn edge_aa_width(dist: f32, paint_inside: bool) -> f32 {
    return select(aa_width(dist), 0.5 * distance_field_band(dist), paint_inside);
}

// Move both outer boundaries to physical pixel boundaries. The derivative
// converts their projected displacement back into panel-local coordinates.
fn snapped_edge_axis(
    screen_position: f32,
    local: f32,
    half_size: f32,
    local_per_screen_pixel: f32,
    keep_inside: bool,
) -> vec2<f32> {
    if abs(local_per_screen_pixel) < 0.000001 {
        return vec2(0.0, half_size);
    }
    let near_screen = screen_position
        + (-half_size - local) / local_per_screen_pixel;
    let far_screen = screen_position
        + (half_size - local) / local_per_screen_pixel;
    let reversed = local_per_screen_pixel < 0.0;
    let inward_near_grid = select(ceil(near_screen), floor(near_screen), reversed);
    let inward_far_grid = select(floor(far_screen), ceil(far_screen), reversed);
    let near_grid = select(round(near_screen), inward_near_grid, keep_inside);
    let far_grid = select(round(far_screen), inward_far_grid, keep_inside);
    let near_shift = (near_grid - near_screen) * local_per_screen_pixel;
    let far_shift = (far_grid - far_screen) * local_per_screen_pixel;
    return vec2(
        0.5 * (near_shift + far_shift),
        half_size + 0.5 * (far_shift - near_shift),
    );
}

fn role_present(mask: u32, role_bit: u32) -> bool {
    return (mask & role_bit) != 0u;
}

fn pull_sdf_vertex(vertex_index: u32, instance_index: u32) -> PulledSdfVertex {
    var out: PulledSdfVertex;
    out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.world_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
    out.box_uv = vec2<f32>(0.0);
    out.record_index = 4294967295u;

    // Keep binding 108 referenced for layout parity without loading the
    // runtime-sized array (a whole-array load fails naga validation).
    _ = arrayLength(&sdf_mesh_records);

    let local_index = vertex_index - mesh[instance_index].first_vertex_index;
    let record_index = local_index / 4u;
    let corner = local_index % 4u;

    if record_index >= arrayLength(&sdf_records) {
        return out;
    }
    let record = sdf_records[record_index];
    if record.mesh_half_size.x <= 0.0 || record.mesh_half_size.y <= 0.0 {
        return out;
    }
    if record.paint_mask == 0u {
        return out;
    }

    let corner_x = f32(corner == 1u || corner == 2u);
    let corner_top = f32(corner <= 1u);
    let signs = vec2<f32>(corner_x * 2.0 - 1.0, corner_top * 2.0 - 1.0);
    let local = signs * record.mesh_half_size;
    let box_uv = vec2<f32>(corner_x, 1.0 - corner_top);

    let world = record.transform * vec4<f32>(local, 0.0, 1.0);
    var clip = position_world_to_clip(world.xyz);
@if(!MATERIAL_OIT_ENABLED) {
    clip.z += record.clip_depth_nudge * CLIP_DEPTH_NUDGE_PER_LAYER * clip.w;
}
@if(MATERIAL_OIT_ENABLED) {
    if (record.flags & TOOLTIP_DEPTH_PRIORITY_FLAG) != 0u {
        clip.z += TOOLTIP_NDC_DEPTH_PRIORITY * clip.w;
    }
}

    out.clip_position = clip;
    out.world_position = world;
    out.world_normal = normalize((record.transform * vec4<f32>(0.0, 0.0, 1.0, 0.0)).xyz);
    out.box_uv = box_uv;
    out.record_index = record_index;
    return out;
}

@if(PREPASS_PIPELINE) {
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let pulled = pull_sdf_vertex(vertex_index, instance_index);
    var out: VertexOutput;
    out.position = pulled.clip_position;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = pulled.clip_position.z;
    out.position.z = min(out.position.z, 1.0);
}
@if(VERTEX_UVS_A) {
    out.uv = pulled.box_uv;
}
@if(VERTEX_UVS_B) {
    out.uv_b = vec2<f32>(f32(pulled.record_index), 0.0);
}
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
    out.world_normal = pulled.world_normal;
}
    out.world_position = pulled.world_position;
@if(VERTEX_OUTPUT_INSTANCE_INDEX) {
    out.instance_index = instance_index;
}
    return out;
}
}
@if(!PREPASS_PIPELINE) {
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let pulled = pull_sdf_vertex(vertex_index, instance_index);
    var out: VertexOutput;
    out.position = pulled.clip_position;
    out.world_position = pulled.world_position;
    out.world_normal = pulled.world_normal;
@if(VERTEX_UVS_A) {
    out.uv = pulled.box_uv;
}
@if(VERTEX_UVS_B) {
    out.uv_b = vec2<f32>(f32(pulled.record_index), 0.0);
}
@if(VERTEX_OUTPUT_INSTANCE_INDEX) {
    out.instance_index = instance_index;
}
@if(VISIBILITY_RANGE_DITHER) {
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        instance_index, mesh_functions::get_world_from_local(instance_index)[3]);
}
    return out;
}
}

fn record_index_from_vertex_output(in: VertexOutput) -> u32 {
@if(VERTEX_UVS_B) {
    return u32(floor(in.uv_b.x + 0.5));
}
@if(!VERTEX_UVS_B) {
    return 4294967295u;
}
}

fn local_from_vertex_output(in: VertexOutput, record: SdfRenderRecord) -> vec2<f32> {
@if(VERTEX_UVS_A) {
    return (in.uv - 0.5) * 2.0 * record.mesh_half_size;
}
@if(!VERTEX_UVS_A) {
    return vec2<f32>(0.0);
}
}

@if(MATERIAL_OIT_ENABLED) {
// Rasterizers snap each quad's projected vertices to subpixel coordinates.
// Their independently interpolated depths can disagree by more than a packed
// OIT quantum even when the quads share a plane. Intersect the pixel's clip
// ray with the common panel plane so only the authored layer offset orders it.
fn panel_plane_depth(position: vec4<f32>, record: SdfRenderRecord) -> f32 {
    var depth = oit_plane_depth(position, record.panel_plane);
    if (record.flags & TOOLTIP_DEPTH_PRIORITY_FLAG) != 0u {
        depth += TOOLTIP_NDC_DEPTH_PRIORITY;
    }
    return depth;
}
}

fn fill_alpha_for_prepass(in: VertexOutput, is_front: bool, record: SdfRenderRecord) -> f32 {
    let pbr_input = pbr_input_from_material_table(
        in,
        is_front,
        role_present(record.paint_mask, SDF_PAINT_FILL),
        record.fill_material,
    );
    return pbr_input.material.base_color.a;
}

fn resolved_paint_clip_rect(
    record: SdfRenderRecord,
    view_aligned_edge_snap: bool,
) -> vec4<f32> {
    let edge_snap_border_clip = view_aligned_edge_snap
        && role_present(record.paint_mask, SDF_PAINT_BORDER);
    let expanded_border_clip = record.clip_rect
        + vec4<f32>(
            -record.border_widths.w,
            -record.border_widths.z,
            record.border_widths.y,
            record.border_widths.x,
        );
    return select(record.clip_rect, expanded_border_clip, edge_snap_border_clip);
}

@if(PREPASS_PIPELINE) {
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) {
    let record_index = record_index_from_vertex_output(in);
    if record_index >= arrayLength(&sdf_records) {
        discard;
    }
    let record = sdf_records[record_index];
    if record.paint_mask == 0u {
        discard;
    }
@if(SDF_TOOLTIP_SHADOW) {
    discard;
}

    let local = local_from_vertex_output(in, record);
    let view_aligned_edge_snap = (record.flags & SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG) != 0u;
    let clip_rect = resolved_paint_clip_rect(record, view_aligned_edge_snap);
    if local.x < clip_rect.x || local.x > clip_rect.z
        || local.y < clip_rect.y || local.y > clip_rect.w {
        discard;
    }

    let dist = sd_rounded_box(local, record.half_size, record.corner_radii);
    if dist > 0.0 {
        discard;
    }

    let has_border = role_present(record.paint_mask, SDF_PAINT_BORDER)
        && record.border_material != INVALID_GPU_MATERIAL_SLOT
        && (record.border_widths.x > 0.0
            || record.border_widths.y > 0.0
            || record.border_widths.z > 0.0
            || record.border_widths.w > 0.0);
    let fill_alpha = fill_alpha_for_prepass(in, is_front, record);
    let has_fill = fill_alpha > 0.001;

    if has_border && !has_fill {
        let pixel_size = vec2<f32>(fwidth(local.x), fwidth(local.y));
        let min_shadow_widths = vec4<f32>(pixel_size.y, pixel_size.x, pixel_size.y, pixel_size.x);
        let shadow_border_widths = select(
            vec4<f32>(0.0),
            max(record.border_widths, min_shadow_widths),
            record.border_widths > vec4<f32>(0.0),
        );
        let inner_hs = inner_half_size(record, shadow_border_widths);
        let inner_offset = border_center_offset(shadow_border_widths);
        let inner_radii = inner_corner_radii(record, shadow_border_widths);
        let inner_dist = sd_rounded_box(local - inner_offset, max(inner_hs, vec2(0.0)), inner_radii);
        if inner_dist <= 0.0 {
            discard;
        }
    }
}
}
@if(!PREPASS_PIPELINE) {
@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let record_index = record_index_from_vertex_output(in);
    if record_index >= arrayLength(&sdf_records) {
        discard;
    }
    let record = sdf_records[record_index];
    if record.paint_mask == 0u {
        discard;
    }

    let local = local_from_vertex_output(in, record);
    let view_aligned_border_clip = (record.flags & SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG) != 0u;
    let clip_rect = resolved_paint_clip_rect(record, view_aligned_border_clip);
    if local.x < clip_rect.x || local.x > clip_rect.z
        || local.y < clip_rect.y || local.y > clip_rect.w {
        discard;
    }

@if(SDF_TOOLTIP_SHADOW) {
    var pbr_input = tooltip_shadow_pbr_input(in, is_front, record, local);
}
@if(!SDF_TOOLTIP_SHADOW) {
    let view_aligned_edge_snap = (record.flags & SDF_VIEW_ALIGNED_EDGE_SNAP_FLAG) != 0u;
    let pixel_size = vec2<f32>(fwidth(local.x), fwidth(local.y));
    let local_dx = dpdx(local.x);
    let local_dy = dpdy(local.y);
    let inflated = inflate_subpixel_half_size(record.half_size, pixel_size);
    let paint_inside_border = (record.flags & SDF_PAINT_INSIDE_BORDER_FLAG) != 0u;
    let keep_snapped_edges_inside =
        (record.flags & TOOLTIP_KEEP_SNAPPED_EDGES_INSIDE_FLAG) != 0u;
    let snapped_x = snapped_edge_axis(
        in.position.x, local.x, record.half_size.x, local_dx,
        keep_snapped_edges_inside,
    );
    let snapped_y = snapped_edge_axis(
        in.position.y, local.y, record.half_size.y, local_dy,
        keep_snapped_edges_inside,
    );
    let edge_local = select(
        local,
        local - vec2(snapped_x.x, snapped_y.x),
        view_aligned_edge_snap,
    );
    let effective_half_size = select(
        inflated.xy,
        vec2(snapped_x.y, snapped_y.y),
        view_aligned_edge_snap,
    );
    let coverage_scale = inflated.z;
    let outer_dist = sd_rounded_box(edge_local, effective_half_size, record.corner_radii);
    let outer_aa = edge_aa_width(outer_dist, view_aligned_edge_snap);
    if paint_inside_border && outer_dist >= 0.0 {
        discard;
    }
    var outer_alpha = (1.0 - smoothstep(-outer_aa, outer_aa, outer_dist)) * coverage_scale;

    let fill_pbr = pbr_input_from_material_table(
        in,
        is_front,
        role_present(record.paint_mask, SDF_PAINT_FILL),
        record.fill_material,
    );
    let border_pbr = pbr_input_from_material_table(
        in,
        is_front,
        role_present(record.paint_mask, SDF_PAINT_BORDER),
        record.border_material,
    );
    var fill = fill_pbr.material.base_color;
    var border = border_pbr.material.base_color;
    let has_fill = fill.a > 0.001;
    let has_border = role_present(record.paint_mask, SDF_PAINT_BORDER)
        && record.border_material != INVALID_GPU_MATERIAL_SLOT
        && (record.border_widths.x > 0.0
            || record.border_widths.y > 0.0
            || record.border_widths.z > 0.0
            || record.border_widths.w > 0.0);
    if has_fill && (fill_pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u {
        fill = vec4<f32>(compensated_unlit_rgb(fill.rgb), fill.a);
    }
    let border_widths = select(
        record.border_widths,
        max(record.border_widths, vec4<f32>(pixel_size.y, pixel_size.x, pixel_size.y, pixel_size.x)),
        paint_inside_border,
    );
    let inner_hs = vec2<f32>(
        effective_half_size.x - 0.5 * (border_widths.y + border_widths.w),
        effective_half_size.y - 0.5 * (border_widths.x + border_widths.z),
    );
    let inner_offset = border_center_offset(border_widths);
    let inner_radii = inner_corner_radii(record, border_widths);
    let inner_dist = sd_rounded_box(edge_local - inner_offset, max(inner_hs, vec2(0.0)), inner_radii);
    let inner_aa = edge_aa_width(inner_dist, view_aligned_edge_snap);
    let inner_alpha = 1.0 - smoothstep(-inner_aa, inner_aa, inner_dist);
    let classic_border_alpha = outer_alpha * (1.0 - inner_alpha);
    let stroke_center = 0.5 * (outer_dist + inner_dist);
    let stroke_half_width = max(0.5 * (inner_dist - outer_dist), 0.0);
    let stroke_aa = max(distance_field_band(stroke_center), 0.0001);
    let logical_pixel_band = stroke_aa * max(device_pixels_per_logical_pixel, 1.0);
    let thin_stroke_alpha = centered_stroke_alpha(outer_dist, inner_dist, logical_pixel_band);

    var border_alpha = classic_border_alpha;
    if has_border {
        // Painting inside keeps the classic edges down to half a pixel, so a
        // whole-pixel frame never sits on the switch to the thin stroke.
        let thin_stroke_limit = select(
            logical_pixel_band,
            0.5 * logical_pixel_band,
            paint_inside_border,
        );
        let thin_border_mix = select(1.0, 0.0, 2.0 * stroke_half_width >= thin_stroke_limit);
        border_alpha = mix(classic_border_alpha, thin_stroke_alpha, thin_border_mix);
        outer_alpha = max(outer_alpha, border_alpha);
    }
    if has_border && border_alpha > 0.0
        && (border_pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u {
        border = vec4<f32>(compensated_unlit_rgb(border.rgb), border.a);
    }

    var final_color: vec4<f32>;
    if has_border {
        if has_fill {
            let border_mix = clamp(border_alpha / max(outer_alpha, 0.001), 0.0, 1.0);
            final_color = vec4<f32>(
                mix(fill.rgb, border.rgb, border_mix),
                outer_alpha * mix(fill.a, border.a, border_mix),
            );
        } else {
            final_color = vec4<f32>(border.rgb, border.a * border_alpha);
        }
    } else {
        final_color = vec4<f32>(fill.rgb, fill.a * outer_alpha);
    }

    if final_color.a < 0.001 {
        discard;
    }

    var pbr_input = fill_pbr;
    let use_border_material = has_border && (!has_fill || border_alpha >= outer_alpha * 0.5);
    if use_border_material {
        pbr_input.material.emissive = border_pbr.material.emissive;
        pbr_input.material.attenuation_color = border_pbr.material.attenuation_color;
        pbr_input.material.uv_transform = border_pbr.material.uv_transform;
        pbr_input.material.reflectance = border_pbr.material.reflectance;
        pbr_input.material.perceptual_roughness = border_pbr.material.perceptual_roughness;
        pbr_input.material.metallic = border_pbr.material.metallic;
        pbr_input.material.diffuse_transmission = border_pbr.material.diffuse_transmission;
        pbr_input.material.specular_transmission = border_pbr.material.specular_transmission;
        pbr_input.material.thickness = border_pbr.material.thickness;
        pbr_input.material.ior = border_pbr.material.ior;
        pbr_input.material.attenuation_distance = border_pbr.material.attenuation_distance;
        pbr_input.material.clearcoat = border_pbr.material.clearcoat;
        pbr_input.material.clearcoat_perceptual_roughness =
            border_pbr.material.clearcoat_perceptual_roughness;
        pbr_input.material.anisotropy_strength = border_pbr.material.anisotropy_strength;
        pbr_input.material.anisotropy_rotation = border_pbr.material.anisotropy_rotation;
    }
    pbr_input.material.base_color = final_color;
}
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );

    var out: FragmentOutput;
@if(SDF_TOOLTIP_SHADOW) {
    // A shadow is unlit, so its pipeline compiles no lighting.
    out.color = pbr_input.material.base_color;
}
@if(!SDF_TOOLTIP_SHADOW) {
@if(SDF_PBR_LIGHTING) {
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
}
@if(!SDF_PBR_LIGHTING) {
    out.color = pbr_input.material.base_color;
}
}
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);

@if(MATERIAL_OIT_ENABLED) {
    let alpha_mode = pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    if alpha_mode != STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE {
        var oit_pos = in.position;
@if(DEPTH_PREPASS) {
        if oit_pos.z + OIT_OCCLUSION_TOLERANCE < prepass_utils::prepass_depth(oit_pos, 0u) {
            discard;
        }
}
        oit_pos.z = panel_plane_depth(in.position, record);
        // Bevy 0.20 supplies premultiplied RGB except for AlphaMode::Blend.
        if alpha_mode == STANDARD_MATERIAL_FLAGS_ALPHA_MODE_BLEND {
            out.color = vec4<f32>(out.color.rgb * out.color.a, out.color.a);
        }
        oit_draw_layer(oit_pos, out.color, record.oit_paint_order);
        discard;
    }
}

    return out;
}

@if(SDF_TOOLTIP_SHADOW) {
// The tooltip body's rounded rectangle moved down by the record's offset and
// blurred across its support. It paints from one antialias width inside the
// body's edge outward: the body sits in front of it and hides that overlap, so
// no pixel row between the body's edge and the shadow is left to the host.
fn tooltip_shadow_pbr_input(
    in: VertexOutput,
    is_front: bool,
    record: SdfRenderRecord,
    local: vec2<f32>,
) -> PbrInput {
    let body_dist = sd_rounded_box(local, record.half_size, record.corner_radii);
    if body_dist < -aa_width(body_dist) {
        discard;
    }
    let blur_support = record.tooltip_shadow.x;
    let shadow_dist = sd_rounded_box(
        local - vec2<f32>(0.0, record.tooltip_shadow.y),
        record.half_size,
        record.corner_radii,
    );
    let coverage = 1.0 - smoothstep(-blur_support, blur_support, shadow_dist);
    var pbr_input = pbr_input_from_material_table(
        in,
        is_front,
        role_present(record.paint_mask, SDF_PAINT_FILL),
        record.fill_material,
    );
    let shadow = pbr_input.material.base_color;
    let alpha = shadow.a * coverage;
    if alpha < 0.001 {
        discard;
    }
    pbr_input.material.base_color = vec4<f32>(compensated_unlit_rgb(shadow.rgb), alpha);
    pbr_input.material.flags |= STANDARD_MATERIAL_FLAGS_UNLIT_BIT;
    return pbr_input;
}
}
}
}
