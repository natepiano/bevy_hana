//! A successful startup asset set becomes usable before the application enters
//! `Ready`.
//!
//! The persistent status panel distinguishes `hana_lading` evidence from the
//! application decision: `Loaded<StartupAssets>` applies the loaded PNG to the
//! cube, then `AllSetsLoaded` confirms the batch resolved with no failures and
//! the application enters `Ready`.
//!
//! Hana Lading is installed on the underlying Bevy `App`; Fairy Dust supplies
//! the surrounding scene and controls.

use std::any::type_name;

use bevy::prelude::App;
use bevy::prelude::AppExtStates;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Image;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Name;
use bevy::prelude::NextState;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectResource;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::State;
use bevy::prelude::States;
use bevy::prelude::Transform;
use bevy::prelude::With;
use bevy::prelude::error;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::Face;
use fairy_dust::FairyDustCube;
use fairy_dust::LABEL_SIZE;
use fairy_dust::TitleBar;
use fairy_dust::example_cube_on_ground;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material_handle;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::Fit;
use hana_diegetic::PanelBuildError;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_lading::AllSetsLoaded;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;
use hana_lading::LoadProgress;
use hana_lading::Loaded;
use hana_lagrange::OrbitCamPreset;

const CUBE_CLEARANCE: f32 = 0.1;
const CUBE_FACE_LABEL: &str = "Loaded PNG";
const DISPLAY_NAME: &str = "Successful Loading";
const LOADING_PANEL_TEXT: &str = "Successful Loading\nSET: StartupAssets — loading\nPATH: successful.png\nPROGRESS: 0 / 1\nSTATE: Loading";
const READY_DECISION: &str = "Enter Ready because all startup content is usable.";
const SUCCESSFUL_ASSET: &str = "successful.png";
const SUCCESSFUL_SCENE_NAME: &str = "Successfully loaded startup image";

struct SuccessfulLoadingPlugin;

impl Plugin for SuccessfulLoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<ExampleState>()
            .register_type::<State<ExampleState>>()
            .init_resource::<SuccessEvidence>()
            .add_systems(Startup, spawn_loading_panel)
            .add_observer(on_startup_assets_loaded)
            .add_observer(on_all_sets_loaded);
    }
}

#[derive(States, Reflect, Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ExampleState {
    #[default]
    Loading,
    Ready,
}

#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
enum EvidenceStatus {
    #[default]
    Pending,
    Confirmed,
}

/// Persistent BRP-readable evidence for the successful startup sequence.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
struct SuccessEvidence {
    set_name:     String,
    tracked_path: String,
    set:          EvidenceStatus,
    batch:        EvidenceStatus,
    scene:        EvidenceStatus,
    loaded:       usize,
    total:        usize,
}

#[derive(Resource)]
struct StartupAssets {
    image: Handle<Image>,
}

impl DiskAssets for StartupAssets {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            image: loader.load(SUCCESSFUL_ASSET),
        }
    }
}

#[derive(Component)]
struct LoadingPanel;

#[derive(Component)]
struct SceneTarget;

/// Marks scene content that uses the successfully loaded startup image.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct SuccessfulSceneContent;

/// Reflected copy of the persistent success panel's exact rendered text.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct SuccessPanelContent {
    text: String,
}

fn main() {
    let asset_root = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");

    let mut example = fairy_dust::sprinkle_example().with_asset_root(asset_root);

    example
        .app_mut()
        .add_plugins(DiskAssetsPlugin::<StartupAssets>::default())
        .add_plugins(SuccessfulLoadingPlugin);

    example
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .transform(Transform::from_translation(example_cube_on_ground(
            CUBE_CLEARANCE,
        )))
        .face_label(Face::Front, CUBE_FACE_LABEL)
        .insert((CameraHomeTarget, SceneTarget))
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_stable_transparency()
        .with_camera_home()
        .with_title_bar(TitleBar::new().with_title(DISPLAY_NAME))
        .with_camera_control_panel()
        .run();
}

fn spawn_loading_panel(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    match status_panel(&mut materials, LOADING_PANEL_TEXT) {
        Ok(panel) => {
            commands.spawn((LoadingPanel, panel, Transform::default()));
        },
        Err(error) => {
            error!("failed to build loading status panel: {error}");
        },
    }
}

fn on_startup_assets_loaded(
    _loaded: On<Loaded<StartupAssets>>,
    startup_assets: Res<StartupAssets>,
    target: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        (With<FairyDustCube>, With<SceneTarget>),
    >,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut evidence: ResMut<SuccessEvidence>,
) {
    let Ok((entity, material_handle)) = target.single() else {
        error!("successful-loading cube is missing or ambiguous");
        return;
    };
    let Some(mut material) = materials.get_mut(material_handle) else {
        error!("successful-loading cube material is unavailable");
        return;
    };

    material.base_color = Color::WHITE;
    material.base_color_texture = Some(startup_assets.image.clone());
    evidence.set_name = type_name::<StartupAssets>().to_string();
    evidence.tracked_path = SUCCESSFUL_ASSET.to_string();
    evidence.set = EvidenceStatus::Confirmed;
    evidence.scene = EvidenceStatus::Confirmed;
    commands
        .entity(entity)
        .insert((SuccessfulSceneContent, Name::new(SUCCESSFUL_SCENE_NAME)));
}

fn on_all_sets_loaded(
    _all_loaded: On<AllSetsLoaded>,
    progress: Res<LoadProgress>,
    loading_panels: Query<Entity, With<LoadingPanel>>,
    mut next_state: ResMut<NextState<ExampleState>>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut evidence: ResMut<SuccessEvidence>,
) {
    if evidence.set != EvidenceStatus::Confirmed || evidence.scene != EvidenceStatus::Confirmed {
        error!("AllSetsLoaded arrived before the loaded image became usable in the scene");
        return;
    }

    evidence.batch = EvidenceStatus::Confirmed;
    evidence.loaded = progress.loaded();
    evidence.total = progress.total();
    next_state.set(ExampleState::Ready);
    for entity in &loading_panels {
        commands.entity(entity).despawn();
    }

    let text = format!(
        "{DISPLAY_NAME}\nSET: {} — loaded\nPATH: {}\nPROGRESS: {} / {}\nGLOBAL: AllSetsLoaded\nSTATE: Ready\nSCENE: blue PNG applied to cube\nDECISION: {READY_DECISION}",
        evidence.set_name, evidence.tracked_path, evidence.loaded, evidence.total
    );
    match status_panel(&mut materials, text.as_str()) {
        Ok(panel) => {
            commands.spawn((SuccessPanelContent { text }, panel, Transform::default()));
        },
        Err(error) => {
            error!("failed to build successful-loading panel: {error}");
        },
    }
}

fn status_panel(
    materials: &mut Assets<StandardMaterial>,
    text: &str,
) -> Result<DiegeticPanel, PanelBuildError> {
    let material = screen_panel_material_handle(materials);
    DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::Center)
        .material(material.clone())
        .text_material(material)
        .layout(|builder| {
            screen_panel_frame(
                builder,
                Sizing::FIT,
                Sizing::FIT,
                DEFAULT_PANEL_BACKGROUND,
                |builder| {
                    builder.text((text, TextStyle::new(LABEL_SIZE)));
                },
            );
        })
        .build()
}
