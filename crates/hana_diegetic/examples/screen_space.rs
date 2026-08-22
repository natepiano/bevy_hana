//! Screen-space HUD overlay example.
//!
//! Demonstrates a [`ScreenSpace`] panel rendered as a 2D overlay on top
//! of a 3D scene. The HUD panel stays fixed in screen space regardless
//! of camera movement — text is sized in logical pixels.

use bevy::prelude::AmbientLight;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::DirectionalLight;
use bevy::prelude::EulerRot;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Plane3d;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use hana_clerestory::WindowManagerPlugin;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;

const PANEL_WIDTH: f32 = 480.0;
const PANEL_HEIGHT: f32 = 310.0;
const TITLE_SIZE: f32 = 22.0;
const HEADER_SIZE: f32 = 16.0;
const BODY_SIZE: f32 = 15.0;
const PANEL_PADDING: f32 = 12.0;
const CAPTION_SIZE: f32 = 14.0;
const CAPTION_INSET: f32 = 16.0;
const CAPTION_COLOR: Color = Color::srgba(0.7, 0.8, 1.0, 0.85);

const PANEL_BACKGROUND: Color = Color::srgba(0.08, 0.08, 0.12, 0.85);
const BORDER_COLOR: Color = Color::srgba(0.3, 0.5, 0.9, 0.6);
const TITLE_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
const HEADER_COLOR: Color = Color::srgb(0.8, 0.85, 1.0);
const BODY_COLOR: Color = Color::srgb(0.85, 0.85, 0.9);
const VALUE_COLOR: Color = Color::srgb(0.6, 1.0, 0.7);
const WARN_COLOR: Color = Color::srgb(1.0, 0.8, 0.35);

/// Marker for the spinning cube.
#[derive(Component)]
struct SpinCube;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            DiegeticUiPlugin,
            LagrangePlugin,
            BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
            WindowManagerPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, spin_cube)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    spawn_scene(&mut commands, &mut meshes, &mut materials);
    spawn_hud(&mut commands);
    spawn_caption(&mut commands);
    spawn_camera(&mut commands);
}

fn spawn_scene(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    // Spinning cube.
    commands.spawn((
        SpinCube,
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.5, 0.9),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));

    // Ground plane.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(3.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.15, 0.15, 0.15),
            ..default()
        })),
    ));

    // Lighting.
    commands.spawn((
        DirectionalLight {
            illuminance: 5000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(
            EulerRot::XYZ,
            -std::f32::consts::FRAC_PI_4,
            std::f32::consts::FRAC_PI_4,
            0.0,
        )),
    ));

    commands.spawn(AmbientLight {
        color:                      Color::WHITE,
        brightness:                 300.0,
        affects_lightmapped_meshes: false,
    });
}

fn spawn_hud(commands: &mut Commands) {
    let hud_panel = DiegeticPanel::screen()
        .size(
            Sizing::fixed(Px(PANEL_WIDTH)),
            Sizing::fixed(Px(PANEL_HEIGHT)),
        )
        .anchor(Anchor::Center)
        .layout(|b| {
            // Outer frame.
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .padding(Padding::all(3.0))
                    .background(Color::srgba(0.05, 0.05, 0.08, 0.9))
                    .border(Border::all(2.0, BORDER_COLOR)),
                |b| {
                    b.with(
                        El::column()
                            .width(Sizing::GROW)
                            .height(Sizing::GROW)
                            .padding(Padding::all(PANEL_PADDING))
                            .gap(6.0)
                            .background(PANEL_BACKGROUND)
                            .border(Border::all(1.0, Color::srgba(0.2, 0.3, 0.6, 0.4))),
                        |b| {
                            // Title.
                            b.text((
                                "Mission Control",
                                TextStyle::new(TITLE_SIZE).with_color(TITLE_COLOR),
                            ));
                            divider(b);

                            // Two-column layout.
                            b.with(
                                El::row().width(Sizing::GROW).height(Sizing::GROW).gap(12.0),
                                |b| {
                                    // Left column — Ship Status.
                                    column(
                                        b,
                                        "Ship Status",
                                        &[
                                            ("Hull", "98%", VALUE_COLOR),
                                            ("Shields", "74%", WARN_COLOR),
                                            ("Fuel", "1,247 kg", VALUE_COLOR),
                                            ("Velocity", "342 m/s", VALUE_COLOR),
                                            ("Heading", "045.2\u{00b0}", BODY_COLOR),
                                            ("Altitude", "12.4 km", VALUE_COLOR),
                                        ],
                                    );

                                    // Vertical divider.
                                    b.with(
                                        El::new()
                                            .width(Sizing::fixed(1.0))
                                            .height(Sizing::GROW)
                                            .background(BORDER_COLOR),
                                        |_| {},
                                    );

                                    // Right column — Environment.
                                    column(
                                        b,
                                        "Environment",
                                        &[
                                            ("Sector", "Gamma-7", BODY_COLOR),
                                            ("Hostiles", "3", WARN_COLOR),
                                            ("Friendlies", "12", VALUE_COLOR),
                                            ("Comms", "Online", VALUE_COLOR),
                                            ("Gravity", "0.38 g", BODY_COLOR),
                                            ("Temp", "-142 C", BODY_COLOR),
                                        ],
                                    );
                                },
                            );
                        },
                    );
                },
            );
        })
        .build();
    let Ok(hud_panel) = hud_panel else {
        error!("failed to build HUD dimensions");
        return;
    };

    commands.spawn((hud_panel, Transform::from_xyz(-250.0, 150.0, 0.0)));
}

/// Demonstrates the [`ScreenText`] builder: a one-line, pixel-positioned overlay
/// label spawned with a single fluent chain, in contrast to the hand-built
/// multi-element HUD panel above. Inset from the window's top-left corner.
fn spawn_caption(commands: &mut Commands) {
    DiegeticText::screen("ScreenText \u{2014} one fluent call, pixel-positioned")
        .size(CAPTION_SIZE)
        .color(CAPTION_COLOR)
        .screen_position(CAPTION_INSET, CAPTION_INSET)
        .spawn(commands);
}

fn divider(b: &mut LayoutBuilder) {
    b.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(1.0))
            .background(BORDER_COLOR),
        |_| {},
    );
}

fn column(b: &mut LayoutBuilder, title: &str, rows: &[(&str, &str, Color)]) {
    b.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(4.0),
        |b| {
            b.text((title, TextStyle::new(HEADER_SIZE).with_color(HEADER_COLOR)));
            for &(label, value, color) in rows {
                b.with(
                    El::row().width(Sizing::GROW).height(Sizing::FIT).gap(4.0),
                    |b| {
                        b.text((label, TextStyle::new(BODY_SIZE).with_color(BODY_COLOR)));
                        b.with(
                            El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                            |_| {},
                        );
                        b.text((value, TextStyle::new(BODY_SIZE).with_color(color)));
                    },
                );
            }
        },
    );
}

fn spawn_camera(commands: &mut Commands) {
    commands.spawn((
        Transform {
            translation: Vec3::new(3.27, 2.24, 3.29),
            rotation:    Quat::from_xyzw(-0.1476, 0.4041, 0.0663, 0.9003),
            scale:       Vec3::ONE,
        },
        OrbitCam::default(),
    ));
}

fn spin_cube(time: Res<Time>, mut cubes: Query<&mut Transform, With<SpinCube>>) {
    for mut transform in &mut cubes {
        transform.rotate_y(0.5 * time.delta_secs());
    }
}
