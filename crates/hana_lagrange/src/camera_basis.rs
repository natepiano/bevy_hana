//! Shared camera coordinate basis.

use bevy::prelude::Component;
use bevy::prelude::Mat3;
use bevy::prelude::Quat;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Vec3;

/// World-space basis used by lagrange camera controllers.
///
/// The basis defines the camera's right axis, up axis, and the camera-back axis
/// used for yaw/pitch math. The default basis is Bevy's Y-up convention:
/// right = `Vec3::X`, up = `Vec3::Y`, back = `Vec3::Z`.
#[derive(Component, Reflect, Copy, Clone, Debug, PartialEq)]
#[reflect(Component)]
pub struct CameraBasis {
    /// World-space right axis.
    pub right: Vec3,
    /// World-space up axis.
    pub up:    Vec3,
    /// World-space camera-back axis.
    pub back:  Vec3,
}

impl CameraBasis {
    /// Bevy's default Y-up basis.
    pub const Y_UP: Self = Self {
        right: Vec3::X,
        up:    Vec3::Y,
        back:  Vec3::Z,
    };

    /// A right-handed Z-up basis with camera-back along `-Y`.
    pub const Z_UP: Self = Self {
        right: Vec3::X,
        up:    Vec3::Z,
        back:  Vec3::NEG_Y,
    };

    pub(crate) fn rotation(self) -> Quat {
        Quat::from_mat3(&Mat3::from_cols(self.right, self.up, self.back))
    }

    pub(crate) const fn axes(self) -> [Vec3; 3] { [self.right, self.up, self.back] }
}

impl Default for CameraBasis {
    fn default() -> Self { Self::Y_UP }
}

impl From<[Vec3; 3]> for CameraBasis {
    fn from(axes: [Vec3; 3]) -> Self {
        Self {
            right: axes[0],
            up:    axes[1],
            back:  axes[2],
        }
    }
}

impl From<CameraBasis> for [Vec3; 3] {
    fn from(basis: CameraBasis) -> Self { basis.axes() }
}
