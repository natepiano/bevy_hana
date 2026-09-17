//! Capability: render Bevy's physically-based sky behind the scene.
//!
//! Two pieces have to line up. [`Atmosphere`] describes the planet and lives on
//! its own entity with a [`ScatteringMedium`] asset; [`AtmosphereSettings`]
//! goes on the camera, where it selects the LUT sizes and contributes the LUT
//! and sampler entries to that camera's view bind group.
//!
//! [`AtmosphereSettings`] requires `Hdr`, which covers the orbit camera by
//! itself. It does not cover the rest of a diegetic render chain, where any
//! camera left in LDR clamps the over-bright sky and leaves the 3D view black,
//! so this installs [`crate::hdr`] as well.
//!
//! Both components stay for the life of the app. Bevy 0.19.1 leaves stale
//! render state behind when either one is torn down and the view then alternates
//! between two frames matching neither state, so there is no teardown path here.
//!
//! Gated behind the `SprinkleBuilder<WithOrbitCam>` typestate — see
//! [`crate::SprinkleBuilder::with_atmosphere`].

use bevy::light::Atmosphere;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::pbr::AtmosphereSettings;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Startup;

use crate::constants::ATMOSPHERE_PHASE_RESOLUTION;
use crate::constants::ATMOSPHERE_TRANSMITTANCE_RESOLUTION;
use crate::hdr;
use crate::orbit_cam::FairyDustOrbitCam;

pub(crate) fn install(app: &mut App) {
    hdr::install(app);
    app.add_observer(insert_atmosphere_settings)
        .add_systems(Startup, spawn_atmosphere);
}

fn insert_atmosphere_settings(trigger: On<Add, FairyDustOrbitCam>, mut commands: Commands) {
    commands
        .entity(trigger.entity)
        .insert(AtmosphereSettings::default());
}

fn spawn_atmosphere(
    mut commands: Commands,
    mut scattering_media: ResMut<Assets<ScatteringMedium>>,
) {
    let scattering_medium = scattering_media.add(ScatteringMedium::earth(
        ATMOSPHERE_TRANSMITTANCE_RESOLUTION,
        ATMOSPHERE_PHASE_RESOLUTION,
    ));
    commands.spawn(Atmosphere::earth(scattering_medium));
}
