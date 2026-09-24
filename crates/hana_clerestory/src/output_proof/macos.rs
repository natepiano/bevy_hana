//! `AppKit` backend: reads a native window's screen association and occlusion state.

use bevy::prelude::Entity;
use bevy::winit::WINIT_WINDOWS;
use hana_rigging::prelude::ConfirmationSignal;
use objc2_app_kit::NSView;
use objc2_app_kit::NSWindowOcclusionState;
use raw_window_handle::HasWindowHandle;
use raw_window_handle::RawWindowHandle;

use super::WindowOnScreenObservation;
use super::WindowOnScreenReading;

/// `AppKit` reports every native window's screen association and occlusion state.
pub(super) const SIGNAL: ConfirmationSignal = ConfirmationSignal::Provided;

/// Read `AppKit`'s screen association and occlusion state for one exact Bevy window entity.
pub(super) fn observe_native_window_on_screen(window: Entity) -> WindowOnScreenObservation {
    WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        let Some(winit_window) = winit_windows.get_window(window) else {
            return WindowOnScreenObservation::NoNativeWindowYet;
        };
        let Ok(handle) = winit_window.window_handle() else {
            return WindowOnScreenObservation::NoNativeWindowYet;
        };
        let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() else {
            return WindowOnScreenObservation::NoNativeWindowYet;
        };
        // SAFETY: `ns_view` is a valid `NSView` pointer from winit's window handle.
        let ns_view: &NSView = unsafe { appkit_handle.ns_view.cast().as_ref() };
        let Some(ns_window) = ns_view.window() else {
            return WindowOnScreenObservation::NoNativeWindowYet;
        };
        let reading = if ns_window.screen().is_none() {
            WindowOnScreenReading::OnNoScreen
        } else if ns_window
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible)
        {
            WindowOnScreenReading::ReportedVisible
        } else {
            WindowOnScreenReading::ReportedOccluded
        };
        WindowOnScreenObservation::Reading(reading)
    })
}
