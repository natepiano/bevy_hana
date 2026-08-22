use bevy_app::App;
use bevy_app::Plugin;
use bevy_app::PostUpdate;
use bevy_asset::AssetPlugin;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_scene::ScenePlugin;

use crate::AnchorSystems;
use crate::hinge_to_pose;

/// Installs scene support required by arrangement construction commands.
///
/// Applications must add Bevy's [`AssetPlugin`] before this plugin because
/// [`super::ArrangementCommandsExt::spawn_arrangement`] queues member scenes
/// through Bevy's asset-backed scene machinery. `ArrangementPlugin` adds
/// [`ScenePlugin`] only when it is absent, so composing it directly or through
/// [`crate::FoldPlugin`] never duplicates scene registration.
///
/// This plugin owns arrangement construction and materialization support, and
/// it is the sole registrar of [`hinge_to_pose`]: every hinge is created by
/// arrangement materialization, so hinge-to-pose conversion belongs to the same
/// plugin. It deliberately does not add anchor providers, anchor resolution,
/// transform propagation, or application-specific arrangement systems.
#[derive(Default)]
pub struct ArrangementPlugin;

impl Plugin for ArrangementPlugin {
    fn build(&self, app: &mut App) {
        assert!(
            app.is_plugin_added::<AssetPlugin>(),
            "ArrangementPlugin requires the application to add AssetPlugin first",
        );
        if !app.is_plugin_added::<ScenePlugin>() {
            app.add_plugins(ScenePlugin);
        }
        app.add_systems(PostUpdate, hinge_to_pose.in_set(AnchorSystems::AnimatePose));
    }
}
