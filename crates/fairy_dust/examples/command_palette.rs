//! Command palette example: open the box, search the command registry by title
//! or by raw id, and run the selected command.
//!
//! The two `example::` commands below are declared by this example, not by Fairy
//! Dust, so the palette shows an application's own commands sitting beside the
//! ones the library ships. Because they are the example's own ids, the example
//! also owns the keymap document that binds them: Fairy Dust's shipped document
//! binds Fairy Dust's own authored capabilities, while its protected Cmd/Ctrl+P
//! recovery chord opens the palette directly. A document naming a command the
//! registry has not declared is rejected whole.
//!
//! The example configures an application name, so the keymap reads and writes
//! `keymap.jsonc` in the platform configuration directory and the palette opens
//! with no unresolved-path failure row. `command_palette.keymap.jsonc` then
//! misspells one block member on purpose, so the palette still demonstrates
//! what an authoring mistake looks like — as an advisory row, which leaves the
//! rest of the document loaded.

use bevy::prelude::Color;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use commands::ExampleHome;
use commands::ExampleSwapCamera;
use fairy_dust::CameraHomeTarget;
use fairy_dust::CommandPaletteKeymap;
use fairy_dust::Face;
use fairy_dust::OrbitCam;
use fairy_dust::OrbitCamPose;
use fairy_dust::TitleBar;
use fairy_dust::sprinkle_example;
/// Names the configuration directory the example's keymap is read from and
/// written to.
const APPLICATION_NAME: &str = "fairy_dust_command_palette";
const CAMERA_FOCUS: Vec3 = Vec3::new(0.0, 0.5, 0.0);
const CUBE_COLOR: Color = Color::srgb(0.18, 0.24, 0.28);
const CUBE_LABEL_COLOR: Color = Color::srgb(0.62, 0.84, 0.94);
const CUBE_LABEL_SIZE: f32 = 0.12;
const CUBE_SIZE: f32 = 1.0;
const GROUND_COLOR: Color = Color::srgba(0.11, 0.13, 0.14, 1.0);
const GROUND_SIZE: f32 = 6.0;
/// Screen fraction the home fit leaves empty around the cube, so the palette
/// box has dark scene behind it rather than a wall of cube.
const HOME_MARGIN: f32 = 0.55;

const HOME_POSE: OrbitCamPose = OrbitCamPose {
    focus:  CAMERA_FOCUS,
    yaw:    -0.6,
    pitch:  0.35,
    radius: 6.0,
};
const SURVEY_POSE: OrbitCamPose = OrbitCamPose {
    focus:  CAMERA_FOCUS,
    yaw:    2.4,
    pitch:  0.9,
    radius: 9.0,
};

mod commands {
    use bevy::prelude::Event;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy_enhanced_input::prelude::InputAction;
    use hana_rubric::ReflectKeymapCommand;
    use hana_rubric::command;

    command! {
        action:      ExampleHomeAction,
        event:       ExampleHome,
        id:          "example::home",
        title:       "Reset Camera to Home",
        description: "Return the camera to the example's authored home framing.",
    }

    command! {
        action:      ExampleSwapCameraAction,
        event:       ExampleSwapCamera,
        id:          "example::swap_camera",
        title:       "Swap Camera Viewpoint",
        description: "Toggle between the home framing and the high survey framing.",
    }
}

/// Which authored framing the camera currently holds.
#[derive(Default, Resource)]
enum CameraViewpoint {
    #[default]
    Home,
    Survey,
}

fn main() {
    sprinkle_example()
        .with_title_bar(title_bar())
        .with_studio_lighting()
        .aim_at(CAMERA_FOCUS)
        .with_ground_plane()
        .size(GROUND_SIZE)
        .color(GROUND_COLOR)
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(Transform::from_xyz(0.0, CUBE_SIZE * 0.5, 0.0))
        .face_text(Face::Front, "PALETTE", CUBE_LABEL_SIZE, CUBE_LABEL_COLOR)
        .insert(CameraHomeTarget)
        .with_orbit_cam_configured(configure_camera)
        .with_camera_home()
        .yaw(HOME_POSE.yaw)
        .pitch(HOME_POSE.pitch)
        .margin(HOME_MARGIN)
        .with_camera_control_panel()
        .with_stable_transparency()
        .with_command_palette_keymap(
            CommandPaletteKeymap::new(include_str!("command_palette.keymap.jsonc"))
                .for_application(APPLICATION_NAME),
        )
        .with_save_window_position()
        .with_brp_extras()
        .init_resource::<CameraViewpoint>()
        .add_observer(reset_camera_home)
        .add_observer(swap_camera_viewpoint)
        .run();
}

fn title_bar() -> TitleBar {
    TitleBar::new().with_title("Command palette").controls([
        palette_control(),
        "Enter Run",
        "Esc Close",
    ])
}

/// The palette's recovery chord renders per platform. Fairy Dust reads it
/// directly so it also works while authored keymap bindings are unavailable.
const fn palette_control() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd+P Palette"
    } else {
        "Ctrl+P Palette"
    }
}

fn configure_camera(orbit_cam: &mut OrbitCam) { HOME_POSE.apply_to(orbit_cam); }

fn reset_camera_home(
    _home: On<ExampleHome>,
    mut cameras: Query<&mut OrbitCam>,
    mut viewpoint: ResMut<CameraViewpoint>,
) {
    *viewpoint = CameraViewpoint::Home;
    for mut orbit_cam in &mut cameras {
        HOME_POSE.apply_to(&mut orbit_cam);
    }
}

fn swap_camera_viewpoint(
    _swap: On<ExampleSwapCamera>,
    mut cameras: Query<&mut OrbitCam>,
    mut viewpoint: ResMut<CameraViewpoint>,
) {
    let pose = match *viewpoint {
        CameraViewpoint::Home => {
            *viewpoint = CameraViewpoint::Survey;
            SURVEY_POSE
        },
        CameraViewpoint::Survey => {
            *viewpoint = CameraViewpoint::Home;
            HOME_POSE
        },
    };
    for mut orbit_cam in &mut cameras {
        pose.apply_to(&mut orbit_cam);
    }
}
