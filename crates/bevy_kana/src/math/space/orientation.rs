use std::ops::Mul;
use std::ops::MulAssign;

use bevy::math::Quat;
use bevy::math::Vec3;
use bevy::prelude::Deref;
use bevy::reflect::Reflect;
use thiserror::Error;

/// A rotation in 3D space.
///
/// Every `Orientation` contains a finite, normalized `Quat`. Unlike the
/// semantic `Vec3` types, `Orientation` has custom arithmetic:
///
/// - `Orientation * Orientation → Orientation` (rotation composition)
/// - `Orientation * Vec3 → Vec3` (rotate a vector)
///
/// Read-only `Quat` methods and fields are available through `Deref`.
///
/// # Examples
///
/// ```
/// use std::f32::consts::FRAC_PI_2;
///
/// use bevy::math::Quat;
/// use bevy::math::Vec3;
/// use bevy_kana::Orientation;
/// use bevy_kana::OrientationError;
///
/// # fn main() -> Result<(), OrientationError> {
/// let orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
/// let rotated = orientation * Vec3::X;
/// assert!((rotated - Vec3::NEG_Z).length() < 1e-6);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default, Deref, Reflect)]
#[reflect(opaque)]
pub struct Orientation(Quat);

impl Orientation {
    /// Consumes `self` and returns the inner `Quat`.
    #[must_use]
    pub const fn into_inner(self) -> Quat { self.0 }

    /// Returns the inverse rotation.
    #[must_use]
    pub fn inverse(self) -> Self { Self(self.0.inverse()) }

    /// Spherical linear interpolation between `self` and `other`.
    ///
    /// # Errors
    ///
    /// Returns [`OrientationError::NonFiniteInterpolationFactor`] when `t` is
    /// NaN or infinite. A finite factor that produces an invalid quaternion
    /// returns the corresponding [`OrientationError`].
    pub fn slerp(self, other: Self, t: f32) -> Result<Self, OrientationError> {
        Self::validate_interpolation_factor(t)?;
        Self::try_from(self.0.slerp(other.0, t))
    }

    /// Linear interpolation between `self` and `other`.
    ///
    /// Faster than [`Orientation::slerp`] but less accurate for large
    /// angular differences.
    ///
    /// # Errors
    ///
    /// Returns [`OrientationError::NonFiniteInterpolationFactor`] when `t` is
    /// NaN or infinite. A finite factor that produces an invalid quaternion
    /// returns the corresponding [`OrientationError`].
    pub fn lerp(self, other: Self, t: f32) -> Result<Self, OrientationError> {
        Self::validate_interpolation_factor(t)?;
        Self::try_from(self.0.lerp(other.0, t))
    }

    const fn validate_interpolation_factor(t: f32) -> Result<(), OrientationError> {
        if t.is_finite() {
            Ok(())
        } else {
            Err(OrientationError::NonFiniteInterpolationFactor)
        }
    }
}

impl TryFrom<Quat> for Orientation {
    type Error = OrientationError;

    fn try_from(quaternion: Quat) -> Result<Self, Self::Error> {
        if !quaternion.is_finite() {
            return Err(OrientationError::NonFiniteQuaternion);
        }

        let largest_component = quaternion
            .to_array()
            .into_iter()
            .map(f32::abs)
            .fold(0.0, f32::max);
        if largest_component <= 1.0 && quaternion.length_squared() <= f32::EPSILON {
            return Err(OrientationError::EffectivelyZeroLengthQuaternion);
        }

        let safely_scaled = quaternion * largest_component.recip();
        Ok(Self(safely_scaled.normalize()))
    }
}

impl From<Orientation> for Quat {
    fn from(value: Orientation) -> Self { value.0 }
}

/// Errors returned while constructing or interpolating an [`Orientation`].
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum OrientationError {
    /// A raw quaternion contained NaN or an infinite component.
    #[error("orientation quaternion must contain only finite components")]
    NonFiniteQuaternion,
    /// The raw quaternion's squared length was at most `f32::EPSILON`.
    #[error("orientation quaternion length is effectively zero")]
    EffectivelyZeroLengthQuaternion,
    /// An interpolation factor was NaN or infinite.
    #[error("orientation interpolation factor must be finite")]
    NonFiniteInterpolationFactor,
}

/// Rotation composition: applying `right_hand_side` then `self`.
impl Mul for Orientation {
    type Output = Self;

    fn mul(self, right_hand_side: Self) -> Self { Self((self.0 * right_hand_side.0).normalize()) }
}

impl MulAssign for Orientation {
    fn mul_assign(&mut self, right_hand_side: Self) { *self = *self * right_hand_side; }
}

/// Rotates a vector by this orientation.
impl Mul<Vec3> for Orientation {
    type Output = Vec3;

    fn mul(self, right_hand_side: Vec3) -> Vec3 { self.0 * right_hand_side }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use bevy::reflect::FromReflect;
    use bevy::reflect::PartialReflect;
    use bevy::reflect::ReflectKind;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;

    use super::*;

    // orientation expectations
    const COMPOSED_ROTATION_TOLERANCE: f32 = 1e-5;
    const HALF_TURN_ANGLE_TOLERANCE: f32 = 1e-5;
    const IDENTITY_ROTATION_TOLERANCE: f32 = 1e-6;
    const IDENTITY_W_COMPONENT: f32 = 1.0;
    const NON_UNIT_W_COMPONENT: f32 = 2.0;
    const ROTATED_VECTOR_TOLERANCE: f32 = 1e-6;
    const SLERP_FACTOR: f32 = 0.5;

    #[test]
    fn orientation_deref_provides_quat_access() -> Result<(), OrientationError> {
        let orientation = Orientation::try_from(Quat::IDENTITY)?;
        assert!((orientation.w - IDENTITY_W_COMPONENT).abs() < f32::EPSILON);
        Ok(())
    }

    #[test]
    fn orientation_try_from_into_roundtrip() -> Result<(), OrientationError> {
        let quat = Quat::from_rotation_y(FRAC_PI_2);
        let orientation = Orientation::try_from(quat)?;
        let round_tripped_quat: Quat = orientation.into();
        assert!(quat.abs_diff_eq(round_tripped_quat, f32::EPSILON));
        Ok(())
    }

    #[test]
    fn orientation_try_from_normalizes_raw_quaternion() -> Result<(), OrientationError> {
        let raw_quaternion = Quat::from_xyzw(0.0, 0.0, 0.0, NON_UNIT_W_COMPONENT);
        let orientation = Orientation::try_from(raw_quaternion)?;

        assert!(orientation.is_normalized());
        assert!((orientation.w - IDENTITY_W_COMPONENT).abs() < f32::EPSILON);
        Ok(())
    }

    #[test]
    fn orientation_try_from_rejects_non_finite_quaternions() {
        let nan_quaternion = Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0);
        let infinite_quaternion = Quat::from_xyzw(0.0, f32::INFINITY, 0.0, 1.0);

        assert_eq!(
            Orientation::try_from(nan_quaternion),
            Err(OrientationError::NonFiniteQuaternion)
        );
        assert_eq!(
            Orientation::try_from(infinite_quaternion),
            Err(OrientationError::NonFiniteQuaternion)
        );
    }

    #[test]
    fn orientation_try_from_rejects_effectively_zero_length_quaternion() {
        let quaternion = Quat::from_xyzw(f32::EPSILON.sqrt(), 0.0, 0.0, 0.0);

        assert_eq!(
            Orientation::try_from(quaternion),
            Err(OrientationError::EffectivelyZeroLengthQuaternion)
        );
    }

    #[test]
    fn orientation_inverse_undoes_rotation() -> Result<(), OrientationError> {
        let orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
        let inverse_orientation = orientation.inverse();
        let composed_orientation = orientation * inverse_orientation;
        let result = composed_orientation * Vec3::X;
        assert!((result - Vec3::X).length() < IDENTITY_ROTATION_TOLERANCE);
        assert!(composed_orientation.is_normalized());
        Ok(())
    }

    #[test]
    fn orientation_rotate_vector() -> Result<(), OrientationError> {
        let orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
        let result = orientation * Vec3::X;
        assert!((result - Vec3::NEG_Z).length() < ROTATED_VECTOR_TOLERANCE);
        Ok(())
    }

    #[test]
    fn orientation_rotation_composition() -> Result<(), OrientationError> {
        let first_orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
        let second_orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
        let composed_orientation = first_orientation * second_orientation;
        let result = composed_orientation * Vec3::X;
        assert!((result - Vec3::NEG_X).length() < COMPOSED_ROTATION_TOLERANCE);
        assert!(composed_orientation.is_normalized());
        Ok(())
    }

    #[test]
    fn orientation_slerp_halfway() -> Result<(), OrientationError> {
        let start_orientation = Orientation::try_from(Quat::IDENTITY)?;
        let end_orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
        let midpoint_orientation = start_orientation.slerp(end_orientation, SLERP_FACTOR)?;
        let result = midpoint_orientation * Vec3::X;
        let angle = result.angle_between(Vec3::X);
        assert!(FRAC_PI_2.mul_add(-SLERP_FACTOR, angle).abs() < HALF_TURN_ANGLE_TOLERANCE);
        assert!(midpoint_orientation.is_normalized());
        Ok(())
    }

    #[test]
    fn orientation_lerp_preserves_normalization() -> Result<(), OrientationError> {
        let start_orientation = Orientation::try_from(Quat::IDENTITY)?;
        let end_orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
        let midpoint_orientation = start_orientation.lerp(end_orientation, SLERP_FACTOR)?;

        assert!(midpoint_orientation.is_normalized());
        Ok(())
    }

    #[test]
    fn orientation_interpolation_rejects_non_finite_factor() -> Result<(), OrientationError> {
        let start_orientation = Orientation::try_from(Quat::IDENTITY)?;
        let end_orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;

        assert_eq!(
            start_orientation.slerp(end_orientation, f32::NAN),
            Err(OrientationError::NonFiniteInterpolationFactor)
        );
        assert_eq!(
            start_orientation.lerp(end_orientation, f32::INFINITY),
            Err(OrientationError::NonFiniteInterpolationFactor)
        );
        Ok(())
    }

    #[test]
    fn orientation_reflection_is_opaque() {
        let mut invalid_structural_value = DynamicTupleStruct::default();
        invalid_structural_value.insert(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0));

        assert_eq!(Orientation::default().reflect_kind(), ReflectKind::Opaque);
        assert!(Orientation::from_reflect(&invalid_structural_value).is_none());
    }
}
