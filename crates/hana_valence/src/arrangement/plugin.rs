use bevy_app::App;
use bevy_app::Plugin;
use bevy_asset::AssetPlugin;
use bevy_scene::ScenePlugin;

use crate::HingePlugin;

/// Installs scene support required by arrangement construction commands.
///
/// Applications must add Bevy's [`AssetPlugin`] before this plugin because
/// [`super::ArrangementCommandsExt::spawn_arrangement`] queues member scenes
/// through Bevy's asset-backed scene machinery. `ArrangementPlugin` adds
/// [`ScenePlugin`] only when it is absent, so composing it directly or through
/// [`crate::FoldPlugin`] never duplicates scene registration.
///
/// This plugin owns arrangement construction and materialization support. It
/// adds [`HingePlugin`] when that is absent, because every materialized
/// arrangement writes hinges and those hinges need their driver. Add
/// [`HingePlugin`] alone when hinges are authored by hand and no arrangement is
/// spawned: it needs neither assets nor scenes. This plugin adds no anchor
/// providers, no anchor resolution, no transform propagation, and no
/// application-specific arrangement systems.
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
        if !app.is_plugin_added::<HingePlugin>() {
            app.add_plugins(HingePlugin);
        }
    }
}
