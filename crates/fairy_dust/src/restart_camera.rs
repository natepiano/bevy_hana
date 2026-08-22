//! Optional camera-pose handoff for Fairy Dust hot restart.
//!
//! When enabled, `Ctrl+Shift+R` carries the current `OrbitCam` pose into the
//! relaunched process and exposes an event for the app to animate back to it
//! when its startup scene is ready.

use std::process::Command;
use std::str::FromStr;

use bevy::math::curve::easing::EaseFunction;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::warn;
use hana_lagrange::CameraMove;
use hana_lagrange::CameraMoveError;
use hana_lagrange::Focus;
use hana_lagrange::FreeCamRollTarget;
use hana_lagrange::OrbitAngles;
use hana_lagrange::OrbitCam;
use hana_lagrange::PlayAnimation;
use hana_lagrange::Radius;

use crate::constants::POSE_ENV;
use crate::constants::POSE_FIELD_COUNT;
use crate::constants::POSE_FIELD_SEPARATOR;
use crate::constants::RESTART_CAMERA_RESTORE_DURATION;
use crate::orbit_cam::FairyDustOrbitCam;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RestartCameraRestoreStatus {
    #[default]
    Pending,
    Applied,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RestartCameraPose {
    focus:  Vec3,
    yaw:    f32,
    pitch:  f32,
    radius: f32,
}

impl RestartCameraPose {
    fn decode(encoded: &str) -> Option<Self> {
        let fields = encoded
            .split(POSE_FIELD_SEPARATOR)
            .map(parse_field)
            .collect::<Option<Vec<_>>>()?;
        if fields.len() != POSE_FIELD_COUNT {
            return None;
        }
        Some(Self {
            focus:  Vec3::new(fields[0], fields[1], fields[2]),
            yaw:    fields[3],
            pitch:  fields[4],
            radius: fields[5],
        })
    }

    fn encode(self) -> String {
        format!(
            "{}{}{}{}{}{}{}{}{}{}{}",
            self.focus.x,
            POSE_FIELD_SEPARATOR,
            self.focus.y,
            POSE_FIELD_SEPARATOR,
            self.focus.z,
            POSE_FIELD_SEPARATOR,
            self.yaw,
            POSE_FIELD_SEPARATOR,
            self.pitch,
            POSE_FIELD_SEPARATOR,
            self.radius,
        )
    }

    fn camera_move(self) -> Result<CameraMove, CameraMoveError> {
        CameraMove::try_to_orbital_look_at(
            Focus(self.focus),
            OrbitAngles {
                yaw:   self.yaw,
                pitch: self.pitch,
            },
            Radius(self.radius),
            FreeCamRollTarget::InheritPrevious,
            RESTART_CAMERA_RESTORE_DURATION,
            EaseFunction::CubicOut,
        )
    }
}

impl From<&OrbitCam> for RestartCameraPose {
    fn from(orbit_cam: &OrbitCam) -> Self {
        let angles = orbit_cam.orbit.current();
        Self {
            focus:  orbit_cam.pan.current().0,
            yaw:    angles.yaw,
            pitch:  angles.pitch,
            radius: orbit_cam.zoom.current().0,
        }
    }
}

/// Resource inserted when restart-camera restoration is enabled.
///
/// Examples can read this resource to branch startup camera behavior when a
/// restart pose is available.
#[derive(Resource, Debug)]
pub struct RestartCameraRestore {
    pose:   Option<RestartCameraPose>,
    status: RestartCameraRestoreStatus,
}

impl RestartCameraRestore {
    fn from_env() -> Self {
        Self {
            pose:   std::env::var(POSE_ENV)
                .ok()
                .and_then(|encoded| RestartCameraPose::decode(&encoded)),
            status: RestartCameraRestoreStatus::Pending,
        }
    }

    /// Returns true when this process was launched with a Fairy Dust restart
    /// camera pose.
    #[must_use]
    pub const fn has_restart_camera_pose(&self) -> bool { self.pose.is_some() }
}

/// Plays the camera animation back to the pose captured before Fairy Dust hot
/// restart relaunched the example.
#[derive(Event)]
pub struct RestoreWindowAnimation;

pub(super) fn install(app: &mut App) {
    app.insert_resource(RestartCameraRestore::from_env());
    app.add_observer(on_restore_window_animation);
}

pub(super) fn encode_child_pose(
    cameras: &Query<&OrbitCam, With<FairyDustOrbitCam>>,
    state: Option<&RestartCameraRestore>,
) -> Option<String> {
    state?;
    cameras
        .single()
        .ok()
        .map(|orbit_cam| RestartCameraPose::from(orbit_cam).encode())
}

pub(super) fn apply_child_env(command: &mut Command, encoded_pose: Option<String>) {
    if let Some(encoded_pose) = encoded_pose {
        command.env(POSE_ENV, encoded_pose);
    }
}

fn on_restore_window_animation(
    _trigger: On<RestoreWindowAnimation>,
    mut commands: Commands,
    mut state: ResMut<RestartCameraRestore>,
    cameras: Query<Entity, With<FairyDustOrbitCam>>,
) {
    if state.status == RestartCameraRestoreStatus::Applied {
        return;
    }
    let Some(pose) = state.pose else {
        return;
    };
    let Ok(camera) = cameras.single() else {
        return;
    };
    let restore_move = match pose.camera_move() {
        Ok(restore_move) => restore_move,
        Err(error) => {
            warn!("restart camera pose rejected: {error}");
            return;
        },
    };
    commands.trigger(PlayAnimation::new(camera, [restore_move]));
    state.status = RestartCameraRestoreStatus::Applied;
}

fn parse_field(field: &str) -> Option<f32> {
    f32::from_str(field).ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_camera_pose_round_trips_through_env_string() {
        let pose = RestartCameraPose {
            focus:  Vec3::new(1.25, -2.5, 3.75),
            yaw:    0.5,
            pitch:  -0.25,
            radius: 8.0,
        };

        assert_eq!(RestartCameraPose::decode(&pose.encode()), Some(pose));
    }

    #[test]
    fn restart_camera_pose_rejects_wrong_field_count() {
        assert_eq!(RestartCameraPose::decode("1,2,3"), None);
    }
}
