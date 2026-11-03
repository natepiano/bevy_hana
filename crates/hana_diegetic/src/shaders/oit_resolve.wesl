// Bevy 0.20 OIT resolve, with physical depth and local paint compared separately.
// Reference:
// Order-independent transparency with per-pixel ordered linked lists - https://github.com/KhronosGroup/Vulkan-Samples/blob/main/shaders/oit_linked_lists/combine.frag

import bevy_render::view::View;
import bevy_core_pipeline::oit::draw::{OitPaintNode, oit_depth_key, oit_in_front, oit_alpha};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> nodes: array<u32>;
@group(0) @binding(2) var<storage, read_write> heads: array<u32>; // No need to be atomic
@group(0) @binding(3) var<storage, read_write> atomic_counter: u32; // No need to be atomic

@if(!DEPTH_PREPASS)
@group(1) @binding(0) var depth: texture_depth_2d;

struct OitFragment {
    color: u32,
    depth: u32,
    paint_alpha: u32,
}

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

const LINKED_LIST_END_SENTINEL: u32 = 0xFFFFFFFFu;
const SORTED_FRAGMENT_MAX_COUNT: u32 = constants::SORTED_FRAGMENT_MAX_COUNT;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    atomic_counter = 0u;
    let screen_index = u32(floor(in.position.x) + floor(in.position.y) * view.viewport.z);

    let head = heads[screen_index] - 1u;
    if head == LINKED_LIST_END_SENTINEL {
        // https://github.com/gfx-rs/wgpu/issues/4416
        if true {
            discard;
        }
        return vec4(0.0);
    } else {
@if(!DEPTH_PREPASS)
        // If depth prepass is disabled, load depth for manual depth testing.
        // This is necessary because early z doesn't seem to trigger in the transparent pass.
        // This should be done during the draw pass so those fragments simply don't exist in the list,
        // but this requires a bigger refactor
        let d = textureLoad(depth, vec2<i32>(in.position.xy), 0);
@if(DEPTH_PREPASS)
        let d = 0.0;
        let color = resolve(head, d);
        heads[screen_index] = 0u; // LINKED_LIST_END_SENTINEL + 1u;
        return color;
    }
}

fn resolve(head: u32, opaque_depth: f32) -> vec4<f32> {
    // Contains all the colors and depth for this specific fragment
    // Fragments are sorted from front to back (depth values are in descending order)
    // This should make insertion sort slightly faster
    // because transparent pass sorts objects so the linked list iteration is usually in descending order.
    var fragment_list: array<OitFragment, SORTED_FRAGMENT_MAX_COUNT>;
    var final_color = vec4<f32>(0.0);

    let packed_opaque_depth = oit_depth_key(opaque_depth);

    // fill list
    var current_node = head;
    var sorted_frag_count = 0u;
    while current_node != LINKED_LIST_END_SENTINEL {
        let fragment_node = OitPaintNode(nodes[current_node * 4u], nodes[current_node * 4u + 1u],
            nodes[current_node * 4u + 2u], nodes[current_node * 4u + 3u]);
        current_node = fragment_node.next;

@if(!DEPTH_PREPASS) {
        // depth testing
        if fragment_node.depth < packed_opaque_depth {
            continue;
        }
}

        if sorted_frag_count < SORTED_FRAGMENT_MAX_COUNT {
            // There is still room in the sorted list.
            // Insert the fragment so that the list stay sorted.
            var i = sorted_frag_count;
            for (; i > 0 && oit_in_front(fragment_node.depth, fragment_node.paint_alpha, fragment_list[i - 1].depth, fragment_list[i - 1].paint_alpha); i -= 1) {
                fragment_list[i] = fragment_list[i - 1];
            }
            fragment_list[i].color = fragment_node.color;
            fragment_list[i].depth = fragment_node.depth;
            fragment_list[i].paint_alpha = fragment_node.paint_alpha;
            sorted_frag_count += 1;
        } else if oit_in_front(fragment_list[0].depth, fragment_list[0].paint_alpha, fragment_node.depth, fragment_node.paint_alpha) {
            // The fragment is farther than the nearest sorted one.
            // First, make room by blending the nearest fragment from the sorted list.
            // Then, insert the fragment in the sorted list.
            // This is an approximation.
            let nearest_color = bevy_pbr::render::rgb9e5::rgb9e5_to_vec3_(fragment_list[0].color);
            let nearest_alpha = oit_alpha(fragment_list[0].paint_alpha);
            final_color = blend(final_color, vec4f(nearest_color, nearest_alpha));
            var i = 0u;
            for (; i < SORTED_FRAGMENT_MAX_COUNT - 1 && oit_in_front(fragment_list[i + 1].depth, fragment_list[i + 1].paint_alpha, fragment_node.depth, fragment_node.paint_alpha); i += 1) {
                fragment_list[i] = fragment_list[i + 1];
            }
            fragment_list[i].color = fragment_node.color;
            fragment_list[i].depth = fragment_node.depth;
            fragment_list[i].paint_alpha = fragment_node.paint_alpha;
        } else {
            // The next fragment is nearer than any of the sorted ones.
            // Blend it early.
            // This is an approximation.
            let color = bevy_pbr::render::rgb9e5::rgb9e5_to_vec3_(fragment_node.color);
            let alpha = oit_alpha(fragment_node.paint_alpha);
            final_color = blend(final_color, vec4f(color, alpha));
        }
    }

    // blend sorted fragments
    for (var i = 0u; i < sorted_frag_count; i += 1) {
        let color = bevy_pbr::render::rgb9e5::rgb9e5_to_vec3_(fragment_list[i].color);
        let alpha = oit_alpha(fragment_list[i].paint_alpha);
        var base_color = vec4(color.rgb, alpha);
        final_color = blend(final_color, base_color);
        if final_color.a == 1.0 {
            break;
        }
    }

    return final_color;
}

// OVER operator using premultiplied alpha
// see: https://en.wikipedia.org/wiki/Alpha_compositing
fn blend(color_a: vec4<f32>, color_b: vec4<f32>) -> vec4<f32> {
    let final_color = color_a.rgb + (1.0 - color_a.a) * color_b.rgb;
    let alpha = color_a.a + (1.0 - color_a.a) * color_b.a;
    return vec4(final_color.rgb, alpha);
}
