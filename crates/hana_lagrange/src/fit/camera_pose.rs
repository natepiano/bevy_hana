use bevy::prelude::EulerRot;
use bevy::prelude::Transform;

use crate::CameraBasis;
use crate::Initialization;
use crate::free_cam::FreeCam;
use crate::operation::LookAngles;
use crate::operation::Position;
use crate::operation::Roll;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FreeCamFitPose {
    pub(super) position: Position,
    pub(super) look:     LookAngles,
    pub(super) roll:     Roll,
}

impl FreeCamFitPose {
    pub(super) const fn from_free_cam_current(free_cam: &FreeCam) -> Self {
        Self {
            position: free_cam.translate.current(),
            look:     free_cam.look.current(),
            roll:     free_cam.roll.current(),
        }
    }

    pub(super) fn from_free_cam_or_transform(
        free_cam: &FreeCam,
        transform: &Transform,
        basis: CameraBasis,
    ) -> Self {
        if free_cam.initialization == Initialization::FromTransform {
            Self::from_transform(transform, basis)
        } else {
            Self::from_free_cam_current(free_cam)
        }
    }

    pub(super) fn from_transform(transform: &Transform, basis: CameraBasis) -> Self {
        let local_rotation = basis.rotation().inverse() * transform.rotation;
        let (yaw, pitch, roll) = local_rotation.to_euler(EulerRot::YXZ);
        Self {
            position: Position(transform.translation),
            look:     LookAngles { yaw, pitch: -pitch },
            roll:     Roll(roll),
        }
    }
}
