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
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::{STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
        STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE,
        STANDARD_MATERIAL_FLAGS_ALPHA_MODE_BLEND,
        STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS},
};
import bevy_pbr::render::pbr_bindings;
@if(MATERIAL_OIT_ENABLED)
import bevy_core_pipeline::oit::draw::{oit_draw_layer, oit_plane_depth};
@if(MATERIAL_OIT_ENABLED)
import bevy_pbr::prepass::utils as prepass_utils;

@if(MATERIAL_OIT_ENABLED)
// One layer step permits a fragment on its own opaque surface.
const OIT_OCCLUSION_TOLERANCE: f32 = 0.0000001192092967;

const CLIP_DEPTH_NUDGE_PER_LAYER: f32 = 0.0000002;
const IMAGE_ALPHA_DISCARD: f32 = 0.001;
const INVALID_RECORD_INDEX: u32 = 4294967295u;

struct ImageRenderRecord {
    panel_plane: vec4<f32>,
    transform: mat4x4<f32>,
    size: vec2<f32>,
    uv_rect: vec4<f32>,
    tint: vec4<f32>,
    clip_depth_nudge: f32,
    oit_paint_order: f32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(107) var<storage, read> image_records: array<ImageRenderRecord>;

struct PulledImageVertex {
    clip_position: vec4<f32>,
    world_position: vec4<f32>,
    world_normal: vec3<f32>,
    uv: vec2<f32>,
    record_index: u32,
}

fn pulled_image_default() -> PulledImageVertex {
    var out: PulledImageVertex;
    out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.world_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.record_index = INVALID_RECORD_INDEX;
    return out;
}

fn pull_image_vertex(vertex_index: u32, instance_index: u32) -> PulledImageVertex {
    var out = pulled_image_default();

    let local_index = vertex_index - mesh[instance_index].first_vertex_index;
    let record_index = local_index / 4u;
    let corner = local_index % 4u;

    if record_index >= arrayLength(&image_records) {
        return out;
    }
    let record = image_records[record_index];
    if record.size.x <= 0.0 || record.size.y <= 0.0 {
        return out;
    }

    let corner_x = f32(corner == 1u || corner == 2u);
    let corner_top = f32(corner <= 1u);
    let signs = vec2<f32>(corner_x * 2.0 - 1.0, corner_top * 2.0 - 1.0);
    let local = signs * record.size * 0.5;
    let box_uv = vec2<f32>(corner_x, 1.0 - corner_top);
    let uv = mix(record.uv_rect.xy, record.uv_rect.zw, box_uv);

    let world = record.transform * vec4<f32>(local, 0.0, 1.0);
    var clip = position_world_to_clip(world.xyz);
@if(!MATERIAL_OIT_ENABLED) {
    clip.z += record.clip_depth_nudge * CLIP_DEPTH_NUDGE_PER_LAYER * clip.w;
}

    out.clip_position = clip;
    out.world_position = world;
    out.world_normal = normalize((record.transform * vec4<f32>(0.0, 0.0, 1.0, 0.0)).xyz);
    out.uv = uv;
    out.record_index = record_index;
    return out;
}

@if(PREPASS_PIPELINE) {
@vertex
fn vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let pulled = pull_image_vertex(vertex_index, instance_index);
    var out: VertexOutput;
    out.position = pulled.clip_position;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = pulled.clip_position.z;
    out.position.z = min(out.position.z, 1.0);
}
@if(VERTEX_UVS_A) {
    out.uv = pulled.uv;
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
    let pulled = pull_image_vertex(vertex_index, instance_index);
    var out: VertexOutput;
    out.position = pulled.clip_position;
    out.world_position = pulled.world_position;
    out.world_normal = pulled.world_normal;
@if(VERTEX_UVS_A) {
    out.uv = pulled.uv;
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
    return INVALID_RECORD_INDEX;
}
}

fn image_uv_from_vertex_output(in: VertexOutput) -> vec2<f32> {
@if(VERTEX_UVS_A) {
    return in.uv;
}
@if(!VERTEX_UVS_A) {
    return vec2<f32>(0.0);
}
}

@if(PREPASS_PIPELINE) {
fn image_texture_sample(in: VertexOutput) -> vec4<f32> {
    return textureSampleBias(
        pbr_bindings::base_color_texture,
        pbr_bindings::base_color_sampler,
        image_uv_from_vertex_output(in),
        view.mip_bias
    );
}

@fragment
fn fragment(in: VertexOutput) {
    let record_index = record_index_from_vertex_output(in);
    if record_index >= arrayLength(&image_records) {
        discard;
    }
    let record = image_records[record_index];
    let color = image_texture_sample(in) * record.tint;
    if color.a <= IMAGE_ALPHA_DISCARD {
        discard;
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
    if record_index >= arrayLength(&image_records) {
        discard;
    }
    let record = image_records[record_index];

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color *= record.tint;
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
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
        oit_pos.z = oit_plane_depth(in.position, record.panel_plane);
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
}
