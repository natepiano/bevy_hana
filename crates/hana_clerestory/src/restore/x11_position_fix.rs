//! Workaround for winit #4445: fix X11 saved-position drift by querying
//! `_NET_FRAME_EXTENTS` and subtracting the title bar height before restore.
//!
//! On X11, winit's `outer_position()` returns the client-area position instead of
//! the frame position, so a save/restore cycle drifts the window down by the title
//! bar height each time. This module queries the X11 frame extents and rewrites
//! `TargetPosition` before `place_window_at_saved_geometry` runs.
//!
//! See: <https://github.com/rust-windowing/winit/issues/4445>

#[cfg(test)]
use std::collections::HashMap;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::App;
use bevy::prelude::ApplyDeferred;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IVec2;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Query;
use bevy::prelude::Res;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy::prelude::Update;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::World;
use bevy::prelude::debug;
use bevy::prelude::info;
use bevy::window::WindowPosition;
use bevy::winit::WINIT_WINDOWS;
use hana_kana::ToI32;
use hana_kana::ToU32;
use hana_kana::ToUsize;
use raw_window_handle::HasWindowHandle;
use raw_window_handle::RawWindowHandle;
use x11rb::protocol::xproto::AtomEnum;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::xcb_ffi::XCBConnection;

use super::MonitorScaleStrategy;
use super::WindowRestoreAttempt;
use super::check_restore_settling;
use super::place_window_at_saved_geometry;
use super::prepare_driver_restore_targets;
use super::target_position::NativeWindowReadbackPosition;
use super::target_position::TargetPosition;
use super::target_position::WindowSettleProgress;
use super::winit_info::X11FrameCompensated;
use crate::ClerestoryWindowDriverSet;
use crate::Platform;
use crate::constants::FRAME_EXTENT_COUNT;
use crate::constants::FRAME_EXTENT_PROPERTY_OFFSET;
use crate::constants::FRAME_EXTENT_TOP_INDEX;
use crate::constants::FRAME_EXTENTS_ATOM_NAME;
use crate::events::ExpectedPhysicalPosition;
use crate::persistence::ReadbackWindowPosition;
use crate::persistence::WindowPlacementConfiguration;

/// Native readiness for converting an X11 readback coordinate into a frame request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum X11FrameReadback {
    Available { physical_top: i32 },
    AwaitingNativeWindow,
    AwaitingFrameExtents,
    NativeHandleUnavailable,
    NotX11,
    QueryFailed,
}

/// Why an exact windowed return still needs another native observation.
pub(crate) enum X11WindowedPositionProgress {
    Restored,
    UncompensatedFallback,
    AwaitingNativeExit,
    AwaitingFrameReadback,
    AwaitingPositionReadback,
}

impl X11WindowedPositionProgress {
    pub(crate) const fn is_complete(&self) -> bool { matches!(self, Self::Restored) }

    pub(crate) const fn requires_fallback(&self) -> bool {
        matches!(self, Self::UncompensatedFallback)
    }
}

#[cfg(test)]
#[derive(Resource, Default)]
struct InjectedX11FrameTops(HashMap<Entity, X11FrameReadback>);

impl WindowPlacementConfiguration {
    /// Complete only after the window manager reports the retained physical coordinate.
    pub(crate) fn restore_x11_windowed_position(
        world: &mut World,
        entity: Entity,
        position: NativeWindowReadbackPosition,
    ) -> X11WindowedPositionProgress {
        let physical_observed = position.physical_pixels();
        if WINIT_WINDOWS.with_borrow(|windows| {
            windows
                .get_window(entity)
                .is_some_and(|window| window.fullscreen().is_some())
        }) {
            return X11WindowedPositionProgress::AwaitingNativeExit;
        }
        let readback = query_frame_top_for_entity(
            entity,
            #[cfg(test)]
            world.get_resource::<InjectedX11FrameTops>(),
        );
        let X11FrameReadback::Available { physical_top } = readback else {
            // There is no native request to compensate before winit creates the window.
            // Keep the initial geometry in ECS, but wait for native readback before completion.
            if readback == X11FrameReadback::AwaitingNativeWindow
                && let Some(mut window) = world.get_mut::<Window>(entity)
            {
                window.position = WindowPosition::At(physical_observed);
            }
            return match readback {
                X11FrameReadback::AwaitingNativeWindow | X11FrameReadback::AwaitingFrameExtents => {
                    X11WindowedPositionProgress::AwaitingFrameReadback
                },
                _ => X11WindowedPositionProgress::UncompensatedFallback,
            };
        };
        let observed = WINIT_WINDOWS.with_borrow(|windows| {
            ReadbackWindowPosition::from(
                windows
                    .get_window(entity)
                    .and_then(|window| window.outer_position().ok())
                    .map(|position| IVec2::new(position.x, position.y)),
            )
        });
        #[cfg(test)]
        let observed = if world
            .get_resource::<InjectedX11FrameTops>()
            .is_some_and(|injected| injected.0.contains_key(&entity))
        {
            match world.get::<Window>(entity).map(|window| window.position) {
                Some(WindowPosition::At(position)) => ReadbackWindowPosition::Observed(position),
                _ => ReadbackWindowPosition::Unavailable,
            }
        } else {
            observed
        };
        if observed == ReadbackWindowPosition::Observed(physical_observed) {
            return if world
                .get::<Window>(entity)
                .is_some_and(|window| window.position == WindowPosition::At(physical_observed))
            {
                X11WindowedPositionProgress::Restored
            } else {
                X11WindowedPositionProgress::AwaitingPositionReadback
            };
        }
        if let Some(mut window) = world.get_mut::<Window>(entity) {
            window.position =
                WindowPosition::At(position.x11_frame_request(physical_top).physical_pixels());
        }
        X11WindowedPositionProgress::AwaitingPositionReadback
    }

    #[cfg(test)]
    pub(crate) fn observe_x11_frame_top_for_test(
        world: &mut World,
        entity: Entity,
        physical_top: i32,
    ) {
        world.init_resource::<InjectedX11FrameTops>();
        world
            .resource_mut::<InjectedX11FrameTops>()
            .0
            .insert(entity, X11FrameReadback::Available { physical_top });
    }

    #[cfg(test)]
    pub(crate) fn await_x11_frame_extents_for_test(world: &mut World, entity: Entity) {
        world.init_resource::<InjectedX11FrameTops>();
        world
            .resource_mut::<InjectedX11FrameTops>()
            .0
            .insert(entity, X11FrameReadback::AwaitingFrameExtents);
    }

    #[cfg(test)]
    pub(crate) fn fail_x11_frame_query_for_test(world: &mut World, entity: Entity) {
        world.init_resource::<InjectedX11FrameTops>();
        world
            .resource_mut::<InjectedX11FrameTops>()
            .0
            .insert(entity, X11FrameReadback::QueryFailed);
    }
}

/// Compensate each restore target for the X11 frame before placement, then re-apply it once the
/// window is mapped. Runs only on X11.
pub(super) fn register(app: &mut App) {
    app.add_systems(
        Update,
        (
            compensate_target_position
                .after(prepare_driver_restore_targets)
                .before(place_window_at_saved_geometry)
                .in_set(ClerestoryWindowDriverSet::TargetPreparation),
            ApplyDeferred
                .after(compensate_target_position)
                .in_set(ClerestoryWindowDriverSet::TargetPreparation),
            // Re-apply the compensated position once the window is mapped: bevy 0.19
            // can ignore the first `set_outer_position` request while the X11 window is
            // unmapped, while a mapped window's `Window.position` readback matches the
            // requested compensated position plus `X11FrameTop`.
            reapply_compensated_position
                .after(place_window_at_saved_geometry)
                .before(check_restore_settling)
                .in_set(ClerestoryWindowDriverSet::TargetPreparation),
        )
            .run_if(|p: Res<Platform>| p.is_x11()),
    );
}

/// The `_NET_FRAME_EXTENTS` top (physical pixels) queried during W6 compensation.
///
/// Recorded so [`reapply_compensated_position`] can compute the mapped-window readback it
/// compares against (`compensated_position + frame_top`) without re-opening an X11 connection
/// every frame. Present only on windows whose position was compensated.
#[derive(Component)]
struct X11FrameTop(i32);

/// Subtract the X11 title bar height from `TargetPosition::placement_decision`.
///
/// Inserts `X11FrameCompensated` once frame extents are available; this gates
/// `place_window_at_saved_geometry`. If `_NET_FRAME_EXTENTS` is not yet set by the WM, returns
/// silently and retries next frame.
fn compensate_target_position(
    mut commands: Commands,
    mut windows: Query<
        (Entity, &mut TargetPosition),
        (With<WindowRestoreAttempt>, Without<X11FrameCompensated>),
    >,
    _: NonSendMarker,
    #[cfg(test)] injected: Option<Res<InjectedX11FrameTops>>,
) {
    for (entity, mut target) in &mut windows {
        let ExpectedPhysicalPosition::Specified(physical_position) = target.physical_position()
        else {
            commands.entity(entity).insert(X11FrameCompensated);
            continue;
        };

        let X11FrameReadback::Available {
            physical_top: physical_frame_top,
        } = query_frame_top_for_entity(
            entity,
            #[cfg(test)]
            injected.as_deref(),
        )
        else {
            continue;
        };

        let physical_compensated =
            NativeWindowReadbackPosition::from_physical_pixels(physical_position)
                .x11_frame_request(physical_frame_top)
                .physical_pixels();
        info!(
            "[W6] Compensating position: {physical_position:?} -> {physical_compensated:?} (physical_frame_top={physical_frame_top})"
        );
        target.compensate_physical_position(physical_frame_top);
        commands
            .entity(entity)
            .insert((X11FrameCompensated, X11FrameTop(physical_frame_top)));
    }
}

/// Re-issue the W6-compensated position once the window is mapped.
///
/// During the initial restore the X11 WM can set a freshly-mapped window's `y`
/// from `_NET_FRAME_EXTENTS` and client-area coordinates, ignoring the requested
/// position from winit #4445. Under bevy 0.19 the same requested `y` reads back
/// as a fixed value until mapping completes. Once the window is mapped,
/// `set_outer_position` readback equals the requested position plus a stable
/// `frame_top`. This system watches the settling
/// window and, while the readback hasn't reached `compensated + frame_top`, re-issues the
/// compensated position so the mapped window converges on the saved position.
///
/// Same-scale (`ApplyUnchanged`) windowed restores only — cross-DPI strategies drive
/// position through their own multi-phase move and tolerate the W6 offset.
fn reapply_compensated_position(
    mut windows: Query<(&TargetPosition, &X11FrameTop, &mut Window), With<WindowRestoreAttempt>>,
) {
    for (target_position, physical_frame_top, mut window) in &mut windows {
        if !matches!(
            &target_position.window_settle_progress,
            WindowSettleProgress::Settling(_)
        ) {
            continue;
        }
        if !matches!(
            target_position.monitor_scale_strategy,
            MonitorScaleStrategy::ApplyUnchanged
        ) {
            continue;
        }
        let ExpectedPhysicalPosition::Specified(physical_compensated) =
            target_position.physical_position()
        else {
            continue;
        };
        let WindowPosition::At(physical_actual) = window.position else {
            continue;
        };
        let physical_expected = IVec2::new(
            physical_compensated.x,
            physical_compensated.y + physical_frame_top.0,
        );
        if physical_actual != physical_expected {
            debug!(
                "[W6] Re-applying compensated position {physical_compensated:?}: \
                 actual {physical_actual:?} != expected {physical_expected:?} (mapped window)"
            );
            window.position = WindowPosition::At(physical_compensated);
        }
    }
}

fn query_frame_top_for_entity(
    entity: Entity,
    #[cfg(test)] injected: Option<&InjectedX11FrameTops>,
) -> X11FrameReadback {
    #[cfg(test)]
    if let Some(readback) = injected.and_then(|injected| injected.0.get(&entity)) {
        return *readback;
    }
    WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        let Some(window) = winit_windows.get_window(entity) else {
            return X11FrameReadback::AwaitingNativeWindow;
        };
        match get_x11_window_id(&**window) {
            Ok(window_id) => query_frame_top(window_id),
            Err(readback) => readback,
        }
    })
}

fn query_frame_top(window_id: u32) -> X11FrameReadback {
    match read_frame_top(window_id) {
        Ok(readback) => readback,
        Err(X11FrameQueryFailure) => X11FrameReadback::QueryFailed,
    }
}

struct X11FrameQueryFailure;

fn read_frame_top(window_id: u32) -> Result<X11FrameReadback, X11FrameQueryFailure> {
    let (conn, _) = XCBConnection::connect(None).map_err(|_| X11FrameQueryFailure)?;

    let atom_cookie = conn
        .intern_atom(false, FRAME_EXTENTS_ATOM_NAME)
        .map_err(|_| X11FrameQueryFailure)?;
    let atom = atom_cookie.reply().map_err(|_| X11FrameQueryFailure)?.atom;

    let property_cookie = conn
        .get_property(
            false,
            window_id,
            atom,
            AtomEnum::CARDINAL,
            FRAME_EXTENT_PROPERTY_OFFSET,
            FRAME_EXTENT_COUNT,
        )
        .map_err(|_| X11FrameQueryFailure)?;
    let property = property_cookie.reply().map_err(|_| X11FrameQueryFailure)?;

    let Some(values) = property.value32() else {
        return Ok(X11FrameReadback::AwaitingFrameExtents);
    };
    let values: Vec<u32> = values.collect();
    if values.len() >= FRAME_EXTENT_COUNT.to_usize() {
        Ok(X11FrameReadback::Available {
            physical_top: values[FRAME_EXTENT_TOP_INDEX].to_i32(),
        })
    } else {
        Ok(X11FrameReadback::AwaitingFrameExtents)
    }
}

fn get_x11_window_id<W: HasWindowHandle>(window: &W) -> Result<u32, X11FrameReadback> {
    let handle = window
        .window_handle()
        .map_err(|_| X11FrameReadback::NativeHandleUnavailable)?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(h) => Ok(h.window.to_u32()),
        RawWindowHandle::Xcb(h) => Ok(h.window.get()),
        _ => Err(X11FrameReadback::NotX11),
    }
}
