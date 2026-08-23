//! @generated `bevy_example_template`
//! Interactive gallery of all outline methods applied to various mesh types.

use std::time::Duration;

use bevy::picking::mesh_picking::MeshPickingPlugin;
use bevy::prelude::App;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Click;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::DirectionalLight;
use bevy::prelude::Entity;
use bevy::prelude::EulerRot;
use bevy::prelude::FontSize;
use bevy::prelude::Handle;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Name;
use bevy::prelude::Node;
use bevy::prelude::On;
use bevy::prelude::Plane3d;
use bevy::prelude::Pointer;
use bevy::prelude::PointerButton;
use bevy::prelude::PositionType;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Sphere;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Text;
use bevy::prelude::Text2d;
use bevy::prelude::TextColor;
use bevy::prelude::TextFont;
use bevy::prelude::Torus;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Val;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::WorldAssetRoot;
use bevy::prelude::default;
use bevy::prelude::info;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use hana_clerestory::WindowManagerPlugin;
use hana_kana::ToF32;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::ZoomToFit;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;
use hana_liminal::OutlineMethod;
use hana_liminal::OverlapMode;

// assets
const SCENE_ASSET_PATH: &str = "spaceship.glb#Scene0";

// camera and lighting
const CAMERA_FOCUS: Vec3 = Vec3::ZERO;
const CAMERA_POSITION: Vec3 = Vec3::new(0.0, 12.0, 18.0);
const LIGHT_POSITION: Vec3 = Vec3::new(4.0, 8.0, 4.0);

// entity names
const CUBE_NAME_PREFIX: &str = "Cube (";
const NAME_SUFFIX: &str = ")";
const SPACESHIP_NAME_PREFIX: &str = "Spaceship (";
const SPHERE_NEGATIVE_X_NAME: &str = "Sphere -X";
const SPHERE_POSITIVE_X_NAME: &str = "Sphere +X";
const TORUS_NAME_PREFIX: &str = "Torus (";

// environment
const GROUND_COLOR: Color = Color::srgb(0.3, 0.5, 0.3);
const GROUND_SIZE: f32 = 18.0;

// grid layout
const CUBE_ROW_Y: f32 = 1.0;
const CUBE_ROW_Z: f32 = 0.0;
const GRID_CENTER_COLUMN: f32 = 1.0;
const GRID_SPACING: f32 = 5.0;
const SPACESHIP_ROW_Y: f32 = 1.5;
const SPACESHIP_ROW_Z: f32 = GRID_SPACING;
const SPACESHIP_SCALE: f32 = 0.3;
const TORUS_ROW_Y: f32 = 1.0;
const TORUS_ROW_Z: f32 = -GRID_SPACING;

// labels
const COLUMN_LABEL_FONT_SIZE: f32 = 24.0;
const COLUMN_LABEL_X_SCALE: f32 = 80.0;
const COLUMN_LABEL_Y: f32 = 280.0;
const COLUMN_LABELS: &[(OutlineMethod, &str)] = &[
    (OutlineMethod::WorldHull, "WorldHull"),
    (OutlineMethod::ScreenHull, "ScreenHull"),
    (OutlineMethod::JumpFlood, "JumpFlood"),
];
const INITIAL_OVERLAP_LABEL: &str = "Overlap: Merged";
const OVERLAP_LABEL_PREFIX: &str = "Overlap: ";
const OVERLAP_MODE_GROUPED_LABEL: &str = "Grouped";
const OVERLAP_MODE_MERGED_LABEL: &str = "Merged";
const OVERLAP_MODE_PER_MESH_LABEL: &str = "PerMesh";

// logging
const GROUND_CLICK_LOG_MESSAGE: &str = "Ground clicked, zooming to scene bounds";
const MESH_CLICK_LOG_PREFIX: &str = "Mesh clicked: ";

// meshes
const CUBE_BASE_COLOR: Color = Color::srgb(0.8, 0.7, 0.6);
const SPHERE_BASE_COLOR: Color = Color::srgb(0.65, 0.55, 0.75);
const SPHERE_CHILD_OFFSET_X: f32 = 0.5;
const SPHERE_RADIUS: f32 = 0.25;
const SPHERE_UV_LATITUDES: u32 = 16;
const SPHERE_UV_LONGITUDES: u32 = 32;
const TORUS_BASE_COLOR: Color = Color::srgb(0.2, 0.7, 0.3);
const TORUS_INNER_RADIUS: f32 = 0.25;
const TORUS_MAJOR_RESOLUTION: usize = 64;
const TORUS_MINOR_RESOLUTION: usize = 64;
const TORUS_OUTER_RADIUS: f32 = 0.75;

// outline
const OUTLINE_COLOR: Color = Color::srgb(0.0, 0.8, 1.0);
const OUTLINE_INTENSITY: f32 = 1.5;
const OUTLINE_WIDTH: f32 = 4.0;
const WORLD_HULL_OUTLINE_WIDTH: f32 = 0.03;

// rotations
const COLUMN_ROTATION_ANGLES: [(f32, f32, f32); 3] =
    [(0.7, 0.4, 0.0), (0.0, 0.0, 0.0), (-0.7, -0.9, 0.15)];

// ui
const GROUND_CLICK_UI_TEXT: &str = "Click a mesh to zoom-to-fit\nClick the ground to zoom back out\nPress 'O' to toggle overlap mode\n\nColumns: WorldHull | ScreenHull | JumpFlood\nRows: Torus | Cube | Spaceship";
const OVERLAP_LABEL_COLOR: Color = Color::srgba(1.0, 1.0, 0.5, 0.9);
const UI_PADDING: f32 = 12.0;
const UI_TEXT_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.8);
const UI_TEXT_FONT_SIZE: f32 = 16.0;

// zoom
const ZOOM_DURATION_MS: u64 = 1000;
const ZOOM_MARGIN_MESH: f32 = 0.15;
const ZOOM_MARGIN_SCENE: f32 = 0.08;

struct MeshAndMaterial {
    mesh:     Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

#[derive(Resource)]
struct SceneBounds(Entity);

#[derive(Component)]
struct OverlapLabel;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            LagrangePlugin,
            BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
            WindowManagerPlugin,
            MeshPickingPlugin,
            LiminalPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, toggle_overlap)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let cube = MeshAndMaterial {
        mesh:     meshes.add(Cuboid::default()),
        material: materials.add(StandardMaterial {
            base_color: CUBE_BASE_COLOR,
            ..default()
        }),
    };
    let sphere = MeshAndMaterial {
        mesh:     meshes.add(
            Sphere::new(SPHERE_RADIUS)
                .mesh()
                .uv(SPHERE_UV_LONGITUDES, SPHERE_UV_LATITUDES),
        ),
        material: materials.add(StandardMaterial {
            base_color: SPHERE_BASE_COLOR,
            ..default()
        }),
    };
    let torus = MeshAndMaterial {
        mesh:     meshes.add(
            Torus::new(TORUS_INNER_RADIUS, TORUS_OUTER_RADIUS)
                .mesh()
                .minor_resolution(TORUS_MINOR_RESOLUTION)
                .major_resolution(TORUS_MAJOR_RESOLUTION),
        ),
        material: materials.add(StandardMaterial {
            base_color: TORUS_BASE_COLOR,
            ..default()
        }),
    };

    spawn_outline_grid(&mut commands, &cube, &sphere, &torus, &asset_server);
    spawn_environment(&mut commands, &mut meshes, &mut materials);
    spawn_ui(&mut commands);
}

fn spawn_outline_grid(
    commands: &mut Commands,
    cube: &MeshAndMaterial,
    sphere: &MeshAndMaterial,
    torus: &MeshAndMaterial,
    asset_server: &AssetServer,
) {
    let column_rotations = COLUMN_ROTATION_ANGLES
        .map(|(yaw, pitch, roll)| Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll));

    for (col, &(mode, label)) in COLUMN_LABELS.iter().enumerate() {
        let x = (col.to_f32() - GRID_CENTER_COLUMN) * GRID_SPACING;
        let rotation = column_rotations[col];
        let outline = match mode {
            OutlineMethod::JumpFlood => Outline::jump_flood(OUTLINE_WIDTH)
                .with_color(OUTLINE_COLOR)
                .with_intensity(OUTLINE_INTENSITY)
                .build(),
            OutlineMethod::WorldHull => Outline::world_hull(WORLD_HULL_OUTLINE_WIDTH)
                .with_color(OUTLINE_COLOR)
                .with_intensity(OUTLINE_INTENSITY)
                .build(),
            OutlineMethod::ScreenHull => Outline::screen_hull(OUTLINE_WIDTH)
                .with_color(OUTLINE_COLOR)
                .with_intensity(OUTLINE_INTENSITY)
                .build(),
        };

        commands
            .spawn((
                Name::new(format!("{TORUS_NAME_PREFIX}{label}{NAME_SUFFIX}")),
                Mesh3d(torus.mesh.clone()),
                MeshMaterial3d(torus.material.clone()),
                Transform {
                    translation: Vec3::new(x, TORUS_ROW_Y, TORUS_ROW_Z),
                    rotation,
                    ..default()
                },
                outline.clone(),
            ))
            .observe(on_mesh_clicked);

        commands
            .spawn((
                Name::new(format!("{CUBE_NAME_PREFIX}{label}{NAME_SUFFIX}")),
                Mesh3d(cube.mesh.clone()),
                MeshMaterial3d(cube.material.clone()),
                Transform {
                    translation: Vec3::new(x, CUBE_ROW_Y, CUBE_ROW_Z),
                    rotation,
                    ..default()
                },
                outline.clone(),
            ))
            .observe(on_mesh_clicked)
            .with_children(|parent| {
                parent.spawn((
                    Name::new(SPHERE_POSITIVE_X_NAME),
                    Mesh3d(sphere.mesh.clone()),
                    MeshMaterial3d(sphere.material.clone()),
                    Transform::from_xyz(SPHERE_CHILD_OFFSET_X, 0.0, 0.0),
                ));
                parent.spawn((
                    Name::new(SPHERE_NEGATIVE_X_NAME),
                    Mesh3d(sphere.mesh.clone()),
                    MeshMaterial3d(sphere.material.clone()),
                    Transform::from_xyz(-SPHERE_CHILD_OFFSET_X, 0.0, 0.0),
                ));
            });

        commands
            .spawn((
                Name::new(format!("{SPACESHIP_NAME_PREFIX}{label}{NAME_SUFFIX}")),
                WorldAssetRoot(asset_server.load(SCENE_ASSET_PATH)),
                Transform {
                    translation: Vec3::new(x, SPACESHIP_ROW_Y, SPACESHIP_ROW_Z),
                    rotation,
                    scale: Vec3::splat(SPACESHIP_SCALE),
                },
                outline.clone(),
            ))
            .observe(on_mesh_clicked);

        commands.spawn((
            Text2d::new(label),
            TextFont {
                font_size: FontSize::Px(COLUMN_LABEL_FONT_SIZE),
                ..default()
            },
            TextColor(Color::WHITE),
            Transform::from_xyz(x * COLUMN_LABEL_X_SCALE, COLUMN_LABEL_Y, 0.0),
        ));
    }
}

fn spawn_environment(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let ground = commands
        .spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(GROUND_SIZE, GROUND_SIZE))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: GROUND_COLOR,
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
        ))
        .observe(on_ground_clicked)
        .id();

    commands.insert_resource(SceneBounds(ground));

    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_translation(LIGHT_POSITION).looking_at(CAMERA_FOCUS, Vec3::Y),
    ));

    commands.spawn((
        OutlineCamera,
        OrbitCam::default(),
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        Transform::from_translation(CAMERA_POSITION).looking_at(CAMERA_FOCUS, Vec3::Y),
    ));
}

fn spawn_ui(commands: &mut Commands) {
    commands.spawn((
        Text::new(GROUND_CLICK_UI_TEXT),
        TextFont {
            font_size: FontSize::Px(UI_TEXT_FONT_SIZE),
            ..default()
        },
        TextColor(UI_TEXT_COLOR),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(UI_PADDING),
            left: Val::Px(UI_PADDING),
            ..default()
        },
    ));

    commands.spawn((
        OverlapLabel,
        Text::new(INITIAL_OVERLAP_LABEL),
        TextFont {
            font_size: FontSize::Px(UI_TEXT_FONT_SIZE),
            ..default()
        },
        TextColor(OVERLAP_LABEL_COLOR),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(UI_PADDING),
            left: Val::Px(UI_PADDING),
            ..default()
        },
    ));
}

fn on_mesh_clicked(click: On<Pointer<Click>>, mut commands: Commands) {
    if click.button != PointerButton::Primary {
        return;
    }
    info!("{MESH_CLICK_LOG_PREFIX}{entity:?}", entity = click.entity);
    let camera = click.hit.camera;
    commands.trigger(
        ZoomToFit::new(camera, click.entity)
            .margin(ZOOM_MARGIN_MESH)
            .duration(Duration::from_millis(ZOOM_DURATION_MS)),
    );
}

fn on_ground_clicked(click: On<Pointer<Click>>, mut commands: Commands, scene: Res<SceneBounds>) {
    if click.button != PointerButton::Primary {
        return;
    }
    info!(GROUND_CLICK_LOG_MESSAGE);
    let camera = click.hit.camera;
    commands.trigger(
        ZoomToFit::new(camera, scene.0)
            .margin(ZOOM_MARGIN_SCENE)
            .duration(Duration::from_millis(ZOOM_DURATION_MS)),
    );
}

fn toggle_overlap(
    keys: Res<ButtonInput<KeyCode>>,
    mut outlines: Query<&mut Outline>,
    mut label: Query<&mut Text, With<OverlapLabel>>,
) {
    if !keys.just_pressed(KeyCode::KeyO) {
        return;
    }

    let mut new_mode = None;
    for mut outline in &mut outlines {
        let toggled = match outline.overlap_mode {
            OverlapMode::Merged => OverlapMode::Grouped,
            OverlapMode::Grouped => OverlapMode::PerMesh,
            OverlapMode::PerMesh => OverlapMode::Merged,
        };
        outline.overlap_mode = toggled;
        new_mode = Some(toggled);
    }

    if let Some(mode) = new_mode
        && let Ok(mut text) = label.single_mut()
    {
        let label_str = match mode {
            OverlapMode::Merged => OVERLAP_MODE_MERGED_LABEL,
            OverlapMode::Grouped => OVERLAP_MODE_GROUPED_LABEL,
            OverlapMode::PerMesh => OVERLAP_MODE_PER_MESH_LABEL,
        };
        **text = format!("{OVERLAP_LABEL_PREFIX}{label_str}");
    }
}
