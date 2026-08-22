//! Workaround for Windows DPI change bug when dragging between mixed-DPI monitors.
//!
//! On Windows 11 with monitors of different DPI scales, winit's `WM_DPICHANGED`
//! handler can overwrite the `WM_DPICHANGED` suggested rectangle with the old
//! Bevy/winit window position or size when dragged between monitors.
//!
//! This module subclasses the window to intercept `WM_DPICHANGED` and handle it
//! using Microsoft's recommended simple approach: use the suggested `RECT` from `lparam`.
//!
//! See: <https://github.com/rust-windowing/winit/issues/4041>
//!
//! **This workaround can be removed when winit releases a version with the fix
//! from <https://github.com/rust-windowing/winit/pull/4341>**

use std::cell::Cell;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Added;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::With;
use bevy::prelude::debug;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use bevy::winit::WINIT_WINDOWS;
use raw_window_handle::HasWindowHandle;
use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Foundation::LRESULT;
use windows::Win32::Foundation::RECT;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::UI::Shell::DefSubclassProc;
use windows::Win32::UI::Shell::RemoveWindowSubclass;
use windows::Win32::UI::Shell::SetWindowSubclass;
use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
use windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE;
use windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER;
use windows::Win32::UI::WindowsAndMessaging::SetWindowPos;
use windows::Win32::UI::WindowsAndMessaging::WM_DPICHANGED;

use super::ManagedWindow;
use super::constants::DPI_CHANGE_HANDLED_RESULT;
use super::constants::SUBCLASS_ID;
use super::constants::SUBCLASS_REFERENCE_DATA;

/// Wrapper around `HWND` that implements `Send` + `Sync`.
///
/// # Safety
///
/// `HWND` is a handle to a window created on the main thread. Window handles
/// are valid to use from any thread for read-only operations like removing
/// a subclass. The guard only stores the handle and removes the subclass on drop.
struct SendSyncHwnd(HWND);

// SAFETY: `HWND` is just a pointer/handle that can be sent between threads.
// `RemoveWindowSubclass`, `SetWindowSubclass`, and `SetWindowPos` are safe to
// call with the stored handle under this guard's usage.
unsafe impl Send for SendSyncHwnd {}
unsafe impl Sync for SendSyncHwnd {}

/// Guard resource that removes the window subclass on drop.
#[derive(Resource)]
struct DpiFixGuard {
    hwnd: SendSyncHwnd,
}

impl Drop for DpiFixGuard {
    fn drop(&mut self) {
        // SAFETY: `RemoveWindowSubclass` is safe with a valid `HWND` and matching subclass ID.
        let result = unsafe { RemoveWindowSubclass(self.hwnd.0, Some(subclass_proc), SUBCLASS_ID) };
        if result.as_bool() {
            debug!("[windows_dpi_fix] Removed DPI fix subclass");
        }
    }
}

/// Get the `HWND` from a Bevy window entity.
fn get_hwnd(window_entity: Entity) -> Option<HWND> {
    WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        let winit_window = winit_windows.get_window(window_entity)?;
        match winit_window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
            _ => None,
        }
    })
}

thread_local! {
    /// Set while [`handle_dpi_changed`] applies the suggested rect. `SetWindowPos` can
    /// synchronously re-dispatch `WM_DPICHANGED` to the same window when the move crosses a
    /// DPI boundary (as a mixed-DPI monitor-reconnect restore does); the flag lets
    /// [`subclass_proc`] make the re-entrant callback a no-op so it terminates instead of
    /// recursing without a base case and overflowing the stack.
    static APPLYING_SUGGESTED_RECT: Cell<bool> = const { Cell::new(false) };
}

/// Clears [`APPLYING_SUGGESTED_RECT`] on scope exit so a failed `SetWindowPos` cannot leave
/// the re-entrancy guard stuck on.
struct ClearOnDrop;

impl Drop for ClearOnDrop {
    fn drop(&mut self) { APPLYING_SUGGESTED_RECT.set(false); }
}

/// Handle `WM_DPICHANGED` using Microsoft's recommended simple approach.
///
/// The `lparam` contains a pointer to a `RECT` with the suggested new size/position.
/// We simply apply it using `SetWindowPos`.
fn handle_dpi_changed(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    // SAFETY: `lparam` is a valid pointer to `RECT` per the `WM_DPICHANGED` contract.
    let suggested_rect = unsafe { &*(lparam.0 as *const RECT) };

    // Guard the reposition: the `SetWindowPos` below can synchronously re-dispatch
    // `WM_DPICHANGED`, which `subclass_proc` then skips instead of repositioning again.
    APPLYING_SUGGESTED_RECT.set(true);
    let _clear = ClearOnDrop;

    // SAFETY: `SetWindowPos` is safe with a valid `HWND` and dimensions.
    let result = unsafe {
        SetWindowPos(
            hwnd,
            None,
            suggested_rect.left,
            suggested_rect.top,
            suggested_rect.right - suggested_rect.left,
            suggested_rect.bottom - suggested_rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };

    if result.is_err() {
        warn!("[windows_dpi_fix] SetWindowPos failed: {result:?}");
    }

    LRESULT(DPI_CHANGE_HANDLED_RESULT)
}

/// Whether the window is currently shown. A restore hides the window while it moves it across
/// the DPI boundary, so an invisible window here means a restore is in progress rather than a
/// user drag.
fn window_is_visible(hwnd: HWND) -> bool {
    // SAFETY: `IsWindowVisible` is safe to call with a valid `HWND`.
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

/// Subclass window procedure that intercepts `WM_DPICHANGED`.
///
/// # Safety
///
/// This is a Windows callback. It must be called only by Windows with valid parameters.
unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if msg == WM_DPICHANGED {
        if !window_is_visible(hwnd) {
            // A hidden window is the cross-DPI restore moving itself across the DPI boundary.
            // Let winit process `WM_DPICHANGED` so it emits `WindowScaleFactorChanged`, the
            // signal the restore's scale-wait completes on. winit's mixed-DPI repositioning bug
            // is invisible on a hidden window, and the restore's `ApplySize` sets the final
            // geometry — so the drag workaround only needs to guard visible windows.
            debug!("[windows_dpi_fix] Forwarding WM_DPICHANGED to winit for a hidden window");
            // SAFETY: `DefSubclassProc` is safe when called from a subclass proc.
            return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
        }
        if APPLYING_SUGGESTED_RECT.get() {
            // Re-entrant `WM_DPICHANGED` from our own `SetWindowPos`; the window already sits
            // on the suggested rect. Report it handled without repositioning again so the
            // native callback terminates instead of recursing until the stack overflows.
            return LRESULT(DPI_CHANGE_HANDLED_RESULT);
        }
        debug!("[windows_dpi_fix] Intercepted WM_DPICHANGED");
        return handle_dpi_changed(hwnd, lparam);
    }

    // Pass all other messages to the original window procedure.
    // SAFETY: `DefSubclassProc` is safe when called from a subclass proc.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// System to install the DPI fix subclass on the primary window.
pub(crate) fn install_dpi_fix(
    mut commands: Commands,
    window_entity: Single<Entity, With<PrimaryWindow>>,
    _: NonSendMarker,
) {
    let Some(hwnd) = get_hwnd(*window_entity) else {
        warn!("[windows_dpi_fix] Could not get HWND for primary window");
        return;
    };

    // SAFETY: `SetWindowSubclass` is safe with a valid `HWND`.
    let result = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(subclass_proc),
            SUBCLASS_ID,
            SUBCLASS_REFERENCE_DATA,
        )
    };

    if result.as_bool() {
        debug!("[windows_dpi_fix] Installed DPI change workaround");
        commands.insert_resource(DpiFixGuard {
            hwnd: SendSyncHwnd(hwnd),
        });
    } else {
        warn!("[windows_dpi_fix] Failed to install subclass");
    }
}

/// Install DPI fix on newly added `ManagedWindow` entities.
pub(crate) fn install_dpi_fix_on_managed(
    new_windows: Query<Entity, Added<ManagedWindow>>,
    _: NonSendMarker,
) {
    for entity in &new_windows {
        let Some(hwnd) = get_hwnd(entity) else {
            warn!("[windows_dpi_fix] Could not get HWND for managed window {entity:?}");
            continue;
        };

        // SAFETY: `SetWindowSubclass` is safe with a valid `HWND`.
        let result = unsafe {
            SetWindowSubclass(
                hwnd,
                Some(subclass_proc),
                SUBCLASS_ID,
                SUBCLASS_REFERENCE_DATA,
            )
        };

        if result.as_bool() {
            debug!(
                "[windows_dpi_fix] Installed DPI change workaround on managed window {entity:?}"
            );
        } else {
            warn!("[windows_dpi_fix] Failed to install subclass on managed window {entity:?}");
        }
    }
}
