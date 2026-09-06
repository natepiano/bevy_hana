//! Projection math for screen-space panel attachment poses.

use bevy::prelude::Quat;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;

/// Projects `hana_valence::AnchorPose::rotation` onto the shared screen plane.
///
/// In-plane rotation is applied; out-of-plane rotation has no screen effect.
/// The panel cannot leave the plane, so the screen resolver keeps only the
/// quaternion twist around the view normal.
pub(super) fn screen_in_plane_angle(rotation: Quat) -> f32 {
    let Some(twist) = Vec2::new(rotation.w, rotation.z).try_normalize() else {
        return 0.0;
    };
    2.0 * twist.y.atan2(twist.x)
}

pub(super) fn rotate_screen_offset(offset: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(
        offset.y.mul_add(sin, offset.x * cos),
        offset.y.mul_add(cos, -offset.x * sin),
    )
}

pub(super) fn project_panel_local_offset(panel_local: Vec2, scale: Vec2, angle: f32) -> Vec2 {
    let scaled_screen = Vec2::new(panel_local.x * scale.x, -panel_local.y * scale.y);
    rotate_screen_offset(scaled_screen, angle)
}

pub(super) fn screen_panel_scale(transform: &Transform) -> Option<Vec2> {
    let scale = transform.scale.truncate();
    (scale.is_finite() && scale.x.abs() > f32::EPSILON && scale.y.abs() > f32::EPSILON)
        .then_some(scale)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;

    const ASSERT_CLOSE_EPSILON: f32 = 1e-4;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < ASSERT_CLOSE_EPSILON,
            "expected {expected}, got {actual}",
        );
    }

    fn assert_vec2_close(actual: Vec2, expected: Vec2) {
        assert_close(actual.x, expected.x);
        assert_close(actual.y, expected.y);
    }

    #[test]
    fn screen_in_plane_angle_reads_z_twist() {
        assert_close(
            screen_in_plane_angle(Quat::from_rotation_z(FRAC_PI_2)),
            FRAC_PI_2,
        );
    }

    #[test]
    fn screen_in_plane_angle_ignores_out_of_plane_rotation() {
        assert_close(screen_in_plane_angle(Quat::from_rotation_x(FRAC_PI_2)), 0.0);
    }

    #[test]
    fn rotate_screen_offset_uses_screen_y_axis() {
        assert_vec2_close(
            rotate_screen_offset(Vec2::new(5.0, 2.0), FRAC_PI_2),
            Vec2::new(2.0, -5.0),
        );
    }
}
