use bevy::prelude::Entity;

use crate::animation::AnimationSource;
use crate::animation::CameraMove;
use crate::animation::PlayAnimation;

pub(super) fn timed_animation_request(
    camera: Entity,
    target: Entity,
    source: AnimationSource,
    camera_moves: impl IntoIterator<Item = CameraMove>,
) -> PlayAnimation {
    PlayAnimation::new(camera, camera_moves)
        .source(source)
        .target(target)
}
