//! [`DiegeticPanel`] — the main panel component and its computed companion.

use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Add;
use bevy::prelude::AlphaMode;
use bevy::prelude::Camera;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::Has;
use bevy::prelude::In;
use bevy::prelude::Insert;
use bevy::prelude::On;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Res;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;

use super::PanelProjectionError;
use super::PanelScreenConversion;
use super::PanelScreenHandoff;
use super::PanelWorldConversion;
use super::ResolvedScreenPanelPosition;
use super::SavedPanelWorldState;
use super::anchoring;
use super::anchoring::PanelAttachment;
use super::apply_screen_conversion;
use super::apply_screen_root_sizing;
use super::apply_world_conversion;
use super::builder::DiegeticPanelBuilder;
use super::builder::NeedsSize;
use super::builder::PanelBuildError;
use super::builder::PanelEntity;
use super::builder::Screen;
use super::builder::WidgetEntity;
use super::builder::World;
use super::constants::PANEL_RESIZE_EPSILON;
use super::conversion;
use super::coordinate_space::CoordinateSpace;
use super::coordinate_space::PanelSpace;
use super::coordinate_space::ScreenPosition;
use super::events::LastPanelDimensions;
use super::field;
use super::field::PanelFieldRecord;
use super::lifecycle;
use super::precompose::PanelPrecomposeCache;
use super::validate_screen_conversion;
use super::validate_world_conversion;
use crate::cascade::Cascade;
use crate::cascade::CascadeAttribute;
use crate::cascade::FontUnit;
use crate::cascade::HdrTextCoverageBias;
use crate::cascade::PanelDefaults;
use crate::cascade::Resolved;
use crate::cascade::SdfMaterial;
use crate::cascade::ShapeMaterial;
use crate::cascade::TextAlpha;
use crate::cascade::TextMaterial;
use crate::layout::Anchor;
use crate::layout::BoundingBox;
use crate::layout::Dimension;
use crate::layout::InvalidSize;
use crate::layout::LayoutResult;
use crate::layout::LayoutTree;
use crate::layout::LayoutTreeChange;
use crate::layout::Lighting;
use crate::layout::PanelSize;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::layout::Sizing;
use crate::layout::TextStyle;
use crate::layout::Unit;
use crate::render::AntiAlias;
use crate::render::DrawOrder;
use crate::render::HairlineFade;
use crate::widgets;
use crate::widgets::ComputedTooltipRecord;
use crate::widgets::ComputedWidgetRecord;
use crate::widgets::PanelPicking;
use crate::widgets::PanelWidget;
use crate::widgets::PanelWidgetIndex;
use crate::widgets::TooltipControllerIndex;
use crate::widgets::WidgetDisabledAppearance;
use crate::widgets::WidgetFocusedAppearance;
use crate::widgets::WidgetHoveredAppearance;
use crate::widgets::WidgetInteractivity;
use crate::widgets::WidgetOf;
use crate::widgets::WidgetPressedAppearance;

/// Source tree plus the revision token used by derived tree caches.
#[derive(Clone, Default)]
pub(super) struct PanelTree {
    tree:     LayoutTree,
    revision: TreeRevision,
}

impl PanelTree {
    pub(super) const fn tree(&self) -> &LayoutTree { &self.tree }

    pub(super) const fn revision(&self) -> TreeRevision { self.revision }

    pub(super) const fn next_revision(&self) -> TreeRevision { self.revision.next() }

    fn replace(&mut self, tree: LayoutTree) {
        self.tree = tree;
        self.revision.bump();
    }

    fn set_element_text(&mut self, index: usize, text: &str) -> bool {
        if self.tree.set_element_text(index, text) {
            self.revision.bump();
            true
        } else {
            false
        }
    }

    fn set_element_style(&mut self, index: usize, style: TextStyle) -> bool {
        if self.tree.set_element_style(index, style) {
            self.revision.bump();
            true
        } else {
            false
        }
    }

    fn set_widget_interactivity(
        &mut self,
        id: &crate::PanelElementId,
        authored: Cascade<WidgetInteractivity>,
    ) -> bool {
        if self.tree.widget_interactivity(id) == Some(authored) {
            return false;
        }
        if self.tree.set_widget_interactivity(id, authored) {
            self.revision.bump();
            true
        } else {
            false
        }
    }

    const fn use_next_revision_after_replacement(&mut self, previous: &Self) {
        self.revision = previous.next_revision();
    }
}

impl From<LayoutTree> for PanelTree {
    fn from(tree: LayoutTree) -> Self {
        Self {
            tree,
            revision: TreeRevision::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TreeRevision(u64);

impl TreeRevision {
    const fn next(self) -> Self { Self(self.0.wrapping_add(1)) }

    const fn bump(&mut self) { *self = self.next(); }
}

impl From<TreeRevision> for u64 {
    fn from(value: TreeRevision) -> Self { value.0 }
}

/// A diegetic UI panel attached to a 3D entity.
///
/// Defines a layout tree and the panel's dimensions in layout units.
/// World-space size is computed automatically from the panel's
/// `layout_unit`. Font sizes in the tree are interpreted in `font_unit`
/// (defaults through [`PanelDefaults::panel_font_unit`]).
///
/// Construct via [`DiegeticPanel::world`] or [`DiegeticPanel::screen`]:
///
/// ```ignore
/// commands.spawn((
///     DiegeticPanel::world()
///         .size(Mm(210.0), Mm(297.0))
///         .world_height(0.5)
///         .layout(|b| {
///             b.text(("Hello", TextStyle::new(48.0)));
///         })
///         .build()?,
///     Transform::from_xyz(0.0, 0.0, 0.0),
/// ));
/// ```
///
/// The layout engine runs automatically when this component changes,
/// storing results in [`ComputedDiegeticPanel`].
///
/// Requires a [`Transform`] for world-space positioning.
#[derive(Clone, Component, Reflect)]
#[reflect(Component)]
#[require(
    ComputedDiegeticPanel,
    DiegeticPanelChangeClassification,
    LastPanelDimensions,
    PanelPrecomposeCache,
    PanelSpace,
    PanelWidgetIndex,
    TooltipControllerIndex,
    ResolvedScreenPanelPosition,
    ScaledLayoutTreeCache,
    Transform,
    Visibility
)]
pub struct DiegeticPanel {
    /// The layout tree defining this panel's UI structure.
    #[reflect(ignore)]
    tree:                                  PanelTree,
    /// Panel width in layout `Unit`s. Prefer [`set_size`](Self::set_size) for
    /// mutation to keep dimensions and unit in sync.
    pub(super) width:                      f32,
    /// Panel height in layout `Unit`s. Prefer [`set_size`](Self::set_size) for
    /// mutation to keep dimensions and unit in sync.
    pub(super) height:                     f32,
    /// Unit for `width`/`height`. Set automatically by
    /// [`DiegeticPanelBuilder::size`] or [`set_size`](Self::set_size).
    pub(super) layout_unit:                Unit,
    /// Construction seed for the panel's font-unit cascade.
    #[reflect(ignore)]
    pub(super) font_unit:                  Cascade<Unit>,
    /// Which point on the panel sits at the entity's [`Transform`] position.
    /// Defaults to [`Anchor::TopLeft`].
    pub(super) anchor:                     Anchor,
    /// Target world width in meters. When set, the panel is uniformly scaled
    /// so its width matches this value (height follows aspect ratio).
    /// If both `world_width` and `world_height` are set, non-uniform scaling
    /// is applied.
    pub(super) world_width:                Option<f32>,
    /// Target world height in meters. When set, the panel is uniformly scaled
    /// so its height matches this value (width follows aspect ratio).
    pub(super) world_height:               Option<f32>,
    /// Construction seed for the panel's shadow-casting cascade.
    #[reflect(ignore)]
    pub(super) shadow_casting:             Cascade<ShadowCasting>,
    /// Construction seed for the panel's hovered widget-appearance cascade.
    #[reflect(ignore)]
    pub(super) widget_hovered_appearance:  Cascade<WidgetHoveredAppearance>,
    /// Construction seed for the panel's pressed widget-appearance cascade.
    #[reflect(ignore)]
    pub(super) widget_pressed_appearance:  Cascade<WidgetPressedAppearance>,
    /// Construction seed for the panel's focused widget-appearance cascade.
    #[reflect(ignore)]
    pub(super) widget_focused_appearance:  Cascade<WidgetFocusedAppearance>,
    /// Construction seed for the panel's disabled widget-appearance cascade.
    #[reflect(ignore)]
    pub(super) widget_disabled_appearance: Cascade<WidgetDisabledAppearance>,
    /// Construction seed for the panel's SDF source-material cascade.
    ///
    /// Individual elements can override via `El::material`; `base_color` is
    /// overridden by the layout color when both are set.
    #[reflect(ignore)]
    pub(super) material:                   Cascade<Handle<StandardMaterial>>,
    /// Construction seed for the panel's text source-material cascade.
    ///
    /// `base_color` is overridden by `TextStyle::color` when set.
    #[reflect(ignore)]
    pub(super) text_material:              Cascade<Handle<StandardMaterial>>,
    /// Construction seed for the panel primitive source-material cascade.
    ///
    /// Shape-local colors override `base_color` before projection.
    #[reflect(ignore)]
    pub(super) shape_material:             Cascade<Handle<StandardMaterial>>,
    /// Construction seed for the panel's text [`AlphaMode`] cascade.
    #[reflect(ignore)]
    pub(super) text_alpha_mode:            Cascade<AlphaMode>,
    /// Construction seed for the panel's HDR text coverage-bias cascade.
    #[reflect(ignore)]
    pub(super) hdr_text_coverage_bias:     Cascade<f32>,
    /// Builder-provided initial value for the sibling [`PanelPicking`]
    /// component. Applied by
    /// [`sync_panel_picking_on_insert`] only when the entity has no live
    /// `PanelPicking`, so an existing component of any value — including an
    /// explicit [`PanelPicking::INTERACTIVE`] — stays authoritative. An
    /// installed value is recorded in `PanelComponentOwnership<PanelPicking>`
    /// and is removed with the panel role unless application code has
    /// replaced or mutated it.
    #[reflect(ignore)]
    pub(super) picking:                    PanelPicking,
    /// Whether the panel is world-space or screen-space.
    pub(super) coordinate_space:           CoordinateSpace,
    /// Maps each text run's [`PanelElementId`](crate::PanelElementId) to the entity
    /// reification created for it, so
    /// [`text_child`](Self::text_child) resolves a named run in O(1).
    ///
    /// `reify_text_entities` rebuilds this from scratch every pass and writes it
    /// without tripping change detection, so it never re-triggers layout. A
    /// non-identical [`set_tree`](DiegeticPanelCommands::set_tree) replacement
    /// clears it so a stale id stops resolving when the replacement applies.
    #[reflect(ignore)]
    pub(crate) text_index:                 HashMap<crate::PanelElementId, Entity>,
}

impl Default for DiegeticPanel {
    fn default() -> Self {
        Self {
            tree:                       PanelTree::default(),
            width:                      0.0,
            height:                     0.0,
            layout_unit:                Unit::Meters,
            font_unit:                  Cascade::Inherit,
            anchor:                     Anchor::TopLeft,
            world_width:                None,
            world_height:               None,
            shadow_casting:             Cascade::Inherit,
            widget_hovered_appearance:  Cascade::Inherit,
            widget_pressed_appearance:  Cascade::Inherit,
            widget_focused_appearance:  Cascade::Inherit,
            widget_disabled_appearance: Cascade::Inherit,
            material:                   Cascade::Inherit,
            text_material:              Cascade::Inherit,
            shape_material:             Cascade::Inherit,
            text_alpha_mode:            Cascade::Inherit,
            hdr_text_coverage_bias:     Cascade::Inherit,
            picking:                    PanelPicking::default(),
            coordinate_space:           CoordinateSpace::default(),
            text_index:                 HashMap::new(),
        }
    }
}

impl DiegeticPanel {
    pub(super) fn with_initial_tree(tree: LayoutTree) -> Self {
        Self {
            tree: PanelTree::from(tree),
            ..Self::default()
        }
    }
}

/// Installs the builder's initial [`PanelPicking`] on panel insert or replacement
/// when the entity has no live `PanelPicking` component. The deferred write
/// goes through `lifecycle::seed_owned_component`, which records the install
/// in `PanelComponentOwnership<PanelPicking>` so panel role teardown can tell
/// the seed apart from application state.
///
/// A live `PanelPicking` is authoritative regardless of its value: one supplied
/// explicitly in the spawn bundle — including [`PanelPicking::INTERACTIVE`],
/// which equals the default — installed by a screen-overlay opt-out, or written
/// at runtime is never overwritten, mutated, or recorded as Hana-owned, so
/// replacing the `DiegeticPanel` preserves the live policy and no equal-value
/// rewrite bumps the component's change tick. Absence is re-checked when the
/// deferred insert applies, so any component already installed by then wins.
///
/// `teardown_owned_shared_state` removes an unchanged initial value together with its
/// ownership record when the `DiegeticPanel` role is removed, so a later
/// re-add installs the new builder value. An application write afterward
/// moves the component's change tick off the recorded write; teardown then
/// keeps the component and drops only the stale ownership record.
pub(super) fn sync_panel_picking_on_insert(
    inserted: On<Insert, DiegeticPanel>,
    panels: Query<(&DiegeticPanel, Has<PanelPicking>)>,
    mut commands: Commands,
) {
    let entity = inserted.entity;
    let Ok((panel, has_live_picking)) = panels.get(entity) else {
        return;
    };
    if has_live_picking {
        return;
    }
    let seed = panel.picking;
    commands.queue(move |world: &mut bevy::ecs::world::World| {
        lifecycle::seed_owned_component(world, entity, entity, seed);
    });
}

// ── Public read-only accessors ──────────────────────────────────────────────

impl DiegeticPanel {
    /// Returns a reference to the layout tree.
    #[must_use]
    pub const fn tree(&self) -> &LayoutTree { self.tree.tree() }

    /// Revision of the current layout tree.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn tree_revision(&self) -> TreeRevision { self.tree.revision() }

    /// Returns the revision-owned tree source used by derived caches.
    #[must_use]
    pub(super) const fn tree_source(&self) -> &PanelTree { &self.tree }

    /// Returns the revision a successful tree replacement will produce.
    #[must_use]
    pub(crate) const fn next_tree_revision(&self) -> TreeRevision { self.tree.next_revision() }

    /// Panel width in layout units.
    #[must_use]
    pub const fn width(&self) -> f32 { self.width }

    /// Panel height in layout units.
    #[must_use]
    pub const fn height(&self) -> f32 { self.height }

    /// The layout unit for this panel's dimensions.
    #[must_use]
    pub const fn layout_unit(&self) -> Unit { self.layout_unit }

    /// The panel's anchor point.
    #[must_use]
    pub const fn anchor(&self) -> Anchor { self.anchor }

    /// The panel's coordinate space (world or screen).
    #[must_use]
    pub const fn coordinate_space(&self) -> &CoordinateSpace { &self.coordinate_space }

    pub(super) const fn authored_world_width(&self) -> Option<f32> { self.world_width }

    pub(super) const fn authored_world_height(&self) -> Option<f32> { self.world_height }

    /// Resolves a text run's [`PanelElementId`](crate::PanelElementId) to the entity
    /// reification created for it (its `line_index == 0` child), or `None` if
    /// no run carries that id.
    ///
    /// This is an **unchecked** index read: it returns the stored `Entity` as-is.
    /// The method takes `&self` with no `World`/`Entities` access, so it cannot
    /// confirm the entity is still alive — an out-of-flow `despawn` would leave a
    /// stale mapping until the next reification rebuilds the index. Liveness is
    /// validated one layer up by the [`PanelText`](crate::PanelText) `SystemParam`,
    /// whose `Query::get` on the returned entity yields `None` for a dead child.
    ///
    /// A non-identical `set_tree` clears the index when its deferred replacement
    /// applies, so a lookup before the next reification pass returns `None`.
    /// Identical replacements retain the index because they schedule no
    /// reification pass.
    #[must_use]
    pub fn text_child(&self, id: &crate::PanelElementId) -> Option<Entity> {
        self.text_index.get(id).copied()
    }
}

// ── Public mutators ─────────────────────────────────────────────────────────

impl DiegeticPanel {
    /// Atomically updates the panel's width, height, and layout unit.
    ///
    /// This is the preferred way to resize a panel at runtime (e.g. for
    /// animation) because it keeps dimensions and unit in sync.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidSize`] if either dimension is zero or negative.
    pub fn set_size(&mut self, size: impl PanelSize) -> Result<(), InvalidSize> {
        let (w, h, unit) = size.dimensions();
        if w <= 0.0 || h <= 0.0 {
            return Err(InvalidSize {
                width:  w,
                height: h,
            });
        }
        self.width = w;
        self.height = h;
        self.layout_unit = unit;
        Ok(())
    }

    /// Replaces the layout tree and clears the text-run index, so the next layout
    /// pass runs a full solve.
    ///
    /// A method on the component alone cannot write the sibling
    /// change-classification component, so a visual-only edit that skips the full
    /// solve goes through [`DiegeticPanelCommands::set_tree`] instead.
    pub(crate) fn replace_tree_full_rebuild(&mut self, tree: LayoutTree) {
        self.tree.replace(tree);
        self.text_index.clear();
    }

    fn replace_classified_tree(&mut self, tree: LayoutTree, change: LayoutTreeChange) {
        if change == LayoutTreeChange::Identical {
            self.tree.replace(tree);
        } else {
            self.replace_tree_full_rebuild(tree);
        }
    }

    fn replace_from_precompose_helper(&mut self, panel: Self) {
        let mut panel = panel;
        panel.tree.use_next_revision_after_replacement(&self.tree);
        panel.text_index.clear();
        // `apply_precompose_helper_panel` replaces structural panel data only;
        // the assignments below retain the existing entity's cascade seeds.
        panel.font_unit = self.font_unit;
        panel.shadow_casting = self.shadow_casting;
        panel
            .widget_hovered_appearance
            .clone_from(&self.widget_hovered_appearance);
        panel
            .widget_pressed_appearance
            .clone_from(&self.widget_pressed_appearance);
        panel
            .widget_focused_appearance
            .clone_from(&self.widget_focused_appearance);
        panel
            .widget_disabled_appearance
            .clone_from(&self.widget_disabled_appearance);
        panel.material.clone_from(&self.material);
        panel.text_material.clone_from(&self.text_material);
        panel.shape_material.clone_from(&self.shape_material);
        panel.text_alpha_mode = self.text_alpha_mode;
        panel.hdr_text_coverage_bias = self.hdr_text_coverage_bias;
        panel.picking = self.picking;
        *self = panel;
    }

    /// Bench-support wrapper for [`Self::replace_tree_full_rebuild`].
    #[cfg(feature = "bench_support")]
    #[doc(hidden)]
    pub fn set_tree_full_rebuild(&mut self, tree: LayoutTree) {
        self.replace_tree_full_rebuild(tree);
    }

    /// Writes a run-text edit into the authoritative `El.text` tree and bumps the
    /// edit-session revision. Returns whether the cache changed.
    ///
    /// `El.text` is the single source for run text; the child
    /// [`TextContent`](crate::TextContent) is derived output — reification overwrites
    /// it from the tree each frame. The public edit path (`PanelText` / `DiegeticTextMut`
    /// via `TextEdit`) calls this to change a run's string. Skips the revision bump
    /// when the string is unchanged, avoiding a layout pass and a cache lookup.
    pub(crate) fn sync_run_text_cache(&mut self, index: usize, text: &str) -> bool {
        self.tree.set_element_text(index, text)
    }

    /// Writes a restyle into the authored `El.config` and bumps the edit-session
    /// revision. Returns whether the style changed.
    ///
    /// Like run text (see [`sync_run_text_cache`](Self::sync_run_text_cache)),
    /// the tree is the single authoritative source: `El.config` for style,
    /// `El.text` for the string, while the run child is derived output reification
    /// rewrites. A label restyle (font, size) mutates `El.config` through this
    /// method; the relayout it triggers flows the new config to the run via
    /// reification, so measurement and rendering stay on the same source. Skips the
    /// revision bump (and so the layout) when the style already matches.
    pub(crate) fn restyle_run(&mut self, index: usize, style: TextStyle) -> bool {
        self.tree.set_element_style(index, style)
    }

    pub(crate) fn set_widget_interactivity(
        &mut self,
        id: &crate::PanelElementId,
        authored: Cascade<WidgetInteractivity>,
    ) -> bool {
        self.tree.set_widget_interactivity(id, authored)
    }

    /// Sets the panel width directly (in layout units).
    ///
    /// Used by the screen-space positioning system to resize panels
    /// whose dimensions are derived from the window.
    pub const fn set_width(&mut self, width: f32) { self.width = width; }

    /// Sets the panel height directly (in layout units).
    pub const fn set_height(&mut self, height: f32) { self.height = height; }
}

// ── Builder entry points ────────────────────────────────────────────────────

impl DiegeticPanel {
    /// Returns a builder for a world-space panel.
    ///
    /// Bare floats in `.size()` default to [`Unit::Meters`].
    #[must_use]
    pub fn world() -> DiegeticPanelBuilder<World, NeedsSize> { DiegeticPanelBuilder::new_world() }

    /// Returns a builder for a screen-space panel.
    ///
    /// Bare floats in `.size()` default to [`Unit::Pixels`]. Screen panels
    /// author [`AlphaMode::Blend`] for stable overlay text instead of inheriting
    /// an application's world-text alpha default. Call
    /// [`CascadeEntityCommandsExt::inherit_text_alpha`](crate::CascadeEntityCommandsExt::inherit_text_alpha)
    /// after spawning to opt a screen panel into that global cascade.
    #[must_use]
    pub fn screen() -> DiegeticPanelBuilder<Screen, NeedsSize> {
        DiegeticPanelBuilder::new_screen()
    }
}

// ── Computation methods ─────────────────────────────────────────────────────

impl DiegeticPanel {
    /// Physical width in meters before world scaling.
    fn physical_width(&self) -> f32 { self.width * self.layout_unit.meters_per_unit() }

    /// Physical height in meters before world scaling.
    fn physical_height(&self) -> f32 { self.height * self.layout_unit.meters_per_unit() }

    /// Panel width in meters (world units), incorporating `world_width`
    /// and `world_height` scaling.
    ///
    /// - `world_width` set: returns `world_width`.
    /// - `world_height` only: uniform scale from height, width follows aspect ratio.
    /// - Neither: physical size from layout units.
    #[must_use]
    pub fn world_width(&self) -> f32 {
        let physical_width = self.physical_width();
        let physical_height = self.physical_height();
        match (self.world_width, self.world_height) {
            (Some(target_width), _) => target_width,
            (None, Some(target_height)) => {
                if physical_height > 0.0 {
                    physical_width * (target_height / physical_height)
                } else {
                    physical_width
                }
            },
            (None, None) => physical_width,
        }
    }

    /// Panel height in meters (world units), incorporating `world_width`
    /// and `world_height` scaling.
    ///
    /// - `world_height` set: returns `world_height`.
    /// - `world_width` only: uniform scale from width, height follows aspect ratio.
    /// - Neither: physical size from layout units.
    #[must_use]
    pub fn world_height(&self) -> f32 {
        let physical_width = self.physical_width();
        let physical_height = self.physical_height();
        match (self.world_width, self.world_height) {
            (_, Some(target_height)) => target_height,
            (Some(target_width), None) => {
                if physical_width > 0.0 {
                    physical_height * (target_width / physical_width)
                } else {
                    physical_height
                }
            },
            (None, None) => physical_height,
        }
    }

    /// Returns the (`x_offset`, `y_offset`) in world meters for converting
    /// layout coordinates (top-left origin, Y-down) to panel-local
    /// coordinates relative to the anchor point.
    ///
    /// Layout local position = `(layout_x * scale - x_offset, -layout_y * scale + y_offset)`
    /// where `scale` is `points_to_world()`.
    ///
    /// Screen panels render under an orthographic camera where 1 world
    /// unit = 1 logical pixel, so anchor offsets are computed directly
    /// from `panel.width`/`panel.height` (in layout units = pixels).
    /// This keeps anchor positioning correct for dynamic-sized screen
    /// panels (`Sizing::Fit` / `Sizing::Grow` / `Sizing::Percent`) whose
    /// `world_width` / `world_height` fields are left unset at build
    /// time.
    #[must_use]
    pub fn anchor_offsets(&self) -> (f32, f32) {
        let (fx, fy) = self.anchor.offset_fraction();
        if self.coordinate_space.is_screen() {
            return (self.width * fx, self.height * fy);
        }
        (self.world_width() * fx, self.world_height() * fy)
    }

    /// Conversion factor from layout points to world meters.
    ///
    /// The layout engine works in points internally. Multiply a layout-space
    /// value (in points) by this factor to get world meters. Incorporates
    /// `world_width`/`world_height` scaling.
    #[must_use]
    pub fn points_to_world(&self) -> f32 {
        // Screen panels render under an orthographic camera where 1 world
        // unit = 1 logical pixel. The layout engine scales dimensions by
        // `layout_unit.to_points()`; reversing that factor returns values
        // to the layout unit (which equals world units for screen panels),
        // independent of panel height. This keeps points_to_world stable
        // across dynamic `Sizing::Fit` / `Sizing::Grow` sizing where
        // `panel.height` is recomputed each frame.
        if self.coordinate_space.is_screen() {
            let to_pts = self.layout_unit.to_points();
            if to_pts > 0.0 {
                return 1.0 / to_pts;
            }
        }
        let viewport_points_height = self.height * self.layout_unit.to_points();
        let world_height = self.world_height();
        if viewport_points_height > 0.0 {
            world_height / viewport_points_height
        } else {
            Unit::Points.meters_per_unit()
        }
    }

    /// Font-to-layout conversion factor for this panel.
    ///
    /// Multiply a font size by this value to convert from font units
    /// to layout units. Callers pass the panel's resolved font unit —
    /// read from `Resolved<FontUnit>` on the panel
    /// entity (every panel carries one, seeded from `font_unit` or
    /// [`PanelDefaults::panel_font_unit`]).
    #[must_use]
    pub fn font_scale(&self, panel_font_unit: Unit) -> f32 {
        let font_meters_per_unit = panel_font_unit.meters_per_unit();
        let layout_meters_per_unit = self.layout_unit.meters_per_unit();
        font_meters_per_unit / layout_meters_per_unit
    }
}

/// Extension methods for mutating diegetic panels through [`Commands`].
///
/// Attachment and conversion operations queued on one `Commands` value apply in
/// call order. Every operation validates the live panel and attachment graph at
/// execution; a conflict emits a warning and leaves that operation's state
/// unchanged.
///
/// `PanelEntity<World>` accepts only world panel or widget targets, and
/// `PanelEntity<Screen>` accepts only screen targets.
///
/// ```compile_fail
/// use bevy::prelude::Commands;
/// use hana_diegetic::{Anchor, DiegeticPanelCommands, PanelAttachment};
/// use hana_diegetic::{PanelEntity, Screen, World};
///
/// fn cross_space_panel(
///     mut commands: Commands<'_, '_>,
///     source: PanelEntity<World>,
///     target: PanelEntity<Screen>,
/// ) {
///     let authored = PanelAttachment::new(Anchor::Center, Anchor::Center);
///     commands.attach_to_panel(source, target, authored);
/// }
/// ```
///
/// ```compile_fail
/// use bevy::prelude::Commands;
/// use hana_diegetic::{DiegeticPanelCommands, PanelEntity, Screen, WidgetEntity, World};
///
/// fn cross_space_widget_retarget(
///     mut commands: Commands<'_, '_>,
///     source: PanelEntity<Screen>,
///     target: WidgetEntity<World>,
/// ) {
///     commands.retarget_to_widget(source, target);
/// }
/// ```
pub trait DiegeticPanelCommands {
    /// Queues a layout-tree replacement that records whether the change is
    /// visual-only or layout-affecting.
    ///
    /// The queued setter is deferred. Schedule systems that call this before
    /// panel layout systems when the update must be visible in the same frame.
    ///
    /// `Ok(())` means the tree passed synchronous validation and its replacement
    /// was queued. Because [`Commands`] is deferred, it does not guarantee that
    /// `entity` still exists when the queued replacement applies.
    ///
    /// # Errors
    ///
    /// Returns [`PanelBuildError`] when the tree contains a duplicate id or an
    /// invalid widget declaration. A rejected tree queues no replacement.
    fn set_tree(&mut self, entity: Entity, tree: LayoutTree) -> Result<(), PanelBuildError>;

    /// Attaches `source` to a panel in the same coordinate space.
    fn attach_to_panel<Space>(
        &mut self,
        source: PanelEntity<Space>,
        target: PanelEntity<Space>,
        authored: PanelAttachment,
    );

    /// Attaches `source` to a widget in the same coordinate space.
    fn attach_to_widget<Space>(
        &mut self,
        source: PanelEntity<Space>,
        target: WidgetEntity<Space>,
        authored: PanelAttachment,
    );

    /// Retargets an existing attachment to a panel in the same coordinate space.
    fn retarget_to_panel<Space>(&mut self, source: PanelEntity<Space>, target: PanelEntity<Space>);

    /// Retargets an existing attachment to a widget in the same coordinate space.
    fn retarget_to_widget<Space>(
        &mut self,
        source: PanelEntity<Space>,
        target: WidgetEntity<Space>,
    );

    /// Detaches `source` and removes its authored offset.
    fn detach<Space>(&mut self, source: PanelEntity<Space>);

    /// Prepares a world panel for an animated screen handoff without claiming a
    /// screen-space identity.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when `conversion` is invalid. Live panel
    /// and attachment state is checked when Bevy applies the operation.
    fn begin_to_screen<C>(
        &mut self,
        panel: PanelEntity<World>,
        camera: Entity,
        conversion: C,
    ) -> Result<(), PanelProjectionError>
    where
        C: Into<PanelScreenConversion>;

    /// Finishes an animated world-to-screen handoff.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when `conversion` is invalid. Live panel
    /// and attachment state is checked when Bevy applies the operation.
    fn finish_to_screen<C>(
        &mut self,
        panel: PanelEntity<World>,
        camera: Entity,
        conversion: C,
    ) -> Result<(), PanelProjectionError>
    where
        C: Into<PanelScreenConversion>;

    /// Queues a resolved world-to-screen conversion without saving a handoff
    /// camera.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when `conversion` is invalid. Live panel
    /// and attachment state is checked when Bevy applies the operation.
    fn apply_to_screen<C>(
        &mut self,
        panel: PanelEntity<World>,
        conversion: C,
    ) -> Result<(), PanelProjectionError>
    where
        C: Into<PanelScreenConversion>;

    /// Queues a resolved screen-to-world conversion.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when `conversion` is invalid. Live panel
    /// and attachment state is checked when Bevy applies the operation.
    fn apply_to_world<C>(
        &mut self,
        panel: PanelEntity<Screen>,
        conversion: C,
    ) -> Result<(), PanelProjectionError>
    where
        C: Into<PanelWorldConversion>;
}

#[derive(Clone)]
pub(super) enum PanelConversionOperation {
    BeginScreen {
        panel:      Entity,
        camera:     Entity,
        conversion: PanelScreenConversion,
    },
    Screen {
        panel:      Entity,
        camera:     Option<Entity>,
        conversion: PanelScreenConversion,
    },
    World {
        panel:      Entity,
        conversion: PanelWorldConversion,
    },
}

impl PanelConversionOperation {
    const fn panel(&self) -> Entity {
        match self {
            Self::BeginScreen { panel, .. }
            | Self::Screen { panel, .. }
            | Self::World { panel, .. } => *panel,
        }
    }

    const fn expected_space(&self) -> PanelSpace {
        match self {
            Self::BeginScreen { .. } | Self::Screen { .. } => PanelSpace::World,
            Self::World { .. } => PanelSpace::Screen,
        }
    }

    const fn name(&self) -> &'static str {
        match self {
            Self::BeginScreen { .. } => "begin world-to-screen conversion",
            Self::Screen {
                camera: Some(_), ..
            } => "finish world-to-screen conversion",
            Self::Screen { camera: None, .. } => "world-to-screen conversion",
            Self::World { .. } => "screen-to-world conversion",
        }
    }

    const fn camera(&self) -> Option<Entity> {
        match self {
            Self::BeginScreen { camera, .. } => Some(*camera),
            Self::Screen { camera, .. } => *camera,
            Self::World { .. } => None,
        }
    }
}

#[derive(Component, Clone, Copy)]
pub(super) struct PreparedPanelScreenConversion {
    size: Vec2,
}

impl PreparedPanelScreenConversion {
    fn matches(self, size: Vec2) -> bool {
        (self.size.x - size.x).abs() <= PANEL_RESIZE_EPSILON
            && (self.size.y - size.y).abs() <= PANEL_RESIZE_EPSILON
    }
}

#[derive(Clone, Copy)]
struct ScreenConversionSource<'a> {
    defaults:           &'a PanelDefaults,
    font_resolved:      Option<&'a Resolved<FontUnit>>,
    lighting_resolved:  Option<&'a Resolved<Lighting>>,
    sidedness_resolved: Option<&'a Resolved<Sidedness>>,
    render_layers:      Option<&'a RenderLayers>,
}

impl ScreenConversionSource<'_> {
    fn font_unit(self) -> Unit {
        self.font_resolved
            .map_or(self.defaults.panel_font_unit, |resolved| resolved.0.0)
    }

    fn saved_world_state(
        self,
        panel: &DiegeticPanel,
        transform: &Transform,
    ) -> SavedPanelWorldState {
        conversion::saved_world_state_from_panel(
            panel,
            transform,
            self.font_unit(),
            self.lighting_resolved
                .map_or(Lighting::Lit, |resolved| resolved.0),
            self.sidedness_resolved
                .map_or(Sidedness::BothSides, |resolved| resolved.0),
            self.render_layers,
        )
    }
}

impl DiegeticPanelCommands for Commands<'_, '_> {
    fn set_tree(&mut self, entity: Entity, tree: LayoutTree) -> Result<(), PanelBuildError> {
        widgets::validate_tree(&tree)?;
        self.run_system_cached_with(set_tree_command, (entity, tree));
        Ok(())
    }

    fn attach_to_panel<Space>(
        &mut self,
        source: PanelEntity<Space>,
        target: PanelEntity<Space>,
        authored: PanelAttachment,
    ) {
        anchoring::queue_attach_to_panel(self, source, target, authored);
    }

    fn attach_to_widget<Space>(
        &mut self,
        source: PanelEntity<Space>,
        target: WidgetEntity<Space>,
        authored: PanelAttachment,
    ) {
        anchoring::queue_attach_to_widget(self, source, target, authored);
    }

    fn retarget_to_panel<Space>(&mut self, source: PanelEntity<Space>, target: PanelEntity<Space>) {
        anchoring::queue_retarget_to_panel(self, source, target);
    }

    fn retarget_to_widget<Space>(
        &mut self,
        source: PanelEntity<Space>,
        target: WidgetEntity<Space>,
    ) {
        anchoring::queue_retarget_to_widget(self, source, target);
    }

    fn detach<Space>(&mut self, source: PanelEntity<Space>) {
        anchoring::queue_detach(self, source);
    }

    fn begin_to_screen<C>(
        &mut self,
        panel: PanelEntity<World>,
        camera: Entity,
        conversion: C,
    ) -> Result<(), PanelProjectionError>
    where
        C: Into<PanelScreenConversion>,
    {
        let conversion = conversion.into();
        validate_screen_conversion(&conversion)?;
        self.run_system_cached_with(
            apply_panel_conversion_operation,
            PanelConversionOperation::BeginScreen {
                panel: panel.entity(),
                camera,
                conversion,
            },
        );
        Ok(())
    }

    fn finish_to_screen<C>(
        &mut self,
        panel: PanelEntity<World>,
        camera: Entity,
        conversion: C,
    ) -> Result<(), PanelProjectionError>
    where
        C: Into<PanelScreenConversion>,
    {
        let conversion = conversion.into();
        validate_screen_conversion(&conversion)?;
        self.run_system_cached_with(
            apply_panel_conversion_operation,
            PanelConversionOperation::Screen {
                panel: panel.entity(),
                camera: Some(camera),
                conversion,
            },
        );
        Ok(())
    }

    fn apply_to_screen<C>(
        &mut self,
        panel: PanelEntity<World>,
        conversion: C,
    ) -> Result<(), PanelProjectionError>
    where
        C: Into<PanelScreenConversion>,
    {
        let conversion = conversion.into();
        validate_screen_conversion(&conversion)?;
        self.run_system_cached_with(
            apply_panel_conversion_operation,
            PanelConversionOperation::Screen {
                panel: panel.entity(),
                camera: None,
                conversion,
            },
        );
        Ok(())
    }

    fn apply_to_world<C>(
        &mut self,
        panel: PanelEntity<Screen>,
        conversion: C,
    ) -> Result<(), PanelProjectionError>
    where
        C: Into<PanelWorldConversion>,
    {
        let conversion = conversion.into();
        validate_world_conversion(&conversion)?;
        self.run_system_cached_with(
            apply_panel_conversion_operation,
            PanelConversionOperation::World {
                panel: panel.entity(),
                conversion,
            },
        );
        Ok(())
    }
}

fn set_tree_command(
    In((entity, next_tree)): In<(Entity, LayoutTree)>,
    mut panels: Query<(
        &mut DiegeticPanel,
        &mut DiegeticPanelChangeClassification,
        &mut PanelWidgetIndex,
    )>,
) {
    let Ok((mut panel, mut classification, mut widget_index)) = panels.get_mut(entity) else {
        return;
    };
    let change = panel.tree().classify_change(&next_tree);
    classification.record_tree_change(change);
    panel.replace_classified_tree(next_tree, change);
    if change != LayoutTreeChange::Identical {
        widget_index.clear();
    }
}

pub(crate) fn apply_precompose_helper_panel(
    In((entity, next_panel)): In<(Entity, DiegeticPanel)>,
    mut panels: Query<(
        &mut DiegeticPanel,
        &mut DiegeticPanelChangeClassification,
        &mut PanelWidgetIndex,
    )>,
) {
    let Ok((mut panel, mut classification, mut widget_index)) = panels.get_mut(entity) else {
        return;
    };
    classification.record_tree_change(LayoutTreeChange::LayoutAffecting);
    panel.replace_from_precompose_helper(next_panel);
    widget_index.clear();
}

pub(super) fn apply_panel_conversion_operation(
    In(operation): In<PanelConversionOperation>,
    defaults: Res<PanelDefaults>,
    mut commands: Commands,
    primary: Query<Entity, With<PrimaryWindow>>,
    windows: Query<&Window>,
    cameras: Query<&GlobalTransform, With<Camera>>,
    attachments: Query<(Entity, &super::PanelAttachmentAuthored)>,
    widgets: Query<&WidgetOf, With<PanelWidget>>,
    mut panels: Query<(
        &mut DiegeticPanel,
        &mut PanelWidgetIndex,
        &mut Transform,
        &mut DiegeticPanelChangeClassification,
        &mut ResolvedScreenPanelPosition,
        Option<&Resolved<FontUnit>>,
        Option<&Resolved<Lighting>>,
        Option<&Resolved<Sidedness>>,
        Option<&SavedPanelWorldState>,
        Option<&PreparedPanelScreenConversion>,
        Option<&RenderLayers>,
    )>,
) {
    let entity = operation.panel();
    let Ok((
        mut panel,
        mut widget_index,
        mut transform,
        mut classification,
        mut resolved_position,
        font_resolved,
        lighting_resolved,
        sidedness_resolved,
        saved,
        prepared_screen,
        source_render_layers,
    )) = panels.get_mut(entity)
    else {
        warn_rejected_conversion(
            &operation,
            "panel is missing or conversion state is incomplete",
        );
        return;
    };
    if PanelSpace::from(panel.coordinate_space()) != operation.expected_space() {
        warn_rejected_conversion(&operation, "panel coordinate space changed");
        return;
    }
    if let Some(rejection) = conversion_attachment_rejection(entity, &attachments, &widgets) {
        warn_rejected_conversion_conflict(&operation, rejection);
        return;
    }

    let source = ScreenConversionSource {
        defaults: &defaults,
        font_resolved,
        lighting_resolved,
        sidedness_resolved,
        render_layers: source_render_layers,
    };
    match operation {
        PanelConversionOperation::BeginScreen {
            camera, conversion, ..
        } => {
            begin_panel_to_screen_now(
                entity,
                camera,
                conversion,
                &mut commands,
                &cameras,
                &mut panel,
                &mut widget_index,
                &transform,
                &mut classification,
                saved,
                source,
            );
        },
        PanelConversionOperation::Screen {
            camera, conversion, ..
        } => {
            apply_panel_screen_conversion_now(
                entity,
                camera,
                conversion,
                &mut commands,
                &primary,
                &windows,
                &cameras,
                &mut panel,
                &mut widget_index,
                &mut transform,
                &mut resolved_position,
                source,
                prepared_screen,
            );
        },
        PanelConversionOperation::World { conversion, .. } => {
            apply_panel_world_conversion_now(
                entity,
                conversion,
                &mut commands,
                &mut panel,
                &mut widget_index,
                &mut transform,
                &mut classification,
                saved,
                &mut resolved_position,
            );
        },
    }
}

fn conversion_attachment_rejection(
    panel: Entity,
    attachments: &Query<(Entity, &super::PanelAttachmentAuthored)>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
) -> Option<PanelConversionAttachmentConflict> {
    if let Ok((_, authored)) = attachments.get(panel) {
        return Some(PanelConversionAttachmentConflict::Outgoing {
            target: authored.target(),
        });
    }
    for (source, authored) in attachments {
        let target = authored.target();
        if target == panel {
            return Some(PanelConversionAttachmentConflict::IncomingPanel { source });
        }
        if widgets
            .get(target)
            .is_ok_and(|widget_of| widget_of.panel() == panel)
        {
            return Some(PanelConversionAttachmentConflict::IncomingWidget {
                source,
                widget: target,
            });
        }
    }
    None
}

#[derive(Clone, Copy)]
enum PanelConversionAttachmentConflict {
    Outgoing { target: Entity },
    IncomingPanel { source: Entity },
    IncomingWidget { source: Entity, widget: Entity },
}

fn warn_rejected_conversion(operation: &PanelConversionOperation, reason: &str) {
    let panel = operation.panel();
    if let Some(camera) = operation.camera() {
        warn!(
            "panel operation `{}` rejected for panel {panel:?} and camera {camera:?}: {reason}",
            operation.name(),
        );
    } else {
        warn!(
            "panel operation `{}` rejected for panel {panel:?}: {reason}",
            operation.name(),
        );
    }
}

fn warn_rejected_conversion_conflict(
    operation: &PanelConversionOperation,
    conflict: PanelConversionAttachmentConflict,
) {
    let panel = operation.panel();
    match conflict {
        PanelConversionAttachmentConflict::Outgoing { target } => warn!(
            "panel operation `{}` rejected for panel {panel:?}: outgoing attachment targets \
             {target:?}",
            operation.name(),
        ),
        PanelConversionAttachmentConflict::IncomingPanel { source } => warn!(
            "panel operation `{}` rejected for panel {panel:?}: attachment source {source:?} \
             targets the panel",
            operation.name(),
        ),
        PanelConversionAttachmentConflict::IncomingWidget { source, widget } => warn!(
            "panel operation `{}` rejected for panel {panel:?}: attachment source {source:?} \
             targets owned widget {widget:?}",
            operation.name(),
        ),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "screen conversion applies one coordinated ECS handoff"
)]
fn apply_panel_screen_conversion_now(
    entity: Entity,
    camera: Option<Entity>,
    conversion: PanelScreenConversion,
    commands: &mut Commands<'_, '_>,
    primary: &Query<Entity, With<PrimaryWindow>>,
    windows: &Query<&Window>,
    cameras: &Query<&GlobalTransform, With<Camera>>,
    panel: &mut DiegeticPanel,
    widget_index: &mut PanelWidgetIndex,
    transform: &mut Transform,
    resolved_position: &mut ResolvedScreenPanelPosition,
    source: ScreenConversionSource<'_>,
    prepared_screen: Option<&PreparedPanelScreenConversion>,
) {
    if let Err(error) = validate_screen_conversion(&conversion) {
        warn!("failed to convert panel {entity:?} to screen space: {error}");
        return;
    }
    let was_world_panel = !panel.coordinate_space().is_screen();
    let rotation = conversion.rotation;
    let anchor_position = conversion.anchor_position;
    let render_layers = conversion.render_layers.clone();
    let handoff_conversion = conversion.clone();
    let window_size = screen_conversion_window_size(&conversion, primary, windows);
    let prepared = prepared_screen.is_some_and(|prepared| prepared.matches(conversion.size));
    if was_world_panel
        && !prepared
        && !prepare_world_panel_for_screen_conversion(
            entity,
            &conversion,
            commands,
            panel,
            widget_index,
            transform,
            None,
            source,
        )
    {
        return;
    }
    let handoff = camera
        .and_then(|camera| {
            cameras
                .get(camera)
                .ok()
                .map(|transform| (camera, transform))
        })
        .and_then(|(camera, camera_transform)| {
            screen_handoff(camera, handoff_conversion, camera_transform, transform)
        });
    if let Err(error) = apply_screen_conversion(panel, conversion) {
        warn!("failed to convert panel {entity:?} to screen space: {error}");
        return;
    }
    transform.translation.z = 0.0;
    if let Some(window_size) = window_size {
        let half_size = window_size * 0.5;
        transform.translation.x = anchor_position.x - half_size.x;
        transform.translation.y = half_size.y - anchor_position.y;
    }
    transform.rotation = Quat::from_rotation_z(rotation);
    transform.scale = Vec3::ONE;
    *resolved_position = ResolvedScreenPanelPosition::default();
    commands.entity(entity).insert(PanelSpace::Screen);
    lifecycle::write_owned_render_layers(commands, entity, entity, Some(render_layers));
    if let Some(handoff) = handoff.filter(|_| was_world_panel) {
        commands.entity(entity).insert(handoff);
    }
    if was_world_panel {
        commands
            .entity(entity)
            .remove::<PreparedPanelScreenConversion>();
    }
}

fn begin_panel_to_screen_now(
    entity: Entity,
    camera: Entity,
    conversion: PanelScreenConversion,
    commands: &mut Commands<'_, '_>,
    cameras: &Query<&GlobalTransform, With<Camera>>,
    panel: &mut DiegeticPanel,
    widget_index: &mut PanelWidgetIndex,
    transform: &Transform,
    classification: &mut DiegeticPanelChangeClassification,
    saved: Option<&SavedPanelWorldState>,
    source: ScreenConversionSource<'_>,
) {
    if let Err(error) = validate_screen_conversion(&conversion) {
        warn!("failed to prepare panel {entity:?} for screen conversion: {error}");
        return;
    }
    if panel.coordinate_space().is_screen() {
        return;
    }
    if prepare_world_panel_for_screen_conversion(
        entity,
        &conversion,
        commands,
        panel,
        widget_index,
        transform,
        saved,
        source,
    ) {
        if let Ok(camera_transform) = cameras.get(camera)
            && let Some(handoff) = screen_handoff(camera, conversion, camera_transform, transform)
        {
            commands.entity(entity).insert(handoff);
        }
        classification.record_tree_change(LayoutTreeChange::LayoutAffecting);
    }
}

fn prepare_world_panel_for_screen_conversion(
    entity: Entity,
    conversion: &PanelScreenConversion,
    commands: &mut Commands<'_, '_>,
    panel: &mut DiegeticPanel,
    widget_index: &mut PanelWidgetIndex,
    transform: &Transform,
    saved: Option<&SavedPanelWorldState>,
    source: ScreenConversionSource<'_>,
) -> bool {
    let old_world_width = panel.world_width();
    let old_world_height = panel.world_height();
    let old_points_to_world = panel.points_to_world();
    let points_to_pixels = old_points_to_world * (conversion.size.y / old_world_height);
    if !points_to_pixels.is_finite() || points_to_pixels <= 0.0 {
        warn!("failed to scale panel {entity:?} tree for screen conversion: invalid source scale");
        return false;
    }

    if saved.is_none() {
        commands
            .entity(entity)
            .insert(source.saved_world_state(panel, transform));
    }

    let mut tree = panel.tree().screen_source_scaled(
        panel.layout_unit().to_points(),
        source.font_unit().to_points(),
        points_to_pixels,
    );
    apply_screen_root_sizing(&mut tree, conversion.width, conversion.height);
    panel.replace_tree_full_rebuild(tree);
    widget_index.clear();
    panel.width = conversion.size.x;
    panel.height = conversion.size.y;
    panel.layout_unit = Unit::Pixels;
    panel.world_width = Some(old_world_width);
    panel.world_height = Some(old_world_height);
    panel.coordinate_space = CoordinateSpace::World {
        width:  fixed_pixel_sizing(conversion.size.x),
        height: fixed_pixel_sizing(conversion.size.y),
    };
    commands
        .entity(entity)
        .insert(PreparedPanelScreenConversion {
            size: conversion.size,
        });
    lifecycle::write_owned_cascade(
        commands,
        entity,
        entity,
        Cascade::Override(FontUnit(Unit::Pixels)),
    );
    lifecycle::write_owned_cascade(commands, entity, entity, Cascade::Override(Lighting::Unlit));
    lifecycle::write_owned_cascade(
        commands,
        entity,
        entity,
        Cascade::Override(Sidedness::FrontOnly),
    );
    true
}

fn screen_handoff(
    camera: Entity,
    conversion: PanelScreenConversion,
    camera_transform: &GlobalTransform,
    panel_transform: &Transform,
) -> Option<PanelScreenHandoff> {
    let camera_transform = camera_transform.compute_transform();
    let forward = camera_transform.rotation * Vec3::NEG_Z;
    let distance = (panel_transform.translation - camera_transform.translation).dot(forward);
    if !distance.is_finite() || distance <= 0.0 {
        return None;
    }
    Some(conversion::panel_screen_handoff(
        camera, conversion, distance,
    ))
}

const fn fixed_pixel_sizing(value: f32) -> Sizing {
    Sizing::Fixed(Dimension {
        value,
        unit: Some(Unit::Pixels),
    })
}

fn screen_conversion_window_size(
    conversion: &PanelScreenConversion,
    primary: &Query<Entity, With<PrimaryWindow>>,
    windows: &Query<&Window>,
) -> Option<Vec2> {
    let entity = match conversion.window {
        WindowRef::Primary => primary.single().ok()?,
        WindowRef::Entity(entity) => entity,
    };
    let window = windows.get(entity).ok()?;
    let size = Vec2::new(window.width(), window.height());
    (size.is_finite() && size.x > 0.0 && size.y > 0.0).then_some(size)
}

fn apply_panel_world_conversion_now(
    entity: Entity,
    conversion: PanelWorldConversion,
    commands: &mut Commands<'_, '_>,
    panel: &mut DiegeticPanel,
    widget_index: &mut PanelWidgetIndex,
    transform: &mut Transform,
    classification: &mut DiegeticPanelChangeClassification,
    saved: Option<&SavedPanelWorldState>,
    resolved_position: &mut ResolvedScreenPanelPosition,
) {
    if let Err(error) = validate_world_conversion(&conversion) {
        warn!("failed to convert panel {entity:?} to world space: {error}");
        return;
    }
    let saved = if matches!(
        conversion.restore_saved_world,
        conversion::SavedWorldRestoreMode::Restore
    ) {
        let Some(saved) = saved.cloned() else {
            warn!("failed to convert panel {entity:?} to saved world space: no saved world state");
            return;
        };
        Some(saved)
    } else {
        None
    };
    let next_transform = conversion.transform;
    if let Some(saved) = saved.as_ref() {
        if let Err(error) = saved.apply_world_conversion(panel, &conversion) {
            warn!("failed to convert panel {entity:?} to saved world space: {error}");
            return;
        }
        widget_index.clear();
        classification.record_tree_change(LayoutTreeChange::LayoutAffecting);
    } else if let Err(error) = apply_world_conversion(panel, conversion) {
        warn!("failed to convert panel {entity:?} to world space: {error}");
        return;
    }
    *transform = next_transform;
    *resolved_position = ResolvedScreenPanelPosition::default();
    commands.entity(entity).insert(PanelSpace::World);
    if let Some(saved) = saved {
        lifecycle::write_owned_cascade(
            commands,
            entity,
            entity,
            Cascade::Override(FontUnit(saved.resolved_font_unit)),
        );
        lifecycle::write_owned_cascade(
            commands,
            entity,
            entity,
            Cascade::Override(saved.resolved_lighting),
        );
        lifecycle::write_owned_cascade(
            commands,
            entity,
            entity,
            Cascade::Override(saved.resolved_sidedness),
        );
        lifecycle::write_owned_render_layers(commands, entity, entity, saved.render_layers);
    } else {
        lifecycle::write_owned_render_layers(commands, entity, entity, None);
    }
    commands
        .entity(entity)
        .remove::<PreparedPanelScreenConversion>();
}

/// Spawn-time authoring bridge for panel cascade overrides.
///
/// Reads a newly-added [`DiegeticPanel`]'s construction seeds and queues the
/// matching `Cascade<A>` components for font unit, materials, text alpha, HDR
/// coverage bias, shadow casting, anti-aliasing, hairline fade, lighting, and
/// sidedness. The construction-seed fields are never replayed after spawn.
/// `resolve_inserted_cascade<A>` in `hana_kana` seeds the panel's
/// `Resolved<FontUnit>`, which `compute_panel_layouts` reads.
///
/// At construction, a panel has no cascade ancestor and receives an overriding
/// `Cascade<FontUnit>` from its builder seed when set, otherwise from
/// [`PanelDefaults::panel_font_unit`]. Thus `panel_font_unit` is the normal
/// construction-time seed rather than the cascade root. A later explicit
/// `inherit_font_unit` command can replace that seed with `Cascade::Inherit`, in
/// which case the panel resolves through `FontUnit`.
/// `text_alpha_mode` supplies the panel value inherited by its labels; when it
/// inherits, both panel and labels resolve to `TextAlpha`.
/// `CascadePlugin<TextAlpha>` maintains the panel's `Resolved<TextAlpha>`
/// cache, while labels follow `CascadeFrom` to the panel's authored
/// `Cascade<TextAlpha>`. This is the panel equivalent of the standalone
/// `TextStyle` authoring bridge.
pub(super) fn seed_panel_overrides(
    trigger: On<Add, DiegeticPanel>,
    panels: Query<&DiegeticPanel>,
    defaults: Res<PanelDefaults>,
    mut commands: Commands,
) {
    let entity = trigger.event_target();
    let Ok(panel) = panels.get(entity) else {
        return;
    };
    let font_unit = panel.font_unit.resolve(defaults.panel_font_unit);
    let sdf_material = panel.material.as_ref().cloned().map(SdfMaterial);
    let text_material = panel.text_material.as_ref().cloned().map(TextMaterial);
    let shape_material = panel.shape_material.as_ref().cloned().map(ShapeMaterial);
    let text_alpha = panel.text_alpha_mode.map(TextAlpha);
    let hdr_text_coverage_bias = panel.hdr_text_coverage_bias.map(HdrTextCoverageBias);
    let shadow_casting = panel.shadow_casting;
    let widget_hovered_appearance = panel.widget_hovered_appearance.clone();
    let widget_pressed_appearance = panel.widget_pressed_appearance.clone();
    let widget_focused_appearance = panel.widget_focused_appearance.clone();
    let widget_disabled_appearance = panel.widget_disabled_appearance.clone();
    let (lighting, sidedness) = if panel.coordinate_space().is_screen() {
        (
            Cascade::Override(Lighting::Unlit),
            Cascade::Override(Sidedness::FrontOnly),
        )
    } else {
        (Cascade::Inherit, Cascade::Inherit)
    };

    commands.queue(move |world: &mut bevy::ecs::world::World| {
        seed_panel_value(world, entity, Cascade::Override(FontUnit(font_unit)));
        seed_panel_value(world, entity, sdf_material);
        seed_panel_value(world, entity, text_material);
        seed_panel_value(world, entity, shape_material);
        seed_panel_value(world, entity, text_alpha);
        seed_panel_value(world, entity, hdr_text_coverage_bias);
        seed_panel_value(world, entity, shadow_casting);
        seed_panel_value(world, entity, widget_hovered_appearance);
        seed_panel_value(world, entity, widget_pressed_appearance);
        seed_panel_value(world, entity, widget_focused_appearance);
        seed_panel_value(world, entity, widget_disabled_appearance);
        seed_panel_value(world, entity, Cascade::<AntiAlias>::Inherit);
        seed_panel_value(world, entity, Cascade::<HairlineFade>::Inherit);
        seed_panel_value(world, entity, lighting);
        seed_panel_value(world, entity, sidedness);
    });
}

fn seed_panel_value<A: CascadeAttribute>(
    world: &mut bevy::ecs::world::World,
    entity: Entity,
    authored: Cascade<A>,
) {
    lifecycle::seed_owned_cascade(world, entity, entity, authored);
}

/// Per-frame tree-change classification consumed by the panel layout system.
#[derive(Component, Default)]
pub(crate) struct DiegeticPanelChangeClassification {
    pending:                     Option<LayoutTreeChange>,
    tree_visual_geometry_stable: bool,
}

impl DiegeticPanelChangeClassification {
    fn record(&mut self, change: LayoutTreeChange) {
        self.pending = Some(match self.pending {
            None => change,
            Some(prior) => prior.combine(change),
        });
    }

    pub(super) fn record_tree_change(&mut self, change: LayoutTreeChange) {
        let prior = self.pending;
        let prior_stable = self.tree_visual_geometry_stable;
        self.record(change);

        self.tree_visual_geometry_stable = match self.pending {
            Some(LayoutTreeChange::VisualOnly) => match change {
                LayoutTreeChange::VisualOnly => match prior {
                    None | Some(LayoutTreeChange::Identical) => true,
                    Some(LayoutTreeChange::VisualOnly) => prior_stable,
                    Some(LayoutTreeChange::LayoutAffecting) => false,
                },
                LayoutTreeChange::Identical => prior_stable,
                LayoutTreeChange::LayoutAffecting => false,
            },
            Some(LayoutTreeChange::Identical | LayoutTreeChange::LayoutAffecting) | None => false,
        };
    }

    /// Records a run-text edit as `VisualOnly`: a text change moves no other
    /// element, so `compute_panel_layouts` re-measures only the edited leaf and
    /// takes the geometry-stable skip when its box is unchanged. Combining with a
    /// same-frame `LayoutAffecting` change still resolves to a full solve.
    pub(crate) fn note_text_edit(&mut self) {
        self.record(LayoutTreeChange::VisualOnly);
        self.tree_visual_geometry_stable = false;
    }

    pub(crate) fn note_widget_interactivity_edit(&mut self) {
        self.record_tree_change(LayoutTreeChange::VisualOnly);
    }

    pub(super) fn take_with_tree_visual_geometry_stable(
        &mut self,
    ) -> (Option<LayoutTreeChange>, bool) {
        let pending = self.pending.take();
        let tree_visual_geometry_stable =
            pending == Some(LayoutTreeChange::VisualOnly) && self.tree_visual_geometry_stable;
        self.tree_visual_geometry_stable = false;
        (pending, tree_visual_geometry_stable)
    }

    pub(super) const fn pending(&self) -> Option<LayoutTreeChange> { self.pending }
}

/// Cached point-scaled layout tree for one [`DiegeticPanel`].
///
/// The source [`LayoutTree`] remains owned by [`DiegeticPanel`]. This component
/// stores the derived tree used by the layout engine after layout and font units
/// are converted to points.
#[derive(Component, Default)]
pub(super) struct ScaledLayoutTreeCache {
    source_revision:       TreeRevision,
    layout_to_points_bits: F32Bits,
    font_to_points_bits:   F32Bits,
    tree:                  Option<LayoutTree>,
    #[cfg(test)]
    hits:                  usize,
    #[cfg(test)]
    misses:                usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct F32Bits(u32);

impl F32Bits {
    const fn new(value: f32) -> Self { Self(value.to_bits()) }
}

impl ScaledLayoutTreeCache {
    /// Test helper that drops the source-tree-derived value while keeping the
    /// last source revision and scale keys.
    #[cfg(test)]
    pub(super) fn invalidate_source(&mut self) { self.tree = None; }

    /// Returns a point-scaled tree, rebuilding the cache when the source tree
    /// or scale factors change.
    pub(super) fn get_or_update(
        &mut self,
        source: &PanelTree,
        layout_to_points: f32,
        font_to_points: f32,
    ) -> &LayoutTree {
        let source_revision = source.revision();
        let layout_to_points_bits = F32Bits::new(layout_to_points);
        let font_to_points_bits = F32Bits::new(font_to_points);
        let cache_hit = self.tree.is_some()
            && self.source_revision == source_revision
            && self.layout_to_points_bits == layout_to_points_bits
            && self.font_to_points_bits == font_to_points_bits;

        if cache_hit {
            #[cfg(test)]
            {
                self.hits += 1;
            }
        } else {
            self.source_revision = source_revision;
            self.layout_to_points_bits = layout_to_points_bits;
            self.font_to_points_bits = font_to_points_bits;
            self.tree = None;
            #[cfg(test)]
            {
                self.misses += 1;
            }
        }

        self.tree
            .get_or_insert_with(|| source.tree().scaled(layout_to_points, font_to_points))
    }

    #[cfg(test)]
    pub(super) const fn hits(&self) -> usize { self.hits }

    #[cfg(test)]
    pub(super) const fn misses(&self) -> usize { self.misses }
}

/// Whether a panel's layout has been solved.
///
/// A panel starts [`Unsolved`](Self::Unsolved) and reaches
/// [`Solved`](Self::Solved) on its first layout pass. Nothing returns it to
/// `Unsolved`: [`ComputedDiegeticPanel::set_result`] and
/// `ComputedDiegeticPanel::set_result_with_fields`
/// are the only writers and both store `Solved`. Readers that need geometry
/// must therefore treat `Unsolved` as "too early", never as "this panel has
/// nothing to draw".
#[derive(Debug, Default)]
pub enum PanelLayout {
    /// No layout pass has run for this panel yet.
    #[default]
    Unsolved,
    /// The most recent layout pass produced this result.
    Solved(LayoutResult),
}

impl PanelLayout {
    /// The most recent layout result, or `None` while unsolved.
    #[must_use]
    pub const fn solved(&self) -> Option<&LayoutResult> {
        match self {
            Self::Solved(layout_result) => Some(layout_result),
            Self::Unsolved => None,
        }
    }
}

/// Computed layout result for a [`DiegeticPanel`].
///
/// Automatically added via required components when a [`DiegeticPanel`] is inserted.
/// Updated by the layout system whenever the panel changes.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct ComputedDiegeticPanel {
    #[reflect(ignore)]
    layout:             PanelLayout,
    #[reflect(ignore)]
    draw_order:         DrawOrder,
    #[reflect(ignore)]
    field_records:      Vec<PanelFieldRecord>,
    #[reflect(ignore)]
    field_id_conflicts: Vec<crate::PanelElementId>,
    #[reflect(ignore)]
    widget_records:     Vec<ComputedWidgetRecord>,
    #[reflect(ignore)]
    tooltip_records:    Vec<ComputedTooltipRecord>,
    content_width:      f32,
    content_height:     f32,
    #[cfg(test)]
    #[reflect(ignore)]
    layout_solves:      usize,
}

impl ComputedDiegeticPanel {
    /// Actual computed panel-surface width in world units.
    ///
    /// This is the solved root element width, including root padding and
    /// border. With `Sizing::FIT`, this shrinks the panel viewport to the
    /// visible surface instead of clipping root chrome.
    #[must_use]
    pub const fn content_width(&self) -> f32 { self.content_width }

    /// Actual computed panel-surface height in world units.
    #[must_use]
    pub const fn content_height(&self) -> f32 { self.content_height }

    /// Returns the bounding box of the first child content in layout units, or
    /// `None` if layout has not yet been computed.
    #[must_use]
    pub fn content_bounds(&self) -> Option<BoundingBox> {
        self.layout.solved().and_then(LayoutResult::content_bounds)
    }

    /// Returns computed editable fields in draw-independent element order.
    #[must_use]
    pub fn field_records(&self) -> &[PanelFieldRecord] { &self.field_records }

    /// Returns duplicated editable field ids found during the latest layout.
    #[must_use]
    pub fn field_id_conflicts(&self) -> &[crate::PanelElementId] { &self.field_id_conflicts }

    /// Resolves an editable field at a panel-local layout point.
    ///
    /// Records with duplicated ids are ignored because their semantic target
    /// is ambiguous.
    #[must_use]
    pub fn field_at_local_position(&self, panel_local: Vec2) -> Option<&PanelFieldRecord> {
        self.field_records
            .iter()
            .rev()
            .find(|record| !record.duplicate_id && record.contains(panel_local))
    }

    /// Returns whether this panel's layout has been solved.
    ///
    /// Match on [`layout`](Self::layout) instead when the solved
    /// `LayoutResult` is needed; this is for the callers that only gate on
    /// readiness.
    #[must_use]
    pub const fn is_solved(&self) -> bool { self.layout.solved().is_some() }

    /// Returns this panel's layout, solved or not.
    #[must_use]
    pub const fn layout(&self) -> &PanelLayout { &self.layout }

    /// Returns the command-indexed `DrawOrder` for the latest result.
    #[must_use]
    pub(crate) const fn draw_order(&self) -> &DrawOrder { &self.draw_order }

    pub(crate) fn widget_records(&self) -> &[ComputedWidgetRecord] { &self.widget_records }

    pub(crate) fn tooltip_records(&self) -> &[ComputedTooltipRecord] { &self.tooltip_records }

    #[cfg(test)]
    pub(crate) const fn layout_solves(&self) -> usize { self.layout_solves }

    /// Regenerates `LayoutResult::commands` and keeps `DrawOrder` synchronized.
    ///
    /// Returns `false` when this panel's layout is still unsolved.
    pub(super) fn regenerate_commands(&mut self, tree: &LayoutTree) -> bool {
        let PanelLayout::Solved(result) = &mut self.layout else {
            return false;
        };

        result.regenerate_commands(tree);
        self.draw_order = DrawOrder::from_commands(&result.commands);
        let (field_records, field_id_conflicts) = field::collect_panel_field_records(tree, result);
        self.field_records = field_records;
        self.field_id_conflicts = field_id_conflicts;
        self.widget_records = tree.computed_widget_records(result);
        self.tooltip_records = tree.computed_tooltip_records();
        true
    }

    /// Stores the computed layout result.
    pub fn set_result(&mut self, result: LayoutResult) {
        self.draw_order = DrawOrder::from_commands(&result.commands);
        self.layout = PanelLayout::Solved(result);
        self.field_records.clear();
        self.field_id_conflicts.clear();
        self.widget_records.clear();
        self.tooltip_records.clear();
    }

    pub(super) fn set_result_with_fields(
        &mut self,
        result: LayoutResult,
        field_records: Vec<PanelFieldRecord>,
        field_id_conflicts: Vec<crate::PanelElementId>,
        widget_records: Vec<ComputedWidgetRecord>,
        tooltip_records: Vec<ComputedTooltipRecord>,
    ) {
        self.draw_order = DrawOrder::from_commands(&result.commands);
        self.layout = PanelLayout::Solved(result);
        self.field_records = field_records;
        self.field_id_conflicts = field_id_conflicts;
        self.widget_records = widget_records;
        self.tooltip_records = tooltip_records;
        #[cfg(test)]
        {
            self.layout_solves += 1;
        }
    }

    /// Sets the content dimensions in world units.
    pub const fn set_content_size(&mut self, width: f32, height: f32) {
        self.content_width = width;
        self.content_height = height;
    }
}

impl DiegeticPanel {
    /// Places this screen-space panel at an explicit pixel position.
    ///
    /// The origin is the window's top-left corner and y grows downward. The
    /// panel's [`Anchor`] determines which point of the panel is placed at this
    /// position. Returns `false` for world-space panels.
    #[must_use]
    pub const fn set_screen_position(&mut self, screen_position: Vec2) -> bool {
        let CoordinateSpace::Screen { position, .. } = &mut self.coordinate_space else {
            return false;
        };
        *position = ScreenPosition::At(screen_position);
        true
    }

    /// Resizes this screen-space panel, taking the same [`Sizing`] pair its
    /// builder's [`size`](DiegeticPanelBuilder::size) took.
    ///
    /// The screen-space systems resolve both axes against the target window
    /// every frame, so an application that derives its dimensions from the
    /// window sends the recomputed sizing here after a resize. Returns `false`
    /// for world-space panels.
    #[must_use]
    pub const fn set_screen_size(&mut self, width: Sizing, height: Sizing) -> bool {
        let CoordinateSpace::Screen {
            width: width_sizing,
            height: height_sizing,
            ..
        } = &mut self.coordinate_space
        else {
            return false;
        };
        *width_sizing = width;
        *height_sizing = height;
        true
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact revision and cache counter values"
)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic if fixture panel construction fails"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Camera;
    use bevy::prelude::Color;
    use bevy::prelude::Commands;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Quat;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::default;
    use bevy::window::PrimaryWindow;
    use bevy::window::Window;
    use bevy::window::WindowRef;

    use super::ComputedDiegeticPanel;
    use super::CoordinateSpace;
    use super::DiegeticPanel;
    use super::DiegeticPanelChangeClassification;
    use super::DiegeticPanelCommands;
    use super::PanelScreenHandoff;
    use super::PanelSpace;
    use super::PanelTree;
    use super::PreparedPanelScreenConversion;
    use super::SavedPanelWorldState;
    use super::ScaledLayoutTreeCache;
    use crate::Appearance;
    use crate::CascadeEntityCommandsExt as _;
    use crate::DiegeticTextMeasurer;
    use crate::EditorStateColors;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelBuildError;
    use crate::PanelElementId;
    use crate::PanelEntity;
    use crate::PanelScreenConversion;
    use crate::TextStyle;
    use crate::Unit;
    use crate::cascade::Cascade;
    use crate::cascade::FontUnit;
    use crate::cascade::Resolved;
    use crate::layout::LayoutTreeChange;
    use crate::layout::ShadowCasting;

    fn test_tree(text: &str) -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((text, TextStyle::new(10.0)));
        builder.build()
    }

    fn auto_widget_tree() -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button(PanelElementId::auto(7)), |_| {});
        builder.build()
    }

    fn precomposed_widget_tree() -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::column().precompose_ldr(), |builder| {
            builder.with(El::new().button("action"), |_| {});
        });
        builder.build()
    }

    fn editable_tree_with_editor_text(fill: Color) -> crate::LayoutTree {
        let field =
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .editable_field("field", field)
                .editor_text(EditorStateColors::new().focused(fill)),
            |builder| {
                builder.text(("display", TextStyle::new(10.0)));
            },
        );
        builder.build()
    }

    #[test]
    fn scaled_tree_cache_hits_until_source_invalidated_or_scale_changes() {
        let mut source = PanelTree::from(test_tree("cache"));
        let mut cache = ScaledLayoutTreeCache::default();

        let _ = cache.get_or_update(&source, 2.0, 3.0);
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 1);

        let _ = cache.get_or_update(&source, 2.0, 3.0);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);

        cache.invalidate_source();
        let _ = cache.get_or_update(&source, 2.0, 3.0);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 2);

        source.replace(test_tree("cache"));
        let _ = cache.get_or_update(&source, 2.0, 3.0);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 3);

        let _ = cache.get_or_update(&source, 4.0, 3.0);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 4);

        let _ = cache.get_or_update(&source, 4.0, 5.0);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 5);
    }

    #[test]
    fn scaled_tree_cache_hits_across_unrelated_panel_component_changes() {
        let source = PanelTree::from(test_tree("cache"));
        let mut cache = ScaledLayoutTreeCache::default();

        let _ = cache.get_or_update(&source, 2.0, 3.0);
        let _ = cache.get_or_update(&source, 2.0, 3.0);

        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);
    }

    #[test]
    fn visual_only_editor_declaration_refreshes_field_presentation() {
        let initial_tree = editable_tree_with_editor_text(Color::srgb(0.2, 0.3, 0.4));
        let replacement_tree = editable_tree_with_editor_text(Color::srgb(0.7, 0.6, 0.5));
        assert_eq!(
            initial_tree.classify_change(&replacement_tree),
            LayoutTreeChange::VisualOnly,
        );

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(DiegeticTextMeasurer::default());
        app.add_plugins(HeadlessLayoutPlugin);
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(initial_tree)
                    .build()
                    .expect("editable panel should build"),
            )
            .id();
        app.update();

        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("editable panel should compute");
        let layout_solves = computed.layout_solves();
        let initial_declaration = computed
            .field_records()
            .first()
            .expect("editable panel should have one field")
            .presentation()
            .editor_text
            .clone();

        app.world_mut()
            .commands()
            .set_tree(panel, replacement_tree)
            .expect("visual-only editable tree should be accepted");
        app.update();

        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("editable panel should remain computed");
        assert_eq!(computed.layout_solves(), layout_solves);
        assert_ne!(
            computed
                .field_records()
                .first()
                .expect("editable panel should retain its field")
                .presentation()
                .editor_text,
            initial_declaration,
        );
    }

    #[test]
    fn precompose_helper_replace_invalidates_scaled_tree_cache() {
        let mut helper = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .font_unit(Unit::Millimeters)
            .widget_hovered_appearance(Appearance::new().background(Color::srgb(0.1, 0.2, 0.3)))
            .widget_pressed_appearance(Appearance::new().background(Color::srgb(0.3, 0.2, 0.1)))
            .widget_focused_appearance(Appearance::new().background(Color::srgb(0.2, 0.3, 0.1)))
            .widget_disabled_appearance(Appearance::new().background(Color::srgb(0.1, 0.3, 0.2)))
            .world_height(0.5)
            .with_tree(test_tree("Blend"))
            .build()
            .expect("helper panel should build");
        let appearance_seeds = (
            helper.widget_hovered_appearance.clone(),
            helper.widget_pressed_appearance.clone(),
            helper.widget_focused_appearance.clone(),
            helper.widget_disabled_appearance.clone(),
        );
        let mut cache = ScaledLayoutTreeCache::default();

        let scaled = cache.get_or_update(helper.tree_source(), 1.0, 1.0);
        assert_eq!(scaled.element_text(1), Some("Blend"));
        assert_eq!(cache.misses(), 1);

        let next = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .font_unit(Unit::Millimeters)
            .widget_hovered_appearance(Appearance::new().background(Color::srgb(0.8, 0.7, 0.6)))
            .widget_pressed_appearance(Appearance::new().background(Color::srgb(0.6, 0.7, 0.8)))
            .widget_focused_appearance(Appearance::new().background(Color::srgb(0.7, 0.8, 0.6)))
            .widget_disabled_appearance(Appearance::new().background(Color::srgb(0.6, 0.8, 0.7)))
            .world_height(0.5)
            .with_tree(test_tree("Add"))
            .build()
            .expect("replacement helper panel should build");

        helper.replace_from_precompose_helper(next);
        let scaled = cache.get_or_update(helper.tree_source(), 1.0, 1.0);

        assert_eq!(scaled.element_text(1), Some("Add"));
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 2);
        assert_eq!(
            (
                helper.widget_hovered_appearance,
                helper.widget_pressed_appearance,
                helper.widget_focused_appearance,
                helper.widget_disabled_appearance,
            ),
            appearance_seeds,
        );
    }

    #[test]
    fn same_frame_text_edit_clears_tree_visual_geometry_stability() {
        let mut classification = DiegeticPanelChangeClassification::default();

        classification.record_tree_change(LayoutTreeChange::VisualOnly);
        assert_eq!(
            classification.take_with_tree_visual_geometry_stable(),
            (Some(LayoutTreeChange::VisualOnly), true)
        );

        classification.record_tree_change(LayoutTreeChange::VisualOnly);
        classification.note_text_edit();

        assert_eq!(
            classification.take_with_tree_visual_geometry_stable(),
            (Some(LayoutTreeChange::VisualOnly), false)
        );
    }

    #[test]
    fn panel_shadow_seeds_and_chained_commands_obey_first_frame_precedence() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(DiegeticTextMeasurer::default());
        app.add_plugins(HeadlessLayoutPlugin);
        let (seeded, inherited, overridden) = {
            let mut commands = app.world_mut().commands();
            let seeded = commands
                .spawn(
                    DiegeticPanel::world()
                        .size(Mm(100.0), Mm(50.0))
                        .shadow_casting(ShadowCasting::Off)
                        .with_tree(test_tree("seeded"))
                        .build()
                        .expect("seeded panel should build"),
                )
                .id();
            let inherited = commands
                .spawn(
                    DiegeticPanel::world()
                        .size(Mm(100.0), Mm(50.0))
                        .shadow_casting(ShadowCasting::Off)
                        .with_tree(test_tree("inherit"))
                        .build()
                        .expect("inheriting panel should build"),
                )
                .inherit_shadow_casting()
                .id();
            let overridden = commands
                .spawn(
                    DiegeticPanel::world()
                        .size(Mm(100.0), Mm(50.0))
                        .with_tree(test_tree("override"))
                        .build()
                        .expect("overriding panel should build"),
                )
                .override_shadow_casting(ShadowCasting::Off)
                .id();
            (seeded, inherited, overridden)
        };

        app.update();

        assert_eq!(
            app.world()
                .get::<Cascade<ShadowCasting>>(seeded)
                .expect("seeded panel should participate in the cascade"),
            &Cascade::Override(ShadowCasting::Off)
        );
        assert_eq!(
            app.world()
                .get::<Resolved<ShadowCasting>>(seeded)
                .expect("seeded panel should resolve shadow casting")
                .0,
            ShadowCasting::Off
        );
        assert_eq!(
            app.world()
                .get::<Cascade<ShadowCasting>>(inherited)
                .expect("inheriting panel should participate in the cascade"),
            &Cascade::Inherit
        );
        assert_eq!(
            app.world()
                .get::<Resolved<ShadowCasting>>(inherited)
                .expect("inheriting panel should resolve shadow casting")
                .0,
            ShadowCasting::On
        );
        assert_eq!(
            app.world()
                .get::<Cascade<ShadowCasting>>(overridden)
                .expect("overriding panel should participate in the cascade"),
            &Cascade::Override(ShadowCasting::Off)
        );
        assert_eq!(
            app.world()
                .get::<Resolved<ShadowCasting>>(overridden)
                .expect("overriding panel should resolve shadow casting")
                .0,
            ShadowCasting::Off
        );
    }

    #[test]
    fn unrelated_panel_tree_change_preserves_runtime_shadow_override() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(DiegeticTextMeasurer::default());
        app.add_plugins(HeadlessLayoutPlugin);
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .shadow_casting(ShadowCasting::Off)
                    .with_tree(test_tree("seed"))
                    .build()
                    .expect("panel should build"),
            )
            .id();
        app.update();

        {
            let mut commands = app.world_mut().commands();
            commands
                .entity(panel)
                .override_shadow_casting(ShadowCasting::On);
            assert!(commands.set_tree(panel, test_tree("replacement")).is_ok());
        }
        app.update();

        assert_eq!(
            app.world()
                .get::<Cascade<ShadowCasting>>(panel)
                .expect("panel should retain authored shadow casting"),
            &Cascade::Override(ShadowCasting::On)
        );
        assert_eq!(
            app.world()
                .get::<Resolved<ShadowCasting>>(panel)
                .expect("panel should retain resolved shadow casting")
                .0,
            ShadowCasting::On
        );
    }

    #[test]
    fn set_tree_rejects_invalid_widgets_without_queueing_replacement() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(DiegeticTextMeasurer::default());
        app.add_plugins(HeadlessLayoutPlugin);
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(test_tree("seed"))
            .build();
        assert!(result.is_ok());
        let Ok(panel_component) = result else {
            return;
        };
        let panel = app.world_mut().spawn(panel_component).id();
        app.update();
        let revision = app
            .world()
            .get::<DiegeticPanel>(panel)
            .map(DiegeticPanel::tree_revision);

        let auto_error = app
            .world_mut()
            .commands()
            .set_tree(panel, auto_widget_tree());
        assert!(matches!(
            auto_error,
            Err(PanelBuildError::WidgetRequiresNamedId(id))
                if id == PanelElementId::auto(7)
        ));
        let precompose_error = app
            .world_mut()
            .commands()
            .set_tree(panel, precomposed_widget_tree());
        assert!(matches!(
            precompose_error,
            Err(PanelBuildError::WidgetInsidePrecomposedSubtree(id))
                if id == PanelElementId::named("action")
        ));
        app.update();

        let unchanged = app.world().get::<DiegeticPanel>(panel);
        assert_eq!(unchanged.map(DiegeticPanel::tree_revision), revision);
        assert_eq!(
            unchanged.and_then(|panel| panel.tree().element_text(1)),
            Some("seed")
        );

        let valid = app
            .world_mut()
            .commands()
            .set_tree(panel, test_tree("replacement"));
        assert!(valid.is_ok());
        app.update();
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(panel)
                .and_then(|panel| panel.tree().element_text(1)),
            Some("replacement")
        );
    }

    #[test]
    fn begin_screen_conversion_preserves_world_state_until_handoff() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(DiegeticTextMeasurer::default());
        app.add_plugins(HeadlessLayoutPlugin);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .font_unit(Unit::Millimeters)
            .world_height(0.5)
            .with_tree(test_tree("prepare"))
            .build()
            .expect("world panel should build");
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 5.0)),
            ))
            .id();
        let entity = app
            .world_mut()
            .spawn((panel, Transform::from_scale(Vec3::new(2.0, 2.0, 1.0))))
            .id();
        let conversion =
            PanelScreenConversion::at_pixels(Vec2::new(400.0, 300.0), Vec2::new(200.0, 100.0));

        let owner = PanelEntity::from_validated(entity, PanelSpace::World);
        app.world_mut()
            .run_system_once({
                let conversion = conversion.clone();
                move |mut conversions: Commands| {
                    conversions.begin_to_screen(owner, camera, conversion.clone())
                }
            })
            .expect("screen preparation system runs")
            .expect("screen preparation is valid");
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel should still exist");
        assert!(matches!(
            panel.coordinate_space(),
            CoordinateSpace::World { .. }
        ));
        assert_eq!(panel.width(), 200.0);
        assert_eq!(panel.height(), 100.0);
        assert_eq!(panel.layout_unit(), Unit::Pixels);
        assert_eq!(panel.font_unit, Cascade::Override(Unit::Millimeters));
        assert_eq!(
            app.world()
                .get::<Cascade<FontUnit>>(entity)
                .expect("screen preparation should override the live font unit"),
            &Cascade::Override(FontUnit(Unit::Pixels))
        );
        assert_eq!(panel.world_width(), 1.0);
        assert_eq!(panel.world_height(), 0.5);
        assert!(app.world().get::<SavedPanelWorldState>(entity).is_some());
        assert!(app.world().get::<PanelScreenHandoff>(entity).is_some());
        assert!(
            app.world()
                .get::<PreparedPanelScreenConversion>(entity)
                .is_some()
        );

        app.world_mut()
            .run_system_once(move |mut conversions: Commands| {
                conversions.finish_to_screen(owner, camera, conversion.clone())
            })
            .expect("screen conversion system runs")
            .expect("screen conversion is valid");
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel should still exist");
        assert!(panel.coordinate_space().is_screen());
        assert!(
            app.world()
                .get::<PreparedPanelScreenConversion>(entity)
                .is_none()
        );
        assert!(app.world().get::<SavedPanelWorldState>(entity).is_some());
    }

    #[test]
    fn queued_screen_conversion_applies_in_panel_pipeline_and_resets_scale() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(DiegeticTextMeasurer::default());
        app.add_plugins(HeadlessLayoutPlugin);
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: (800_u32, 600_u32).into(),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .font_unit(Unit::Points)
            .world_height(0.5)
            .with_tree(test_tree("convert"))
            .build()
            .expect("world panel should build");
        let entity = app
            .world_mut()
            .spawn((panel, Transform::from_scale(Vec3::new(2.0, 3.0, 4.0))))
            .id();
        let conversion =
            PanelScreenConversion::at_pixels(Vec2::new(400.0, 300.0), Vec2::new(200.0, 100.0))
                .window(WindowRef::Entity(window));

        let owner = PanelEntity::from_validated(entity, PanelSpace::World);
        app.world_mut()
            .run_system_once(move |mut conversions: Commands| {
                conversions.apply_to_screen(owner, conversion.clone())
            })
            .expect("screen conversion system runs")
            .expect("screen conversion is valid");
        app.update();

        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel should still exist");
        assert!(panel.coordinate_space().is_screen());
        let transform = app
            .world()
            .get::<Transform>(entity)
            .expect("panel transform should still exist");
        assert_eq!(transform.translation, Vec3::ZERO);
        assert_eq!(transform.rotation, Quat::IDENTITY);
        assert_eq!(transform.scale, Vec3::ONE);
    }

    #[cfg(feature = "bench_support")]
    #[test]
    fn tree_revision_changes_only_when_tree_is_replaced() {
        let mut panel = DiegeticPanel::default();
        assert_eq!(u64::from(panel.tree_revision()), 0);

        panel.set_width(120.0);
        panel.set_height(80.0);
        assert_eq!(u64::from(panel.tree_revision()), 0);

        let resize_result = panel.set_size((2.0, 1.0));
        assert!(resize_result.is_ok());
        assert_eq!(u64::from(panel.tree_revision()), 0);

        panel.set_tree_full_rebuild(test_tree("next"));
        assert_eq!(u64::from(panel.tree_revision()), 1);
    }

    #[test]
    fn builder_panels_start_at_tree_revision_zero() {
        let panel = DiegeticPanel::world()
            .size(1.0, 0.5)
            .with_tree(test_tree("builder"))
            .build()
            .expect("test panel should build");

        assert_eq!(u64::from(panel.tree_revision()), 0);
    }
}
