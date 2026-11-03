//! Headless input fixtures shared by camera and application tests.

use bevy::input::mouse::MouseScrollPixelsPerLine;
use bevy::prelude::App;

/// Build an app with enhanced input's scroll conversion resource.
///
/// Initialize only the resource so tests retain control of staged input without
/// running Bevy's input-clearing systems.
pub fn input_test_app() -> App {
    let mut app = App::new();
    app.init_resource::<MouseScrollPixelsPerLine>();
    app
}
