// interpolation constants
/// Approximate-equality threshold for floating-point convergence checks.
pub(crate) const EPSILON: f32 = 0.001;
/// Exponent applied to the smoothing response curve.
pub(crate) const SMOOTHNESS_EXPONENT: i32 = 7;

// orbit defaults
/// Default yaw and pitch target when the camera faces forward.
pub(crate) const DEFAULT_ORBIT_ANGLE: f32 = 0.0;
/// Default orbital radius.
pub(crate) const DEFAULT_TARGET_RADIUS: f32 = 1.0;

// shared time constants
/// Conversion factor from seconds to milliseconds.
pub(crate) const MILLIS_PER_SECOND: f32 = 1000.0;
