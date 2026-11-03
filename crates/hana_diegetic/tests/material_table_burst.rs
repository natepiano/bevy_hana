//! A text burst must fit in the material table on the frame that creates it.

use bevy::asset::AssetApp;
use bevy::asset::AssetPlugin;
use bevy::gizmos::GizmoAsset;
use bevy::prelude::App;
use bevy::prelude::Image;
use bevy::prelude::Mesh;
use bevy::prelude::MinimalPlugins;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::Shader;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Mm;
use hana_diegetic::TextStyle;

const BURST_RUNS: usize = 320;

#[test]
fn text_burst_grows_the_material_table_in_its_first_frame() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Shader>()
        .init_asset::<Mesh>()
        .init_asset::<GizmoAsset>()
        .init_asset::<Image>()
        .init_asset::<ShaderBuffer>()
        .add_plugins(DiegeticUiPlugin);

    for index in 0..BURST_RUNS {
        let mut builder = LayoutBuilder::new(20.0, 5.0);
        builder.text((format!("Run {index}"), TextStyle::new(2.0)));
        let panel = DiegeticPanel::world()
            .size(Mm(20.0), Mm(5.0))
            .with_tree(builder.build())
            .build();
        assert!(panel.is_ok(), "burst panel should build");
        let Ok(panel) = panel else {
            return;
        };
        app.world_mut().spawn(panel);
    }

    app.update();

    let perf = app.world().resource::<DiegeticPerfStats>();
    assert_eq!(perf.material_table.rows.get(), BURST_RUNS);
    assert!(perf.material_table.capacity.get() >= BURST_RUNS);
}
