//! Driver-specific window placement established by safe readback.

use bevy::prelude::Component;
use bevy::prelude::IVec2;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use hana_kana::ToI32;
use hana_kana::ToU32;
use hana_rigging::prelude::DeviceKey;

use super::PersistedPosition;
use super::PersistedWindowState;
use super::PersistedWindowTargetV5;
use super::SavedWindowMode;
use crate::Platform;
use crate::monitors::CurrentMonitor;
use crate::monitors::MonitorDescriptor;

/// Meaning of a window position returned by a safe driver readback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum EstablishedWindowPosition {
    /// Logical-pixel offset from the exact display that supplied the readback.
    Restorable { logical_offset: IVec2 },
    /// The compositor does not expose a coordinate Clerestory can safely reapply.
    CompositorControlled,
}

/// Live coordinate produced by rebasing an established window configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestorableWindowPosition {
    /// Both platform and persisted logical coordinate spaces resolved.
    Restorable {
        physical_position: IVec2,
        logical_position:  IVec2,
    },
    /// The compositor remains responsible for placement.
    CompositorControlled,
}

/// Window-driver value retained as the binding's last known good configuration.
///
/// This contains only the configuration a safe readback established. Attempt lifecycle, write
/// eligibility, and role ownership stay in `hana_rigging::Bindings`.
#[derive(Component, Clone, Debug, PartialEq, Reflect)]
#[reflect(Component, PartialEq)]
pub(crate) struct EstablishedWindowPlacement {
    pub(crate) position:          EstablishedWindowPosition,
    pub(crate) logical_size:      UVec2,
    pub(crate) saved_window_mode: SavedWindowMode,
}

impl EstablishedWindowPlacement {
    pub(crate) fn from_readback(
        window: &Window,
        current_monitor: &CurrentMonitor,
        physical_position: Option<IVec2>,
        platform: Platform,
    ) -> Self {
        let monitor_descriptor = current_monitor.descriptor;
        let position = if platform.position_available() {
            physical_position.map_or(
                EstablishedWindowPosition::CompositorControlled,
                |position| {
                    let physical_offset = position - monitor_descriptor.physical_position;
                    EstablishedWindowPosition::Restorable {
                        logical_offset: IVec2::new(
                            (f64::from(physical_offset.x) / monitor_descriptor.scale)
                                .round()
                                .to_i32(),
                            (f64::from(physical_offset.y) / monitor_descriptor.scale)
                                .round()
                                .to_i32(),
                        ),
                    }
                },
            )
        } else {
            EstablishedWindowPosition::CompositorControlled
        };

        Self {
            position,
            logical_size: UVec2::new(
                window.resolution.width().to_u32(),
                window.resolution.height().to_u32(),
            ),
            saved_window_mode: (&current_monitor.effective_window_mode).into(),
        }
    }

    #[must_use]
    pub(crate) fn restorable_position(
        &self,
        live_monitor: &MonitorDescriptor,
    ) -> RestorableWindowPosition {
        match self.position {
            EstablishedWindowPosition::Restorable { logical_offset } => {
                RestorableWindowPosition::Restorable {
                    physical_position: live_monitor.physical_from_logical_offset(logical_offset),
                    logical_position:  live_monitor.logical_from_logical_offset(logical_offset),
                }
            },
            EstablishedWindowPosition::CompositorControlled => {
                RestorableWindowPosition::CompositorControlled
            },
        }
    }

    /// The same placement drawn back inside a monitor it was not captured on.
    ///
    /// A window whose saved display is absent is shown on whatever display it launched on, and a
    /// substitute display is free to be smaller than the one the geometry was measured against.
    /// Reapplied verbatim the window can end up taller than the screen with its title bar out of
    /// reach, so the size is capped to the monitor and the offset is pulled back until the whole
    /// window fits.
    ///
    /// The result is a placement to apply, never one to persist. The record on disk keeps naming
    /// the display the user actually left the window on, so the original geometry returns intact
    /// when that display does.
    #[must_use]
    pub(crate) fn fitted_to(&self, monitor: &MonitorDescriptor) -> Self {
        let monitor_logical_size = monitor.logical_size();
        let logical_size = self.logical_size.min(monitor_logical_size);
        let position = match self.position {
            EstablishedWindowPosition::Restorable { logical_offset } => {
                EstablishedWindowPosition::Restorable {
                    logical_offset: logical_offset.clamp(
                        IVec2::ZERO,
                        (monitor_logical_size - logical_size).as_ivec2(),
                    ),
                }
            },
            EstablishedWindowPosition::CompositorControlled => {
                EstablishedWindowPosition::CompositorControlled
            },
        };

        Self {
            position,
            logical_size,
            saved_window_mode: self.saved_window_mode.clone(),
        }
    }

    pub(crate) fn project(&self, device_key: DeviceKey, app_name: &str) -> PersistedWindowState {
        let position = match self.position {
            EstablishedWindowPosition::Restorable { logical_offset } => {
                PersistedPosition::MonitorOffset(logical_offset)
            },
            EstablishedWindowPosition::CompositorControlled => PersistedPosition::Unpositioned,
        };

        PersistedWindowState {
            target: PersistedWindowTargetV5::Classified(device_key),
            position,
            logical_width: self.logical_size.x,
            logical_height: self.logical_size.y,
            saved_window_mode: self.saved_window_mode.clone(),
            app_name: app_name.to_string(),
        }
    }
}

/// Convert one persisted adapter record into the driver's single configuration type.
///
/// Legacy absolute coordinates become compositor-controlled because their old desktop origin
/// cannot authorize a new monitor-relative position without an exact live target.
impl From<&PersistedWindowState> for EstablishedWindowPlacement {
    fn from(persisted: &PersistedWindowState) -> Self {
        let position = match persisted.position {
            PersistedPosition::MonitorOffset(logical_offset) => {
                EstablishedWindowPosition::Restorable { logical_offset }
            },
            PersistedPosition::Unpositioned | PersistedPosition::Unrebased(_) => {
                EstablishedWindowPosition::CompositorControlled
            },
        };

        Self {
            position,
            logical_size: UVec2::new(persisted.logical_width, persisted.logical_height),
            saved_window_mode: persisted.saved_window_mode.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::window::WindowMode;
    use hana_rigging::prelude::DeviceIdSource;
    use hana_rigging::prelude::DeviceKey;
    use hana_rigging::prelude::DeviceKind;

    use super::*;

    #[test]
    fn established_configuration_projects_only_serialized_fields() {
        let placement = EstablishedWindowPlacement {
            position:          EstablishedWindowPosition::Restorable {
                logical_offset: IVec2::new(20, 40),
            },
            logical_size:      UVec2::new(800, 600),
            saved_window_mode: SavedWindowMode::from(&WindowMode::Windowed),
        };

        let persisted = placement.project(
            DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Synthesized {
                    digest: hana_rigging::prelude::Digest::new(42),
                },
            },
            "clerestory",
        );

        assert_eq!(
            persisted.position,
            PersistedPosition::MonitorOffset(IVec2::new(20, 40))
        );
        assert_eq!(persisted.logical_width, 800);
        assert_eq!(persisted.logical_height, 600);
    }

    #[test]
    fn established_offset_rebases_only_through_the_exact_live_descriptor() {
        let placement = EstablishedWindowPlacement {
            position:          EstablishedWindowPosition::Restorable {
                logical_offset: IVec2::new(20, 40),
            },
            logical_size:      UVec2::new(800, 600),
            saved_window_mode: SavedWindowMode::Windowed,
        };
        let live = MonitorDescriptor::for_current_enumeration(
            7,
            2.0,
            IVec2::new(-2_000, 100),
            UVec2::new(1_920, 1_080),
        );

        assert_eq!(
            placement.restorable_position(&live),
            RestorableWindowPosition::Restorable {
                physical_position: IVec2::new(-1_960, 180),
                logical_position:  IVec2::new(-980, 90),
            }
        );
    }
}
