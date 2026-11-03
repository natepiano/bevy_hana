use std::path::Path;

use bevy::asset::Assets;
use bevy::prelude::App;
use bevy::prelude::Plugin;
use bevy::prelude::Shader;

use super::OutlineDepthTestPlugin;
use super::constants::COMPOSE_SHADER_HANDLE;
use super::constants::FLOOD_SHADER_HANDLE;
use super::constants::HULL_SHADER_HANDLE;
use super::constants::MASK_SHADER_HANDLE;
use super::constants::VIEW_HELPERS_SHADER_HANDLE;

pub(super) struct ShaderPlugin;

impl Plugin for ShaderPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<OutlineDepthTestPlugin>() {
            app.add_plugins(OutlineDepthTestPlugin);
        }
        macro_rules! load_shader {
            ($app:ident, $handle:expr, $path:literal $(,)?) => {{
                let mut assets = $app.world_mut().resource_mut::<Assets<Shader>>();
                let Some(file_name) = Path::new($path).file_name() else {
                    return;
                };
                let asset_path = format!("embedded://hana_liminal/{}", file_name.to_string_lossy());

                let _ = assets.insert(
                    $handle.id(),
                    Shader::from_wesl(include_str!($path), asset_path),
                );
            }};
        }

        load_shader!(app, COMPOSE_SHADER_HANDLE, "shaders/compose_output.wesl",);
        load_shader!(app, FLOOD_SHADER_HANDLE, "shaders/flood.wesl");
        load_shader!(app, HULL_SHADER_HANDLE, "shaders/hull.wesl");
        load_shader!(app, MASK_SHADER_HANDLE, "shaders/mask.wesl");
        load_shader!(app, VIEW_HELPERS_SHADER_HANDLE, "shaders/view_helpers.wesl");
    }
}
