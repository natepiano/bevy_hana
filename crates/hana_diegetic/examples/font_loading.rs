//! @generated `bevy_example_template`
//! Font loading validation.
//!
//! Demonstrates loading fonts via `AssetServer` and observing
//! `FontRegistered` / `FontLoadFailed` events. The embedded `JetBrains
//! Mono` font is available at startup; `Noto Sans` loads asynchronously
//! from the assets directory. A `DiegeticPanel` shows registered fonts as
//! they arrive, and `WorldText` labels render each font name in its own font.

use std::time::Duration;

use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::AlphaMode;
use bevy::prelude::AmbientLight;
use bevy::prelude::App;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::Children;
use bevy::prelude::Click;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::On;
use bevy::prelude::Plane3d;
use bevy::prelude::Pointer;
use bevy::prelude::PointerButton;
use bevy::prelude::Query;
use bevy::prelude::RelationshipTarget;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy::prelude::info;
use bevy::prelude::warn;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use hana_clerestory::WindowManagerPlugin;
use hana_diegetic::Border;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::Font;
use hana_diegetic::FontLoadFailed;
use hana_diegetic::FontRegistered;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_kana::ToF32;
use hana_lagrange::Focus;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitAngles;
use hana_lagrange::OrbitCam;
use hana_lagrange::Radius;
use hana_lagrange::ZoomToFit;

const ZOOM_MARGIN_TEXT: f32 = 0.3;
const ZOOM_MARGIN_SCENE: f32 = 0.08;
const ZOOM_DURATION_MS: u64 = 1000;

/// Font size for the sample text.
const SAMPLE_SIZE: f32 = 0.28;

/// Vertical spacing between font samples.
const LINE_SPACING: f32 = 0.5;

/// Layout dimensions for the status panel (in meters).
const STATUS_LAYOUT_WIDTH: f32 = 0.08;
const STATUS_LAYOUT_HEIGHT: f32 = 0.03;

/// Font size for the status panel text (in millimeters).
const STATUS_FONT_SIZE: f32 = 3.5;

/// Background color for panels.
const PANEL_BG: Color = Color::srgba(0.1, 0.1, 0.12, 0.85);

/// Border color for panels.
const PANEL_BORDER_COLOR: Color = Color::srgb(0.4, 0.4, 0.45);
const MISSING_FONT_ASSET_PATH: &str = "fonts/DoesNotExist.ttf";
const NOTO_SANS_REGULAR_FONT_ASSET_PATH: &str = "fonts/NotoSans-Regular.ttf";

/// Tracks how many fonts have been registered (for vertical positioning).
#[derive(Resource, Default)]
struct FontCount(usize);

/// Keeps font handles alive so Bevy doesn't unload the assets.
#[derive(Resource, Default)]
struct FontHandles(Vec<Handle<Font>>);

/// Marker for the status panel.
#[derive(Component)]
struct StatusPanel;

#[derive(Resource)]
struct SceneBounds(Entity);

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            DiegeticUiPlugin,
            LagrangePlugin,
            BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
            WindowManagerPlugin,
            MeshPickingPlugin,
        ))
        .init_resource::<FontCount>()
        .init_resource::<FontHandles>()
        .add_observer(on_font_registered)
        .add_observer(on_font_load_failed)
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    mut font_handles: ResMut<FontHandles>,
) {
    // Ground plane.
    let ground = commands
        .spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(12.0, 12.0))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgba(0.08, 0.08, 0.08, 0.8),
                alpha_mode: AlphaMode::Blend,
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
        ))
        .observe(on_ground_clicked)
        .id();

    commands.insert_resource(SceneBounds(ground));

    // Load Noto Sans asynchronously from assets directory.
    font_handles
        .0
        .push(asset_server.load(NOTO_SANS_REGULAR_FONT_ASSET_PATH));

    // Try loading a font that doesn't exist to test FontLoadFailed.
    font_handles
        .0
        .push(asset_server.load(MISSING_FONT_ASSET_PATH));

    // Light.
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: false,
            illuminance: 5000.0,
            ..default()
        },
        Transform::from_xyz(2.0, 6.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(-2.0, 6.0, -8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Camera.
    commands.spawn((
        AmbientLight {
            color:                      Color::WHITE,
            brightness:                 300.0,
            affects_lightmapped_meshes: false,
        },
        OrbitCam::from_pose(
            Focus(Vec3::new(0.0, 2.0, 0.0)),
            OrbitAngles {
                yaw:   0.0,
                pitch: -0.1,
            },
            Radius(5.0),
        ),
    ));

    // Status panel.
    let status_panel = DiegeticPanel::world()
        .size(STATUS_LAYOUT_WIDTH, STATUS_LAYOUT_HEIGHT)
        .font_unit(Unit::Millimeters)
        .with_tree(build_status_panel("Fonts: loading..."))
        .build();
    let Ok(status_panel) = status_panel else {
        error!("failed to build status panel dimensions");
        return;
    };

    commands.spawn((
        StatusPanel,
        status_panel,
        Transform::from_xyz(-1.54, 3.515, 0.0),
    ));
}

fn build_status_panel(text: &str) -> LayoutTree {
    let mut builder = LayoutBuilder::new(STATUS_LAYOUT_WIDTH, STATUS_LAYOUT_HEIGHT);
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .padding(Padding::all(0.002))
            .gap(0.001)
            .background(PANEL_BG)
            .border(Border::all(0.0005, PANEL_BORDER_COLOR)),
        |b| {
            b.text((
                text,
                TextStyle::new(STATUS_FONT_SIZE)
                    .with_color(Color::srgba(1.0, 1.0, 1.0, 0.9))
                    .with_shadow_mode(GlyphShadowMode::None),
            ));
        },
    );
    builder.build()
}

/// Observer: fires when a font is successfully registered.
fn on_font_registered(
    trigger: On<FontRegistered>,
    mut font_count: ResMut<FontCount>,
    panels: Query<Entity, With<StatusPanel>>,
    mut commands: Commands,
) {
    let idx = font_count.0;
    font_count.0 += 1;

    let y = idx.to_f32().mul_add(LINE_SPACING, 1.5);

    // Spawn a WorldText label in this font.
    let label = format!(
        "[FontId {}] {} ({})",
        trigger.id.0, trigger.name, trigger.source
    );
    commands
        .spawn(
            DiegeticText::world(label)
                .size(SAMPLE_SIZE)
                .font(trigger.id.0)
                .color(Color::srgb(0.2, 0.3, 0.9))
                .shadow_mode(GlyphShadowMode::None)
                .transform(Transform::from_xyz(0.0, y, 0.0))
                .build(),
        )
        .observe(on_text_clicked);

    // Update status panel.
    let status = format!("Fonts registered: {}", font_count.0);
    for entity in &panels {
        if let Err(error) = commands.set_tree(entity, build_status_panel(&status)) {
            error!("failed to replace font status panel tree: {error}");
        }
    }

    info!(
        "FontRegistered: {} (id: {}, {})",
        trigger.name, trigger.id.0, trigger.source
    );
}

/// Observer: fires when a font fails to load.
fn on_font_load_failed(
    trigger: On<FontLoadFailed>,
    panels: Query<Entity, With<StatusPanel>>,
    mut commands: Commands,
) {
    warn!("FontLoadFailed: {} — {}", trigger.path, trigger.error);

    let status = format!("FAILED: {}", trigger.path);
    for entity in &panels {
        if let Err(error) = commands.set_tree(entity, build_status_panel(&status)) {
            error!("failed to replace font failure panel tree: {error}");
        }
    }
}

fn on_text_clicked(
    mut click: On<Pointer<Click>>,
    children: Query<&Children>,
    meshes_q: Query<(), With<Mesh3d>>,
    mut commands: Commands,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    click.propagate(false);
    let camera = click.hit.camera;
    let target = children
        .get(click.entity)
        .ok()
        .and_then(|kids| kids.iter().find(|&kid| meshes_q.contains(kid)))
        .unwrap_or(click.entity);
    commands.trigger(
        ZoomToFit::new(camera, target)
            .margin(ZOOM_MARGIN_TEXT)
            .duration(Duration::from_millis(ZOOM_DURATION_MS)),
    );
}

fn on_ground_clicked(click: On<Pointer<Click>>, mut commands: Commands, scene: Res<SceneBounds>) {
    if click.button != PointerButton::Primary {
        return;
    }
    let camera = click.hit.camera;
    commands.trigger(
        ZoomToFit::new(camera, scene.0)
            .margin(ZOOM_MARGIN_SCENE)
            .duration(Duration::from_millis(ZOOM_DURATION_MS)),
    );
}
