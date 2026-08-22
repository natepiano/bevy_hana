//! Performance benchmark spawning many outlined meshes with FPS tracking.

mod benchmark_state;
mod constants;
mod grid;
mod hud;
mod results;
mod scenarios;
mod tick;
mod viewport;

use benchmark_state::BenchmarkState;
use bevy::color::palettes::css::DARK_SEA_GREEN;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::AmbientLight;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::FontSize;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Node;
use bevy::prelude::Plane3d;
use bevy::prelude::PluginGroup;
use bevy::prelude::PointLight;
use bevy::prelude::PositionType;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Text;
use bevy::prelude::TextColor;
use bevy::prelude::TextFont;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Val;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::WindowPlugin;
use bevy::prelude::default;
use bevy::prelude::on_message;
use bevy::window::PresentMode;
use bevy::winit::WinitSettings;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use constants::AMBIENT_LIGHT_BRIGHTNESS;
use constants::BENCHMARK_WINDOW_TITLE;
use constants::CAMERA_LOOK_AT;
use constants::CAMERA_POSITION;
use constants::GROUND_PLANE_SIZE;
use constants::GROUND_PLANE_SUBDIVISIONS;
use constants::GROUND_PLANE_Y;
use constants::HEADS_UP_DISPLAY_FONT_SIZE;
use constants::HEADS_UP_DISPLAY_PADDING;
use constants::HEADS_UP_DISPLAY_UPDATE_INTERVAL;
use constants::INITIALIZING_BENCHMARK_TEXT;
use constants::LIGHT_INTENSITY;
use constants::LIGHT_POSITION;
use constants::LIGHT_RANGE;
use hana_clerestory::WindowManagerPlugin;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;
use hana_liminal::LiminalPlugin;
use hana_liminal::OutlineCamera;
use hud::HudText;
use hud::HudUpdateTimer;
use hud::update_hud;
use tick::benchmark_tick;
use tick::handle_input;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: BENCHMARK_WINDOW_TITLE.into(),
                present_mode: PresentMode::AutoNoVsync,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault))
        .add_plugins(LagrangePlugin)
        .add_plugins(LiminalPlugin)
        .add_plugins(WindowManagerPlugin)
        .insert_resource(WinitSettings::continuous())
        .insert_resource(BenchmarkState::new())
        .insert_resource(HudUpdateTimer(Timer::from_seconds(
            HEADS_UP_DISPLAY_UPDATE_INTERVAL,
            TimerMode::Repeating,
        )))
        .add_systems(Startup, setup_benchmark)
        .add_systems(
            Update,
            (
                benchmark_tick,
                handle_input.run_if(on_message::<KeyboardInput>),
                update_hud,
            ),
        )
        .run();
}

fn setup_benchmark(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(CAMERA_POSITION).looking_at(CAMERA_LOOK_AT, Vec3::Y),
        OrbitCam::default(),
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        OutlineCamera,
        AmbientLight {
            brightness: AMBIENT_LIGHT_BRIGHTNESS,
            ..default()
        },
    ));

    commands.spawn((
        PointLight {
            shadow_maps_enabled: true,
            intensity: LIGHT_INTENSITY,
            range: LIGHT_RANGE,
            ..default()
        },
        Transform::from_translation(LIGHT_POSITION),
    ));

    commands.spawn((
        Mesh3d(
            meshes.add(
                Plane3d::default()
                    .mesh()
                    .size(GROUND_PLANE_SIZE, GROUND_PLANE_SIZE)
                    .subdivisions(GROUND_PLANE_SUBDIVISIONS),
            ),
        ),
        MeshMaterial3d(materials.add(Color::from(DARK_SEA_GREEN))),
        Transform::from_xyz(0.0, GROUND_PLANE_Y, 0.0),
    ));

    commands.spawn((
        Text::new(INITIALIZING_BENCHMARK_TEXT),
        TextFont {
            font_size: FontSize::Px(HEADS_UP_DISPLAY_FONT_SIZE),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(HEADS_UP_DISPLAY_PADDING),
            left: Val::Px(HEADS_UP_DISPLAY_PADDING),
            ..default()
        },
        HudText,
    ));
}
