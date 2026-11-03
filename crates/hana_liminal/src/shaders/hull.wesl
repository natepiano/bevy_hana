import bevy_pbr::render::{
    mesh_bindings::mesh,
    mesh_functions,
    skinning,
    morph,
    morph::{morph_position, morph_normal, morph_tangent},
    view_transformations::{position_world_to_clip, direction_world_to_clip, frag_coord_to_uv},
};
import hana_liminal::view_helpers::get_viewport;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
@if(VERTEX_POSITIONS)
    @location(0) position: vec3<f32>,
@if(VERTEX_NORMALS)
    @location(1) normal: vec3<f32>,
@if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
@if(VERTEX_UVS_B)
    @location(3) uv_b: vec2<f32>,
@if(VERTEX_TANGENTS)
    @location(4) tangent: vec4<f32>,
@if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
@if(SKINNED)
    @location(6) joint_indices: vec4<u32>,
@if(SKINNED)
    @location(7) joint_weights: vec4<f32>,
@if(MORPH_TARGETS)
    @builtin(vertex_index) index: u32,
@if(HAS_OUTLINE_NORMALS)
    @location(8) outline_normal: vec3<f32>,
};
@group(4) @binding(0) var occlusion_sampler: sampler;
@group(4) @binding(1) var outline_depth_texture: texture_depth_2d;
@group(4) @binding(2) var owner_texture: texture_2d<f32>;

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
    @location(0) color: vec4<f32>,
    @location(1) surface_depth: f32,
    @location(2) @interpolate(flat) owner_id: f32,
    @location(3) @interpolate(flat) overlap: f32,
};

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
    for (var i: u32 = 0u; i < weight_count; i++) {
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

    let outline = outline_instances[vertex_no_morph.instance_index];

@if(SKINNED) {
    var world_from_local = skinning::skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex.instance_index,
    );
}
@if(!SKINNED) {
    let world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);
}

    var world_position = vec4<f32>(0.0);
@if(VERTEX_POSITIONS) {
    world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
}

    let surface_clip = position_world_to_clip(world_position.xyz);

@if(HAS_OUTLINE_NORMALS) {
    // Pre-computed smoothed outline normals for correct concave mesh extrusion.
    var hull_dir = mesh_functions::mesh_normal_local_to_world(
        vertex.outline_normal,
        vertex_no_morph.instance_index,
    );
}
@if(!HAS_OUTLINE_NORMALS) {
    // Radial extrusion from object origin. All vertices at the same position
    // extrude in the same direction, keeping corners connected on hard-edged
    // geometry (cuboids). Falls back to vertex normal when the vertex is at
    // the origin.
    let object_origin = (world_from_local * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    var hull_dir = world_position.xyz - object_origin;
    let hull_dir_len_sq = dot(hull_dir, hull_dir);
@if(VERTEX_NORMALS) {
    if hull_dir_len_sq < 1e-8 {
        hull_dir = mesh_functions::mesh_normal_local_to_world(
            vertex.normal,
            vertex_no_morph.instance_index,
        );
    }
}
@if(!VERTEX_NORMALS) {
    if hull_dir_len_sq < 1e-8 {
        hull_dir = vec3<f32>(0.0, 1.0, 0.0);
    }
}
}

    let shell_mode = outline.owner_data.y;
    if shell_mode > 0.5 {
        // Clip-space extrusion: consistent pixel-width outlines regardless of distance.
        let viewport = get_viewport();
        let clip_norm = direction_world_to_clip(normalize(hull_dir));
        let aspect = viewport.w / viewport.z;
        let corrected_norm = normalize(clip_norm.xy * vec2<f32>(aspect, 1.0));
        let width_px = max(outline.width, 0.00001);
        let ndc_delta = corrected_norm * (2.0 / viewport.zw) * width_px * surface_clip.w;
        out.position = vec4<f32>(surface_clip.xy + ndc_delta, surface_clip.zw);
    } else {
        // World-space extrusion: outline grows/shrinks with distance.
        let shell_thickness = max(outline.width, 0.00001);
        let expanded_world_position = world_position.xyz + normalize(hull_dir) * shell_thickness;
        out.position = position_world_to_clip(expanded_world_position);
    }

    out.surface_depth = surface_clip.z / surface_clip.w;
    out.owner_id = outline.owner_data.x;
    out.overlap = outline.overlap;
    out.color = vec4<f32>(
        outline.color.rgb * outline.intensity,
        outline.color.a
    );
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = frag_coord_to_uv(in.position.xy);
    let outlined_surface_depth = textureSample(outline_depth_texture, occlusion_sampler, uv);

    if outlined_surface_depth > 0.0 {
        let owner_at_pixel = textureSample(owner_texture, occlusion_sampler, uv).x;

        if owner_at_pixel == in.owner_id {
            // Hull over own mesh surface — always discard to prevent solid fill.
            discard;
        }

        // Different mesh surface is closest at this pixel.
        // In reverse-Z: larger depth = closer to camera.
        // If our mesh surface is further away, this hull belongs to the
        // back mesh — discard it so only the front mesh's outline shows.
        if in.surface_depth < outlined_surface_depth {
            discard;
        }

        // Front mesh hull over back mesh surface.
        // overlap=0.0 (Merged): fully transparent at mesh-overlap pixels.
        // overlap=1.0 (Grouped/PerMesh): fully opaque per-group/per-mesh overlap rendering.
        return vec4<f32>(in.color.rgb, in.color.a * in.overlap);
    }

    // No outlined surface at this pixel — hull at silhouette in open space.
    return in.color;
}
