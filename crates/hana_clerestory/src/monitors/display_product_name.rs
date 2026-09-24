//! Operating-system display product names.

use bevy::prelude::Reflect;
use winit::monitor::MonitorHandle;

use super::identity;
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

pub(super) fn from_platform(
    monitor_handle: Option<&MonitorHandle>,
    winit_name: Option<&str>,
    platform: Platform,
) -> DisplayProductName {
    match platform {
        Platform::MacOs => identity::product_name(monitor_handle),
        Platform::Windows | Platform::X11 => winit_name.map_or_else(
            || DisplayProductName::NotReported,
            |name| DisplayProductName::Reported(name.to_owned()),
        ),
        Platform::Wayland => DisplayProductName::PlatformHasNoConcept,
    }
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
