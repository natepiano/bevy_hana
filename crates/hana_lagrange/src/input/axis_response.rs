//! Per-axis input tuning shared across camera controllers.

use bevy::prelude::Reflect;

/// Multiplier applied to an axis's semantic input as it drives a camera
/// operation (orbit, pan, zoom). A value of `0.0` disables the axis.
///
/// Distinct from [`InputGain`](crate::InputGain): sensitivity acts on the
/// camera operation, downstream of input gain, which scales raw device input
/// at the binding.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Sensitivity(pub f32);

impl Sensitivity {
    /// Returns the multiplier.
    #[must_use]
    pub const fn value(self) -> f32 { self.0 }
}

impl From<f32> for Sensitivity {
    fn from(value: f32) -> Self { Self(value) }
}

/// Damping applied to an axis's motion. `0.0` maps input to camera position
/// one-to-one; higher values lag the camera toward its target, approaching
/// infinite smoothing at `1.0`.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Damping(pub f32);

impl Damping {
    /// Returns the damping factor.
    #[must_use]
    pub const fn value(self) -> f32 { self.0 }
}

impl From<f32> for Damping {
    fn from(value: f32) -> Self { Self(value) }
}

/// Per-axis input tuning: how strongly input drives the axis and how much its
/// motion is dampened. One controller axis (orbit, pan, zoom, …) per value.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct AxisResponse {
    sensitivity: Sensitivity,
    damping:     Damping,
}

impl AxisResponse {
    /// Creates a response from a sensitivity multiplier and a damping factor.
    pub fn new(sensitivity: impl Into<Sensitivity>, damping: impl Into<Damping>) -> Self {
        Self {
            sensitivity: sensitivity.into(),
            damping:     damping.into(),
        }
    }

    /// Returns the sensitivity multiplier.
    #[must_use]
    pub const fn sensitivity(self) -> f32 { self.sensitivity.value() }

    /// Returns the damping factor.
    #[must_use]
    pub const fn damping(self) -> f32 { self.damping.value() }

    /// Replaces the sensitivity multiplier.
    pub fn set_sensitivity(&mut self, sensitivity: impl Into<Sensitivity>) {
        self.sensitivity = sensitivity.into();
    }

    /// Replaces the damping factor.
    pub fn set_damping(&mut self, damping: impl Into<Damping>) { self.damping = damping.into(); }
}
