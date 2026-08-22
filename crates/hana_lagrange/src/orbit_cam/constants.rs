/// Minimum orbit radius when camera and focus coincide.
pub(super) const MIN_ORBIT_RADIUS: f32 = 0.05;

/// Fraction of current radius applied per scroll unit.
pub(super) const SCROLL_ZOOM_FACTOR: f32 = 0.2;

/// Default smoothing factor for orbit motion.
pub(super) const DEFAULT_ORBIT_SMOOTHNESS: f32 = 0.1;

/// Default smoothing factor for pan motion.
pub(super) const DEFAULT_PAN_SMOOTHNESS: f32 = 0.02;

/// Default lower limit on zoom (radius or orthographic scale).
pub(super) const DEFAULT_ZOOM_LOWER_LIMIT: f32 = 1e-7;

/// Default smoothing factor for zoom motion.
pub(super) const DEFAULT_ZOOM_SMOOTHNESS: f32 = 0.1;

/// Perspective near plane as a fraction of the current orbit radius.
pub(super) const PERSPECTIVE_NEAR_RADIUS_FACTOR: f32 = 0.001;

/// Absolute minimum perspective near plane.
pub(super) const PERSPECTIVE_NEAR_MIN: f32 = 1e-6;
