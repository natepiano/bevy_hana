use std::path::Path;

use bevy::asset::Assets;
use bevy::prelude::App;
use bevy::prelude::Plugin;
use bevy::prelude::Shader;

use super::constants::COMPOSE_SHADER_HANDLE;
use super::constants::FLOOD_SHADER_HANDLE;
use super::constants::HULL_SHADER_HANDLE;
use super::constants::MASK_SHADER_HANDLE;
use super::constants::VIEW_HELPERS_SHADER_HANDLE;

pub(super) struct ShaderPlugin;

impl Plugin for ShaderPlugin {
    fn build(&self, app: &mut App) {
        macro_rules! load_shader {
            ($app:ident, $handle:expr, $path:literal $(,)?) => {{
                let mut assets = $app.world_mut().resource_mut::<Assets<Shader>>();
                let Some(shader_dir) = Path::new(file!()).parent() else {
                    return;
                };
                let asset_path = shader_dir.join($path).to_string_lossy().into_owned();

                let _ = assets.insert(
                    $handle.id(),
                    Shader::from_wgsl(include_str!($path), asset_path),
                );
            }};
        }

        load_shader!(app, COMPOSE_SHADER_HANDLE, "shaders/compose_output.wgsl",);
        load_shader!(app, FLOOD_SHADER_HANDLE, "shaders/flood.wgsl");
        load_shader!(app, HULL_SHADER_HANDLE, "shaders/hull.wgsl");
        load_shader!(app, MASK_SHADER_HANDLE, "shaders/mask.wgsl");
        load_shader!(app, VIEW_HELPERS_SHADER_HANDLE, "shaders/view_helpers.wgsl");
    }
}
