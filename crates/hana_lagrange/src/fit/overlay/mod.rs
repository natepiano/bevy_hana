//! Debug overlay system for fit target debugging.
//!
//! Provides retained screen-aligned boundary box, silhouette polygon, margin
//! line, and label visualization for the current camera fit target.

mod constants;
mod geometry;
mod render;

use bevy::asset::AssetServer;
use bevy::camera::visibility::VisibilitySystems;
use bevy::pbr::MaterialPlugin;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Component;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::SystemSet;
use bevy::prelude::any_with_component;
use bevy::transform::TransformSystems;
use render::FitOverlayLineMaterial;
use render::FitOverlayLineMaterials;
pub use render::FitTargetOverlayConfig;

/// Enables the fit target debug overlay on a camera entity.
///
/// Insert this component to enable the overlay; remove it to disable the
/// overlay. The presence or absence of `FitOverlay` is the toggle.
///
/// Generated overlay visuals are owned by this camera. Retained line visuals
/// copy this camera's effective `RenderLayers`, render through normal Bevy
/// layer-intersection visibility, and do not add another render visibility
/// filter. Labels are plain Bevy UI nodes targeted through `UiTargetCamera`.
/// `Camera::order` keeps its normal pass-order meaning.
#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
pub struct FitOverlay;

/// System set for resolving and reconciling fit-overlay visuals.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
struct FitOverlaySystemSet;

/// Plugin that enables fit target debug visualization.
pub(super) struct FitOverlayPlugin;

impl Plugin for FitOverlayPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<AssetServer>() {
            app.add_plugins(MaterialPlugin::<FitOverlayLineMaterial>::default());
        } else {
            app.init_resource::<Assets<FitOverlayLineMaterial>>();
        }

        app.init_resource::<FitTargetOverlayConfig>()
            .init_resource::<FitOverlayLineMaterials>()
            .add_observer(render::on_remove_fit_visualization)
            .configure_sets(
                PostUpdate,
                FitOverlaySystemSet
                    .after(TransformSystems::Propagate)
                    .before(VisibilitySystems::VisibilityPropagate)
                    .before(VisibilitySystems::CheckVisibility),
            )
            .add_systems(
                PostUpdate,
                (
                    render::deduplicate_fit_overlay_visuals,
                    render::draw_fit_target_bounds,
                )
                    .chain()
                    .in_set(FitOverlaySystemSet)
                    .run_if(any_with_component::<FitOverlay>),
            )
            .add_systems(
                PostUpdate,
                render::cleanup_orphan_fit_overlay_visuals.in_set(FitOverlaySystemSet),
            );
    }
}
