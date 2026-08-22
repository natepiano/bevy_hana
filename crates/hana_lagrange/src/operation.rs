//! A camera operation's driven state: a value the user pushes toward a target,
//! eased by a response, kept legal by a limit.
//!
//! [`Operation`] bundles the three parts every camera operation shares —
//! sensitivity + damping (the response), the smoothed `current`/`target` pair,
//! and the limit that bounds the coordinate. The one per-kind part, the motion
//! mapping (how an input delta becomes a change in `target`), stays outside in
//! each camera's controller, so `Operation` is kind-agnostic state with no
//! trait dispatch of its own.
//!
//! `OrbitCam`'s `orbit`/`pan`/`zoom` and `FreeCam`'s `translate`/`look`/`roll`
//! are all instances of this one type, varying only in their coordinate and its
//! limit.

use core::fmt::Debug;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Mul;
use core::ops::MulAssign;

use bevy::prelude::Cuboid;
use bevy::prelude::FromReflect;
use bevy::prelude::Reflect;
use bevy::prelude::Sphere;
use bevy::prelude::TypePath;
use bevy::prelude::Vec3;

use crate::input::Damping;
use crate::input::Sensitivity;
use crate::interpolation;

/// One camera operation's driven state.
///
/// `current` eases toward `target` under `damping`; `sensitivity` is the
/// operation-stage multiplier the controller applies when turning input into
/// `target` changes (the motion mapping that reads it lives per-kind in the
/// controller, not here); `limit` bounds the coordinate. `damping` also carries
/// the after-release glide — when input stops, `target` freezes and `current`
/// keeps easing in until it arrives.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(where V::Limit: FromReflect + TypePath)]
pub struct Operation<V: Smoothable> {
    current:     V,
    target:      V,
    sensitivity: Sensitivity,
    damping:     Damping,
    limit:       V::Limit,
}

impl<V: Smoothable> Operation<V> {
    /// Creates an operation starting at `current`, with `target` equal to it.
    pub fn new(
        current: V,
        sensitivity: impl Into<Sensitivity>,
        damping: impl Into<Damping>,
        limit: V::Limit,
    ) -> Self {
        Self {
            current,
            target: current,
            sensitivity: sensitivity.into(),
            damping: damping.into(),
            limit,
        }
    }

    /// The smoothed current value.
    #[must_use]
    pub const fn current(self) -> V { self.current }

    /// The destination the current value eases toward.
    #[must_use]
    pub const fn target(self) -> V { self.target }

    /// Sets the destination the current value eases toward.
    pub fn set_target(&mut self, target: impl Into<V>) { self.target = target.into(); }

    /// Sets the current value directly, bypassing easing (an instant snap).
    pub fn set_current(&mut self, current: impl Into<V>) { self.current = current.into(); }

    /// Sets both current and target to `value` — an instant snap that leaves
    /// nothing for the next frame to ease toward.
    pub fn snap_to(&mut self, value: impl Into<V>) {
        let value = value.into();
        self.current = value;
        self.target = value;
    }

    /// The operation-stage sensitivity multiplier — applied to input here, at
    /// the operation, distinct from the per-device input gain applied upstream.
    #[must_use]
    pub const fn sensitivity(self) -> f32 { self.sensitivity.value() }

    /// Replaces the operation-stage sensitivity multiplier.
    pub fn set_sensitivity(&mut self, sensitivity: impl Into<Sensitivity>) {
        self.sensitivity = sensitivity.into();
    }

    /// The damping factor.
    #[must_use]
    pub const fn damping(self) -> f32 { self.damping.value() }

    /// Replaces the damping factor.
    pub fn set_damping(&mut self, damping: impl Into<Damping>) { self.damping = damping.into(); }

    /// The limit bounding the coordinate.
    #[must_use]
    pub const fn limit(self) -> V::Limit { self.limit }

    /// The limit bounding the coordinate, for in-place mutation.
    pub const fn limit_mut(&mut self) -> &mut V::Limit { &mut self.limit }

    /// Advances one frame: bounds the target, then eases current toward it.
    pub fn update(&mut self, delta_secs: f32) {
        self.target = self.limit.constrain(self.target);
        self.current = self
            .current
            .lerp_and_snap(self.target, self.damping.value(), delta_secs);
    }
}

/// A value that can be exponentially smoothed toward a target and bounded.
///
/// Implemented for the operation coordinates the cameras use. Each coordinate
/// names the limit type that bounds it.
pub trait Smoothable: Copy {
    /// The limit that bounds this coordinate.
    type Limit: Limit<Self> + Copy + Debug + PartialEq;

    /// Eases `self` toward `target` with frame-rate-independent smoothing.
    #[must_use]
    fn lerp_and_snap(self, target: Self, damping: f32, delta_secs: f32) -> Self;
}

/// How an operation's coordinate is constrained each frame.
///
/// The one polymorphic part of an operation — different coordinates bound
/// differently (scalar clamp, angle wrap, spatial region), so this is where the
/// type system discriminates.
pub trait Limit<V> {
    /// Returns `value` constrained to the legal range.
    fn constrain(self, value: V) -> V;
}

/// A camera's orbit/look angles in radians: rotation about the up axis (`yaw`)
/// and about the right axis (`pitch`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct OrbitAngles {
    /// Rotation about the up axis, in radians.
    pub yaw:   f32,
    /// Rotation about the right axis, in radians.
    pub pitch: f32,
}

/// A free-flight camera's look angles in radians: rotation about the up axis
/// (`yaw`) and about the right axis (`pitch`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct LookAngles {
    /// Rotation about the up axis, in radians.
    pub yaw:   f32,
    /// Rotation about the right axis, in radians.
    pub pitch: f32,
}

impl From<(f32, f32)> for OrbitAngles {
    fn from((yaw, pitch): (f32, f32)) -> Self { Self { yaw, pitch } }
}

impl From<(f32, f32)> for LookAngles {
    fn from((yaw, pitch): (f32, f32)) -> Self { Self { yaw, pitch } }
}

/// The orbit distance from the focus (perspective) or projection scale
/// (orthographic).
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct Radius(pub f32);

impl Mul<f32> for Radius {
    type Output = Self;

    fn mul(self, factor: f32) -> Self { Self(self.0 * factor) }
}

impl MulAssign<f32> for Radius {
    fn mul_assign(&mut self, factor: f32) { self.0 *= factor; }
}

impl From<f32> for Radius {
    fn from(radius: f32) -> Self { Self(radius) }
}

impl From<Radius> for f32 {
    fn from(radius: Radius) -> Self { radius.0 }
}

/// A free-flight camera's roll angle in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct Roll(pub f32);

impl Add<f32> for Roll {
    type Output = Self;

    fn add(self, delta: f32) -> Self { Self(self.0 + delta) }
}

impl AddAssign<f32> for Roll {
    fn add_assign(&mut self, delta: f32) { self.0 += delta; }
}

impl From<f32> for Roll {
    fn from(roll: f32) -> Self { Self(roll) }
}

impl From<Roll> for f32 {
    fn from(roll: Roll) -> Self { roll.0 }
}

/// The world-space point a camera orbits around and looks at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct Focus(pub Vec3);

impl Add<Vec3> for Focus {
    type Output = Self;

    fn add(self, translation: Vec3) -> Self { Self(self.0 + translation) }
}

impl AddAssign<Vec3> for Focus {
    fn add_assign(&mut self, translation: Vec3) { self.0 += translation; }
}

impl From<Vec3> for Focus {
    fn from(focus: Vec3) -> Self { Self(focus) }
}

impl From<Focus> for Vec3 {
    fn from(focus: Focus) -> Self { focus.0 }
}

/// A free-flight camera's world-space position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct Position(pub Vec3);

impl Add<Vec3> for Position {
    type Output = Self;

    fn add(self, translation: Vec3) -> Self { Self(self.0 + translation) }
}

impl AddAssign<Vec3> for Position {
    fn add_assign(&mut self, translation: Vec3) { self.0 += translation; }
}

impl From<Vec3> for Position {
    fn from(position: Vec3) -> Self { Self(position) }
}

impl From<Position> for Vec3 {
    fn from(position: Position) -> Self { position.0 }
}

/// Bound for a scalar coordinate (radius, roll).
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub enum ScalarLimit {
    /// Unbounded.
    #[default]
    None,
    /// Clamp to `[min, max]`.
    Clamp {
        /// Lower bound.
        min: f32,
        /// Upper bound.
        max: f32,
    },
    /// Wrap into `[0, period)`.
    Wrap {
        /// The wrap period.
        period: f32,
    },
}

impl ScalarLimit {
    fn apply_scalar(self, value: f32) -> f32 {
        match self {
            Self::None => value,
            Self::Clamp { min, max } => value.clamp(min, max),
            Self::Wrap { period } => value.rem_euclid(period),
        }
    }
}

impl Limit<Radius> for ScalarLimit {
    fn constrain(self, value: Radius) -> Radius { Radius(self.apply_scalar(value.0)) }
}

impl Limit<Roll> for ScalarLimit {
    fn constrain(self, value: Roll) -> Roll { Roll(self.apply_scalar(value.0)) }
}

/// Bound for the two angular axes (yaw, pitch), each bounded independently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct AnglePairLimit {
    /// Bound on the yaw axis.
    pub yaw:   ScalarLimit,
    /// Bound on the pitch axis.
    pub pitch: ScalarLimit,
}

impl Limit<OrbitAngles> for AnglePairLimit {
    fn constrain(self, value: OrbitAngles) -> OrbitAngles {
        OrbitAngles {
            yaw:   self.yaw.apply_scalar(value.yaw),
            pitch: self.pitch.apply_scalar(value.pitch),
        }
    }
}

impl Limit<LookAngles> for AnglePairLimit {
    fn constrain(self, value: LookAngles) -> LookAngles {
        LookAngles {
            yaw:   self.yaw.apply_scalar(value.yaw),
            pitch: self.pitch.apply_scalar(value.pitch),
        }
    }
}

/// Bound for the focus point, clamped into a region centered on `origin`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub enum RegionLimit {
    /// Unbounded.
    #[default]
    None,
    /// Clamp into a sphere centered on `origin`.
    Sphere {
        /// Center of the bounding sphere.
        origin: Vec3,
        /// The bounding sphere.
        sphere: Sphere,
    },
    /// Clamp into a cuboid centered on `origin`.
    Cuboid {
        /// Center of the bounding cuboid.
        origin: Vec3,
        /// The bounding cuboid.
        cuboid: Cuboid,
    },
}

impl Limit<Focus> for RegionLimit {
    fn constrain(self, value: Focus) -> Focus {
        match self {
            Self::None => value,
            Self::Sphere { origin, sphere } => {
                Focus(sphere.closest_point(value.0 - origin) + origin)
            },
            Self::Cuboid { origin, cuboid } => {
                Focus(cuboid.closest_point(value.0 - origin) + origin)
            },
        }
    }
}

impl Limit<Position> for RegionLimit {
    fn constrain(self, value: Position) -> Position {
        match self {
            Self::None => value,
            Self::Sphere { origin, sphere } => {
                Position(sphere.closest_point(value.0 - origin) + origin)
            },
            Self::Cuboid { origin, cuboid } => {
                Position(cuboid.closest_point(value.0 - origin) + origin)
            },
        }
    }
}

impl Smoothable for Radius {
    type Limit = ScalarLimit;

    fn lerp_and_snap(self, target: Self, damping: f32, delta_secs: f32) -> Self {
        Self(interpolation::lerp_and_snap_f32(
            self.0, target.0, damping, delta_secs,
        ))
    }
}

impl Smoothable for Roll {
    type Limit = ScalarLimit;

    fn lerp_and_snap(self, target: Self, damping: f32, delta_secs: f32) -> Self {
        Self(interpolation::lerp_and_snap_f32(
            self.0, target.0, damping, delta_secs,
        ))
    }
}

impl Smoothable for OrbitAngles {
    type Limit = AnglePairLimit;

    fn lerp_and_snap(self, target: Self, damping: f32, delta_secs: f32) -> Self {
        Self {
            yaw:   interpolation::lerp_and_snap_f32(self.yaw, target.yaw, damping, delta_secs),
            pitch: interpolation::lerp_and_snap_f32(self.pitch, target.pitch, damping, delta_secs),
        }
    }
}

impl Smoothable for LookAngles {
    type Limit = AnglePairLimit;

    fn lerp_and_snap(self, target: Self, damping: f32, delta_secs: f32) -> Self {
        Self {
            yaw:   interpolation::lerp_and_snap_f32(self.yaw, target.yaw, damping, delta_secs),
            pitch: interpolation::lerp_and_snap_f32(self.pitch, target.pitch, damping, delta_secs),
        }
    }
}

impl Smoothable for Focus {
    type Limit = RegionLimit;

    fn lerp_and_snap(self, target: Self, damping: f32, delta_secs: f32) -> Self {
        Self(*interpolation::lerp_and_snap_position(
            self.0, target.0, damping, delta_secs,
        ))
    }
}

impl Smoothable for Position {
    type Limit = RegionLimit;

    fn lerp_and_snap(self, target: Self, damping: f32, delta_secs: f32) -> Self {
        Self(*interpolation::lerp_and_snap_position(
            self.0, target.0, damping, delta_secs,
        ))
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "test assertions verify deterministic bitwise-exact float results"
)]
mod scalar_limit_tests {
    use super::*;

    #[test]
    fn none_passes_through() {
        assert_eq!(ScalarLimit::None.constrain(Radius(42.0)), Radius(42.0));
        assert_eq!(ScalarLimit::None.constrain(Radius(-1.0)), Radius(-1.0));
    }

    #[test]
    fn clamp_bounds_both_ends() {
        let limit = ScalarLimit::Clamp {
            min: 0.0,
            max: 10.0,
        };
        assert_eq!(limit.constrain(Radius(-5.0)), Radius(0.0));
        assert_eq!(limit.constrain(Radius(15.0)), Radius(10.0));
        assert_eq!(limit.constrain(Radius(4.0)), Radius(4.0));
    }

    #[test]
    fn wrap_folds_into_zero_to_period() {
        let limit = ScalarLimit::Wrap { period: 360.0 };
        assert_eq!(limit.constrain(Radius(-1.0)), Radius(359.0));
        assert_eq!(limit.constrain(Radius(370.0)), Radius(10.0));
        assert_eq!(limit.constrain(Radius(45.0)), Radius(45.0));
    }
}

#[cfg(test)]
mod angle_pair_limit_tests {
    use super::*;

    #[test]
    fn each_axis_is_bounded_independently() {
        let limit = AnglePairLimit {
            yaw:   ScalarLimit::Clamp {
                min: -90.0,
                max: 90.0,
            },
            pitch: ScalarLimit::None,
        };
        // yaw clamps, pitch passes through unchanged.
        assert_eq!(
            limit.constrain(OrbitAngles {
                yaw:   180.0,
                pitch: 180.0,
            }),
            OrbitAngles {
                yaw:   90.0,
                pitch: 180.0,
            }
        );
        assert_eq!(
            limit.constrain(OrbitAngles {
                yaw:   -180.0,
                pitch: -5.0,
            }),
            OrbitAngles {
                yaw:   -90.0,
                pitch: -5.0,
            }
        );
    }
}

#[cfg(test)]
mod region_limit_tests {
    use super::*;

    #[test]
    fn none_passes_through() {
        assert_eq!(
            RegionLimit::None.constrain(Focus(Vec3::new(1.0, 2.0, 3.0))),
            Focus(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn sphere_pulls_outside_point_to_surface_offset_by_origin() {
        let limit = RegionLimit::Sphere {
            origin: Vec3::new(10.0, 0.0, 0.0),
            sphere: Sphere::new(2.0),
        };
        // 10 units out along x from a radius-2 sphere centered at x=10 → surface at x=12.
        assert_eq!(
            limit.constrain(Focus(Vec3::new(20.0, 0.0, 0.0))),
            Focus(Vec3::new(12.0, 0.0, 0.0))
        );
        // A point already inside is left where it is.
        assert_eq!(
            limit.constrain(Focus(Vec3::new(10.5, 0.0, 0.0))),
            Focus(Vec3::new(10.5, 0.0, 0.0))
        );
    }

    #[test]
    fn cuboid_clamps_outside_point_to_face_offset_by_origin() {
        let limit = RegionLimit::Cuboid {
            origin: Vec3::new(10.0, 0.0, 0.0),
            cuboid: Cuboid::new(2.0, 2.0, 2.0),
        };
        // Half-extent 1 on each axis, centered at x=10 → x clamps to 11, y to 1.
        assert_eq!(
            limit.constrain(Focus(Vec3::new(20.0, 5.0, 0.0))),
            Focus(Vec3::new(11.0, 1.0, 0.0))
        );
        assert_eq!(
            limit.constrain(Focus(Vec3::new(10.5, 0.5, 0.0))),
            Focus(Vec3::new(10.5, 0.5, 0.0))
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "test assertions verify deterministic bitwise-exact float results"
)]
mod operation_tests {
    use super::*;

    #[test]
    fn update_bounds_target_before_easing() {
        let mut op = Operation::new(
            Radius(0.0),
            1.0,
            0.5,
            ScalarLimit::Clamp {
                min: 0.0,
                max: 10.0,
            },
        );
        op.set_target(Radius(100.0));
        op.update(1.0);
        // The out-of-range target is clamped at the limit, not left at 100.
        assert_eq!(op.target(), Radius(10.0));
        // current eases toward the clamped target — partway there, not past it.
        assert!(op.current().0 > 0.0 && op.current().0 < 10.0);
    }

    #[test]
    fn update_converges_and_snaps_to_target() {
        let mut op = Operation::new(Radius(0.0), 1.0, 0.5, ScalarLimit::None);
        op.set_target(Radius(5.0));
        for _ in 0..200 {
            op.update(0.1);
        }
        // Smoothing's snap threshold lands current exactly on target.
        assert_eq!(op.current(), Radius(5.0));
    }

    #[test]
    fn set_target_accepts_bare_scalar_via_into() {
        let mut op = Operation::new(Radius(0.0), 1.0, 0.5, ScalarLimit::None);
        op.set_target(5.0);
        assert_eq!(op.target(), Radius(5.0));
    }

    #[test]
    fn update_eases_each_angle_axis() {
        let mut op = Operation::new(OrbitAngles::default(), 1.0, 0.5, AnglePairLimit::default());
        op.set_target(OrbitAngles {
            yaw:   2.0,
            pitch: -3.0,
        });
        op.update(1.0);
        let current = op.current();
        // Both axes moved toward their target sign, neither overshot.
        assert!(current.yaw > 0.0 && current.yaw < 2.0);
        assert!(current.pitch < 0.0 && current.pitch > -3.0);
    }

    #[test]
    fn update_clamps_focus_target_into_region() {
        let limit = RegionLimit::Cuboid {
            origin: Vec3::ZERO,
            cuboid: Cuboid::new(2.0, 2.0, 2.0),
        };
        let mut op = Operation::new(Focus(Vec3::ZERO), 1.0, 0.5, limit);
        op.set_target(Focus(Vec3::new(100.0, 0.0, 0.0)));
        op.update(1.0);
        // Target is pulled to the cuboid face (half-extent 1) before easing.
        assert_eq!(op.target(), Focus(Vec3::new(1.0, 0.0, 0.0)));
    }
}
