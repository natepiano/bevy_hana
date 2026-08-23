//! Frame-rate-independent smoothing shared across camera kinds.

use bevy::prelude::FloatExt;
use hana_kana::Position;

use crate::constants::EPSILON;
use crate::constants::SMOOTHNESS_EXPONENT;

const fn approx_equal(a: f32, b: f32) -> bool { (a - b).abs() < EPSILON }

pub(crate) fn lerp_and_snap_f32(from: f32, to: f32, smoothness: f32, delta_secs: f32) -> f32 {
    let t = smoothness.powi(SMOOTHNESS_EXPONENT);
    let mut new_value = from.lerp(to, 1.0 - t.powf(delta_secs));
    if smoothness < 1.0 && approx_equal(new_value, to) {
        new_value = to;
    }
    new_value
}

pub(crate) fn lerp_and_snap_position(
    from: impl Into<Position>,
    to: impl Into<Position>,
    smoothness: f32,
    delta_secs: f32,
) -> Position {
    let from = from.into();
    let to = to.into();
    let t = smoothness.powi(SMOOTHNESS_EXPONENT);
    let mut new_value = (*from).lerp(*to, 1.0 - t.powf(delta_secs));
    if smoothness < 1.0 && approx_equal((new_value - *to).length(), 0.0) {
        new_value.x = to.x;
    }
    Position(new_value)
}

#[cfg(test)]
#[allow(
    clippy::unreadable_literal,
    clippy::float_cmp,
    reason = "test assertions verify deterministic bitwise-exact float results"
)]
mod approx_equal_tests {
    use super::*;

    #[test]
    fn same_value_is_approx_equal() {
        assert!(approx_equal(1.0, 1.0));
    }

    #[test]
    fn value_within_threshold_is_approx_equal() {
        assert!(approx_equal(1.0, EPSILON.mul_add(0.1, 1.0)));
    }

    #[test]
    fn value_outside_threshold_is_not_approx_equal() {
        assert!(!approx_equal(1.0, EPSILON.mul_add(10.0, 1.0)));
    }
}

#[cfg(test)]
#[allow(
    clippy::unreadable_literal,
    clippy::float_cmp,
    reason = "test assertions verify deterministic bitwise-exact float results"
)]
mod lerp_and_snap_f32_tests {
    use super::*;

    #[test]
    fn lerps_when_output_outside_snap_threshold() {
        let out = lerp_and_snap_f32(1.0, 2.0, 0.5, 1.0);
        // Due to the frame rate independence, this value is not easily predictable
        assert_eq!(out, 1.9921875);
    }

    #[test]
    fn snaps_to_target_when_inside_threshold() {
        let out = lerp_and_snap_f32(1.9991, 2.0, 0.5, 1.0);
        assert_eq!(out, 2.0);
        let out = lerp_and_snap_f32(1.9991, 2.0, 0.1, 1.0);
        assert_eq!(out, 2.0);
        let out = lerp_and_snap_f32(1.9991, 2.0, 0.9, 1.0);
        assert_eq!(out, 2.0);
    }

    #[test]
    fn does_not_snap_if_smoothness_is_one() {
        // Smoothness of one results in the value not changing, so it doesn't make sense to snap
        let out = lerp_and_snap_f32(1.9991, 2.0, 1.0, 1.0);
        assert_eq!(out, 1.9991);
    }
}

#[cfg(test)]
#[allow(
    clippy::unreadable_literal,
    clippy::float_cmp,
    reason = "test assertions verify deterministic bitwise-exact float results"
)]
mod lerp_and_snap_position_tests {
    use bevy::prelude::Vec3;

    use super::*;

    #[test]
    fn lerps_when_output_outside_snap_threshold() {
        let out = lerp_and_snap_position(Position::default(), Position(Vec3::X), 0.5, 1.0);
        // Due to the frame rate independence, this value is not easily predictable
        assert_eq!(out, Position::new(0.9921875, 0.0, 0.0));
    }

    #[test]
    fn snaps_to_target_when_inside_threshold() {
        let out = lerp_and_snap_position(Position(Vec3::X * 0.9991), Position(Vec3::X), 0.5, 1.0);
        assert_eq!(out, Position(Vec3::X));
        let out = lerp_and_snap_position(Position(Vec3::X * 0.9991), Position(Vec3::X), 0.1, 1.0);
        assert_eq!(out, Position(Vec3::X));
        let out = lerp_and_snap_position(Position(Vec3::X * 0.9991), Position(Vec3::X), 0.9, 1.0);
        assert_eq!(out, Position(Vec3::X));
    }

    #[test]
    fn does_not_snap_if_smoothness_is_one() {
        // Smoothness of one results in the value not changing, so it doesn't make sense to snap
        let out = lerp_and_snap_position(Position(Vec3::X * 0.9991), Position(Vec3::X), 1.0, 1.0);
        assert_eq!(out, Position(Vec3::X * 0.9991));
    }
}
