use bevy::prelude::Added;
use bevy::prelude::Changed;
use bevy::prelude::ColorToComponents;
use bevy::prelude::Entity;
use bevy::prelude::LinearRgba;
use bevy::prelude::Mesh3d;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Vec4;
use bevy::prelude::With;
use bevy_kana::ToF32;
use bevy_render::Extract;
use bevy_render::sync_world::MainEntity;
use bevy_render::sync_world::MainEntityHashMap;

use super::constants::DEFAULT_OUTLINE_DRAW_PRIORITY;
use super::constants::OWNER_ID_OFFSET;
use super::outline::Outline;
use super::outline::OutlineMethod;
use super::outline::OverlapMode;

/// Tracks `ActiveOutlineModes::methods` for outline resources needed this frame.
/// Derived from the extracted outline cache; `ActiveOutlineMethods::has_hull`
/// controls hull resource allocation.
#[derive(Resource, Default)]
pub(crate) struct ActiveOutlineModes {
    /// Which outline methods are active this frame.
    pub(crate) methods: ActiveOutlineMethods,
}

/// Render-world cache of all extracted outlines, keyed by main-world entity.
#[derive(Resource, Default)]
pub(crate) struct ExtractedOutlineUniforms {
    /// Map from main-world entity to its extracted outline data.
    pub(crate) by_main_entity:       MainEntityHashMap<ExtractedOutline>,
    /// Which outline methods appear in the extracted cache.
    methods:                         ActiveOutlineMethods,
    /// Largest jump-flood outline width across all extracted outlines.
    pub(crate) max_jump_flood_width: f32,
}

/// Result of upserting an entry into the extracted outline cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheUpdate {
    Changed,
    Unchanged,
}

impl CacheUpdate {
    const fn is_changed(self) -> bool { matches!(self, Self::Changed) }

    const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unchanged, Self::Unchanged) => Self::Unchanged,
            _ => Self::Changed,
        }
    }
}

impl<T> From<Option<T>> for CacheUpdate {
    fn from(opt: Option<T>) -> Self { opt.map_or(Self::Unchanged, |_| Self::Changed) }
}

impl ExtractedOutlineUniforms {
    fn upsert(&mut self, entity: MainEntity, outline: ExtractedOutline) -> CacheUpdate {
        if let Some(existing) = self.by_main_entity.get_mut(&entity) {
            if *existing == outline {
                return CacheUpdate::Unchanged;
            }
            *existing = outline;
            return CacheUpdate::Changed;
        }

        self.by_main_entity.insert(entity, outline);
        CacheUpdate::Changed
    }

    fn recompute_flags_and_width(&mut self) {
        self.methods = ActiveOutlineMethods::None;
        self.max_jump_flood_width = 0.0;

        for outline in self.by_main_entity.values() {
            self.methods = self.methods.with_outline_method(outline.outline_method);
            if outline.outline_method == OutlineMethod::JumpFlood {
                self.max_jump_flood_width = self.max_jump_flood_width.max(outline.width);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ActiveOutlineMethods {
    #[default]
    None,
    JumpFloodOnly,
    HullOnly,
    JumpFloodAndHull,
}

impl ActiveOutlineMethods {
    pub(crate) const fn has_jump_flood(self) -> bool {
        matches!(self, Self::JumpFloodOnly | Self::JumpFloodAndHull)
    }

    pub(crate) const fn has_hull(self) -> bool {
        matches!(self, Self::HullOnly | Self::JumpFloodAndHull)
    }

    const fn with_outline_method(self, outline_method: OutlineMethod) -> Self {
        let includes_jump_flood =
            self.has_jump_flood() || matches!(outline_method, OutlineMethod::JumpFlood);
        let includes_hull = self.has_hull()
            || matches!(
                outline_method,
                OutlineMethod::WorldHull | OutlineMethod::ScreenHull
            );

        match (includes_jump_flood, includes_hull) {
            (false, false) => Self::None,
            (true, false) => Self::JumpFloodOnly,
            (false, true) => Self::HullOnly,
            (true, true) => Self::JumpFloodAndHull,
        }
    }
}

/// GPU-ready outline data extracted from the main world.
#[derive(Debug, Reflect, Clone, PartialEq)]
pub(crate) struct ExtractedOutline {
    /// Color multiplier for HDR glow via bloom.
    pub(crate) intensity:      f32,
    /// Outline width in pixels or world units depending on `outline_method`.
    pub(crate) width:          f32,
    /// Draw priority for ordering (reserved for future use).
    pub(crate) priority:       f32,
    /// Shader overlap factor derived from `OverlapMode`.
    pub(crate) overlap:        f32,
    /// Unique owner ID used for per-mesh and grouped overlap resolution.
    pub(crate) owner_id:       f32,
    /// Linear RGBA outline color as a `Vec4`.
    pub(crate) color:          Vec4,
    /// Which outline algorithm this entity uses.
    pub(crate) outline_method: OutlineMethod,
}

impl ExtractedOutline {
    fn from_main_world(entity: Entity, outline: &Outline) -> Self {
        let linear_color: LinearRgba = outline.color.into();
        let owner_entity = match outline.overlap_mode {
            OverlapMode::Grouped => outline.group_source.unwrap_or(entity),
            _ => entity,
        };
        Self {
            intensity:      outline.intensity,
            width:          outline.width,
            priority:       DEFAULT_OUTLINE_DRAW_PRIORITY,
            overlap:        outline.overlap_mode.as_shader_factor(),
            owner_id:       owner_entity.index().index().to_f32() + OWNER_ID_OFFSET,
            color:          linear_color.to_vec4(),
            outline_method: outline.method,
        }
    }
}

type OutlineEntityAndOutline = (Entity, &'static Outline);
type AddedOrChangedOutlineFilter = (With<Mesh3d>, Or<(Added<Outline>, Changed<Outline>)>);
type AddedOutlineFilter = (Added<Mesh3d>, With<Outline>);

pub(crate) fn extract_outline_uniforms(
    mut extracted_outlines: ResMut<ExtractedOutlineUniforms>,
    added_or_changed_outlines: Extract<Query<OutlineEntityAndOutline, AddedOrChangedOutlineFilter>>,
    added_mesh_outlines: Extract<Query<OutlineEntityAndOutline, AddedOutlineFilter>>,
    mut removed_outlines: Extract<RemovedComponents<Outline>>,
    mut removed_meshes: Extract<RemovedComponents<Mesh3d>>,
) {
    let mut cache_update = CacheUpdate::Unchanged;

    for entity in removed_outlines.read() {
        cache_update = cache_update.merge(
            extracted_outlines
                .by_main_entity
                .remove(&MainEntity::from(entity))
                .into(),
        );
    }

    for entity in removed_meshes.read() {
        cache_update = cache_update.merge(
            extracted_outlines
                .by_main_entity
                .remove(&MainEntity::from(entity))
                .into(),
        );
    }

    for (entity, outline) in &added_or_changed_outlines {
        if outline.activity.is_enabled() {
            cache_update = cache_update.merge(extracted_outlines.upsert(
                MainEntity::from(entity),
                ExtractedOutline::from_main_world(entity, outline),
            ));
        } else {
            cache_update = cache_update.merge(
                extracted_outlines
                    .by_main_entity
                    .remove(&MainEntity::from(entity))
                    .into(),
            );
        }
    }

    for (entity, outline) in &added_mesh_outlines {
        if outline.activity.is_enabled() {
            cache_update = cache_update.merge(extracted_outlines.upsert(
                MainEntity::from(entity),
                ExtractedOutline::from_main_world(entity, outline),
            ));
        }
    }

    if cache_update.is_changed() {
        extracted_outlines.recompute_flags_and_width();
    }
}

pub(crate) fn update_active_outline_modes(
    extracted_outlines: Res<ExtractedOutlineUniforms>,
    mut active: ResMut<ActiveOutlineModes>,
) {
    active.methods = extracted_outlines.methods;
}
