use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Query;
use bevy::prelude::With;

/// Marks a text entity whose glyphs are ready and whose meshes are spawned.
/// [`emit_world_text_ready`] removes the marker and fires [`WorldTextReady`]
/// once Bevy's transform propagation has run.
#[derive(Component)]
pub(crate) struct AwaitingReady;

/// Fired on a [`TextContent`](super::TextContent) entity when all its glyphs are
/// ready and the text is fully rendered for the first time (or after a
/// text/style change).
///
/// Observe per-entity:
/// ```ignore
/// commands.spawn((TextContent::new("Hello"), ...))
///     .observe(|trigger: On<WorldTextReady>| {
///         info!("Text ready on {:?}", trigger.entity());
///     });
/// ```
#[derive(EntityEvent)]
pub struct WorldTextReady {
    /// The [`TextContent`](super::TextContent) entity that is now fully rendered.
    pub entity: Entity,
}

/// Fires [`WorldTextReady`] for entities whose meshes and transforms are now
/// fully propagated. Runs after `CalculateBounds` so that `Aabb` and
/// `GlobalTransform` are available on mesh children.
pub(crate) fn emit_world_text_ready(
    awaiting: Query<Entity, With<AwaitingReady>>,
    mut commands: Commands,
) {
    for entity in &awaiting {
        commands.entity(entity).remove::<AwaitingReady>();
        commands
            .entity(entity)
            .trigger(|current_entity| WorldTextReady {
                entity: current_entity,
            });
    }
}
