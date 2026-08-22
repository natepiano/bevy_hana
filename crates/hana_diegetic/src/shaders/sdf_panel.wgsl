// Vertex-pulled SDF panel fill and border shader.

#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::VertexOutput
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#endif

#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#import bevy_pbr::pbr_types::{
    STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS,
    STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE,
}

const OIT_MIN_DEPTH: f32 = 0.000003;
#endif

#import hana_diegetic::sdf_material_table::pbr_input_from_material_table
#import hana_diegetic::material_table::INVALID_GPU_MATERIAL_SLOT
#import hana_diegetic::sdf_stroke::{
    centered_stroke_alpha,
    distance_field_band,
    inflate_subpixel_half_size,
}

const CLIP_DEPTH_NUDGE_PER_LAYER: f32 = 0.0000002;
const SDF_PAINT_FILL: u32 = 1u;
const SDF_PAINT_BORDER: u32 = 2u;

#ifdef SDF_STRIPPED_MATERIAL_GROUP
#ifdef PREPASS_PIPELINE
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    _ = vertex_index;
    var out: VertexOutput;
    out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
#endif
#ifdef VERTEX_UVS_A
    out.uv = vec2<f32>(0.0);
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vec2<f32>(0.0);
#endif
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
#endif
    out.world_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = instance_index;
#else
    _ = instance_index;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) {
    _ = in;
    _ = is_front;
    discard;
}
#else
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
#ifdef VERTEX_UVS_A
    out.uv = vec2<f32>(0.0);
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vec2<f32>(0.0);
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = instance_index;
#else
    _ = instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = 0;
#endif
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
#endif
#else

struct SdfRenderRecord {
    transform: mat4x4<f32>,
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
    oit_depth_offset: f32,
}

struct SdfMeshRecord {
    reserved: vec4<u32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<storage, read> sdf_records: array<SdfRenderRecord>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var<storage, read> sdf_mesh_records: array<SdfMeshRecord>;

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
#ifndef OIT_ENABLED
    clip.z += record.clip_depth_nudge * CLIP_DEPTH_NUDGE_PER_LAYER * clip.w;
#endif

    out.clip_position = clip;
    out.world_position = world;
    out.world_normal = normalize((record.transform * vec4<f32>(0.0, 0.0, 1.0, 0.0)).xyz);
    out.box_uv = box_uv;
    out.record_index = record_index;
    return out;
}

#ifdef PREPASS_PIPELINE
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let pulled = pull_sdf_vertex(vertex_index, instance_index);
    var out: VertexOutput;
    out.position = pulled.clip_position;
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = pulled.clip_position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = pulled.box_uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vec2<f32>(f32(pulled.record_index), 0.0);
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
    let pulled = pull_sdf_vertex(vertex_index, instance_index);
    var out: VertexOutput;
    out.position = pulled.clip_position;
    out.world_position = pulled.world_position;
    out.world_normal = pulled.world_normal;
#ifdef VERTEX_UVS_A
    out.uv = pulled.box_uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vec2<f32>(f32(pulled.record_index), 0.0);
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

fn record_index_from_vertex_output(in: VertexOutput) -> u32 {
#ifdef VERTEX_UVS_B
    return u32(floor(in.uv_b.x + 0.5));
#else
    return 4294967295u;
#endif
}

fn local_from_vertex_output(in: VertexOutput, record: SdfRenderRecord) -> vec2<f32> {
#ifdef VERTEX_UVS_A
    return (in.uv - 0.5) * 2.0 * record.mesh_half_size;
#else
    return vec2<f32>(0.0);
#endif
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

#ifdef PREPASS_PIPELINE
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

    let local = local_from_vertex_output(in, record);
    if local.x < record.clip_rect.x || local.x > record.clip_rect.z
        || local.y < record.clip_rect.y || local.y > record.clip_rect.w {
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
#else
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
    if local.x < record.clip_rect.x || local.x > record.clip_rect.z
        || local.y < record.clip_rect.y || local.y > record.clip_rect.w {
        discard;
    }

    let pixel_size = vec2<f32>(fwidth(local.x), fwidth(local.y));
    let inflated = inflate_subpixel_half_size(record.half_size, pixel_size);
    let effective_half_size = inflated.xy;
    let coverage_scale = inflated.z;
    let outer_dist = sd_rounded_box(local, effective_half_size, record.corner_radii);
    let outer_aa = aa_width(outer_dist);
    let outer_alpha = (1.0 - smoothstep(-outer_aa, outer_aa, outer_dist)) * coverage_scale;
    if outer_alpha < 0.001 {
        discard;
    }

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
    let fill = fill_pbr.material.base_color;
    let border = border_pbr.material.base_color;
    let has_fill = fill.a > 0.001;
    let has_border = role_present(record.paint_mask, SDF_PAINT_BORDER)
        && record.border_material != INVALID_GPU_MATERIAL_SLOT
        && (record.border_widths.x > 0.0
            || record.border_widths.y > 0.0
            || record.border_widths.z > 0.0
            || record.border_widths.w > 0.0);

    let inner_hs = inner_half_size(record, record.border_widths);
    let inner_offset = border_center_offset(record.border_widths);
    let inner_radii = inner_corner_radii(record, record.border_widths);
    let inner_dist = sd_rounded_box(local - inner_offset, max(inner_hs, vec2(0.0)), inner_radii);
    let inner_aa = aa_width(inner_dist);
    let inner_alpha = 1.0 - smoothstep(-inner_aa, inner_aa, inner_dist);
    let classic_border_alpha = outer_alpha * (1.0 - inner_alpha);
    let thin_stroke_alpha = centered_stroke_alpha(outer_dist, inner_dist);

    var border_alpha = classic_border_alpha;
    if !has_fill && has_border {
        let stroke_center = 0.5 * (outer_dist + inner_dist);
        let stroke_half_width = max(0.5 * (inner_dist - outer_dist), 0.0);
        let stroke_aa = max(fwidth(stroke_center), 0.0001);
        let thin_border_mix = 1.0 - smoothstep(0.75, 1.5, stroke_half_width / stroke_aa);
        border_alpha = mix(classic_border_alpha, thin_stroke_alpha, thin_border_mix);
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
        var oit_pos = in.position;
        oit_pos.z = max(oit_pos.z + record.oit_depth_offset, OIT_MIN_DEPTH);
        oit_draw(oit_pos, out.color);
        discard;
    }
#endif

    return out;
}
#endif
#endif
