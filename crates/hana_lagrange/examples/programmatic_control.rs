//! Drives an `OrbitCam` from app code by setting the `pan`/`orbit`/`zoom`
//! operation targets directly. Pressing **H** starts a home animation that
//! raises the camera's per-axis damping so the lerp reads as a slow camera
//! move, then restores the previous damping once the camera arrives. The
//! `HomeAnimationBegin` / `HomeAnimationEnd` events expose the animation window
//! for other systems (here, the title-bar control chip) to react to.
//!
//! Controls:
//!   H — home the camera

use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Event;
use bevy::prelude::KeyCode;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use fairy_dust::Anchor;
use fairy_dust::DescriptionPanel;
use fairy_dust::OrbitCamPose;
use fairy_dust::TitleBar;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamPreset;
use hana_rubric::event;

const EXAMPLE_TITLE: &str = "Programmatic Control";

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_orbit_cam_preset_pose_bundle(
            OrbitCamPose {
                focus:  HOME_FOCUS,
                yaw:    HOME_YAW,
                pitch:  HOME_PITCH,
                radius: HOME_RADIUS,
            },
            OrbitCamPreset::blender_like(),
            ProgrammaticCamera,
        )
        .with_ground_plane()
        .with_studio_lighting()
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(Transform::from_translation(CUBE_TRANSLATION))
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(HOME_CONTROL),
        )
        .wire_chip_to_events::<HomeAnimationBegin, HomeAnimationEnd>(HOME_CONTROL)
        .with_description_panel(description_panel())
        .with_camera_control_panel()
        .init_resource::<HomeReset>()
        // `H` runs `home_camera` through Fairy Dust's shortcut binding, which
        // fires it only when no modifier is held. `home_camera` writes the
        // targets and starts the smoothness override; `update_home_reset` then
        // polls each frame for arrival or user takeover.
        .with_shortcut(KeyCode::KeyH, home_camera)
        .add_systems(Update, update_home_reset)
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// CAMERA HOME — programmatic OrbitCam control via the pan/orbit/zoom operation targets.
//
// How it works:
//   1. The Fairy Dust pose helper seeds the initial `pan`/`orbit`/`zoom` values when the camera
//      spawns.
//   2. On **H**, `home_camera` records the camera's current smoothness into `HomeReset`, raises
//      smoothness so the lerp is slow, sets the home `pan`/`orbit`/`zoom` targets, and emits
//      `HomeAnimationBegin`.
//   3. `OrbitCam` itself eases each operation's current toward its target each frame.
//   4. `update_home_reset` checks each frame whether the targets still point at home (user takeover
//      changes them) and whether the camera has arrived; either condition restores the saved
//      smoothness and emits `HomeAnimationEnd`.
// ═════════════════════════════════════════════════════════════════════════════

const HOME_CONTROL: &str = "H Home";
const HOME_FOCUS: Vec3 = Vec3::new(0.0, CUBE_SIZE * 0.5, 0.0);
const HOME_PITCH: f32 = 0.42;
const HOME_RADIUS: f32 = 6.0;
const HOME_YAW: f32 = -0.85;
const HOME_SMOOTHNESS: f32 = 0.35;
const HOME_FOCUS_EPSILON: f32 = 0.01;
const HOME_ORBIT_EPSILON: f32 = 0.01;

event!(
    /// Fires when the example begins driving the camera toward home.
    HomeAnimationBegin
);
event!(
    /// Fires when the camera reaches home or the user takes manual control.
    HomeAnimationEnd
);

#[derive(Component)]
struct ProgrammaticCamera;

#[derive(Resource, Default)]
struct HomeReset {
    previous_smoothness: Option<CameraSmoothness>,
}

impl HomeReset {
    fn start(&mut self, camera: &mut OrbitCam) {
        self.previous_smoothness
            .get_or_insert_with(|| CameraSmoothness::from_camera(camera));
        camera.orbit.set_damping(HOME_SMOOTHNESS);
        camera.pan.set_damping(HOME_SMOOTHNESS);
        camera.zoom.set_damping(HOME_SMOOTHNESS);
    }

    fn finish(&mut self, camera: &mut OrbitCam) {
        if let Some(previous) = self.previous_smoothness.take() {
            previous.apply(camera);
        }
    }

    const fn is_active(&self) -> bool { self.previous_smoothness.is_some() }
}

#[derive(Clone, Copy)]
struct CameraSmoothness {
    orbit: f32,
    pan:   f32,
    zoom:  f32,
}

impl CameraSmoothness {
    const fn from_camera(camera: &OrbitCam) -> Self {
        Self {
            orbit: camera.orbit.damping(),
            pan:   camera.pan.damping(),
            zoom:  camera.zoom.damping(),
        }
    }

    fn apply(self, camera: &mut OrbitCam) {
        camera.orbit.set_damping(self.orbit);
        camera.pan.set_damping(self.pan);
        camera.zoom.set_damping(self.zoom);
    }
}

fn home_camera(
    mut commands: Commands,
    mut reset: ResMut<HomeReset>,
    mut camera: Single<&mut OrbitCam, With<ProgrammaticCamera>>,
) {
    reset.start(&mut camera);
    camera.pan.set_target(HOME_FOCUS);
    camera.orbit.set_target((HOME_YAW, HOME_PITCH));
    camera.zoom.set_target(HOME_RADIUS);
    commands.trigger(HomeAnimationBegin);
}

fn update_home_reset(
    mut commands: Commands,
    mut reset: ResMut<HomeReset>,
    mut cameras: Query<&mut OrbitCam, With<ProgrammaticCamera>>,
) {
    if !reset.is_active() {
        return;
    }

    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };

    // Finish when the user has taken control (targets no longer point at home)
    // or when the camera has actually arrived.
    if !camera_targets_home(&camera) || camera_at_home(&camera) {
        reset.finish(&mut camera);
        commands.trigger(HomeAnimationEnd);
    }
}

fn camera_targets_home(camera: &OrbitCam) -> bool {
    camera.pan.target().0.distance(HOME_FOCUS) <= HOME_FOCUS_EPSILON
        && (camera.orbit.target().yaw - HOME_YAW).abs() <= HOME_ORBIT_EPSILON
        && (camera.orbit.target().pitch - HOME_PITCH).abs() <= HOME_ORBIT_EPSILON
        && (camera.zoom.target().0 - HOME_RADIUS).abs() <= HOME_FOCUS_EPSILON
}

fn camera_at_home(camera: &OrbitCam) -> bool {
    let angles = camera.orbit.current();
    camera.pan.current().0.distance(HOME_FOCUS) <= HOME_FOCUS_EPSILON
        && (angles.yaw - HOME_YAW).abs() <= HOME_ORBIT_EPSILON
        && (angles.pitch - HOME_PITCH).abs() <= HOME_ORBIT_EPSILON
        && (camera.zoom.current().0 - HOME_RADIUS).abs() <= HOME_FOCUS_EPSILON
}

// ═════════════════════════════════════════════════════════════════════════════
// SCENE SCAFFOLDING — cube the camera homes onto, ground sized to match.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = fairy_dust::example_cube_on_ground(0.2);

// ═════════════════════════════════════════════════════════════════════════════
// UI — description panel explaining the home flow on screen.
// ═════════════════════════════════════════════════════════════════════════════

const DESCRIPTION_BODY_SIZE: f32 = 10.0;
const DESCRIPTION_HEADING: &str = "How it works";
const DESCRIPTION_LINES: [&str; 5] = [
    "1. When you press H to home the camera",
    "2. The code saves the current smoothness in HomeReset resource",
    "3. Raises smoothness so OrbitCam lerps more slowly",
    "4. Writes the home target focus, yaw, pitch, and radius",
    "5. Restores the saved smoothness when home is reached",
];

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_HEADING)
        .with_fit_width()
        .with_body_size(DESCRIPTION_BODY_SIZE)
        .lines(DESCRIPTION_LINES)
}
