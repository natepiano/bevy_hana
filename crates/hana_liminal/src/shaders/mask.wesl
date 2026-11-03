import bevy_pbr::render::{
    mesh_bindings::mesh,
    mesh_functions,
    skinning,
    morph,
    morph::{morph_position, morph_normal, morph_tangent},
    forward_io::{Vertex},
    mesh_view_bindings::view,
    view_transformations::{position_world_to_clip, position_world_to_view, frag_coord_to_uv},
};

struct Instance {
    intensity: f32,
    width: f32,
    priority: f32,
    overlap: f32,
    color: vec4<f32>,
    owner_data: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) @interpolate(flat) instance_index: u32,
};

struct FragmentOutput {
    @location(0) flood_data: vec4<f32>,
    @location(1) appearance_data: vec4<f32>,
    @location(2) owner_data: vec4<f32>,
}

@if(OUTLINE_INSTANCE_BATCH_SIZE) {
@group(3) @binding(0) var<uniform> outline_instances: array<Instance, u32(constants::OUTLINE_INSTANCE_BATCH_SIZE)>;
}
@if(!OUTLINE_INSTANCE_BATCH_SIZE) {
@group(3) @binding(0) var<storage> outline_instances: array<Instance>;
}


@if(MORPH_TARGETS) {
fn morph_vertex(vertex_in: Vertex, instance_index: u32) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;
    let morph_descriptor_index = mesh[instance_index].morph_descriptor_index;

    let weight_count = morph::layer_count(morph_descriptor_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = morph::weight_at(i, morph_descriptor_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, morph_descriptor_index);
@if(VERTEX_NORMALS) {
        vertex.normal += weight * morph_normal(vertex_index, i, morph_descriptor_index);
}
@if(VERTEX_TANGENTS) {
        vertex.tangent += vec4(weight * morph_tangent(vertex_index, i, morph_descriptor_index), 0.0);
}
    }
    return vertex;
}
}


@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
    var out: VertexOutput;

@if(MORPH_TARGETS) {
        var vertex = morph_vertex(vertex_no_morph, vertex_no_morph.instance_index);
}
@if(!MORPH_TARGETS) {
        var vertex = vertex_no_morph;
}

@if(SKINNED) {
        var world_from_local = skinning::skin_model(vertex.joint_indices, vertex.joint_weights,
            vertex.instance_index
        );
}
@if(!SKINNED) {
        let world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);
}

@if(VERTEX_POSITIONS) {
        out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
        out.position = position_world_to_clip(out.world_position.xyz);
}

    out.instance_index = vertex_no_morph.instance_index;

    return out;
}

@fragment
fn fragment(vertex: VertexOutput) -> FragmentOutput {
    let uv = frag_coord_to_uv(vertex.position.xy);
    let depth = vertex.position.z;
    let outline = outline_instances[vertex.instance_index];

    var output: FragmentOutput;
    // RT0: seed_uv.xy, outline_width, depth
    output.flood_data = vec4<f32>(uv, outline.width, depth);
    // RT1: color.rgb, priority
    output.appearance_data = vec4<f32>(outline.color.rgb * outline.intensity, outline.priority);
    // RT2: owner ID (x) and overlap factor (y). Hull fragments compare x against
    // their own owner ID; the jump-flood compose pass reads both to decide whether
    // an outline may draw over another group's outlined surface.
    // The remaining channels describe this surface's view distance and the
    // orthographic depth change per metre (zero for a perspective view). Compose
    // compares foreground gaps in metres, not projection-dependent reverse-Z.
    let view_distance = -position_world_to_view(vertex.world_position.xyz).z;
    let orthographic_depth_per_meter = select(
        0.0, view.clip_from_view[2].z, view.clip_from_view[3].w == 1.0,
    );
    output.owner_data = vec4<f32>(
        outline.owner_data.x, outline.overlap, view_distance, orthographic_depth_per_meter,
    );

    return output;
}
