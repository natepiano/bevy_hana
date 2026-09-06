//! Workaround for winit #4445: fix X11 saved-position drift by querying
//! `_NET_FRAME_EXTENTS` and subtracting the title bar height before restore.
//!
//! On X11, winit's `outer_position()` returns the client-area position instead of
//! the frame position, so a save/restore cycle drifts the window down by the title
//! bar height each time. This module queries the X11 frame extents and rewrites
//! `TargetPosition` before `place_window_at_saved_geometry` runs.
//!
//! See: <https://github.com/rust-windowing/winit/issues/4445>

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IVec2;
use bevy::prelude::Query;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
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
use super::TargetPosition;
use super::WindowRestoreAttempt;
use super::X11FrameCompensated;
use super::target_position::WindowSettleProgress;
use crate::constants::FRAME_EXTENT_COUNT;
use crate::constants::FRAME_EXTENT_PROPERTY_OFFSET;
use crate::constants::FRAME_EXTENT_TOP_INDEX;
use crate::constants::FRAME_EXTENTS_ATOM_NAME;
use crate::events::ExpectedPhysicalPosition;

/// The `_NET_FRAME_EXTENTS` top (physical pixels) queried during W6 compensation.
///
/// Recorded so [`reapply_compensated_position`] can compute the mapped-window readback it
/// compares against (`compensated_position + frame_top`) without re-opening an X11 connection
/// every frame. Present only on windows whose position was compensated.
#[derive(Component)]
pub(crate) struct X11FrameTop(i32);

/// Subtract the X11 title bar height from `TargetPosition::placement_decision`.
///
/// Inserts `X11FrameCompensated` once frame extents are available; this gates
/// `place_window_at_saved_geometry`. If `_NET_FRAME_EXTENTS` is not yet set by the WM, returns
/// silently and retries next frame.
pub(crate) fn compensate_target_position(
    mut commands: Commands,
    mut windows: Query<
        (Entity, &mut TargetPosition),
        (With<WindowRestoreAttempt>, Without<X11FrameCompensated>),
    >,
    _: NonSendMarker,
) {
    for (entity, mut target) in &mut windows {
        let ExpectedPhysicalPosition::Specified(physical_position) = target.physical_position()
        else {
            commands.entity(entity).insert(X11FrameCompensated);
            continue;
        };

        let Some(physical_frame_top) = query_frame_top_for_entity(entity) else {
            continue;
        };

        let physical_compensated = IVec2::new(
            physical_position.x,
            physical_position.y - physical_frame_top,
        );
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
pub(crate) fn reapply_compensated_position(
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

fn query_frame_top_for_entity(entity: Entity) -> Option<i32> {
    WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        winit_windows.get_window(entity).and_then(|winit_window| {
            let window_id = get_x11_window_id(&**winit_window)?;
            query_frame_top(window_id)
        })
    })
}

fn query_frame_top(window_id: u32) -> Option<i32> {
    let (conn, _) = XCBConnection::connect(None).ok()?;

    let atom_cookie = conn.intern_atom(false, FRAME_EXTENTS_ATOM_NAME).ok()?;
    let atom = atom_cookie.reply().ok()?.atom;

    let property_cookie = conn
        .get_property(
            false,
            window_id,
            atom,
            AtomEnum::CARDINAL,
            FRAME_EXTENT_PROPERTY_OFFSET,
            FRAME_EXTENT_COUNT,
        )
        .ok()?;
    let property = property_cookie.reply().ok()?;

    let values: Vec<u32> = property.value32()?.collect();
    if values.len() >= FRAME_EXTENT_COUNT.to_usize() {
        Some(values[FRAME_EXTENT_TOP_INDEX].to_i32())
    } else {
        None
    }
}

fn get_x11_window_id<W: HasWindowHandle>(window: &W) -> Option<u32> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window.to_u32()),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}
