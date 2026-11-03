
import bevy_pbr::render::mesh_view_bindings::view;

fn get_viewport() -> vec4<f32> {
    return view.viewport;
}
