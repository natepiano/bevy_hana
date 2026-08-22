//! Material helpers for diegetic panel rendering.

use bevy::asset::AssetServer;
use bevy::asset::Assets;
use bevy::asset::Handle;
use bevy::asset::LoadState;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::prelude::default;
use bevy::render::render_resource::Face;

use super::constants::DEFAULT_METALLIC;
use super::constants::DEFAULT_REFLECTANCE;
use super::constants::DEFAULT_ROUGHNESS;
use crate::layout::Sidedness;

/// Configures a `StandardMaterial`'s `double_sided` and `cull_mode` fields from
/// a [`Sidedness`] choice. Shared by shape and text material builders.
pub(crate) const fn apply_sidedness(base: &mut StandardMaterial, sidedness: Sidedness) {
    match sidedness {
        Sidedness::BothSides => {
            base.double_sided = true;
            base.cull_mode = None;
        },
        Sidedness::FrontOnly => {
            base.double_sided = false;
            base.cull_mode = Some(Face::Back);
        },
        Sidedness::BackOnly => {
            base.double_sided = false;
            base.cull_mode = Some(Face::Front);
        },
    }
}

/// Returns the library's default matte `StandardMaterial`.
///
/// `AlphaMode::Blend` so panel surfaces composite through transparency rather
/// than writing the depth buffer and occluding coplanar panel text; it also
/// avoids the visible banding of the opaque SDF route.
#[must_use]
pub fn default_panel_material() -> StandardMaterial {
    StandardMaterial {
        perceptual_roughness: DEFAULT_ROUGHNESS,
        metallic: DEFAULT_METALLIC,
        reflectance: DEFAULT_REFLECTANCE,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}

/// Resolves a handle to an asset for the current frame.
///
/// A present authored asset is used directly. A path-backed handle with an
/// in-flight [`LoadState::Loading`] load returns `None` so the producer
/// holds/skips this frame. Missing, not-loaded, or failed authored handles fall
/// back to the seeded default handle instead of reusing stale scalar values.
#[must_use]
pub(crate) fn material_asset_for_frame<'a>(
    materials: &'a Assets<StandardMaterial>,
    asset_server: &AssetServer,
    handle: &'a Handle<StandardMaterial>,
    default_handle: &'a Handle<StandardMaterial>,
) -> Option<&'a StandardMaterial> {
    if let Some(material) = materials.get(handle) {
        return Some(material);
    }
    if handle.path().is_some() && matches!(asset_server.load_state(handle), LoadState::Loading) {
        return None;
    }
    materials.get(default_handle)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::asset::AssetPlugin;
    use bevy::prelude::App;
    use bevy::prelude::AssetApp;
    use bevy::prelude::Color;
    use bevy::prelude::MinimalPlugins;

    use super::*;

    fn material_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<StandardMaterial>();
        app
    }

    fn add_material(app: &mut App, base_color: Color) -> Handle<StandardMaterial> {
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color,
                ..Default::default()
            })
    }

    #[test]
    fn loaded_authored_handle_wins_over_default() {
        let mut app = material_app();
        let asset_server = app.world().resource::<AssetServer>().clone();
        let default_handle = add_material(&mut app, Color::srgb(0.1, 0.2, 0.3));
        let authored_handle = add_material(&mut app, Color::srgb(0.8, 0.7, 0.6));
        let materials = app.world().resource::<Assets<StandardMaterial>>();

        let material =
            material_asset_for_frame(materials, &asset_server, &authored_handle, &default_handle)
                .expect("loaded authored material should resolve");

        assert_eq!(material.base_color, Color::srgb(0.8, 0.7, 0.6));
    }

    #[test]
    fn missing_non_path_handle_falls_back_to_default() {
        let mut app = material_app();
        let asset_server = app.world().resource::<AssetServer>().clone();
        let default_handle = add_material(&mut app, Color::srgb(0.1, 0.2, 0.3));
        let missing_handle = Handle::<StandardMaterial>::default();
        let materials = app.world().resource::<Assets<StandardMaterial>>();

        let material =
            material_asset_for_frame(materials, &asset_server, &missing_handle, &default_handle)
                .expect("missing in-memory handle should fall back");

        assert_eq!(material.base_color, Color::srgb(0.1, 0.2, 0.3));
    }

    #[test]
    fn loading_path_handle_holds_the_record() {
        let mut app = material_app();
        let asset_server = app.world().resource::<AssetServer>().clone();
        let default_handle = add_material(&mut app, Color::srgb(0.1, 0.2, 0.3));
        let loading_handle: Handle<StandardMaterial> =
            asset_server.load("materials/still_loading.standard_material");
        let materials = app.world().resource::<Assets<StandardMaterial>>();

        assert!(matches!(
            asset_server.load_state(&loading_handle),
            LoadState::Loading
        ));
        assert!(
            material_asset_for_frame(materials, &asset_server, &loading_handle, &default_handle,)
                .is_none(),
            "in-flight authored path material should hold for this frame"
        );
    }

    #[test]
    fn missing_path_handle_falls_back_after_load_finishes() {
        let mut app = material_app();
        let asset_server = app.world().resource::<AssetServer>().clone();
        let default_handle = add_material(&mut app, Color::srgb(0.1, 0.2, 0.3));
        let missing_handle: Handle<StandardMaterial> =
            asset_server.load("materials/does_not_exist.standard_material");

        for _ in 0..32 {
            app.update();
            if !matches!(asset_server.load_state(&missing_handle), LoadState::Loading) {
                break;
            }
        }

        assert!(
            !matches!(asset_server.load_state(&missing_handle), LoadState::Loading),
            "missing path handle should leave Loading before fallback is asserted"
        );
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let material =
            material_asset_for_frame(materials, &asset_server, &missing_handle, &default_handle)
                .expect("failed or absent path handle should fall back");

        assert_eq!(material.base_color, Color::srgb(0.1, 0.2, 0.3));
    }
}
