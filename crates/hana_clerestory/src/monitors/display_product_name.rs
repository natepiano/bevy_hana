//! Operating-system display product names.

use bevy::prelude::Reflect;
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSScreen;
#[cfg(target_os = "macos")]
use objc2_foundation::NSNumber;
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
use winit::monitor::MonitorHandle;
#[cfg(target_os = "macos")]
use winit::platform::macos::MonitorHandleExtMacOS;

use crate::Platform;

/// The product name the operating system shows the user for a display.
///
/// This presentation metadata is not device identity evidence and must not be used to derive a
/// durable display key.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
#[type_path = "hana_clerestory::monitors"]
pub enum DisplayProductName {
    /// The operating system supplied a product name for this display.
    Reported(String),
    /// This platform supports product names but supplied none for this display.
    NotReported,
    /// This platform has no operating-system concept of a display product name.
    PlatformHasNoConcept,
}

impl DisplayProductName {
    /// Return the operating-system product name when it was reported.
    #[must_use]
    pub fn as_reported(&self) -> Option<&str> {
        match self {
            Self::Reported(name) => Some(name),
            Self::NotReported | Self::PlatformHasNoConcept => None,
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn from_platform(
    monitor_handle: Option<&MonitorHandle>,
    winit_name: Option<&str>,
    platform: Platform,
) -> DisplayProductName {
    let _ = (winit_name, platform);
    macos_product_name(monitor_handle)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn from_platform(
    monitor_handle: Option<&MonitorHandle>,
    winit_name: Option<&str>,
    platform: Platform,
) -> DisplayProductName {
    let _ = monitor_handle;
    match platform {
        Platform::Windows | Platform::X11 => winit_name.map_or_else(
            || DisplayProductName::NotReported,
            |name| DisplayProductName::Reported(name.to_owned()),
        ),
        Platform::MacOs | Platform::Wayland => DisplayProductName::PlatformHasNoConcept,
    }
}

/// Resolve one winit monitor's product name from the matching `AppKit` screen.
#[cfg(target_os = "macos")]
fn macos_product_name(monitor_handle: Option<&MonitorHandle>) -> DisplayProductName {
    let Some(monitor_handle) = monitor_handle else {
        return DisplayProductName::NotReported;
    };
    let Some(main_thread) = MainThreadMarker::new() else {
        return DisplayProductName::NotReported;
    };
    let screen_number_key = NSString::from_str("NSScreenNumber");
    let display_id = monitor_handle.native_id();
    NSScreen::screens(main_thread)
        .iter()
        .find(|screen| {
            screen
                .deviceDescription()
                .objectForKey(&screen_number_key)
                .is_some_and(|screen_number| {
                    screen_number
                        .downcast_ref::<NSNumber>()
                        .is_some_and(|screen_number| screen_number.unsignedIntValue() == display_id)
                })
        })
        .map_or(DisplayProductName::NotReported, |screen| {
            DisplayProductName::Reported(screen.localizedName().to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::DisplayProductName;

    #[test]
    fn reported_product_name_exposes_its_rendering_value() {
        let product_name = DisplayProductName::Reported(String::from("DELL S3425DW"));

        assert_eq!(product_name.as_reported(), Some("DELL S3425DW"));
    }

    #[test]
    fn absent_product_names_expose_no_rendering_value() {
        assert_eq!(DisplayProductName::PlatformHasNoConcept.as_reported(), None);
        assert_eq!(DisplayProductName::NotReported.as_reported(), None);
        assert_ne!(
            DisplayProductName::PlatformHasNoConcept,
            DisplayProductName::NotReported,
        );
    }
}
