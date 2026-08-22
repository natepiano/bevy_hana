use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::Remove;

use super::sequence::CameraSequencePlayback;
use super::sequence::FreeFlightControllerInstallation;
use super::sequence::OrbitControllerInstallation;
use crate::free_cam::FreeCam;
use crate::orbit_cam::OrbitCam;

/// Controls what happens when a new retained animation request conflicts with
/// an active native camera journey.
///
/// Insert this component on a camera entity to configure conflict resolution.
/// When absent, the policy is [`LastWins`](AnimationConflictPolicy::LastWins).
///
/// This component is independent from
/// [`CameraInputInterruptBehavior`](crate::CameraInputInterruptBehavior), which
/// handles physical camera input during an active journey.
#[derive(Component, Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
#[reflect(Component, Default)]
pub enum AnimationConflictPolicy {
    /// Cancel the active journey and accept the new request.
    #[default]
    LastWins,
    /// Reject the incoming request and preserve the active journey.
    FirstWins,
}

/// Controller damping captured while retained playback owns an `OrbitCam`.
///
/// The installation identity prevents a later controller instance from
/// receiving damping captured from an earlier installation.
#[derive(Component, Debug, Clone, Copy, Default)]
pub(super) struct OrbitControllerOverrideRestoration {
    pub(super) installation: OrbitControllerInstallation,
    pub(super) zoom:         f32,
    pub(super) pan:          f32,
    pub(super) orbit:        f32,
}

/// Controller damping captured while retained playback owns a `FreeCam`.
///
/// The installation identity prevents a later controller instance from
/// receiving damping captured from an earlier installation.
#[derive(Component, Debug, Clone, Copy, Default)]
pub(super) struct FreeFlightControllerOverrideRestoration {
    pub(super) installation: FreeFlightControllerInstallation,
    pub(super) translate:    f32,
    pub(super) look:         f32,
    pub(super) roll:         f32,
}

/// Restores damping after retained playback is removed.
///
/// Replacing a definition leaves retained playback installed, so this observer
/// does not expose a restore-and-recapture transition.
pub(super) fn restore_retained_camera_state(
    remove: On<Remove, CameraSequencePlayback>,
    mut commands: Commands,
    orbit_restorations: Query<&OrbitControllerOverrideRestoration>,
    free_restorations: Query<&FreeFlightControllerOverrideRestoration>,
    orbit_installations: Query<&OrbitControllerInstallation>,
    free_installations: Query<&FreeFlightControllerInstallation>,
    mut orbit_cameras: Query<&mut OrbitCam>,
    mut free_cameras: Query<&mut FreeCam>,
) {
    let camera = remove.entity;

    if let Ok(restoration) = orbit_restorations.get(camera) {
        if orbit_installations
            .get(camera)
            .is_ok_and(|installation| *installation == restoration.installation)
            && let Ok(mut orbit_cam) = orbit_cameras.get_mut(camera)
        {
            orbit_cam.zoom.set_damping(restoration.zoom);
            orbit_cam.pan.set_damping(restoration.pan);
            orbit_cam.orbit.set_damping(restoration.orbit);
        }
        commands
            .entity(camera)
            .remove::<OrbitControllerOverrideRestoration>();
    }
    if let Ok(restoration) = free_restorations.get(camera) {
        if free_installations
            .get(camera)
            .is_ok_and(|installation| *installation == restoration.installation)
            && let Ok(mut free_cam) = free_cameras.get_mut(camera)
        {
            free_cam.translate.set_damping(restoration.translate);
            free_cam.look.set_damping(restoration.look);
            free_cam.roll.set_damping(restoration.roll);
        }
        commands
            .entity(camera)
            .remove::<FreeFlightControllerOverrideRestoration>();
    }
}
