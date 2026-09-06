use bevy::ecs::observer::On;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Insert;
use bevy::prelude::Or;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::With;
use bevy::prelude::Without;
use hana_kana::SequenceCommands;
use hana_kana::SequenceOwner;

use super::CameraSequence;
use super::playback::CameraPose;
use super::playback::CameraSequencePlayback;
use super::playback::LastAppliedCameraPose;
use super::request::CameraSequencePreparationRequested;
use crate::CameraBasis;
use crate::CameraHomePending;
use crate::FreeCam;
use crate::Initialization;
use crate::OrbitCam;
use crate::animation::lifecycle::FreeFlightControllerOverrideRestoration;
use crate::animation::lifecycle::OrbitControllerOverrideRestoration;

/// The camera-controller tuple a retained sequence can prepare against.
///
/// This is deliberately private: applications observe retained playback and
/// controller output, not this scheduling classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraControllerAvailability {
    InitializedOrbit,
    InitializedFreeFlight,
    NoController,
    FreeFlightWithoutBasis,
    ConflictingControllers,
}

/// Stable identity assigned to one concrete camera-controller installation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CameraControllerInstallationIdentity(u128);

/// Hands out installation identities. A fresh one is drawn only when an
/// `OrbitCam` or `FreeCam` component is inserted or replaced, never when a
/// controller is mutated in place.
#[derive(Resource, Default)]
pub(in crate::animation) struct CameraControllerInstallationIdentityAllocator(u128);

impl CameraControllerInstallationIdentityAllocator {
    const fn next(&mut self) -> CameraControllerInstallationIdentity {
        let identity = CameraControllerInstallationIdentity(self.0);
        self.0 = self.0.wrapping_add(1);
        identity
    }
}

/// The identity of the currently installed `OrbitCam` component.
#[derive(Component, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::animation) struct OrbitControllerInstallation {
    identity: CameraControllerInstallationIdentity,
}

/// The identity of the currently installed `FreeCam` component.
#[derive(Component, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::animation) struct FreeFlightControllerInstallation {
    identity: CameraControllerInstallationIdentity,
}

/// The concrete initialized controller installation prepared for playback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum CameraControllerInstallation {
    Orbit(OrbitControllerInstallation),
    FreeFlight {
        installation: FreeFlightControllerInstallation,
        basis:        CameraBasis,
    },
}

impl CameraControllerInstallation {
    pub(super) fn is_same_installation(self, other: Self) -> bool {
        match (self, other) {
            (Self::Orbit(left), Self::Orbit(right)) => left.identity == right.identity,
            (
                Self::FreeFlight {
                    installation: left, ..
                },
                Self::FreeFlight {
                    installation: right,
                    ..
                },
            ) => left.identity == right.identity,
            (Self::Orbit(_), Self::FreeFlight { .. })
            | (Self::FreeFlight { .. }, Self::Orbit(_)) => false,
        }
    }
}

/// Why an available controller cannot yet name its installation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraControllerInstallationError {
    ControllerUnavailable,
    IdentityPending,
}

/// Initializes controller state before retained camera preparation. Controller
/// output remains in `PostUpdate`; this private `Update` pass only establishes
/// a valid pose and publishes the home state derived from it.
pub(in crate::animation) fn initialize_camera_controllers(
    mut commands: Commands,
    mut orbit_cameras: Query<
        (
            Entity,
            &mut OrbitCam,
            Option<&CameraBasis>,
            &mut Transform,
            &mut Projection,
            &crate::OrbitCamInput,
            Has<crate::OrbitCamHomePose>,
        ),
        (
            Without<FreeCam>,
            Or<(
                With<CameraSequence>,
                With<CameraSequencePreparationRequested>,
            )>,
        ),
    >,
    mut free_cameras: Query<
        (
            Entity,
            &mut FreeCam,
            &CameraBasis,
            &mut Transform,
            &crate::FreeCamInput,
            Has<crate::FreeCamHomePose>,
        ),
        (
            Without<OrbitCam>,
            Or<(
                With<CameraSequence>,
                With<CameraSequencePreparationRequested>,
            )>,
        ),
    >,
) {
    for (entity, mut camera, basis, mut transform, mut projection, input, has_home) in
        &mut orbit_cameras
    {
        if camera.initialization == Initialization::Active {
            continue;
        }
        crate::orbit_cam::controller::initialize_orbit_cam(
            &mut camera,
            basis.copied().unwrap_or(CameraBasis::Y_UP),
            &mut transform,
            &mut projection,
        );
        if !has_home {
            commands
                .entity(entity)
                .insert(crate::OrbitCamHomePose::from_current(&camera));
            if !input.has_input() {
                commands.entity(entity).insert(CameraHomePending);
            }
        }
    }

    for (entity, mut camera, basis, mut transform, input, has_home) in &mut free_cameras {
        if camera.initialization == Initialization::Active {
            continue;
        }
        crate::free_cam::controller::initialize_free_cam(&mut camera, *basis, &mut transform);
        if !has_home {
            commands
                .entity(entity)
                .insert(crate::FreeCamHomePose::from_current(&camera));
            if !input.has_input() {
                commands.entity(entity).insert(CameraHomePending);
            }
        }
    }
}

/// Assigns a fresh identity whenever an `OrbitCam` value is inserted or
/// replaced. In-place controller evaluation does not trigger this observer.
pub(in crate::animation) fn identify_orbit_controller_installation(
    inserted: On<Insert, OrbitCam>,
    mut commands: Commands,
    mut identities: ResMut<CameraControllerInstallationIdentityAllocator>,
) {
    commands
        .entity(inserted.entity)
        .insert(OrbitControllerInstallation {
            identity: identities.next(),
        });
}

/// Assigns a fresh identity whenever a `FreeCam` value is inserted or
/// replaced. In-place controller evaluation does not trigger this observer.
pub(in crate::animation) fn identify_free_flight_controller_installation(
    inserted: On<Insert, FreeCam>,
    mut commands: Commands,
    mut identities: ResMut<CameraControllerInstallationIdentityAllocator>,
) {
    commands
        .entity(inserted.entity)
        .insert(FreeFlightControllerInstallation {
            identity: identities.next(),
        });
}

pub(super) fn camera_controller_interrupted(
    camera: Entity,
    last_applied: LastAppliedCameraPose,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut crate::OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut crate::FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) -> bool {
    if let Ok((orbit, input, _)) = orbit_cameras.get_mut(camera) {
        return input.has_input() || !orbit_target_matches_pose(&orbit, last_applied.0);
    }
    if let Ok((free, input, _)) = free_cameras.get_mut(camera) {
        return input.has_input() || !free_target_matches_pose(&free, last_applied.0);
    }
    false
}

pub(super) fn clear_camera_input(
    camera: Entity,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut crate::OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut crate::FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    if let Ok((_, mut input, _)) = orbit_cameras.get_mut(camera) {
        input.clear();
        return;
    }
    if let Ok((_, mut input, _)) = free_cameras.get_mut(camera) {
        input.clear();
    }
}

pub(super) fn stash_camera_controller_override(
    commands: &mut Commands,
    camera: Entity,
    controller_installation: CameraControllerInstallation,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut crate::OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut crate::FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    match controller_installation {
        CameraControllerInstallation::Orbit(installation) => {
            if let Ok((mut orbit, _, stash)) = orbit_cameras.get_mut(camera) {
                stash_orbit_for_retained_playback(
                    commands,
                    camera,
                    &mut orbit,
                    stash,
                    installation,
                );
            }
        },
        CameraControllerInstallation::FreeFlight { installation, .. } => {
            if let Ok((mut free, _, stash)) = free_cameras.get_mut(camera) {
                stash_free_for_retained_playback(commands, camera, &mut free, stash, installation);
            }
        },
    }
}

pub(super) fn restore_camera_controller_override(
    commands: &mut Commands,
    camera: Entity,
    controller_installation: CameraControllerInstallation,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut crate::OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut crate::FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    match controller_installation {
        CameraControllerInstallation::Orbit(installation) => {
            if let Ok((mut orbit, _, Some(stash))) = orbit_cameras.get_mut(camera)
                && stash.installation == installation
            {
                orbit.zoom.set_damping(stash.zoom);
                orbit.pan.set_damping(stash.pan);
                orbit.orbit.set_damping(stash.orbit);
            }
        },
        CameraControllerInstallation::FreeFlight { installation, .. } => {
            if let Ok((mut free, _, Some(stash))) = free_cameras.get_mut(camera)
                && stash.installation == installation
            {
                free.translate.set_damping(stash.translate);
                free.look.set_damping(stash.look);
                free.roll.set_damping(stash.roll);
            }
        },
    }
    commands.entity(camera).remove::<(
        OrbitControllerOverrideRestoration,
        FreeFlightControllerOverrideRestoration,
    )>();
}

pub(super) fn orbit_target_matches_pose(camera: &OrbitCam, pose: CameraPose) -> bool {
    let CameraPose::Orbit(pose) = pose else {
        return false;
    };
    camera.pan.target().0.distance(pose.focus.0)
        <= super::super::constants::EXTERNAL_INPUT_TOLERANCE
        && (camera.orbit.target().yaw - pose.orbit_angles.yaw).abs()
            <= super::super::constants::EXTERNAL_INPUT_TOLERANCE
        && (camera.orbit.target().pitch - pose.orbit_angles.pitch).abs()
            <= super::super::constants::EXTERNAL_INPUT_TOLERANCE
        && (camera.zoom.target().0 - pose.radius.0).abs()
            <= super::super::constants::EXTERNAL_INPUT_TOLERANCE
}

pub(super) fn free_target_matches_pose(camera: &FreeCam, pose: CameraPose) -> bool {
    let CameraPose::Free(pose) = pose else {
        return false;
    };
    camera.translate.target().0.distance(pose.position.0)
        <= super::super::constants::EXTERNAL_INPUT_TOLERANCE
        && (camera.look.target().yaw - pose.look.yaw).abs()
            <= super::super::constants::EXTERNAL_INPUT_TOLERANCE
        && (camera.look.target().pitch - pose.look.pitch).abs()
            <= super::super::constants::EXTERNAL_INPUT_TOLERANCE
        && (camera.roll.target().0 - pose.roll.0).abs()
            <= super::super::constants::EXTERNAL_INPUT_TOLERANCE
}

/// Suppresses already-produced input only while a selected producer owns a
/// retained camera. Native interruption policy remains independent.
pub(in crate::animation) fn clear_selected_driver_input(
    sequence_commands: SequenceCommands,
    mut orbit_inputs: Query<(Entity, &mut crate::OrbitCamInput), With<OrbitCam>>,
    mut free_inputs: Query<(Entity, &mut crate::FreeCamInput), With<FreeCam>>,
    playbacks: Query<(), With<CameraSequencePlayback>>,
) {
    for (camera, mut input) in &mut orbit_inputs {
        if playbacks.contains(camera)
            && matches!(sequence_commands.owner(camera), SequenceOwner::Driver(_))
        {
            input.clear();
        }
    }
    for (camera, mut input) in &mut free_inputs {
        if playbacks.contains(camera)
            && matches!(sequence_commands.owner(camera), SequenceOwner::Driver(_))
        {
            input.clear();
        }
    }
}

pub(super) const fn camera_controller_availability(
    orbit: Option<&OrbitCam>,
    free: Option<&FreeCam>,
    basis: Option<&CameraBasis>,
) -> CameraControllerAvailability {
    match (orbit, free, basis) {
        (Some(_), None, _) => CameraControllerAvailability::InitializedOrbit,
        (None, Some(_), Some(_)) => CameraControllerAvailability::InitializedFreeFlight,
        (None, Some(_), None) => CameraControllerAvailability::FreeFlightWithoutBasis,
        (None, None, _) => CameraControllerAvailability::NoController,
        (Some(_), Some(_), _) => CameraControllerAvailability::ConflictingControllers,
    }
}

pub(super) fn camera_controller_installation(
    camera: Entity,
    availability: CameraControllerAvailability,
    coordinate_systems: &Query<&CameraBasis>,
    orbit_installations: &Query<&OrbitControllerInstallation>,
    free_installations: &Query<&FreeFlightControllerInstallation>,
) -> Result<CameraControllerInstallation, CameraControllerInstallationError> {
    match availability {
        CameraControllerAvailability::InitializedOrbit => orbit_installations
            .get(camera)
            .copied()
            .map(CameraControllerInstallation::Orbit)
            .map_err(|_| CameraControllerInstallationError::IdentityPending),
        CameraControllerAvailability::InitializedFreeFlight => {
            let installation = free_installations
                .get(camera)
                .copied()
                .map_err(|_| CameraControllerInstallationError::IdentityPending)?;
            let basis = coordinate_systems
                .get(camera)
                .copied()
                .map_err(|_| CameraControllerInstallationError::ControllerUnavailable)?;
            Ok(CameraControllerInstallation::FreeFlight {
                installation,
                basis,
            })
        },
        CameraControllerAvailability::NoController
        | CameraControllerAvailability::FreeFlightWithoutBasis
        | CameraControllerAvailability::ConflictingControllers => {
            Err(CameraControllerInstallationError::ControllerUnavailable)
        },
    }
}

pub(super) fn stash_orbit_for_retained_playback(
    commands: &mut Commands,
    camera_entity: Entity,
    camera: &mut OrbitCam,
    existing: Option<&OrbitControllerOverrideRestoration>,
    installation: OrbitControllerInstallation,
) {
    if existing.is_none_or(|stash| stash.installation != installation) {
        if existing.is_some() {
            commands
                .entity(camera_entity)
                .remove::<OrbitControllerOverrideRestoration>();
        }
        commands
            .entity(camera_entity)
            .insert(OrbitControllerOverrideRestoration {
                installation,
                zoom: camera.zoom.damping(),
                pan: camera.pan.damping(),
                orbit: camera.orbit.damping(),
            });
    }
    camera
        .zoom
        .set_damping(super::super::constants::INSTANT_SMOOTHNESS);
    camera
        .pan
        .set_damping(super::super::constants::INSTANT_SMOOTHNESS);
    camera
        .orbit
        .set_damping(super::super::constants::INSTANT_SMOOTHNESS);
}

pub(super) fn stash_free_for_retained_playback(
    commands: &mut Commands,
    camera_entity: Entity,
    camera: &mut FreeCam,
    existing: Option<&FreeFlightControllerOverrideRestoration>,
    installation: FreeFlightControllerInstallation,
) {
    if existing.is_none_or(|stash| stash.installation != installation) {
        if existing.is_some() {
            commands
                .entity(camera_entity)
                .remove::<FreeFlightControllerOverrideRestoration>();
        }
        commands
            .entity(camera_entity)
            .insert(FreeFlightControllerOverrideRestoration {
                installation,
                translate: camera.translate.damping(),
                look: camera.look.damping(),
                roll: camera.roll.damping(),
            });
    }
    camera
        .translate
        .set_damping(super::super::constants::INSTANT_SMOOTHNESS);
    camera
        .look
        .set_damping(super::super::constants::INSTANT_SMOOTHNESS);
    camera
        .roll
        .set_damping(super::super::constants::INSTANT_SMOOTHNESS);
}
