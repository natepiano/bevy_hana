//! Saved screen state for reversible screen-authored panel conversions.

use bevy::prelude::Component;

use super::PanelScreenConversion;

/// Original screen-space state saved before a screen panel becomes world-space.
#[derive(Component, Clone, Debug)]
pub struct SavedPanelScreenState {
    /// Screen conversion that returns the panel to its saved screen placement.
    pub conversion: PanelScreenConversion,
}
