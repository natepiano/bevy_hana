
import bevy_pbr::render::mesh_view_bindings::{view, oit_nodes, oit_heads, oit_atomic_counter, oit_settings};
import bevy_pbr::prepass::utils as prepass_utils;

// A node is four u32 words: RGB9E5, physical depth, local paint/alpha, next.
// Physical depth is the full positive f32 reverse-Z bit pattern. Paint never
// changes it. The secondary word has 24 paint bits and eight alpha bits.
// Lexicographic comparison makes every distinguishable physical depth win,
// for both perspective and orthographic projections.
struct OitPaintNode {
    color: u32,
    depth: u32,
    paint_alpha: u32,
    next: u32,
}

// One plane and one pixel produce the same physical key for fills, images and
// ink, regardless of their independently rasterized or expanded quads.
fn oit_plane_depth(position: vec4f, plane: vec4f) -> f32 {
    let clip_plane = transpose(view.world_from_clip) * plane;
    if clip_plane.z == 0.0 {
        return position.z;
    }
    let uv = (position.xy - view.viewport.xy) / view.viewport.zw;
    let ndc = uv * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);
    // Anchor depth with the same forward projection as opaque geometry. Solving
    // the inverse-transformed plane's constant alone can round a coplanar layer
    // behind the hardware depth buffer, which then discards its ink at resolve.
    let origin = -plane.w * plane.xyz / dot(plane.xyz, plane.xyz);
    let clip_origin = view.clip_from_world * vec4f(origin, 1.0);
    if clip_origin.w == 0.0 {
        return -(dot(clip_plane.xy, ndc) + clip_plane.w) / clip_plane.z;
    }
    let projected_origin = clip_origin.xyz / clip_origin.w;
    return projected_origin.z - dot(clip_plane.xy, ndc - projected_origin.xy) / clip_plane.z;
}

fn oit_depth_key(depth: f32) -> u32 {
    return bitcast<u32>(max(depth, 0.0));
}

fn oit_in_front(depth_a: u32, paint_a: u32, depth_b: u32, paint_b: u32) -> bool {
    return depth_a > depth_b || (depth_a == depth_b && paint_a > paint_b);
}

fn oit_alpha(paint_alpha: u32) -> f32 {
    return f32(paint_alpha & 255u) / 255.0;
}

@if(OIT_ENABLED) {
// Non-panel materials have no local paint layer and keep Bevy's depth test.
fn oit_draw(position: vec4f, color: vec4f) {
@if(DEPTH_PREPASS) {
    if position.z < prepass_utils::prepass_depth(position, 0u) {
        return;
    }
}
    oit_draw_layer(position, color, 0.0);
}

// Panel callers perform their prepass test before reconstructing their common
// plane. That test is independent of the local paint key stored here.
fn oit_draw_layer(position: vec4f, color: vec4f, paint_order: f32) {
    if color.a < oit_settings.alpha_threshold {
        return;
    }
    let index = atomicAdd(&oit_atomic_counter, 1u);
    // Bevy owns the backing allocation in twelve-byte units. Bind it as words
    // and bound the sixteen-byte nodes by its actual size, including small
    // initial buffers and device-limited pools.
    if index >= arrayLength(&oit_nodes) / 4u {
        return;
    }
    let screen_index = u32(floor(position.x) + floor(position.y) * view.viewport.z);
    let next = atomicExchange(&oit_heads[screen_index], index + 1u) - 1u;
    let paint = u32(clamp(round(paint_order * 16777215.0), 0.0, 16777215.0));
    let alpha = u32(saturate(color.a) * 255.0 + 0.5);
    oit_nodes[index * 4u] = bevy_pbr::render::rgb9e5::vec3_to_rgb9e5_(color.rgb);
    oit_nodes[index * 4u + 1u] = oit_depth_key(position.z);
    oit_nodes[index * 4u + 2u] = (paint << 8u) | alpha;
    oit_nodes[index * 4u + 3u] = next;
}
}
