use bevy::prelude::Window;
use bevy::window::WindowPosition;
use bevy_kana::ToI32;
use bevy_kana::ToU32;
use hana_clerestory::CurrentMonitor;

use crate::constants::AUTOMATIC_TEXT;
use crate::constants::COMPARISON_COLUMN_PADDING;
use crate::constants::NONE_TEXT;
use crate::events::CachedRestoredState;

pub(super) struct CurrentValues {
    pub(super) physical_position: String,
    pub(super) logical_position:  String,
    pub(super) physical_size:     String,
    pub(super) logical_size:      String,
    pub(super) scale:             String,
    pub(super) monitor:           String,
    pub(super) mode:              String,
}

impl CurrentValues {
    pub(super) fn from_window(window: &Window, current_monitor: &CurrentMonitor) -> Self {
        let effective_window_mode = current_monitor.effective_window_mode;
        let scale = window.resolution.scale_factor();

        Self {
            physical_position: match window.position {
                WindowPosition::At(position) => format!("({}, {})", position.x, position.y),
                _ => AUTOMATIC_TEXT.to_string(),
            },
            logical_position:  match window.position {
                WindowPosition::At(position) => {
                    let logical_x = (f64::from(position.x) / f64::from(scale)).round().to_i32();
                    let logical_y = (f64::from(position.y) / f64::from(scale)).round().to_i32();
                    format!("({logical_x}, {logical_y})")
                },
                _ => AUTOMATIC_TEXT.to_string(),
            },
            physical_size:     format!("{}x{}", window.physical_width(), window.physical_height()),
            logical_size:      format!(
                "{}x{}",
                window.resolution.width().to_u32(),
                window.resolution.height().to_u32()
            ),
            scale:             format!("{scale}"),
            monitor:           format!("{}", current_monitor.index),
            mode:              format!("{effective_window_mode:?}"),
        }
    }
}

pub(super) struct RestoredValues {
    pub(super) physical_position: String,
    pub(super) logical_position:  String,
    pub(super) physical_size:     String,
    pub(super) logical_size:      String,
    pub(super) monitor:           String,
    pub(super) mode:              String,
}

impl From<&CachedRestoredState> for RestoredValues {
    fn from(cached_restored_state: &CachedRestoredState) -> Self {
        let physical_size = cached_restored_state.physical_size;
        let logical_size = cached_restored_state.logical_size;
        Self {
            physical_position: cached_restored_state.physical_position.map_or_else(
                || NONE_TEXT.to_string(),
                |position| format!("({}, {})", position.x, position.y),
            ),
            logical_position:  cached_restored_state.logical_position.map_or_else(
                || NONE_TEXT.to_string(),
                |position| format!("({}, {})", position.x, position.y),
            ),
            physical_size:     format!("{}x{}", physical_size.x, physical_size.y),
            logical_size:      format!("{}x{}", logical_size.x, logical_size.y),
            monitor:           cached_restored_state.monitor.to_string(),
            mode:              format!("{:?}", cached_restored_state.window_mode),
        }
    }
}

impl RestoredValues {
    pub(super) fn comparison_width(&self) -> usize {
        [
            self.physical_position.len(),
            self.logical_position.len(),
            self.physical_size.len(),
            self.logical_size.len(),
            self.monitor.len(),
            self.mode.len(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0)
            + COMPARISON_COLUMN_PADDING
    }
}
