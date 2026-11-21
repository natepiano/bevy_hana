//! Concrete window lookup for screen-space attachment resolution.

use bevy::ecs::query::QuerySingleError;
use bevy::platform::collections::HashMap;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Vec2;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;

#[derive(Debug, thiserror::Error)]
pub(super) enum WindowResolveError {
    #[error("could not resolve the primary window")]
    PrimaryWindow(#[from] QuerySingleError),
    #[error("window {0} is missing")]
    Missing(Entity),
    #[error("window {0} has a nonpositive dimension")]
    ZeroSized(Entity),
}

pub(super) fn resolve_window(
    window_ref: WindowRef,
    primary: &Query<Entity, With<PrimaryWindow>>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> Result<(Entity, Vec2), WindowResolveError> {
    let window = match window_ref {
        WindowRef::Primary => primary.single()?,
        WindowRef::Entity(entity) => entity,
    };
    let Some(size) = window_sizes.get(&window).copied() else {
        return Err(WindowResolveError::Missing(window));
    };
    if size.x <= 0.0 || size.y <= 0.0 {
        return Err(WindowResolveError::ZeroSized(window));
    }
    Ok((window, size))
}

pub(super) fn window_size_lookup(windows: &Query<(Entity, &Window)>) -> HashMap<Entity, Vec2> {
    let mut window_sizes = HashMap::default();
    for (entity, window) in windows {
        window_sizes.insert(entity, Vec2::new(window.width(), window.height()));
    }
    window_sizes
}
