//! Screen-space overlay constants.

// screen-space resizing
/// Hysteresis tolerance for screen-space resize writeback.
pub(super) const SCREEN_SPACE_PANEL_RESIZE_EPSILON: f32 = 0.01;

// screen-space view
/// Far plane for the shared screen-space orthographic camera.
pub(super) const SCREEN_SPACE_CAMERA_FAR: f32 = 2000.0;
/// Z position for the shared screen-space orthographic camera.
pub(super) const SCREEN_SPACE_CAMERA_Z: f32 = 1000.0;
/// Illuminance for the shared screen-space directional light.
pub(super) const SCREEN_SPACE_LIGHT_ILLUMINANCE: f32 = 5000.0;
/// First render layer reserved for camera-specific screen-space views. Layers
/// below this remain available for authored panel and scene isolation.
pub(super) const FIRST_SCREEN_SPACE_VIEW_RENDER_LAYER: usize = 32;
