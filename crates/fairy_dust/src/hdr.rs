//! Capability: enable HDR output on every camera.
//!
//! Diegetic panel content can render through a chain of cameras; any camera left
//! in LDR clamps over-bright (>1.0) colors at that step, so HDR must be set on
//! all of them for an over-bright base color to survive to the final image.
//!
//! See [`crate::SprinkleBuilder::with_hdr`].

use bevy::camera::Hdr;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Camera;
use bevy::prelude::Commands;
use bevy::prelude::On;

pub(crate) fn install(app: &mut App) { app.add_observer(enable_camera_hdr); }

fn enable_camera_hdr(trigger: On<Add, Camera>, mut commands: Commands) {
    commands.entity(trigger.entity).insert(Hdr);
}
