use bevy::prelude::Commands;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::EulerRot;
use bevy::prelude::Has;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Real;
use bevy::prelude::Ref;
use bevy::prelude::Res;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Virtual;

use super::FreeCam;
use super::FreeCamHomePose;
use super::FreeCamInput;
use super::FreeCamUpdateRequest;
use crate::CameraBasis;
use crate::CameraHomePending;
use crate::Initialization;
use crate::operation::Limit;
use crate::operation::LookAngles;
use crate::operation::Position;
use crate::operation::Roll;
use crate::time_source::TimeSource;

struct CameraInput {
    translate: Vec3,
    look:      Vec2,
    roll:      f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MotionStatus {
    Changed,
    Unchanged,
}

impl MotionStatus {
    const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unchanged, Self::Unchanged) => Self::Unchanged,
            _ => Self::Changed,
        }
    }

    const fn is_changed(self) -> bool { matches!(self, Self::Changed) }
}

fn rotation_from_pose(basis: CameraBasis, look: LookAngles, roll: Roll) -> Quat {
    let yaw = Quat::from_rotation_y(look.yaw);
    let pitch = Quat::from_rotation_x(-look.pitch);
    let roll = Quat::from_rotation_z(roll.0);
    basis.rotation() * yaw * pitch * roll
}

fn pose_from_transform(transform: &Transform, basis: CameraBasis) -> (Position, LookAngles, Roll) {
    let local_rotation = basis.rotation().inverse() * transform.rotation;
    let (yaw, pitch, roll) = local_rotation.to_euler(EulerRot::YXZ);
    (
        Position(transform.translation),
        LookAngles { yaw, pitch: -pitch },
        Roll(roll),
    )
}

pub(crate) fn initialize_free_cam(
    free_cam: &mut FreeCam,
    basis: CameraBasis,
    transform: &mut Transform,
) {
    let (position, look, roll) = match free_cam.initialization {
        Initialization::FromPose => (
            free_cam.translate.current(),
            free_cam.look.current(),
            free_cam.roll.current(),
        ),
        Initialization::FromTransform | Initialization::Active => {
            pose_from_transform(transform, basis)
        },
    };

    let position = free_cam.translate.limit().constrain(position);
    let look = free_cam.look.limit().constrain(look);
    let roll = free_cam.roll.limit().constrain(roll);

    free_cam.translate.snap_to(position);
    free_cam.look.snap_to(look);
    free_cam.roll.snap_to(roll);
    write_transform(free_cam, basis, transform);
    free_cam.initialization = Initialization::Active;
}

fn collect_camera_input(free_cam: &FreeCam, input: &FreeCamInput) -> CameraInput {
    CameraInput {
        translate: if input.has_translate() {
            input.translate().vector() * free_cam.translate.sensitivity()
        } else {
            Vec3::ZERO
        },
        look:      if input.has_look() {
            input.look().pixels() * free_cam.look.sensitivity()
        } else {
            Vec2::ZERO
        },
        roll:      if input.has_roll() {
            input.roll().amount() * free_cam.roll.sensitivity()
        } else {
            0.0
        },
    }
}

fn apply_translate_input(
    translate: Vec3,
    free_cam: &mut FreeCam,
    transform: &Transform,
    delta: f32,
) -> MotionStatus {
    if translate == Vec3::ZERO {
        return MotionStatus::Unchanged;
    }

    free_cam
        .translate
        .set_target(free_cam.translate.target() + transform.rotation * translate * delta);
    MotionStatus::Changed
}

fn apply_look_input(look: Vec2, free_cam: &mut FreeCam) -> MotionStatus {
    if look == Vec2::ZERO {
        return MotionStatus::Unchanged;
    }

    let mut target = free_cam.look.target();
    target.yaw -= look.x;
    target.pitch -= look.y;
    free_cam.look.set_target(target);
    MotionStatus::Changed
}

fn apply_roll_input(roll: f32, free_cam: &mut FreeCam, delta: f32) -> MotionStatus {
    if roll == 0.0 {
        return MotionStatus::Unchanged;
    }

    free_cam
        .roll
        .set_target(free_cam.roll.target() + roll * delta);
    MotionStatus::Changed
}

fn smooth_and_update_transform(
    free_cam: &mut FreeCam,
    basis: CameraBasis,
    transform: &mut Transform,
    delta: f32,
) {
    free_cam.translate.update(delta);
    free_cam.look.update(delta);
    free_cam.roll.update(delta);
    write_transform(free_cam, basis, transform);
}

fn write_transform(free_cam: &FreeCam, basis: CameraBasis, transform: &mut Transform) {
    transform.translation = free_cam.translate.current().0;
    transform.rotation =
        rotation_from_pose(basis, free_cam.look.current(), free_cam.roll.current());
}

pub(super) fn free_cam(
    mut cameras: Query<(
        Entity,
        &mut FreeCam,
        Ref<CameraBasis>,
        &FreeCamInput,
        &mut Transform,
        &TimeSource,
        Has<FreeCamHomePose>,
    )>,
    time_real: Res<Time<Real>>,
    time_virt: Res<Time<Virtual>>,
    mut commands: Commands,
) {
    for (entity, mut free_cam, basis, input, mut transform, time_source, has_home) in &mut cameras {
        let basis_changed = basis.is_changed();
        let basis = *basis;

        if free_cam.initialization != Initialization::Active {
            initialize_free_cam(&mut free_cam, basis, &mut transform);
            if !has_home {
                commands
                    .entity(entity)
                    .insert(FreeCamHomePose::from_current(&free_cam));
                if !input.has_input() {
                    commands.entity(entity).insert(CameraHomePending);
                }
            }
        }

        let delta = match time_source {
            TimeSource::Real => time_real.delta_secs(),
            TimeSource::Virtual => time_virt.delta_secs(),
        };
        let input = collect_camera_input(&free_cam, input);
        let motion = apply_translate_input(input.translate, &mut free_cam, &transform, delta)
            .merge(apply_look_input(input.look, &mut free_cam))
            .merge(apply_roll_input(input.roll, &mut free_cam, delta));

        let update_request = free_cam.consume_update_request();
        let needs_update = motion.is_changed()
            || basis_changed
            || update_request == FreeCamUpdateRequest::ForceUpdate
            || free_cam.translate.target() != free_cam.translate.current()
            || free_cam.look.target() != free_cam.look.current()
            || free_cam.roll.target() != free_cam.roll.current();

        if needs_update {
            smooth_and_update_transform(&mut free_cam, basis, &mut transform, delta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pose_round_trips_through_default_basis() {
        let look = LookAngles {
            yaw:   0.5,
            pitch: -0.25,
        };
        let roll = Roll(0.125);
        let mut transform = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: rotation_from_pose(CameraBasis::Y_UP, look, roll),
            ..Default::default()
        };

        let (position, derived_look, derived_roll) =
            pose_from_transform(&transform, CameraBasis::Y_UP);

        assert_eq!(position, Position(transform.translation));
        assert!((derived_look.yaw - look.yaw).abs() <= f32::EPSILON);
        assert!((derived_look.pitch - look.pitch).abs() <= f32::EPSILON);
        assert!((derived_roll.0 - roll.0).abs() <= f32::EPSILON);

        let mut free_cam = FreeCam::from_pose(position, derived_look, derived_roll);
        initialize_free_cam(&mut free_cam, CameraBasis::Y_UP, &mut transform);
        assert_eq!(transform.translation, position.0);
    }

    #[test]
    fn translation_input_moves_along_camera_axes() {
        let mut free_cam = FreeCam::default();
        free_cam.translate.set_sensitivity(1.0);
        let transform = Transform {
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ..Default::default()
        };

        let status = apply_translate_input(Vec3::NEG_Z, &mut free_cam, &transform, 2.0);

        assert!(status.is_changed());
        let expected = Vec3::NEG_X * 2.0;
        let actual = free_cam.translate.target().0;
        assert!((actual - expected).length_squared() <= f32::EPSILON);
    }
}
