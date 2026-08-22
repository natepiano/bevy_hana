/// Tolerance for detecting external camera input during animations.
/// Values within this threshold are considered unchanged (accounts for floating point noise).
pub(super) const EXTERNAL_INPUT_TOLERANCE: f32 = 1e-6;

/// Smoothness value that disables interpolation and applies camera changes immediately.
pub(super) const INSTANT_SMOOTHNESS: f32 = 0.0;
