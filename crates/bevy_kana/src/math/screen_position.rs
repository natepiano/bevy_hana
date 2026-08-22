use std::ops::Add;
use std::ops::AddAssign;
use std::ops::Div;
use std::ops::DivAssign;
use std::ops::Mul;
use std::ops::MulAssign;
use std::ops::Neg;
use std::ops::Sub;
use std::ops::SubAssign;

use bevy::math::Vec2;
use bevy::prelude::Deref;
use bevy::reflect::Reflect;

/// Pixel-space coordinates on screen.
///
/// Wraps `Vec2` to distinguish screen coordinates from other 2D
/// quantities. Useful for UI layout, cursor tracking, and any
/// computation in pixel space.
///
/// # Examples
///
/// ```
/// use bevy_kana::ScreenPosition;
///
/// let cursor = ScreenPosition::new(640.0, 480.0);
/// let offset = ScreenPosition::new(10.0, -5.0);
/// let moved = cursor + offset;
/// assert_eq!(moved.x, 650.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default, Deref, Reflect)]
pub struct ScreenPosition(pub Vec2);

impl ScreenPosition {
    /// Creates a new screen position from `x` and `y` pixel coordinates.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self { Self(Vec2::new(x, y)) }

    /// Consumes `self` and returns the inner `Vec2`.
    #[must_use]
    pub const fn into_inner(self) -> Vec2 { self.0 }
}

impl From<Vec2> for ScreenPosition {
    fn from(value: Vec2) -> Self { Self(value) }
}

impl From<ScreenPosition> for Vec2 {
    fn from(value: ScreenPosition) -> Self { value.0 }
}

impl Add for ScreenPosition {
    type Output = Self;

    fn add(self, right_hand_side: Self) -> Self { Self(self.0 + right_hand_side.0) }
}

impl AddAssign for ScreenPosition {
    fn add_assign(&mut self, right_hand_side: Self) { self.0 += right_hand_side.0; }
}

impl Sub for ScreenPosition {
    type Output = Self;

    fn sub(self, right_hand_side: Self) -> Self { Self(self.0 - right_hand_side.0) }
}

impl SubAssign for ScreenPosition {
    fn sub_assign(&mut self, right_hand_side: Self) { self.0 -= right_hand_side.0; }
}

impl Mul<f32> for ScreenPosition {
    type Output = Self;

    fn mul(self, right_hand_side: f32) -> Self { Self(self.0 * right_hand_side) }
}

impl MulAssign<f32> for ScreenPosition {
    fn mul_assign(&mut self, right_hand_side: f32) { self.0 *= right_hand_side; }
}

impl Div<f32> for ScreenPosition {
    type Output = Self;

    fn div(self, right_hand_side: f32) -> Self { Self(self.0 / right_hand_side) }
}

impl DivAssign<f32> for ScreenPosition {
    fn div_assign(&mut self, right_hand_side: f32) { self.0 /= right_hand_side; }
}

impl Neg for ScreenPosition {
    type Output = Self;

    fn neg(self) -> Self { Self(-self.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // fixtures
    const BASE_SCREEN_POSITION: ScreenPosition = ScreenPosition::new(BASE_SCREEN_X, BASE_SCREEN_Y);
    const BASE_SCREEN_X: f32 = 100.0;
    const BASE_SCREEN_Y: f32 = 200.0;
    const OFFSET_SCREEN_POSITION: ScreenPosition = ScreenPosition::new(10.0, 20.0);

    #[test]
    fn add_returns_self() {
        let cursor_position = BASE_SCREEN_POSITION;
        let offset_position = OFFSET_SCREEN_POSITION;
        let result = cursor_position + offset_position;
        assert_eq!(
            result.into_inner(),
            BASE_SCREEN_POSITION.into_inner() + OFFSET_SCREEN_POSITION.into_inner()
        );
    }

    #[test]
    fn deref_provides_vec2_access() {
        let screen_position = BASE_SCREEN_POSITION;
        assert!((screen_position.x - BASE_SCREEN_X).abs() < f32::EPSILON);
        assert!((screen_position.y - BASE_SCREEN_Y).abs() < f32::EPSILON);
    }

    #[test]
    fn from_into_roundtrip() {
        let vec2 = BASE_SCREEN_POSITION.into_inner();
        let screen_position = ScreenPosition::from(vec2);
        let round_tripped_vec2: Vec2 = screen_position.into();
        assert_eq!(vec2, round_tripped_vec2);
    }
}
