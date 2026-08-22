use std::f32::consts::FRAC_PI_4;

use bevy::prelude::Projection;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::Window;

use crate::constants::CAMERA_LOOK_AT;
use crate::constants::GRID_FILL_FRACTION;
use crate::constants::VIEWPORT_FOV_DIVISOR;
use crate::constants::VIEWPORT_HEIGHT_MULTIPLIER;

pub(super) struct ViewportInfo {
    pub(super) right:   Vec3,
    pub(super) up:      Vec3,
    pub(super) forward: Vec3,
    pub(super) center:  Vec3,
    pub(super) width:   f32,
    pub(super) height:  f32,
}

pub(super) fn compute_viewport_info(
    camera_transform: &Transform,
    projection: &Projection,
    window: &Window,
) -> ViewportInfo {
    let fov = match projection {
        Projection::Perspective(perspective) => perspective.fov,
        Projection::Orthographic(_) | Projection::Custom(_) => FRAC_PI_4,
    };

    let distance = camera_transform.translation.distance(CAMERA_LOOK_AT);
    let aspect = window.width() / window.height();
    let visible_height = VIEWPORT_HEIGHT_MULTIPLIER * distance * (fov / VIEWPORT_FOV_DIVISOR).tan();
    let visible_width = visible_height * aspect;

    ViewportInfo {
        right:   camera_transform.right().as_vec3(),
        up:      camera_transform.up().as_vec3(),
        forward: camera_transform.forward().as_vec3(),
        center:  CAMERA_LOOK_AT,
        width:   visible_width * GRID_FILL_FRACTION,
        height:  visible_height * GRID_FILL_FRACTION,
    }
}
