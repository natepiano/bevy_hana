//! Text rendering systems for panel and world-space glyph meshes.

mod access;
mod alpha;
mod batching;
mod glyph_cascade;
mod layout;
mod reify;
mod relationship;
mod shaping;

use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::App;
use bevy::prelude::Color;
use bevy::prelude::Component;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::TransformSystems;

pub use self::access::DiegeticTextMut;
pub use self::access::PanelText;
pub use self::access::PanelTextReader;
pub use self::access::TextEdit;
use self::alpha::seed_panel_text_child_alpha;
pub use self::batching::DiegeticTextBatch;
use self::batching::commit_batch_buffers;
use self::batching::update_batch_bounds;
use self::batching::update_panel_text_batches;
use self::batching::write_batch_run_transforms;
use self::glyph_cascade::seed_panel_text_child_glyph;
pub use self::layout::PanelTextLayout;
use self::reify::reify_text_entities;
pub use self::relationship::PanelTextRuns;
pub use self::relationship::TextRunOf;
use self::shaping::shape_panel_text_children;
use super::PanelChildSystems;
use super::material_table;
use super::material_table::BatchAssetWrites;
use super::material_table::BatchResourcesReady;
use super::material_table::MaterialTableAppendReady;
use super::precompose;
use super::text_shaping::TextShapingContext;
use super::world_text;
use crate::cascade;
use crate::cascade::Cascade;
use crate::cascade::TextAlpha;
use crate::layout::GlyphRenderMode;
use crate::layout::GlyphShadowMode;
use crate::layout::Lighting;
use crate::layout::ShapedTextCache;
use crate::layout::Sidedness;
use crate::layout::TextStyle;
use crate::panel::DiegeticPerfStats;
use crate::text::PreparedTextRun;
use crate::widgets::VisualOverrideIndex;

/// Stores a prepared text run for a panel [`TextContent`](crate::TextContent) child.
///
/// Internal render-pipeline component, distinct from the public
/// [`PanelText`](crate::PanelText) `SystemParam` callers use to get/set run text
/// by id.
#[derive(Component)]
pub(super) struct PreparedPanelText {
    /// Prepared text run.
    pub prepared:    PreparedTextRun,
    /// Text-style snapshot used by `shape_panel_text_children` to decide
    /// whether a `Changed<TextStyle>` event affects glyph geometry.
    pub style_gate:  TextStyle,
    /// Glyph render mode for this text element.
    pub render_mode: GlyphRenderMode,
    /// Glyph shadow mode for this text element.
    pub shadow_mode: Cascade<GlyphShadowMode>,
    /// Text fill color.
    pub fill_color:  Color,
    /// One em along the run's local Y axis.
    pub em_height:   f32,
    /// Optional panel-local clipping rect.
    pub clip_rect:   Option<[f32; 4]>,
    /// Set when the change that marked this component dirty touched only render
    /// fields (color, render mode, shadow mode) and left glyph geometry intact.
    /// The batching pass reads it to update the run record in place instead of
    /// re-deriving identical glyph quads. `false` on every full reshape.
    pub render_only: bool,
}

/// Plugin that adds text rendering for diegetic panels.
///
/// Reifies panel text entities, runs text shaping for panel text, and builds
/// the glyph meshes.
pub(super) struct TextRenderPlugin;

impl Plugin for TextRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(cascade::cascade_plugin::<TextAlpha>());
        app.add_plugins(cascade::cascade_plugin::<GlyphShadowMode>());
        app.add_plugins(cascade::cascade_plugin::<Lighting>());
        app.add_plugins(cascade::cascade_plugin::<Sidedness>());
        app.add_observer(seed_panel_text_child_alpha);
        app.add_observer(seed_panel_text_child_glyph);
        app.init_resource::<TextShapingContext>();
        app.init_resource::<ShapedTextCache>();
        app.init_resource::<DiegeticPerfStats>();
        app.init_resource::<VisualOverrideIndex>();
        app.add_systems(
            PostUpdate,
            (
                precompose::ensure_panel_precompose_caches.in_set(PanelChildSystems::Build),
                precompose::activate_pending_precompose_cameras
                    .in_set(PanelChildSystems::Build)
                    .after(precompose::ensure_panel_precompose_caches),
                precompose::cleanup_retired_precompose_images
                    .in_set(PanelChildSystems::Build)
                    .after(precompose::activate_pending_precompose_cameras),
                reify_text_entities
                    .in_set(PanelChildSystems::Build)
                    .after(precompose::ensure_panel_precompose_caches),
                // `reify_text_entities` is the sole writer for
                // `DiegeticPerfStats::reify_ms`; `ImageBatchPlugin` routes
                // image records without adding to this text-child timing.
                shape_panel_text_children.after(reify_text_entities),
                world_text::emit_world_text_ready.after(VisibilitySystems::CalculateBounds),
            ),
        );
        // `update_panel_text_batches` is ordered by the `.after(...)` and
        // `.before(TransformSystems::Propagate)` calls below; the later systems
        // update run transforms, refresh batch `Aabb`s, and upload dirty record
        // buffers.
        app.add_systems(
            PostUpdate,
            (
                update_panel_text_batches
                    .after(shape_panel_text_children)
                    .after(MaterialTableAppendReady)
                    .in_set(BatchAssetWrites)
                    .before(TransformSystems::Propagate)
                    .before(BatchResourcesReady),
                material_table::register_path_batch_materials::<DiegeticTextBatch>
                    .after(update_panel_text_batches)
                    .in_set(BatchResourcesReady),
                write_batch_run_transforms
                    .after(TransformSystems::Propagate)
                    .in_set(BatchResourcesReady),
                update_batch_bounds
                    .after(write_batch_run_transforms)
                    .after(VisibilitySystems::CalculateBounds)
                    .before(VisibilitySystems::CheckVisibility)
                    .in_set(BatchResourcesReady),
                commit_batch_buffers
                    .after(update_panel_text_batches)
                    .after(write_batch_run_transforms)
                    .in_set(BatchResourcesReady),
            ),
        );
    }
}
