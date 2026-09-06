//! `OrbitCam` spherical-coordinate transform math.

use bevy::prelude::Mat3;
use bevy::prelude::OrthographicProjection;
use bevy::prelude::PerspectiveProjection;
use bevy::prelude::Projection;
use bevy::prelude::Quat;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::Vec4;
use hana_kana::Position;

use super::constants::MIN_ORBIT_RADIUS;
use super::constants::PERSPECTIVE_NEAR_MIN;
use super::constants::PERSPECTIVE_NEAR_RADIUS_FACTOR;

pub(super) fn calculate_from_translation_and_focus(
    translation: impl Into<Position>,
    focus: impl Into<Position>,
    axis: [Vec3; 3],
) -> (f32, f32, f32) {
    let translation = translation.into();
    let focus = focus.into();
    let axis = Mat3::from_cols(axis[0], axis[1], axis[2]);
    let component_vector = *translation - *focus;
    let mut radius = component_vector.length();
    if radius < f32::EPSILON {
        radius = MIN_ORBIT_RADIUS;
    }
    let component_vector = axis * component_vector;
    let yaw = component_vector.x.atan2(component_vector.z);
    let pitch = (component_vector.y / radius).asin();
    (yaw, pitch, radius)
}

/// Writes `transform` from yaw, pitch, and the camera's focus and radius, and
/// applies the radius to the projection: orthographic scale for
/// `Projection::Orthographic`, near clip plane for `Projection::Perspective`.
pub(super) fn update_orbit_transform(
    yaw: f32,
    pitch: f32,
    radius: f32,
    focus: impl Into<Position>,
    transform: &mut Transform,
    projection: &mut Projection,
    axis: [Vec3; 3],
) {
    match &mut *projection {
        Projection::Orthographic(p) => {
            p.scale = radius;
        },
        Projection::Perspective(p) => sync_perspective_near_clip(p, radius),
        Projection::Custom(_) => {},
    }
    *transform = transform_from_orbit(yaw, pitch, radius, focus, projection, axis);
}

pub(crate) fn transform_from_orbit(
    yaw: f32,
    pitch: f32,
    radius: f32,
    focus: impl Into<Position>,
    projection: &Projection,
    axis: [Vec3; 3],
) -> Transform {
    let focus = focus.into();
    let radius = camera_distance_for_orbit(radius, projection);
    let yaw_rotation = Quat::from_axis_angle(axis[1], yaw);
    let pitch_rotation = Quat::from_axis_angle(axis[0], -pitch);
    let rotation = yaw_rotation * pitch_rotation;
    Transform::from_translation(*focus + rotation * Vec3::new(0.0, 0.0, radius))
        .with_rotation(rotation)
}

fn camera_distance_for_orbit(radius: f32, projection: &Projection) -> f32 {
    match projection {
        Projection::Orthographic(projection) => orthographic_camera_distance(projection),
        Projection::Perspective(_) | Projection::Custom(_) => radius,
    }
}

fn orthographic_camera_distance(projection: &OrthographicProjection) -> f32 {
    // IMPORTANT: Do NOT replace this with `f32::midpoint()`.
    // On aarch64, `midpoint()` promotes to f64 intermediate precision:
    //   ((self as f64 + other as f64) / 2.0) as f32
    // This produces a subtly different camera distance than plain f32 arithmetic.
    // That tiny difference shifts the projected screen-space bounds just enough
    // to flip the fit overlay balance check (tolerance: 0.001) — causing
    // all margin labels to show green/balanced when they should show red/unbalanced.
    #[expect(
        clippy::manual_midpoint,
        reason = "f32::midpoint uses f64 on aarch64, breaking fit visualization balance detection"
    )]
    {
        (projection.near + projection.far) / 2.0
    }
}

fn sync_perspective_near_clip(projection: &mut PerspectiveProjection, radius: f32) {
    let new_near = (radius * PERSPECTIVE_NEAR_RADIUS_FACTOR)
        .max(PERSPECTIVE_NEAR_MIN)
        .min(projection.far);
    projection.near = new_near;
    projection.near_clip_plane = Vec4::new(0.0, 0.0, -1.0, -new_near);
}

#[cfg(test)]
#[allow(
    clippy::unreadable_literal,
    clippy::float_cmp,
    reason = "test assertions verify deterministic bitwise-exact float results"
)]
mod calculate_from_translation_and_focus_tests {
    use std::f32::consts::PI;

    use float_cmp::approx_eq;

    use super::*;
    const AXIS: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
    const AXIS_Z_UP: [Vec3; 3] = [Vec3::X, Vec3::Z, Vec3::Y];

    #[test]
    fn zero() {
        let translation = Position::new(0.0, 0.0, 0.0);
        let focus = Position::default();
        let (yaw, pitch, radius) = calculate_from_translation_and_focus(translation, focus, AXIS);
        assert_eq!(yaw, 0.0);
        assert_eq!(pitch, 0.0);
        assert_eq!(radius, MIN_ORBIT_RADIUS);
    }

    #[test]
    fn zero_z_up_axis() {
        let translation = Position::new(0.0, 0.0, 0.0);
        let focus = Position::default();
        let (yaw, pitch, radius) =
            calculate_from_translation_and_focus(translation, focus, AXIS_Z_UP);
        assert_eq!(yaw, 0.0);
        assert_eq!(pitch, 0.0);
        assert_eq!(radius, MIN_ORBIT_RADIUS);
    }

    #[test]
    fn in_front() {
        let translation = Position::new(0.0, 0.0, 5.0);
        let focus = Position::default();
        let (yaw, pitch, radius) = calculate_from_translation_and_focus(translation, focus, AXIS);
        assert_eq!(yaw, 0.0);
        assert_eq!(pitch, 0.0);
        assert_eq!(radius, 5.0);
    }

    #[test]
    fn in_front_z_up_axis() {
        let translation = Position::new(0.0, 5.0, 0.0);
        let axis = [Vec3::X, Vec3::Z, Vec3::Y];
        let focus = Position::default();
        let (yaw, pitch, radius) = calculate_from_translation_and_focus(translation, focus, axis);
        assert_eq!(yaw, 0.0);
        assert_eq!(pitch, 0.0);
        assert_eq!(radius, 5.0);
    }

    #[test]
    fn to_the_side() {
        let translation = Position::new(5.0, 0.0, 0.0);
        let focus = Position::default();
        let (yaw, pitch, radius) = calculate_from_translation_and_focus(translation, focus, AXIS);
        assert!(approx_eq!(f32, yaw, PI / 2.0));
        assert_eq!(pitch, 0.0);
        assert_eq!(radius, 5.0);
    }

    #[test]
    fn above() {
        let translation = Position::new(0.0, 5.0, 0.0);
        let focus = Position::default();
        let (yaw, pitch, radius) = calculate_from_translation_and_focus(translation, focus, AXIS);
        assert_eq!(yaw, 0.0);
        assert!(approx_eq!(f32, pitch, PI / 2.0));
        assert_eq!(radius, 5.0);
    }

    #[test]
    fn above_z_as_up_axis() {
        let translation = Position::new(0.0, 0.0, 5.0);
        let focus = Position::default();
        let (yaw, pitch, radius) =
            calculate_from_translation_and_focus(translation, focus, AXIS_Z_UP);
        assert_eq!(yaw, 0.0);
        assert!(approx_eq!(f32, pitch, PI / 2.0));
        assert_eq!(radius, 5.0);
    }

    #[test]
    fn arbitrary() {
        let translation = Position::new(0.92563736, 3.864204, -1.0105048);
        let focus = Position::default();
        let (yaw, pitch, radius) = calculate_from_translation_and_focus(translation, focus, AXIS);
        assert!(approx_eq!(f32, yaw, 2.4));
        assert!(approx_eq!(f32, pitch, 1.23));
        assert_eq!(radius, 4.1);
    }

    #[test]
    fn negative_x() {
        let translation = Position::new(-5.0, 5.0, 9.0);
        let focus = Position::default();
        let (yaw, pitch, radius) = calculate_from_translation_and_focus(translation, focus, AXIS);
        assert!(approx_eq!(f32, yaw, -0.5070985));
        assert!(approx_eq!(f32, pitch, 0.45209613));
        assert!(approx_eq!(f32, radius, 11.445523));
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests verify exact near-plane sync behavior"
)]
mod sync_perspective_near_clip_tests {
    use bevy::prelude::default;

    use super::*;

    #[test]
    fn near_plane_tracks_radius() {
        let mut projection = PerspectiveProjection::default();
        sync_perspective_near_clip(&mut projection, 2.0);
        let expected_near = 2.0 * PERSPECTIVE_NEAR_RADIUS_FACTOR;
        assert_eq!(projection.near, expected_near);
        assert_eq!(
            projection.near_clip_plane,
            Vec4::new(0.0, 0.0, -1.0, -expected_near)
        );
    }

    #[test]
    fn near_plane_respects_absolute_minimum() {
        let mut projection = PerspectiveProjection::default();
        sync_perspective_near_clip(&mut projection, 1e-9);
        assert_eq!(projection.near, PERSPECTIVE_NEAR_MIN);
        assert_eq!(
            projection.near_clip_plane,
            Vec4::new(0.0, 0.0, -1.0, -PERSPECTIVE_NEAR_MIN)
        );
    }

    #[test]
    fn near_plane_never_exceeds_far_plane() {
        let mut projection = PerspectiveProjection {
            far: 0.01,
            ..default()
        };
        sync_perspective_near_clip(&mut projection, 20.0);
        assert_eq!(projection.near, 0.01);
        assert_eq!(projection.near_clip_plane, Vec4::new(0.0, 0.0, -1.0, -0.01));
    }
}
