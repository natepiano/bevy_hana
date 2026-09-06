//! Application policy enters `Ready` when required content loads and an
//! optional capability fails.
//!
//! The example exercises every public loading protection except recursive
//! child completion, which the `recursive_dependencies_gate_and_fail` contract
//! test covers. `hana_lading` tracks
//! every returned handle, reports the optional failure, delivers generic
//! failure evidence before global completion, and leaves the degraded-mode
//! decision to this application.
//!
//! Hana Lading is installed on the underlying Bevy `App`; Fairy Dust supplies
//! the surrounding scene and controls.

mod loading_evidence;

use std::any::type_name;
use std::path::Path;

use bevy::prelude::Alpha;
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::Image;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Name;
use bevy::prelude::NextState;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::With;
use bevy::prelude::debug;
use bevy::prelude::error;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::Face;
use fairy_dust::FairyDustCube;
use fairy_dust::TitleBar;
use fairy_dust::example_cube_on_ground;
use hana_lading::AllSetsResolved;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;
use hana_lading::Loaded;
use hana_lagrange::OrbitCamPreset;
use loading_evidence::ExampleState;
use loading_evidence::FailureRecord;
use loading_evidence::LoadingEvidencePlugin;
use loading_evidence::RequiredSceneContent;
use loading_evidence::spawn_failure_panel;

const CUBE_CLEARANCE: f32 = 0.1;
const CUBE_FACE_LABEL: &str = "Required image";
const DISPLAY_NAME: &str = "Degraded Failure";
const MISSING_OPTIONAL_ASSET: &str = "optional-intentionally-absent.png";
const OPAQUE_ALPHA: f32 = 1.0;
const READY_DECISION: &str = "Continue in degraded mode because required content is ready.";
const REQUIRED_ASSET: &str = "successful.png";
const REQUIRED_SCENE_NAME: &str = "Required startup image content";

#[derive(Resource)]
struct RequiredAssets {
    image: Handle<Image>,
}

impl DiskAssets for RequiredAssets {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            image: loader.load(REQUIRED_ASSET),
        }
    }
}

#[derive(Resource)]
struct OptionalAssets {
    image: Handle<Image>,
}

impl DiskAssets for OptionalAssets {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            image: loader.load(MISSING_OPTIONAL_ASSET),
        }
    }
}

#[derive(Component)]
struct RequiredSceneTarget;

/// Immediate application-owned evidence that the required image reached the scene material.
#[derive(Resource, Reflect, Default)]
#[reflect(Resource)]
struct RequiredSceneEvidence {
    applied_image: Option<String>,
}

fn main() {
    let asset_root = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");
    assert!(
        !Path::new(asset_root).join(MISSING_OPTIONAL_ASSET).exists(),
        "the degraded example requires its missing fixture to stay absent"
    );

    let mut example = fairy_dust::sprinkle_example().with_asset_root(asset_root);

    example
        .app_mut()
        .add_plugins(DiskAssetsPlugin::<RequiredAssets>::default())
        .add_plugins(DiskAssetsPlugin::<OptionalAssets>::default())
        .add_plugins(LoadingEvidencePlugin)
        .init_resource::<RequiredSceneEvidence>()
        .add_observer(on_required_loaded)
        .add_observer(on_all_sets_resolved);

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
        .insert((CameraHomeTarget, RequiredSceneTarget))
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_stable_transparency()
        .with_camera_home()
        .with_title_bar(TitleBar::new().with_title(DISPLAY_NAME))
        .with_camera_control_panel()
        .run();
}

fn on_required_loaded(
    _: On<Loaded<RequiredAssets>>,
    required: Res<RequiredAssets>,
    target: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        (With<FairyDustCube>, With<RequiredSceneTarget>),
    >,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut evidence: ResMut<RequiredSceneEvidence>,
) {
    debug!(asset = ?required.image.id(), "required scene asset is usable");
    let Ok((entity, material_handle)) = target.single() else {
        error!("required scene cube is missing or ambiguous");
        return;
    };
    let Some(mut material) = materials.get_mut(material_handle) else {
        error!("required scene cube material is unavailable");
        return;
    };
    material.base_color_texture = Some(required.image.clone());
    evidence.applied_image = Some(format!("{:?}", required.image.id()));
    commands
        .entity(entity)
        .insert((RequiredSceneContent, Name::new(REQUIRED_SCENE_NAME)));
}

fn on_all_sets_resolved(
    event: On<AllSetsResolved>,
    required: Res<RequiredAssets>,
    optional: Res<OptionalAssets>,
    record: Res<FailureRecord>,
    evidence: Res<RequiredSceneEvidence>,
    mut next_state: ResMut<NextState<ExampleState>>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    debug!(asset = ?optional.image.id(), "optional asset reached a terminal failure");
    let required_image = format!("{:?}", required.image.id());
    let required_image_applied = evidence.applied_image.as_deref() == Some(required_image.as_str());
    let expected_failure_set = type_name::<OptionalAssets>();
    let mut blockers = Vec::new();
    if !required_image_applied {
        blockers.push("the required PNG was not applied to the scene cube".to_string());
    }
    if event.failures() != 1 {
        blockers.push(format!(
            "expected one failed asset set but observed {}",
            event.failures()
        ));
    }
    if record.set_name != expected_failure_set {
        blockers.push(format!(
            "the recorded failure set was {:?}, expected {expected_failure_set:?}",
            record.set_name
        ));
    }

    if blockers.is_empty() {
        next_state.set(ExampleState::Ready);
        spawn_failure_panel(
            &mut commands,
            &mut materials,
            &record,
            DISPLAY_NAME,
            READY_DECISION,
            DEFAULT_PANEL_BACKGROUND,
        );
    } else {
        let decision = format!("Remain in Loading because {}.", blockers.join("; "));
        error!(
            failures = event.failures(),
            failure_set = %record.set_name,
            required_image_applied,
            decision = %decision,
            "degraded startup requirements were not satisfied"
        );
        spawn_failure_panel(
            &mut commands,
            &mut materials,
            &record,
            DISPLAY_NAME,
            &decision,
            DEFAULT_PANEL_BACKGROUND.with_alpha(OPAQUE_ALPHA),
        );
    }
}
