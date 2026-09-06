/// How far a camera operation's target may sit from the pose an animation set
/// and still count as unmoved. A difference at or below this threshold comes
/// from floating-point rounding; anything larger came from external input.
pub(super) const EXTERNAL_INPUT_TOLERANCE: f32 = 1e-6;

/// Smoothness value that disables interpolation and applies camera changes immediately.
pub(super) const INSTANT_SMOOTHNESS: f32 = 0.0;
