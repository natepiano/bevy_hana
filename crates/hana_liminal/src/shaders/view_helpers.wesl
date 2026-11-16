
import bevy_pbr::render::mesh_view_bindings::view;

fn get_viewport() -> vec4<f32> {
    return view.viewport;
}

// Limit the depth tolerance to 0.0625 mm in either projection.
fn coplanar_depth_tolerance(depth: f32, depth_per_meter: f32) -> f32 {
    return min(abs(depth) * 0.000001, abs(depth_per_meter) * 0.0000625);
}
