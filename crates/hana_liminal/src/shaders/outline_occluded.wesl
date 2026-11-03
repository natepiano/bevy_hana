import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var outline_depth: texture_depth_2d;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var outline_strokes: texture_2d<f32>;

@group(constants::MATERIAL_BIND_GROUP) @binding(102) var translucent_depth: texture_depth_2d;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let surface_depth = textureLoad(outline_depth, vec2<i32>(in.position.xy), 0);
    // Reverse Z: a larger value belongs to a surface closer to the camera.
    if max(surface_depth, textureLoad(translucent_depth, vec2<i32>(in.position.xy), 0)) > in.position.z {
        discard;
    }
    let seed = textureLoad(outline_strokes, vec2<i32>(in.position.xy), 0);
    let dimensions = vec2<f32>(textureDimensions(outline_strokes));
    if seed.x >= 0.0 && seed.z > 0.0
        && distance(in.position.xy, seed.xy * dimensions) < seed.z
        && seed.w > in.position.z {
        discard;
    }
    let pbr_input = pbr_input_from_standard_material(in, is_front);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, pbr_input.material.base_color);
    return out;
}
