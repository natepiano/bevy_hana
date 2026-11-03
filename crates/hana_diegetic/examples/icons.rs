//! Lucide icons through Hana's Slug text renderer.
//!
//! The scene shows the same typed icon API on a front-facing world panel, a
//! panel slanted 70 degrees, a transparent panel seen from behind (its back is
//! plain, because icons draw on the front face only), and a screen-space row. Lucide's bundled
//! font, codepoints, and font id stay behind [`Icon`] and [`DiegeticIcon`].

use bevy::app::PluginGroup;
use bevy::prelude::AmbientLight;
use bevy::prelude::App;
use bevy::prelude::Camera3d;
use bevy::prelude::ClearColor;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Msaa;
use bevy::prelude::Name;
use bevy::prelude::Quat;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::WindowPlugin;
use bevy::prelude::default;
use bevy::window::WindowResolution;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::DiegeticIcon;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::Icon;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LucideIcon;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::TextStyle;

const BACKGROUND: Color = Color::srgb(0.014, 0.020, 0.035);
const PANEL: Color = Color::srgba(0.055, 0.078, 0.125, 0.94);
const PANEL_TRANSPARENT: Color = Color::srgba(0.08, 0.11, 0.17, 0.52);
const BORDER: Color = Color::srgba(0.27, 0.38, 0.58, 0.8);
const CYAN: Color = Color::srgb(0.28, 0.90, 1.0);
const AMBER: Color = Color::srgb(1.0, 0.62, 0.23);
const VIOLET: Color = Color::srgb(0.76, 0.58, 1.0);
const LABEL: Color = Color::srgb(0.76, 0.82, 0.92);

const PANEL_WIDTH_POINTS: f32 = 420.0;
const PANEL_HEIGHT_POINTS: f32 = 128.0;
const PANEL_WORLD_WIDTH: f32 = 4.15;
const ICON_SIZE_POINTS: f32 = 54.0;
const SCREEN_ICON_SIZE_POINTS: f32 = 28.5;
const SCREEN_ICON_X: [f32; 6] = [28.0, 82.0, 136.0, 190.0, 244.0, 298.0];
const ICONS: [LucideIcon; 6] = [
    LucideIcon::Play,
    LucideIcon::Pause,
    LucideIcon::Search,
    LucideIcon::Settings,
    LucideIcon::Eye,
    LucideIcon::Heart,
];

fn main() {
    App::new()
        .insert_resource(ClearColor(BACKGROUND))
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Lucide icons through Slug".to_string(),
                    resolution: WindowResolution::new(1_200, 800),
                    ..default()
                }),
                ..default()
            }),
            DiegeticUiPlugin,
            BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Name::new("Icon camera"),
        Camera3d::default(),
        Msaa::Off,
        StableTransparency,
        Transform::from_xyz(0.0, 0.1, 10.5).looking_at(Vec3::new(0.0, 0.1, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            ..default()
        },
        Transform::from_xyz(2.0, 5.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn(AmbientLight {
        color:                      Color::WHITE,
        brightness:                 240.0,
        affects_lightmapped_meshes: false,
    });

    spawn_world_panel(
        &mut commands,
        "Front icon panel",
        "WORLD — FRONT",
        Vec3::new(-2.45, 1.45, 0.0),
        Quat::IDENTITY,
        PANEL,
        CYAN,
    );
    spawn_world_panel(
        &mut commands,
        "Slanted icon panel",
        "WORLD — 70° SLANT",
        Vec3::new(2.4, 1.45, 0.0),
        Quat::from_rotation_y(70.0_f32.to_radians()),
        PANEL,
        AMBER,
    );
    spawn_world_panel(
        &mut commands,
        "Rear icon panel",
        "TWO-SIDED — REAR VIEW",
        Vec3::new(0.0, -1.35, 0.0),
        Quat::from_rotation_y(std::f32::consts::PI),
        PANEL_TRANSPARENT,
        VIOLET,
    );
    spawn_screen_row(&mut commands);
}

fn spawn_world_panel(
    commands: &mut Commands,
    name: &'static str,
    label: &'static str,
    translation: Vec3,
    rotation: Quat,
    background: Color,
    icon_color: Color,
) {
    let panel = DiegeticPanel::world()
        .size(Pt(PANEL_WIDTH_POINTS), Pt(PANEL_HEIGHT_POINTS))
        .world_width(PANEL_WORLD_WIDTH)
        .anchor(Anchor::Center)
        .with_tree(icon_panel_tree(label, background, icon_color))
        .build();
    let Ok(panel) = panel else {
        return;
    };
    commands.spawn((
        Name::new(name),
        panel,
        Transform::from_translation(translation).with_rotation(rotation),
    ));
}

fn icon_panel_tree(
    label: &'static str,
    background: Color,
    icon_color: Color,
) -> hana_diegetic::LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(14.0))
            .gap(10.0)
            .background(background)
            .border(Border::all(1.5, BORDER)),
    );
    builder.text((label, TextStyle::new(17.0).with_color(LABEL)));
    builder.with(
        El::row().width(Sizing::FIT).height(Sizing::FIT).gap(11.0),
        |row| {
            for glyph in ICONS {
                row.icon(Icon::new(glyph).size(ICON_SIZE_POINTS).color(icon_color));
            }
        },
    );
    builder.build()
}

fn spawn_screen_row(commands: &mut Commands) {
    DiegeticText::screen("SCREEN SPACE")
        .size(17.0)
        .color(LABEL)
        .screen_position(28.0, 24.0)
        .spawn(commands);

    for (index, glyph) in ICONS.into_iter().enumerate() {
        let color = match index % 3 {
            0 => CYAN,
            1 => AMBER,
            _ => VIOLET,
        };
        DiegeticIcon::screen(glyph)
            .size(SCREEN_ICON_SIZE_POINTS)
            .color(color)
            .screen_position(Vec2::new(SCREEN_ICON_X[index], 52.0))
            .spawn(commands);
    }
}
