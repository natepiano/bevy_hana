use bevy::math::Vec3;
use bevy::prelude::Deref;
use bevy::reflect::Reflect;

use super::semantic_newtype;

semantic_newtype::semantic_newtype!(
    /// A point in 3D space.
    ///
    /// Wraps `Vec3` to distinguish spatial positions from other vector
    /// quantities like velocity or displacement. All arithmetic operations
    /// return `Position`, and `Deref` provides transparent access to
    /// `Vec3` fields and methods.
    ///
    /// # Examples
    ///
    /// ```
    /// use bevy::math::Vec3;
    /// use bevy_kana::Position;
    ///
    /// let start_position = Position(Vec3::new(1.0, 0.0, 0.0));
    /// let end_position = Position(Vec3::new(3.0, 0.0, 0.0));
    ///
    /// // Centroid of two points
    /// let midpoint = (start_position + end_position) / 2.0;
    /// assert_eq!(midpoint.into_inner(), Vec3::new(2.0, 0.0, 0.0));
    /// ```
    Position, Vec3
);

#[cfg(test)]
mod tests {
    use bevy::math::Vec3;

    use super::*;

    // position fixtures
    const POSITION: Position = Position::new(POSITION_X, POSITION_Y, POSITION_Z);
    const POSITION_LEFT: Position = Position::new(1.0, 0.0, 0.0);
    const POSITION_NEGATED_INPUT: Position = Position::new(1.0, -2.0, 3.0);
    const POSITION_RIGHT: Position = Position::new(0.0, 1.0, 0.0);
    const POSITION_WITH_SCALE_INPUT: Position = Position::new(2.0, 4.0, 6.0);
    const POSITION_X: f32 = 1.0;
    const POSITION_Y: f32 = 2.0;
    const POSITION_Z: f32 = 3.0;
    const SCALE_FACTOR: f32 = 2.0;

    #[test]
    fn position_add_assign() {
        let mut position = POSITION_LEFT;
        position += POSITION_RIGHT;
        assert_eq!(
            position.into_inner(),
            POSITION_LEFT.into_inner() + POSITION_RIGHT.into_inner()
        );
    }

    #[test]
    fn position_add_returns_self() {
        let left_position = POSITION_LEFT;
        let right_position = POSITION_RIGHT;
        let result = left_position + right_position;
        assert_eq!(
            result.into_inner(),
            POSITION_LEFT.into_inner() + POSITION_RIGHT.into_inner()
        );
    }

    #[test]
    fn position_deref_provides_vec3_access() {
        let position = POSITION;
        assert!((position.x - POSITION_X).abs() < f32::EPSILON);
        assert!(
            (position.length() - Vec3::new(POSITION_X, POSITION_Y, POSITION_Z).length()).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn position_from_into_roundtrip() {
        let vec3 = POSITION.into_inner();
        let position = Position::from(vec3);
        let round_tripped_vec3: Vec3 = position.into();
        assert_eq!(vec3, round_tripped_vec3);
    }

    #[test]
    fn position_neg() {
        let position = POSITION_NEGATED_INPUT;
        let result = -position;
        assert_eq!(result.into_inner(), -POSITION_NEGATED_INPUT.into_inner());
    }

    #[test]
    fn position_scalar_div() {
        let position = POSITION_WITH_SCALE_INPUT;
        let result = position / SCALE_FACTOR;
        assert_eq!(result.into_inner(), POSITION.into_inner());
    }

    #[test]
    fn position_scalar_mul() {
        let position = POSITION;
        let result = position * SCALE_FACTOR;
        assert_eq!(result.into_inner(), POSITION.into_inner() * SCALE_FACTOR);
    }
}
