//! The Lucide font is supplied by `hana_diegetic`, independent of the app's asset root.

use std::path::Path;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::settings::RenderCreation;
use bevy::render::settings::WgpuSettings;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::FontFailureSource;
use hana_diegetic::FontLoadFailed;
use hana_diegetic::FontRegistered;

const LUCIDE_FONT_FAMILY: &str = "lucide";
const LUCIDE_FONT_PATH: &str = "fonts/lucide.ttf";

#[derive(Resource, Default)]
struct LucideFontEvents {
    registrations: usize,
    load_failures: usize,
}

fn record_lucide_registration(trigger: On<FontRegistered>, mut events: ResMut<LucideFontEvents>) {
    if trigger.name == LUCIDE_FONT_FAMILY {
        events.registrations += 1;
    }
}

fn record_lucide_load_failure(trigger: On<FontLoadFailed>, mut events: ResMut<LucideFontEvents>) {
    if matches!(
        &trigger.source,
        FontFailureSource::AssetPath(path) if path.path() == Path::new(LUCIDE_FONT_PATH)
    ) {
        events.load_failures += 1;
    }
}

#[test]
fn lucide_font_registers_without_a_copy_in_the_apps_asset_root() {
    let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/assets-without-lucide");
    assert!(!asset_root.join(LUCIDE_FONT_PATH).exists());

    let mut app = App::new();
    app.init_resource::<LucideFontEvents>()
        .add_observer(record_lucide_registration)
        .add_observer(record_lucide_load_failure)
        .add_plugins(
            DefaultPlugins
                .build()
                .set(AssetPlugin {
                    file_path: asset_root.to_string_lossy().into_owned(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                        backends: None,
                        ..default()
                    })),
                    ..default()
                })
                .disable::<WinitPlugin>()
                .disable::<PipelinedRenderingPlugin>(),
        )
        .add_plugins(DiegeticUiPlugin);

    loop {
        app.update();
        let events = app.world().resource::<LucideFontEvents>();
        if events.registrations != 0 || events.load_failures != 0 {
            break;
        }
        std::thread::yield_now();
    }

    assert_eq!(
        app.world().resource::<LucideFontEvents>().registrations,
        1,
        "the Lucide font should register from hana_diegetic when it is absent from the app's asset root",
    );
}
