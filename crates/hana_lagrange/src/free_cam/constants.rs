/// Default operation-stage sensitivity for free-flight look input.
pub(super) const DEFAULT_FREE_LOOK_SENSITIVITY: f32 = 0.003;

/// Default smoothing factor for free-flight look motion.
pub(super) const DEFAULT_FREE_LOOK_SMOOTHNESS: f32 = 0.1;

/// Margin below vertical used by [`FreeCam::pitch_limited`].
///
/// [`FreeCam::pitch_limited`]: super::FreeCam::pitch_limited
pub(super) const DEFAULT_FREE_PITCH_LIMIT_MARGIN: f32 = 0.01;

/// Default absolute pitch clamp for [`FreeCam::pitch_limited`].
///
/// [`FreeCam::pitch_limited`]: super::FreeCam::pitch_limited
pub(super) const DEFAULT_FREE_PITCH_LIMIT: f32 =
    std::f32::consts::FRAC_PI_2 - DEFAULT_FREE_PITCH_LIMIT_MARGIN;

/// Default operation-stage sensitivity for free-flight roll input.
pub(super) const DEFAULT_FREE_ROLL_SENSITIVITY: f32 = 1.5;

/// Default smoothing factor for free-flight roll motion.
pub(super) const DEFAULT_FREE_ROLL_SMOOTHNESS: f32 = 0.1;

/// Default operation-stage sensitivity for free-flight translation input.
pub(super) const DEFAULT_FREE_TRANSLATE_SENSITIVITY: f32 = 16.0;

/// Default smoothing factor for free-flight translation motion.
pub(super) const DEFAULT_FREE_TRANSLATE_SMOOTHNESS: f32 = 0.02;
