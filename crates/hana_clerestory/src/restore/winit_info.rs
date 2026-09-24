#[cfg(any(test, feature = "test"))]
use std::collections::HashMap;

use bevy::prelude::Component;
use bevy::prelude::Entity;
#[cfg(any(test, feature = "test"))]
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
#[cfg(feature = "test")]
use bevy::prelude::World;
use bevy::winit::WINIT_WINDOWS;
use bevy::winit::WinitMonitors;

use crate::monitors::Monitors;

/// Exact monitor evidence from the native window, independently of ECS synchronization.
pub(super) enum NativeMonitorAssociation {
    Observed(Entity),
    AwaitingObservation,
    AwaitingTopology,
}

/// Native measurements for the exact Bevy window entity being prepared.
pub(super) struct NativeWindowFrame {
    physical_decoration: UVec2,
}

impl NativeWindowFrame {
    #[must_use]
    pub(super) const fn physical_decoration(&self) -> UVec2 { self.physical_decoration }
}

/// Whether the native window backing one Bevy window entity can yet be measured.
///
/// Restore preparation runs from the frame the entity is spawned, but winit creates the native
/// window later. Absence here is that gap, not a missing measurement: nothing about the window
/// has been decided, so preparation waits rather than deriving geometry from a default.
pub(super) enum NativeWindowInfo {
    /// The native window exists and reported its frame.
    Measured(NativeWindowFrame),
    /// The native window does not exist yet, so no measurement is available.
    AwaitingNativeWindow,
}

#[cfg(any(test, feature = "test"))]
#[derive(Default, Resource)]
pub(crate) struct InjectedWinitWindows {
    physical_decorations: HashMap<Entity, UVec2>,
    current_monitors:     HashMap<Entity, Entity>,
}

#[cfg(any(test, feature = "test"))]
impl InjectedWinitWindows {
    pub(crate) fn insert(&mut self, entity: Entity, physical_decoration: UVec2) {
        self.physical_decorations
            .insert(entity, physical_decoration);
    }

    fn info(&self, entity: Entity) -> NativeWindowInfo {
        self.physical_decorations.get(&entity).copied().map_or(
            NativeWindowInfo::AwaitingNativeWindow,
            |physical_decoration| {
                NativeWindowInfo::Measured(NativeWindowFrame {
                    physical_decoration,
                })
            },
        )
    }
}

#[cfg(feature = "test")]
impl crate::WindowManagerPlugin {
    /// Supply native presence and decoration measurements for one exact window in a test.
    ///
    /// The caller supplies `OnMonitor` and observed `Window` geometry. Placement preparation,
    /// application, settling, and kernel completion still run through the production systems.
    pub fn observe_native_window_for_test(
        world: &mut World,
        window: Entity,
        physical_decoration: UVec2,
    ) {
        world.init_resource::<InjectedWinitWindows>();
        world
            .resource_mut::<InjectedWinitWindows>()
            .insert(window, physical_decoration);
    }

    /// Supply the exact native monitor observation without installing an ECS `OnMonitor`.
    ///
    /// This models a compositor reporting a window's monitor after Bevy last synchronized
    /// the window. The production restore must resolve that observation before placement.
    pub fn observe_native_window_on_monitor_for_test(
        world: &mut World,
        window: Entity,
        monitor: Entity,
        physical_decoration: UVec2,
    ) {
        Self::observe_native_window_for_test(world, window, physical_decoration);
        world
            .resource_mut::<InjectedWinitWindows>()
            .current_monitors
            .insert(window, monitor);
    }
}

#[cfg(test)]
impl Extend<Entity> for InjectedWinitWindows {
    fn extend<T: IntoIterator<Item = Entity>>(&mut self, entities: T) {
        self.physical_decorations
            .extend(entities.into_iter().map(|entity| (entity, UVec2::ZERO)));
    }
}

/// Read winit data for `entity`, awaiting the native window until that exact one exists.
pub(super) fn native_window_info(
    entity: Entity,
    #[cfg(any(test, feature = "test"))] injected_windows: Option<&InjectedWinitWindows>,
) -> NativeWindowInfo {
    #[cfg(any(test, feature = "test"))]
    if let Some(windows) = injected_windows
        && let NativeWindowInfo::Measured(frame) = windows.info(entity)
    {
        return NativeWindowInfo::Measured(frame);
    }

    WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        let Some(winit_window) = winit_windows.get_window(entity) else {
            return NativeWindowInfo::AwaitingNativeWindow;
        };
        let physical_outer_size = winit_window.outer_size();
        let physical_inner_size = winit_window.inner_size();
        NativeWindowInfo::Measured(NativeWindowFrame {
            physical_decoration: UVec2::new(
                physical_outer_size
                    .width
                    .saturating_sub(physical_inner_size.width),
                physical_outer_size
                    .height
                    .saturating_sub(physical_inner_size.height),
            ),
        })
    })
}

/// Resolve the native window's current monitor by handle equality within the installed topology.
///
/// Wayland's surface-enter notification updates winit's current monitor without a window event.
/// Bevy synchronizes `OnMonitor` only for changed `Window` components, so restore must read this
/// evidence even when the ECS association has not arrived or still names the previous monitor.
pub(super) fn native_monitor_association(
    entity: Entity,
    monitors: &Monitors,
    winit_monitors: &WinitMonitors,
    #[cfg(any(test, feature = "test"))] injected_windows: Option<&InjectedWinitWindows>,
) -> NativeMonitorAssociation {
    #[cfg(any(test, feature = "test"))]
    if let Some(windows) = injected_windows
        && windows.physical_decorations.contains_key(&entity)
    {
        return windows.current_monitors.get(&entity).map_or(
            NativeMonitorAssociation::AwaitingObservation,
            |observed| {
                if monitors.iter().any(|monitor| monitor.entity == *observed) {
                    NativeMonitorAssociation::Observed(*observed)
                } else {
                    NativeMonitorAssociation::AwaitingTopology
                }
            },
        );
    }

    WINIT_WINDOWS.with_borrow(|windows| {
        let Some(observed) = windows
            .get_window(entity)
            .and_then(|window| window.current_monitor())
        else {
            return NativeMonitorAssociation::AwaitingObservation;
        };
        monitors
            .iter()
            .find(|monitor| {
                winit_monitors
                    .find_entity(monitor.entity)
                    .is_some_and(|handle| handle == observed)
            })
            .map_or(NativeMonitorAssociation::AwaitingTopology, |monitor| {
                NativeMonitorAssociation::Observed(monitor.entity)
            })
    })
}

pub(super) fn native_window_exists(
    entity: Entity,
    #[cfg(any(test, feature = "test"))] injected_windows: Option<&InjectedWinitWindows>,
) -> bool {
    matches!(
        native_window_info(
            entity,
            #[cfg(any(test, feature = "test"))]
            injected_windows,
        ),
        NativeWindowInfo::Measured(_)
    )
}

/// Token indicating X11 frame extent compensation is complete (W6 workaround).
///
/// This component gates `place_window_at_saved_geometry` - the placement system cannot process
/// a window until this token exists on the entity. A windowed restore on Linux X11 with the
/// W6 workaround enabled receives the token from `compensate_target_position`, once
/// `_NET_FRAME_EXTENTS` yields the title bar height to subtract from the saved position.
/// A fullscreen restore has no title bar to subtract, and every other platform reports frame
/// coordinates already, so driver target preparation inserts the token directly for both.
/// [`Platform::awaits_frame_compensation`](crate::Platform) draws that line.
#[derive(Component)]
pub(crate) struct X11FrameCompensated;
