//! Capability: reconstruct sub-pixel edges from the composited image with
//! Bevy's SMAA, for content that cannot antialias itself.
//!
//! Inserted on the orbit camera alone, so it reaches the 3D view and leaves the
//! screen-space panel cameras untouched.
//!
//! Needs Bevy's `smaa_luts` feature. Without it Bevy substitutes
//! `lut_placeholder` and SMAA still runs but stops finding edges, so the
//! workspace manifest names the feature rather than inheriting it from a
//! dependency's defaults.
//!
//! Gated behind the `SprinkleBuilder<WithOrbitCam>` typestate — see
//! [`crate::SprinkleBuilder::with_experimental_smaa`].

use bevy::anti_alias::smaa::Smaa;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::On;

use crate::orbit_cam::FairyDustOrbitCam;

pub(crate) fn install(app: &mut App) { app.add_observer(insert_smaa); }

fn insert_smaa(trigger: On<Add, FairyDustOrbitCam>, mut commands: Commands) {
    commands.entity(trigger.entity).insert(Smaa::default());
}
