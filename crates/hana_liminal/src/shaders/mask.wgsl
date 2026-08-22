#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    skinning,
    morph::morph,
    forward_io::{Vertex},
    view_transformations::{position_world_to_clip, ndc_to_uv, position_world_to_ndc,frag_coord_to_uv},
}

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

#ifdef PER_OBJECT_BUFFER_BATCH_SIZE
@group(3) @binding(0) var<uniform> outline_instances: array<Instance, #{PER_OBJECT_BUFFER_BATCH_SIZE}u>;
#else
@group(3) @binding(0) var<storage> outline_instances: array<Instance>;
#endif


#ifdef MORPH_TARGETS
fn morph_vertex(vertex_in: Vertex) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh[vertex.instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;

    let weight_count = bevy_pbr::morph::layer_count();
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = bevy_pbr::morph::weight_at(i);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph(vertex_index, bevy_pbr::morph::position_offset, i);
#ifdef VERTEX_NORMALS
        vertex.normal += weight * morph(vertex_index, bevy_pbr::morph::normal_offset, i);
#endif
#ifdef VERTEX_TANGENTS
        vertex.tangent += vec4(weight * morph(vertex_index, bevy_pbr::morph::tangent_offset, i), 0.0);
#endif
    }
    return vertex;
}
#endif


@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
    var out: VertexOutput;

    #ifdef MORPH_TARGETS
        var vertex = morph_vertex(vertex_no_morph);
    #else
        var vertex = vertex_no_morph;
    #endif

    #ifdef SKINNED
        var world_from_local = skinning::skin_model(vertex.joint_indices, vertex.joint_weights,
            vertex.instance_index
        );
    #else
        let world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);
    #endif

    #ifdef VERTEX_POSITIONS
        out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
        out.position = position_world_to_clip(out.world_position.xyz);
    #endif

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
    output.owner_data = vec4<f32>(outline.owner_data.x, outline.overlap, 0.0, 0.0);

    return output;
}
