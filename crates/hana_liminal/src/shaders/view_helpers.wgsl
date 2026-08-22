#define_import_path hana_liminal::view_helpers

#import bevy_pbr::mesh_view_bindings as view_bindings

fn get_viewport() -> vec4<f32> {
    return view_bindings::view.viewport;
}
