//! Capability: add a thresholded bloom to the orbit camera so over-bright
//! (>1.0) colors glow while normal-range content stays crisp.
//!
//! Requires HDR on the camera; [`Bloom`] pulls in the `Hdr` component via its
//! required-components, so this works without [`crate::SprinkleBuilder::with_hdr`],
//! but pairing both keeps every camera in the render chain HDR.
//!
//! Gated behind the `SprinkleBuilder<WithOrbitCam>` typestate — see
//! [`crate::SprinkleBuilder::with_bloom`].

use bevy::post_process::bloom::Bloom;
use bevy::post_process::bloom::BloomCompositeMode;
use bevy::post_process::bloom::BloomPrefilter;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::On;

use crate::constants::BLOOM_INTENSITY;
use crate::constants::BLOOM_THRESHOLD;
use crate::constants::BLOOM_THRESHOLD_SOFTNESS;
use crate::orbit_cam::FairyDustOrbitCam;

pub(crate) fn install(app: &mut App) { app.add_observer(insert_bloom); }

fn insert_bloom(trigger: On<Add, FairyDustOrbitCam>, mut commands: Commands) {
    commands.entity(trigger.entity).insert(Bloom {
        intensity: BLOOM_INTENSITY,
        prefilter: BloomPrefilter {
            threshold:          BLOOM_THRESHOLD,
            threshold_softness: BLOOM_THRESHOLD_SOFTNESS,
        },
        composite_mode: BloomCompositeMode::Additive,
        ..Bloom::OLD_SCHOOL
    });
}
