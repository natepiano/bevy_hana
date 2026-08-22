// animate-to-fit defaults
pub(super) const DEFAULT_ANIMATE_TO_FIT_PITCH: f32 = 0.0;
pub(super) const DEFAULT_ANIMATE_TO_FIT_YAW: f32 = 0.0;

// fit solve constants
/// Maximum centering iterations per candidate radius.
pub(super) const CENTERING_MAX_ITERATIONS: usize = 10;
/// Normalized screen-space center offset tolerance.
pub(super) const CENTERING_TOLERANCE: f32 = 0.0001;
/// Default fit margin applied by event constructors.
pub(super) const DEFAULT_FIT_MARGIN: f32 = 0.1;
/// Minimum screen-space extent before treating a dimension as degenerate (edge-on).
/// Below this threshold the dimension is ignored for fit purposes.
pub(super) const DEGENERATE_EXTENT_THRESHOLD: f32 = 1e-6;
/// Initial best-guess radius as a multiple of the object radius (2x).
pub(super) const INITIAL_RADIUS_MULTIPLIER: f32 = 2.0;
/// Maximum allowed margin value.
pub(super) const MAX_MARGIN: f32 = 0.9999;
/// Maximum binary search iterations.
pub(super) const MAX_ITERATIONS: usize = 200;
/// Maximum search radius as a multiple of the object radius (100x).
pub(super) const MAX_RADIUS_MULTIPLIER: f32 = 100.0;
/// Minimum allowed margin value.
pub(super) const MIN_MARGIN: f32 = 0.0;
/// Minimum search radius as a fraction of the object radius (0.1x).
pub(super) const MIN_RADIUS_MULTIPLIER: f32 = 0.1;
/// Convergence tolerance (0.1% of search range).
pub(super) const TOLERANCE: f32 = 0.001;

// fit dimension labels (used in debug log output of find_constraining_margin)
pub(super) const HORIZONTAL_DIMENSION_LABEL: &str = "horizontal";
pub(super) const VERTICAL_DIMENSION_LABEL: &str = "vertical";

// fit request contexts
pub(super) const ANIMATE_TO_FIT_CONTEXT: &str = "AnimateToFit";
pub(super) const LOOK_AT_AND_ZOOM_TO_FIT_CONTEXT: &str = "LookAtAndZoomToFit";
pub(super) const ZOOM_TO_FIT_CONTEXT: &str = "ZoomToFit";

// look-at fractions
pub(super) const LOOK_AT_AND_ZOOM_TO_FIT_LOOK_FRACTION: f32 = 0.4;

// projection constants
/// Minimum depth for a point to be considered in front of the camera.
/// Points at or below this depth are treated as behind the camera in perspective projection.
pub(super) const MIN_VISIBLE_DEPTH: f32 = 0.1;
