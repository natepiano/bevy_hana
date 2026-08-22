//! Capability: insert `hana_diegetic::StableTransparency` on the spawned
//! orbit camera so the proper OIT + `Msaa::Off` configuration is applied
//! (removes the view-angle color shift on coplanar `WorldText`).
//!
//! Gated behind the `SprinkleBuilder<WithOrbitCam>` typestate — see
//! [`crate::SprinkleBuilder::with_stable_transparency`]. Pulls in
//! `DiegeticUiPlugin` deduplicated, since `StableTransparency` is inert
//! without it.

use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::On;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::StableTransparency;

use crate::ensure_plugin;
use crate::orbit_cam::FairyDustOrbitCam;

pub(crate) fn install(app: &mut App) {
    ensure_plugin(app, DiegeticUiPlugin);
    app.add_observer(insert_stable_transparency);
}

fn insert_stable_transparency(trigger: On<Add, FairyDustOrbitCam>, mut commands: Commands) {
    commands.entity(trigger.entity).insert(StableTransparency);
}
