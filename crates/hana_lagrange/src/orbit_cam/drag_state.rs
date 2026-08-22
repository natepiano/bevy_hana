use bevy::prelude::Component;

/// Internal per-camera state used to keep orbit direction stable during a drag.
#[derive(Component, Default, Copy, Clone, Debug, PartialEq, Eq)]
pub(super) struct OrbitDragState {
    pub(super) orientation: CameraOrientation,
    pub(super) orbit_drag:  DragActivity,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DragActivity {
    Active,
    #[default]
    Idle,
}

impl From<bool> for DragActivity {
    fn from(active: bool) -> Self { if active { Self::Active } else { Self::Idle } }
}

/// Whether the camera was latched as upside down when orbit dragging started.
#[derive(Clone, PartialEq, Eq, Debug, Copy, Default)]
pub(super) enum CameraOrientation {
    #[default]
    Normal,
    UpsideDown,
}
