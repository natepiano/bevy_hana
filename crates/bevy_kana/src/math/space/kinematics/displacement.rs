use bevy::math::Vec3;
use bevy::prelude::Deref;
use bevy::reflect::Reflect;

use super::semantic_newtype;

semantic_newtype::semantic_newtype!(
    /// A delta or offset in 3D space.
    ///
    /// Wraps `Vec3` to distinguish spatial offsets from other vector
    /// quantities like position or velocity. Use `Displacement` to
    /// represent the difference between two points, a movement delta,
    /// or any directional distance.
    ///
    /// # Examples
    ///
    /// ```
    /// use bevy::math::Vec3;
    /// use bevy_kana::Displacement;
    ///
    /// let step = Displacement(Vec3::new(0.0, 0.0, -1.0));
    /// let double_step = step + step;
    /// assert_eq!(double_step.into_inner(), Vec3::new(0.0, 0.0, -2.0));
    /// ```
    Displacement, Vec3
);

#[cfg(test)]
mod tests {
    use bevy::math::Vec3;

    use super::*;

    // displacement fixtures
    const DISPLACEMENT_LEFT: Displacement = Displacement::new(1.0, 0.0, 0.0);
    const DISPLACEMENT_RIGHT: Displacement = Displacement::new(0.0, 1.0, 0.0);
    const DISPLACEMENT_ROUNDTRIP_VECTOR: Vec3 = Vec3::new(1.0, 2.0, 3.0);

    #[test]
    fn displacement_add_returns_self() {
        let left_displacement = DISPLACEMENT_LEFT;
        let right_displacement = DISPLACEMENT_RIGHT;
        let result = left_displacement + right_displacement;
        assert_eq!(
            result.into_inner(),
            DISPLACEMENT_LEFT.into_inner() + DISPLACEMENT_RIGHT.into_inner()
        );
    }

    #[test]
    fn displacement_from_into_roundtrip() {
        let vec3 = DISPLACEMENT_ROUNDTRIP_VECTOR;
        let displacement = Displacement::from(vec3);
        let round_tripped_vec3: Vec3 = displacement.into();
        assert_eq!(vec3, round_tripped_vec3);
    }
}
