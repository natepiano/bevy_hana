#[cfg(test)]
use std::collections::HashMap;

use bevy::prelude::Component;
use bevy::prelude::Entity;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy::prelude::UVec2;
use bevy::winit::WINIT_WINDOWS;

/// Native measurements for the exact Bevy window entity being prepared.
pub(super) struct NativeWindowInfo {
    physical_decoration: UVec2,
}

impl NativeWindowInfo {
    #[must_use]
    pub(super) const fn physical_decoration(&self) -> UVec2 { self.physical_decoration }
}

#[cfg(test)]
#[derive(Default, Resource)]
pub(crate) struct InjectedWinitWindows {
    physical_decorations: HashMap<Entity, UVec2>,
}

#[cfg(test)]
impl InjectedWinitWindows {
    pub(crate) fn insert(&mut self, entity: Entity, physical_decoration: UVec2) {
        self.physical_decorations
            .insert(entity, physical_decoration);
    }

    fn info(&self, entity: Entity) -> Option<NativeWindowInfo> {
        self.physical_decorations
            .get(&entity)
            .copied()
            .map(|physical_decoration| NativeWindowInfo {
                physical_decoration,
            })
    }
}

#[cfg(test)]
impl Extend<Entity> for InjectedWinitWindows {
    fn extend<T: IntoIterator<Item = Entity>>(&mut self, entities: T) {
        self.physical_decorations
            .extend(entities.into_iter().map(|entity| (entity, UVec2::ZERO)));
    }
}

/// Read winit data for `entity`, returning `None` until that exact native window exists.
pub(super) fn native_window_info(
    entity: Entity,
    #[cfg(test)] injected_windows: Option<&InjectedWinitWindows>,
) -> Option<NativeWindowInfo> {
    #[cfg(test)]
    if let Some(info) = injected_windows.and_then(|windows| windows.info(entity)) {
        return Some(info);
    }

    WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        let winit_window = winit_windows.get_window(entity)?;
        let physical_outer_size = winit_window.outer_size();
        let physical_inner_size = winit_window.inner_size();
        Some(NativeWindowInfo {
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

pub(super) fn native_window_exists(
    entity: Entity,
    #[cfg(test)] injected_windows: Option<&InjectedWinitWindows>,
) -> bool {
    native_window_info(
        entity,
        #[cfg(test)]
        injected_windows,
    )
    .is_some()
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
