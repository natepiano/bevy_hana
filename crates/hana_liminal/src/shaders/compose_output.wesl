@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;
@group(0) @binding(2) var flood_texture: texture_2d<f32>;
@group(0) @binding(3) var appearance_texture: texture_2d<f32>;
@if(MULTISAMPLED)
@group(0) @binding(4) var depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(4) var depth_texture: texture_depth_2d;

@group(0) @binding(5) var outline_depth_texture: texture_depth_2d;
@if(MULTISAMPLED)
@group(0) @binding(6) var main_depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(6) var main_depth_texture: texture_depth_2d;

// Mask `owner_data`: owner ID, overlap factor, view distance, orthographic depth per metre.
@group(0) @binding(7) var owner_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Coplanar presentation faces can stand just forward of their outlined body
// (Camera empty plates use 0.2 mm). This source contact is much smaller than
// the expanded stroke's existing contact allowance.
const SOURCE_CONTACT_METERS: f32 = 0.00025;

fn foreground_gap(scene_depth: f32, outline_depth: f32, seed_surface: vec4<f32>) -> f32 {
    if seed_surface.w > 0.0 {
        return (scene_depth - outline_depth) / seed_surface.w;
    }
    return seed_surface.z * (1.0 - outline_depth / max(scene_depth, outline_depth));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(screen_texture, texture_sampler, in.uv);
    let flood_data = textureSample(flood_texture, texture_sampler, in.uv);
    let seed_uv = flood_data.xy;

    // Early return if no outline data
    if seed_uv.x <= 0.0 || seed_uv.y <= 0.0 {
        return color;
    }

    // Check if this pixel is ON an outlined mesh by sampling the outline depth
    // at the current pixel. The outline depth texture is cleared to 0.0 (far plane
    // in reverse-Z), so any pixel with depth > 0 belongs to an outlined mesh.
    // This prevents the outline from drawing on the mesh itself, which is critical
    // for transmissive/transparent materials that don't write to the scene depth buffers.
    //
    // Overlap resolution: a merged seed (overlap factor 0.0) never draws on any
    // outlined surface, and no seed draws on its own group's surface. A grouped or
    // per-mesh seed may draw over ANOTHER owner's surface, subject to the depth
    // test below — so a nested outlined mesh (e.g. a jack on a selected screen)
    // keeps its own outline on top of its host.
    let self_outline_depth = textureSample(outline_depth_texture, texture_sampler, in.uv);
    if self_outline_depth > 0.0 {
        let seed_owner = textureSample(owner_texture, texture_sampler, seed_uv);
        let pixel_owner = textureSample(owner_texture, texture_sampler, in.uv);
        if seed_owner.y == 0.0 || pixel_owner.x == seed_owner.x {
            return color;
        }
    }

    // Get depths — use the closer of prepass and main pass depth so that both
    // opaque (prepass) and transmissive/transparent (main pass) geometry occlude outlines.
    // Bevy uses reverse-Z, so larger depth is closer.
    let coords = vec2<i32>(in.uv * vec2<f32>(textureDimensions(depth_texture)));
@if(MULTISAMPLED)
    let prepass_depth = textureLoad(depth_texture, coords, 0);
@if(!MULTISAMPLED)
    let prepass_depth = textureSample(depth_texture, texture_sampler, in.uv);

@if(MULTISAMPLED)
    let main_depth = textureLoad(main_depth_texture, coords, 0);
@if(!MULTISAMPLED)
    let main_depth = textureSample(main_depth_texture, texture_sampler, in.uv);

    // Include the outlined surface under this pixel: transmissive outlined meshes
    // don't write scene depth, but must still occlude another group's outline.
    let current_depth = max(max(prepass_depth, main_depth), self_outline_depth);
    let outline_depth = textureSample(outline_depth_texture, texture_sampler, seed_uv);
    // A seed behind another body cannot contribute an outline, even when its
    // expanded stroke reaches a neighboring pixel with no scene depth.
    let seed_coords = vec2<i32>(seed_uv * vec2<f32>(textureDimensions(depth_texture)));
@if(MULTISAMPLED)
    let seed_prepass_depth = textureLoad(depth_texture, seed_coords, 0);
@if(MULTISAMPLED)
    let seed_main_depth = textureLoad(main_depth_texture, seed_coords, 0);
@if(!MULTISAMPLED)
    let seed_prepass_depth = textureSample(depth_texture, texture_sampler, seed_uv);
@if(!MULTISAMPLED)
    let seed_main_depth = textureSample(main_depth_texture, texture_sampler, seed_uv);

    // Separate near-coplanar presentation contact from a hidden outline source.
    // Compare in metres so both camera projections enforce the same distance.
    let seed_surface = textureSample(owner_texture, texture_sampler, seed_uv);
    let seed_gap = foreground_gap(max(seed_prepass_depth, seed_main_depth), outline_depth, seed_surface);
    if seed_gap > SOURCE_CONTACT_METERS {
        return color;
    }

    // Get appearance data for this outline
    let appearance = textureSample(appearance_texture, texture_sampler, seed_uv);
    let outline_color = appearance.rgb;

    // Reverse-Z: only render outline when the outline seed depth is closer than the
    // currently visible scene depth at this pixel.
    // A visible source's expanded stroke can touch a near-coplanar floor or
    // border. Keep the same contact fraction of its view distance in either
    // projection; a fraction of orthographic reverse-Z can instead admit metres.
    let view_distance = seed_surface.z;
    let stroke_gap = foreground_gap(current_depth, outline_depth, seed_surface);
    let contact_distance = view_distance * 0.002;
    if stroke_gap <= contact_distance {
        // Apply outline color
        color = vec4<f32>(outline_color, 1.0);
    }

    return color;
}
