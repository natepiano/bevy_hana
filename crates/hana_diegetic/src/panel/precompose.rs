use std::collections::HashMap;

use bevy::asset::Handle;
use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::image::Image;
use bevy::math::UVec2;

/// Marks internal helper entities used to precompose panel subtrees.
///
/// These entities are implementation details: scene-fit and inspection code
/// should ignore the marked entity and its descendants.
#[derive(Component, Debug)]
pub struct PrecomposeHelper;

/// Per-panel cache for element subtrees rendered into intermediate images.
#[derive(Component, Debug, Default)]
pub(crate) struct PanelPrecomposeCache {
    entries:             HashMap<usize, PrecomposeCacheEntry>,
    retired_images:      Vec<RetiredPrecomposeImage>,
    pending_activations: Vec<PendingPrecomposeCamera>,
}

/// Runtime assets and helper entities for one precomposed element.
#[derive(Debug)]
pub(crate) struct PrecomposeCacheEntry {
    pub(crate) image:        Handle<Image>,
    pub(crate) helper_panel: Entity,
    pub(crate) camera:       Entity,
    pub(crate) pixel_size:   UVec2,
}

/// Image handle kept alive briefly after a camera stops targeting it.
#[derive(Debug)]
struct RetiredPrecomposeImage {
    handle: Handle<Image>,
    frames: u8,
}

/// Camera that should be reactivated after its render target has propagated.
#[derive(Debug)]
struct PendingPrecomposeCamera {
    entity: Entity,
    frames: u8,
}

impl PanelPrecomposeCache {
    /// Returns the cache entry for a source element.
    #[must_use]
    pub(crate) fn entry(&self, element_idx: usize) -> Option<&PrecomposeCacheEntry> {
        self.entries.get(&element_idx)
    }

    /// Mutable access to the entry map for reconcile systems.
    pub(crate) const fn entries_mut(&mut self) -> &mut HashMap<usize, PrecomposeCacheEntry> {
        &mut self.entries
    }

    /// Keeps an old render target alive until the render world has observed the
    /// camera pointing at the replacement.
    pub(crate) fn retire_image(&mut self, handle: Handle<Image>) {
        self.retired_images
            .push(RetiredPrecomposeImage { handle, frames: 0 });
    }

    /// Defers camera activation until a newly assigned image target can resolve
    /// to a nonzero viewport.
    pub(crate) fn defer_camera_activation(&mut self, entity: Entity) {
        self.pending_activations
            .retain(|pending| pending.entity != entity);
        self.pending_activations
            .push(PendingPrecomposeCamera { entity, frames: 0 });
    }

    /// Advances deferred camera activations and returns cameras ready to render.
    pub(crate) fn drain_ready_camera_activations(&mut self) -> Vec<Entity> {
        let mut ready = Vec::new();
        let mut pending_cameras = Vec::new();
        for mut pending in self.pending_activations.drain(..) {
            pending.frames = pending.frames.saturating_add(1);
            if pending.frames > 1 {
                ready.push(pending.entity);
            } else {
                pending_cameras.push(pending);
            }
        }
        self.pending_activations = pending_cameras;
        ready
    }

    /// Advances retired image lifetimes and returns handles safe to remove.
    pub(crate) fn drain_ready_retired_images(&mut self) -> Vec<Handle<Image>> {
        let mut ready = Vec::new();
        let mut pending = Vec::new();
        for mut retired in self.retired_images.drain(..) {
            retired.frames = retired.frames.saturating_add(1);
            if retired.frames > 1 {
                ready.push(retired.handle);
            } else {
                pending.push(retired);
            }
        }
        self.retired_images = pending;
        ready
    }
}
