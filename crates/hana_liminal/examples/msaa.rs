//! Outlines with different anti-aliasing modes (MSAA, SMAA, TAA).

use bevy::anti_alias::smaa::Smaa;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::color::palettes::css::SILVER;
use bevy::color::palettes::css::YELLOW;
use bevy::core_pipeline::prepass::MotionVectorPrepass;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::Entity;
use bevy::prelude::FontSize;
use bevy::prelude::ImagePlugin;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Msaa;
use bevy::prelude::Node;
use bevy::prelude::Plane3d;
use bevy::prelude::PluginGroup;
use bevy::prelude::PointLight;
use bevy::prelude::PositionType;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Single;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Text;
use bevy::prelude::TextColor;
use bevy::prelude::TextFont;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Val;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::on_message;
use bevy::render::camera::MipBias;
use bevy::render::camera::TemporalJitter;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;

// animation
const ROTATION_X_SPEED: f32 = 1.0 / 3.0;
const ROTATION_Y_SPEED: f32 = 1.0 / 6.0;

// camera
const CAMERA_FOCUS: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const CAMERA_POSITION: Vec3 = Vec3::new(3.0, 2.0, 3.0);

// input
// Note: Sample8 is not supported on all hardware (e.g. Apple Silicon only
// supports [1, 2, 4]).
const MULTISAMPLE_ANTI_ALIASING_KEYS: [(KeyCode, Msaa); 4] = [
    (KeyCode::Digit1, Msaa::Off),
    (KeyCode::Digit2, Msaa::Sample2),
    (KeyCode::Digit3, Msaa::Sample4),
    (KeyCode::Digit4, Msaa::Sample8),
];

// lighting
const LIGHT_INTENSITY: f32 = 10_000_000.0;
const LIGHT_POSITION: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const LIGHT_RANGE: f32 = 100.0;
const LIGHT_SHADOW_DEPTH_BIAS: f32 = 0.2;

// scene
const GROUND_SIZE: f32 = 50.0;
const GROUND_SUBDIVISIONS: u32 = 10;
const INITIAL_MSAA: Msaa = Msaa::Sample4;
const INITIAL_OUTLINE_WIDTH: f32 = 10.0;
const OUTLINED_CUBE_POSITION: Vec3 = Vec3::new(0.0, 1.0, 0.0);

// ui
const CURRENT_POST_ANTI_ALIASING_LABEL: &str = "\nCurrent Post AA: ";
const MSAA_LABEL_OFF: &str = "Off";
const MSAA_LABEL_SAMPLE_2: &str = "2x";
const MSAA_LABEL_SAMPLE_4: &str = "4x";
const MSAA_LABEL_SAMPLE_8: &str = "8x";
const MSAA_TEXT_HEADER: &str = "MSAA:\n1: Off\n2: 2x\n3: 4x (default)\n4: 8x\n\nPost AA:\nS: Toggle SMAA\nT: Toggle TAA\n\nCurrent MSAA: ";
const POST_ANTI_ALIASING_LABEL_NONE: &str = "None";
const POST_ANTI_ALIASING_LABEL_SMAA: &str = "SMAA";
const POST_ANTI_ALIASING_LABEL_TAA: &str = "TAA";
const UI_FONT_SIZE: f32 = 16.0;
const UI_PADDING: f32 = 10.0;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(ImagePlugin::default_nearest()),
            LagrangePlugin,
            LiminalPlugin,
        ))
        .add_systems(Startup, (setup, setup_ui))
        .add_systems(
            Update,
            (
                rotate,
                switch_anti_aliasing.run_if(on_message::<KeyboardInput>),
            ),
        )
        .run();
}

type TaaComponents = (
    TemporalAntiAliasing,
    TemporalJitter,
    MipBias,
    MotionVectorPrepass,
);
type OutlineCameraAaQuery = (
    Entity,
    &'static mut Msaa,
    Option<&'static Smaa>,
    Option<&'static TemporalAntiAliasing>,
);

#[derive(Clone, Copy)]
enum PostAntiAliasing {
    None,
    Smaa,
    Taa,
}

impl PostAntiAliasing {
    const fn label(self) -> &'static str {
        match self {
            Self::None => POST_ANTI_ALIASING_LABEL_NONE,
            Self::Smaa => POST_ANTI_ALIASING_LABEL_SMAA,
            Self::Taa => POST_ANTI_ALIASING_LABEL_TAA,
        }
    }

    const fn from_components(smaa: Option<&Smaa>, taa: Option<&TemporalAntiAliasing>) -> Self {
        match (smaa.is_some(), taa.is_some()) {
            (false, false) => Self::None,
            (true, false) => Self::Smaa,
            // `(true, true)` cannot arise — the S/T key handlers keep SMAA and
            // TAA mutually exclusive.
            (false | true, true) => Self::Taa,
        }
    }
}

#[derive(Component)]
struct MsaaText;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(CAMERA_POSITION).looking_at(CAMERA_FOCUS, Vec3::Y),
        OrbitCam::default(),
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        OutlineCamera,
    ));

    commands.spawn((
        PointLight {
            shadow_maps_enabled: true,
            intensity: LIGHT_INTENSITY,
            range: LIGHT_RANGE,
            shadow_depth_bias: LIGHT_SHADOW_DEPTH_BIAS,
            ..default()
        },
        Transform::from_translation(LIGHT_POSITION),
    ));

    // ground plane
    commands.spawn((
        Mesh3d(
            meshes.add(
                Plane3d::default()
                    .mesh()
                    .size(GROUND_SIZE, GROUND_SIZE)
                    .subdivisions(GROUND_SUBDIVISIONS),
            ),
        ),
        MeshMaterial3d(materials.add(Color::from(SILVER))),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::default())),
        MeshMaterial3d(materials.add(Color::from(YELLOW))),
        Transform::from_translation(OUTLINED_CUBE_POSITION),
        Outline::jump_flood(INITIAL_OUTLINE_WIDTH).build(),
    ));
}

fn rotate(mut outline_query: Query<&mut Transform, With<Outline>>, time: Res<Time>) {
    for mut transform in &mut outline_query {
        let rotation = Quat::from_rotation_y(time.delta_secs() * ROTATION_Y_SPEED)
            * Quat::from_rotation_x(time.delta_secs() * ROTATION_X_SPEED);

        transform.rotation *= rotation;
    }
}

fn switch_anti_aliasing(
    input: Res<ButtonInput<KeyCode>>,
    camera: Single<OutlineCameraAaQuery, With<OutlineCamera>>,
    mut commands: Commands,
    mut text_query: Single<&mut Text, With<MsaaText>>,
) {
    let (camera_entity, mut msaa, smaa, taa) = camera.into_inner();
    let mut camera_commands = commands.entity(camera_entity);
    let mut post_anti_aliasing = PostAntiAliasing::from_components(smaa, taa);

    if let Some(&(_, new_msaa)) = MULTISAMPLE_ANTI_ALIASING_KEYS
        .iter()
        .find(|(k, _)| input.just_pressed(*k))
    {
        // TAA requires MSAA off; enabling any MSAA sample count drops it.
        if new_msaa != Msaa::Off && matches!(post_anti_aliasing, PostAntiAliasing::Taa) {
            camera_commands.remove::<TaaComponents>();
            post_anti_aliasing = PostAntiAliasing::None;
        }
        *msaa = new_msaa;
    }

    if input.just_pressed(KeyCode::KeyS) {
        match post_anti_aliasing {
            PostAntiAliasing::None => {
                camera_commands.insert(Smaa::default());
                post_anti_aliasing = PostAntiAliasing::Smaa;
            },
            PostAntiAliasing::Smaa => {
                camera_commands.remove::<Smaa>();
                post_anti_aliasing = PostAntiAliasing::None;
            },
            PostAntiAliasing::Taa => {
                camera_commands
                    .remove::<TaaComponents>()
                    .insert(Smaa::default());
                post_anti_aliasing = PostAntiAliasing::Smaa;
            },
        }
    }

    if input.just_pressed(KeyCode::KeyT) {
        match post_anti_aliasing {
            PostAntiAliasing::Taa => {
                camera_commands.remove::<TaaComponents>();
                post_anti_aliasing = PostAntiAliasing::None;
            },
            PostAntiAliasing::None | PostAntiAliasing::Smaa => {
                // TAA requires motion vectors and should run with MSAA disabled.
                *msaa = Msaa::Off;
                camera_commands.remove::<Smaa>().insert((
                    TemporalAntiAliasing::default(),
                    TemporalJitter::default(),
                    MipBias::default(),
                    MotionVectorPrepass,
                ));
                post_anti_aliasing = PostAntiAliasing::Taa;
            },
        }
    }

    let any_relevant_key = input.just_pressed(KeyCode::KeyS)
        || input.just_pressed(KeyCode::KeyT)
        || MULTISAMPLE_ANTI_ALIASING_KEYS
            .iter()
            .any(|(k, _)| input.just_pressed(*k));
    if any_relevant_key {
        text_query.0 = build_msaa_text(msaa_label(*msaa), post_anti_aliasing);
    }
}

fn setup_ui(mut commands: Commands) {
    commands.spawn((
        Text::new(build_msaa_text(
            msaa_label(INITIAL_MSAA),
            PostAntiAliasing::None,
        )),
        TextFont {
            font_size: FontSize::Px(UI_FONT_SIZE),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(UI_PADDING),
            right: Val::Px(UI_PADDING),
            ..default()
        },
        MsaaText,
    ));
}

const fn msaa_label(msaa: Msaa) -> &'static str {
    match msaa {
        Msaa::Off => MSAA_LABEL_OFF,
        Msaa::Sample2 => MSAA_LABEL_SAMPLE_2,
        Msaa::Sample4 => MSAA_LABEL_SAMPLE_4,
        Msaa::Sample8 => MSAA_LABEL_SAMPLE_8,
    }
}

fn build_msaa_text(current_msaa: &str, post_anti_aliasing: PostAntiAliasing) -> String {
    let current_post_anti_aliasing = post_anti_aliasing.label();
    format!(
        "{MSAA_TEXT_HEADER}{current_msaa}{CURRENT_POST_ANTI_ALIASING_LABEL}{current_post_anti_aliasing}"
    )
}
