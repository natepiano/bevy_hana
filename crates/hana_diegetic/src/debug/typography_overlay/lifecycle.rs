use bevy::prelude::Add;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::Transform;
use bevy::prelude::Visibility;
use bevy::prelude::With;

use super::OverlayContainer;
use super::TypographyOverlay;

/// Observer: spawns an [`OverlayContainer`] child when
/// [`TypographyOverlay`] is added to an entity.
pub(in crate::debug) fn on_overlay_added(
    trigger: On<Add, TypographyOverlay>,
    mut commands: Commands,
) {
    commands.entity(trigger.entity).with_child((
        OverlayContainer,
        Transform::IDENTITY,
        Visibility::Inherited,
    ));
}

/// Observer: despawns the [`OverlayContainer`] child (and all its
/// descendants) when [`TypographyOverlay`] is removed from an entity.
pub(in crate::debug) fn on_overlay_removed(
    trigger: On<Remove, TypographyOverlay>,
    containers: Query<(Entity, &ChildOf), With<OverlayContainer>>,
    mut commands: Commands,
) {
    let parent = trigger.entity;
    for (container_entity, child_of) in &containers {
        if child_of.parent() == parent {
            commands.entity(container_entity).despawn();
        }
    }
}
