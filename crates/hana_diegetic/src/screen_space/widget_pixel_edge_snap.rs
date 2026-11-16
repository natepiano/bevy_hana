use bevy::prelude::Component;

/// Snaps widget face edges to physical pixels when viewed face-on.
///
/// Only the widget element receives the treatment; labels, icons and other
/// child elements keep their authored antialiasing and border widths.
#[derive(Clone, Component, Copy, Debug, Default)]
pub struct WidgetPixelEdgeSnap;
