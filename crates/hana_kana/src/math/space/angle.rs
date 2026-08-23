use bevy::reflect::Reflect;
use thiserror::Error;

/// A signed, unwrapped angular displacement in radians.
///
/// `Angle` preserves finite values exactly, including positive and negative
/// values spanning multiple turns. It never normalizes values modulo a turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct Angle(f32);

impl Angle {
    /// Creates an angular displacement from a finite number of radians.
    ///
    /// # Errors
    ///
    /// Returns [`AngleError::NonFiniteRadians`] when `radians` is NaN or
    /// infinite.
    pub const fn from_radians(radians: f32) -> Result<Self, AngleError> {
        if radians.is_finite() {
            Ok(Self(radians))
        } else {
            Err(AngleError::NonFiniteRadians)
        }
    }

    /// Returns the signed, unwrapped angular displacement in radians.
    #[must_use]
    pub const fn radians(self) -> f32 { self.0 }
}

impl From<Angle> for f32 {
    fn from(angle: Angle) -> Self { angle.0 }
}

/// Errors returned while constructing an [`Angle`].
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AngleError {
    /// The supplied radians were NaN or infinite.
    #[error("angle radians must be finite")]
    NonFiniteRadians,
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use bevy::reflect::FromReflect;
    use bevy::reflect::PartialReflect;
    use bevy::reflect::ReflectKind;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;

    use super::*;

    // angle fixtures
    const NEGATIVE_MULTI_TURN_RADIANS: f32 = -2.5 * TAU;
    const POSITIVE_MULTI_TURN_RADIANS: f32 = 3.5 * TAU;

    #[test]
    fn angle_from_radians_preserves_multi_turn_values() -> Result<(), AngleError> {
        let positive_angle = Angle::from_radians(POSITIVE_MULTI_TURN_RADIANS)?;
        let negative_angle = Angle::from_radians(NEGATIVE_MULTI_TURN_RADIANS)?;

        assert_eq!(
            positive_angle.radians().to_bits(),
            POSITIVE_MULTI_TURN_RADIANS.to_bits()
        );
        assert_eq!(
            negative_angle.radians().to_bits(),
            NEGATIVE_MULTI_TURN_RADIANS.to_bits()
        );

        let converted_radians: f32 = positive_angle.into();
        assert_eq!(
            converted_radians.to_bits(),
            POSITIVE_MULTI_TURN_RADIANS.to_bits()
        );
        Ok(())
    }

    #[test]
    fn angle_from_radians_rejects_non_finite_values() {
        assert_eq!(
            Angle::from_radians(f32::NAN),
            Err(AngleError::NonFiniteRadians)
        );
        assert_eq!(
            Angle::from_radians(f32::INFINITY),
            Err(AngleError::NonFiniteRadians)
        );
        assert_eq!(
            Angle::from_radians(f32::NEG_INFINITY),
            Err(AngleError::NonFiniteRadians)
        );
    }

    #[test]
    fn angle_reflection_is_opaque() {
        let mut invalid_structural_value = DynamicTupleStruct::default();
        invalid_structural_value.insert(f32::NAN);

        assert_eq!(Angle::default().reflect_kind(), ReflectKind::Opaque);
        assert!(Angle::from_reflect(&invalid_structural_value).is_none());
    }
}
