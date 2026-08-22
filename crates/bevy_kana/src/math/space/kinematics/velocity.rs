use bevy::math::Vec3;
use bevy::prelude::Deref;
use bevy::reflect::Reflect;

use super::semantic_newtype;

semantic_newtype::semantic_newtype!(
    /// Rate of position change in 3D space.
    ///
    /// Wraps `Vec3` to distinguish velocity from other vector quantities.
    /// Scaling a `Velocity` by a time delta (e.g., `velocity * time_delta`)
    /// gives a per-frame displacement while preserving the `Velocity` type
    /// for the scaled result.
    ///
    /// # Examples
    ///
    /// ```
    /// use bevy::math::Vec3;
    /// use bevy_kana::Velocity;
    ///
    /// let velocity = Velocity(Vec3::new(10.0, 0.0, 0.0));
    /// let time_delta = 0.016;
    /// let frame_velocity = velocity * time_delta;
    /// assert!((frame_velocity.x - 0.16).abs() < 1e-6);
    /// ```
    Velocity, Vec3
);

#[cfg(test)]
mod tests {
    use super::*;

    // velocity fixtures
    const FRAME_TIME_DELTA: f32 = 0.016;
    const LEFT_VELOCITY: Velocity = Velocity::new(1.0, 0.0, 0.0);
    const RIGHT_VELOCITY: Velocity = Velocity::new(0.0, 1.0, 0.0);
    const SCALED_VELOCITY: Velocity = Velocity::new(10.0, 0.0, 0.0);
    const VELOCITY_PER_FRAME_TOLERANCE: f32 = 1e-6;

    #[test]
    fn velocity_add_combines_velocities() {
        let left_velocity = LEFT_VELOCITY;
        let right_velocity = RIGHT_VELOCITY;
        let combined = left_velocity + right_velocity;
        assert_eq!(
            combined.into_inner(),
            LEFT_VELOCITY.into_inner() + RIGHT_VELOCITY.into_inner()
        );
    }

    #[test]
    fn velocity_scalar_mul_for_time_delta() {
        let velocity = SCALED_VELOCITY;
        let frame_velocity = velocity * FRAME_TIME_DELTA;
        assert!(
            SCALED_VELOCITY
                .x
                .mul_add(-FRAME_TIME_DELTA, frame_velocity.x)
                .abs()
                < VELOCITY_PER_FRAME_TOLERANCE
        );
    }
}
