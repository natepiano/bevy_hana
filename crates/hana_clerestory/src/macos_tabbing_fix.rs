//! Disable macOS window tabbing.
//!
//! With the system preference "Prefer tabs when opening documents" at its default
//! ("In Full Screen"), `AppKit` merges a window opened while another of the app's
//! windows is fullscreen into that window as a TAB instead of placing it on its
//! own monitor. Only the front tab is displayed; the vacated display shows an
//! empty black fullscreen Space.
//!
//! What stops it is the app-wide class property
//! `NSWindow.allowsAutomaticWindowTabbing = false`, set in
//! [`disable_automatic_tabbing`] during plugin build — before winit creates any
//! OS window. Per-window `NSWindow.tabbingMode = .disallowed` (the
//! [`disable_tabbing_on_managed`] system) cannot fix automatic tabbing on its
//! own: the tab merge happens at `AppKit` window-creation time, before any ECS
//! system sees the new window. It is kept on secondary windows to also block
//! MANUAL tabbing (dragging a window onto another's tab bar, "Merge All
//! Windows").

use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Added;
use bevy::prelude::Entity;
use bevy::prelude::NonSendMut;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::Without;
use bevy::prelude::debug;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use bevy::winit::WINIT_WINDOWS;
use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_app_kit::NSWindow;
use objc2_app_kit::NSWindowDidEnterFullScreenNotification;
use objc2_app_kit::NSWindowDidExitFullScreenNotification;
use objc2_app_kit::NSWindowStyleMask;
use objc2_app_kit::NSWindowTabbingMode;
use objc2_foundation::NSNotification;
use objc2_foundation::NSNotificationCenter;
use raw_window_handle::HasWindowHandle;
use raw_window_handle::RawWindowHandle;

use super::ManagedWindow;
use crate::restore::TargetPosition;

/// Native fullscreen transition this module observed complete for one window.
///
/// Declared here rather than reusing `restore`'s `NativeFullscreenState` so the restore enum
/// stays inside `crate::restore`; `application` maps this into it at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeFullscreenObserved {
    Unavailable,
    Windowed,
    Fullscreen,
}

const NATIVE_WINDOWED: u8 = 0;
const NATIVE_FULLSCREEN: u8 = 1;

struct NativeFullscreenObservation {
    center:         Retained<NSNotificationCenter>,
    enter_observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    exit_observer:  Retained<ProtocolObject<dyn NSObjectProtocol>>,
    state:          Arc<AtomicU8>,
}

impl NativeFullscreenObservation {
    fn new(window: &NSWindow) -> Self {
        let initial_state = if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            NATIVE_FULLSCREEN
        } else {
            NATIVE_WINDOWED
        };
        let state = Arc::new(AtomicU8::new(initial_state));
        let center = NSNotificationCenter::defaultCenter();

        let enter_state = Arc::clone(&state);
        let enter_block = RcBlock::new(move |_: NonNull<NSNotification>| {
            enter_state.store(NATIVE_FULLSCREEN, Ordering::Release);
        });
        // SAFETY: The notification is filtered to this live `NSWindow`. The
        // block captures only an `Arc<AtomicU8>`, so it is safe if AppKit
        // invokes it from any thread.
        let enter_observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWindowDidEnterFullScreenNotification),
                Some(window),
                None,
                &enter_block,
            )
        };

        let exit_state = Arc::clone(&state);
        let exit_block = RcBlock::new(move |_: NonNull<NSNotification>| {
            exit_state.store(NATIVE_WINDOWED, Ordering::Release);
        });
        // SAFETY: Same as the enter observer above.
        let exit_observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWindowDidExitFullScreenNotification),
                Some(window),
                None,
                &exit_block,
            )
        };

        Self {
            center,
            enter_observer,
            exit_observer,
            state,
        }
    }

    fn state(&self) -> NativeFullscreenObserved {
        match self.state.load(Ordering::Acquire) {
            NATIVE_FULLSCREEN => NativeFullscreenObserved::Fullscreen,
            _ => NativeFullscreenObserved::Windowed,
        }
    }
}

impl Drop for NativeFullscreenObservation {
    fn drop(&mut self) {
        // SAFETY: Both values are observer tokens returned by this notification
        // center and remain alive for the duration of these calls.
        unsafe {
            self.center.removeObserver(self.enter_observer.as_ref());
            self.center.removeObserver(self.exit_observer.as_ref());
        }
    }
}

/// Tracks completed native fullscreen transitions while a restore is active.
#[derive(Default)]
pub(crate) struct NativeFullscreenObservations {
    entries: HashMap<Entity, NativeFullscreenObservation>,
}

impl NativeFullscreenObservations {
    /// Start observing before Clerestory asks `AppKit` to change fullscreen state,
    /// then return the last transition that `AppKit` confirmed as complete.
    pub(crate) fn observe(&mut self, entity: Entity) -> NativeFullscreenObserved {
        if let Some(observation) = self.entries.get(&entity) {
            return observation.state();
        }
        let Some(window) = get_ns_window(entity) else {
            return NativeFullscreenObserved::Unavailable;
        };
        let observation = NativeFullscreenObservation::new(&window);
        let state = observation.state();
        self.entries.insert(entity, observation);
        state
    }

    pub(crate) fn stop(&mut self, entity: Entity) { self.entries.remove(&entity); }
}

/// Opt out of macOS automatic window tabbing for the whole app.
///
/// `NSWindow.allowsAutomaticWindowTabbing` is a class property — one
/// process-wide switch, not per-window state. It must be set before any OS
/// window exists; plugin `build()` runs during `add_plugins`, ahead of winit's
/// window creation at event-loop start.
pub(crate) fn disable_automatic_tabbing() {
    let Some(main_thread) = MainThreadMarker::new() else {
        warn!("[macos_tabbing_fix] Not on the main thread; automatic tabbing stays enabled");
        return;
    };
    NSWindow::setAllowsAutomaticWindowTabbing(false, main_thread);
    debug!("[macos_tabbing_fix] Disabled automatic window tabbing app-wide");
}

/// Get the `NSWindow` for a Bevy window entity.
fn get_ns_window(entity: Entity) -> Option<Retained<NSWindow>> {
    WINIT_WINDOWS.with(|winit_windows| {
        let winit_windows = winit_windows.borrow();
        let winit_window = winit_windows.get_window(entity)?;
        let handle = winit_window.window_handle().ok()?;
        let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: `ns_view` is a valid `NSView` pointer from winit's window handle.
        let ns_view: &NSView = unsafe { appkit_handle.ns_view.cast().as_ref() };
        ns_view.window()
    })
}

/// Match winit's startup fullscreen sequence by making the window key after
/// its runtime fullscreen request has reached `AppKit`.
pub(crate) fn activate_fullscreen_window(entity: Entity) {
    let Some(window) = get_ns_window(entity) else {
        debug!("[macos_tabbing_fix] Could not activate fullscreen window {entity:?}");
        return;
    };
    let was_key = window.isKeyWindow();
    window.makeKeyAndOrderFront(None);
    debug!(
        "[macos_tabbing_fix] Made fullscreen window {entity:?} key (was_key={was_key}, is_key={})",
        window.isKeyWindow()
    );
}

/// Disable manual tabbing on each newly managed secondary window.
pub(crate) fn disable_tabbing_on_managed(
    new_windows: Query<Entity, (Added<ManagedWindow>, Without<PrimaryWindow>)>,
    _: NonSendMarker,
) {
    for entity in &new_windows {
        let Some(ns_window) = get_ns_window(entity) else {
            debug!("[macos_tabbing_fix] Could not get NSWindow for managed window {entity:?}");
            continue;
        };

        ns_window.setTabbingMode(NSWindowTabbingMode::Disallowed);
        debug!("[macos_tabbing_fix] Disabled tabbing on managed window {entity:?}");
    }
}

/// Stop observing if a restore is cancelled before normal completion.
pub(crate) fn clear_fullscreen_observation(
    removed: On<Remove, TargetPosition>,
    mut observations: NonSendMut<NativeFullscreenObservations>,
) {
    observations.stop(removed.entity);
}
