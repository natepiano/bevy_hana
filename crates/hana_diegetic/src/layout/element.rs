//! Element tree representation for layout computation.
//!
//! [`Element`] is the data struct stored in the arena-based [`LayoutTree`]. It holds every
//! layout property (sizing, padding, direction, background, border, etc.) plus an
//! [`ElementContent`] that determines whether the node is a container, a text leaf, or empty.
//!
//! Users rarely construct `Element` directly. Instead, the [`El`](super::builder::El) builder
//! provides a fluent API that converts into an `Element` via `into_element()`. Think of `El`
//! as the ergonomic front door and `Element` as the canonical storage format.

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

use bevy::asset::Handle;
use bevy::color::Color;
use bevy::image::Image;
use bevy::math::Vec2;
use bevy::pbr::StandardMaterial;
use smallvec::SmallVec;

use super::Border;
use super::BoundingBox;
use super::ChildDivider;
use super::CornerRadius;
use super::Dimension;
use super::DrawZIndex;
use super::LayoutResult;
use super::Padding;
use super::PanelDraw;
use super::ShadowCasting;
use super::Sizing;
use super::TextSizing;
use super::TextStyle;
use super::Unit;
use super::builder::EditorPart;
use super::child_layout::ChildLayout;
use super::constants::INLINE_CHILDREN;
use super::engine;
use crate::ImePanelField;
use crate::PanelBuildError;
use crate::PanelElementId;
use crate::cascade::Cascade;
use crate::panel::PanelFieldPresentation;
use crate::render::AntiAlias;
use crate::render::HairlineFade;
use crate::widgets::ComputedTooltipRecord;
use crate::widgets::ComputedVisualSlot;
use crate::widgets::ComputedWidgetRecord;
use crate::widgets::StateAppearance;
use crate::widgets::Tooltip;
use crate::widgets::VisualElementCapabilities;
use crate::widgets::VisualSlotId;
use crate::widgets::WidgetInteractivity;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetSpec;

/// Result of replacing the display text for a panel field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FieldDisplayTextUpdate {
    /// Exactly one field matched and its display text was replaced.
    Updated,
    /// No editable field had the requested id.
    MissingField,
    /// More than one editable field had the requested id.
    DuplicateField,
    /// The field had no text descendant to update.
    MissingText,
}

/// Whether overflowing children are clipped to the parent's content box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ChildOverflow {
    #[default]
    Visible,
    Clipped,
}

/// Which edge [`Element::scroll_offset`] measures from. `Start` is an absolute
/// offset from the top/left (clamped to `[0, max]`); `End` is a distance from
/// the bottom/right, so `0` pins to the end and following a growing tail needs
/// no knowledge of the content size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ScrollAnchor {
    #[default]
    Start,
    End,
}

/// Whether an element subtree should be rendered directly or flattened first.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PrecomposeMode {
    /// Draw this element and its descendants through the normal panel render path.
    #[default]
    Direct,
    /// Render this element's subtree into an LDR image, then draw that image in
    /// the parent panel.
    Ldr,
}

/// A single element in the layout tree.
///
/// Elements are either containers (with children) or text leaves. The tree
/// is built via [`LayoutTree`] and then sized/positioned by the layout engine.
#[derive(Clone, Debug)]
pub(super) struct Element {
    /// Optional panel-local identity for this layout element.
    pub(super) id:                Option<PanelElementId>,
    /// Width sizing rule.
    pub(super) width:             Sizing,
    /// Height sizing rule.
    pub(super) height:            Sizing,
    /// Interior padding.
    pub(super) padding:           Padding,
    /// Child layout mode, spacing, and alignment.
    pub(super) child_layout:      ChildLayout,
    /// Optional background color.
    pub(super) background:        Option<Color>,
    /// Optional border.
    pub(super) border:            Option<Border>,
    /// Corner radius for rounded backgrounds and borders.
    pub(super) corner_radius:     CornerRadius,
    /// How this element handles overflowing children (`Visible` or `Clipped`).
    pub(super) overflow:          ChildOverflow,
    /// Scroll offset (logical px) subtracted from child positions when this
    /// element clips. Clamped during positioning to `[0, content - viewport]`
    /// per axis. Interpreted relative to each axis' scroll anchor.
    pub(super) scroll_offset:     Vec2,
    /// Which horizontal edge `scroll_offset.x` measures from.
    pub(super) scroll_anchor_x:   ScrollAnchor,
    /// Which vertical edge `scroll_offset.y` measures from.
    pub(super) scroll_anchor_y:   ScrollAnchor,
    /// Authored PBR source-material handle for this element's surfaces.
    ///
    /// When overridden, render systems use this over the panel material handle
    /// and global material cascade defaults.
    /// `base_color` is overridden by layout or primitive color when both are set.
    pub(super) material:          Cascade<Handle<StandardMaterial>>,
    /// Authored widget interactivity for this layout scope.
    pub(super) interactivity:     Cascade<WidgetInteractivity>,
    /// Optional editable field contract.
    pub(super) editable:          Option<ImePanelField>,
    /// Optional authored widget contract.
    pub(super) widget:            Option<WidgetSpec>,
    /// Optional per-state appearance for this widget element's root visual
    /// slot. Boxed so an ordinary element pays one pointer rather than four
    /// appearance layers.
    pub(super) appearance:        Option<Box<StateAppearance>>,
    /// Whether the editor created this element at runtime rather than from the
    /// panel's authored tree.
    pub(super) editor_origin:     EditorElementOrigin,
    /// Authored declaration for generated committed and preedit text runs.
    pub(super) editor_text:       Option<Box<EditorPart>>,
    /// Authored declaration for the generated selection highlight box.
    pub(super) editor_selection:  Option<Box<EditorPart>>,
    /// Authored declaration for the generated caret box.
    pub(super) editor_caret:      Option<Box<EditorPart>>,
    /// Authored declaration for the generated validation message run.
    pub(super) editor_validation: Option<Box<EditorPart>>,
    /// Optional tooltip declaration associated with this widget.
    pub(super) tooltip:           Option<Tooltip>,
    /// Optional stable visual-slot id for widget-owned retained records.
    pub(super) visual_slot:       Option<VisualSlotId>,
    /// Optional paint-only draw data.
    pub(super) draw:              Option<PanelDraw>,
    /// `DrawZIndex` stamped onto this element's render commands.
    pub(super) z_index:           DrawZIndex,
    /// Authored anti-alias mode for this element's analytic line marks.
    pub(super) anti_alias:        Cascade<AntiAlias>,
    /// Authored hairline fade policy for this element's analytic line marks.
    pub(super) hairline_fade:     Cascade<HairlineFade>,
    /// Authored shadow-casting policy for this element and its render commands.
    pub(super) shadow_casting:    Cascade<ShadowCasting>,
    /// Optional subtree precomposition mode.
    pub(super) precompose:        PrecomposeMode,
    /// Content of this element.
    pub(super) content:           ElementContent,
}

/// Distinguishes authored layout elements from editor-generated elements.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum EditorElementOrigin {
    /// The panel author declared this element in the layout tree.
    #[default]
    Authored,
    /// The inline editor created this element from its current buffer state.
    Generated,
}

/// What an element contains.
#[derive(Clone, Debug)]
pub(super) enum ElementContent {
    /// Container with child element indices.
    Children(SmallVec<[usize; INLINE_CHILDREN]>),
    /// Text leaf.
    Text {
        /// The text string.
        text:   String,
        /// Text configuration.
        config: TextStyle,
        /// Sizing and wrapping policy.
        sizing: TextSizing,
    },
    /// Image leaf — rendered as a textured quad.
    Image {
        /// Handle to the image asset.
        handle: Handle<Image>,
        /// Tint color multiplied against the texture (white = no tint).
        tint:   Color,
    },
    /// Empty (no children, no text).
    Empty,
}

/// Classifies the difference between two layout trees.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LayoutTreeChange {
    /// Trees are exactly identical for the fields this classifier inspects.
    Identical       = 0,
    /// Trees differ only in fields that should not affect layout bounds.
    VisualOnly      = 1,
    /// Trees differ in structure, sizing, measurement, or placement fields.
    LayoutAffecting = 2,
}

impl LayoutTreeChange {
    pub(crate) fn combine(self, other: Self) -> Self { self.max(other) }
}

impl Default for Element {
    fn default() -> Self {
        Self {
            id:                None,
            width:             Sizing::FIT,
            height:            Sizing::FIT,
            padding:           Padding::default(),
            child_layout:      ChildLayout::default(),
            background:        None,
            border:            None,
            corner_radius:     CornerRadius::ZERO,
            overflow:          ChildOverflow::Visible,
            scroll_offset:     Vec2::ZERO,
            scroll_anchor_x:   ScrollAnchor::Start,
            scroll_anchor_y:   ScrollAnchor::Start,
            material:          Cascade::Inherit,
            interactivity:     Cascade::Inherit,
            editable:          None,
            widget:            None,
            appearance:        None,
            editor_origin:     EditorElementOrigin::Authored,
            editor_text:       None,
            editor_selection:  None,
            editor_caret:      None,
            editor_validation: None,
            tooltip:           None,
            visual_slot:       None,
            draw:              None,
            z_index:           DrawZIndex::default(),
            anti_alias:        Cascade::Inherit,
            hairline_fade:     Cascade::Inherit,
            shadow_casting:    Cascade::Inherit,
            precompose:        PrecomposeMode::Direct,
            content:           ElementContent::Empty,
        }
    }
}

impl Element {
    pub(super) fn child_clip_bounds(&self, bounds: BoundingBox) -> BoundingBox {
        let top = self.border.as_ref().map_or(0.0, |border| border.top.value);
        let right = self
            .border
            .as_ref()
            .map_or(0.0, |border| border.right.value);
        let bottom = self
            .border
            .as_ref()
            .map_or(0.0, |border| border.bottom.value);
        let left = self.border.as_ref().map_or(0.0, |border| border.left.value);
        BoundingBox {
            x:      bounds.x + left,
            y:      bounds.y + top,
            width:  (bounds.width - left - right).max(0.0),
            height: (bounds.height - top - bottom).max(0.0),
        }
    }
}

/// Arena-based layout tree.
///
/// Elements are stored in a flat `Vec` and reference children by index.
/// The first element (index 0) is always the root.
#[derive(Clone, Debug, Default)]
pub struct LayoutTree {
    /// All elements in insertion order.
    pub(super) elements: Vec<Element>,
    /// Index of the root element.
    pub(super) root:     Option<usize>,
}

impl LayoutTree {
    /// Creates a new empty layout tree.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Creates a new empty layout tree with pre-allocated capacity.
    ///
    /// Use this when you know the approximate element count upfront to
    /// avoid reallocation during tree construction.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            elements: Vec::with_capacity(capacity),
            root:     None,
        }
    }

    /// Adds an element and returns its index.
    pub(super) fn add(&mut self, element: Element) -> usize {
        let index = self.elements.len();
        self.elements.push(element);
        index
    }

    /// Adds an element as a child of the given parent.
    ///
    /// Returns the child's index.
    pub(super) fn add_child(&mut self, parent: usize, element: Element) -> usize {
        let child_index = self.add(element);
        if let Some(parent_element) = self.elements.get_mut(parent) {
            match &mut parent_element.content {
                ElementContent::Children(children) => {
                    children.push(child_index);
                },
                ElementContent::Empty => {
                    parent_element.content =
                        ElementContent::Children(SmallVec::from_elem(child_index, 1));
                },
                ElementContent::Text { .. } | ElementContent::Image { .. } => {
                    // Leaf elements cannot have children — this is a programming error.
                    // In release builds we silently ignore it; debug builds will catch it
                    // via the orphan check in layout computation.
                },
            }
        }
        child_index
    }

    /// Sets the root element index.
    pub(super) const fn set_root(&mut self, index: usize) { self.root = Some(index); }

    /// Changes the root element's width sizing to `Grow { min, max }`.
    ///
    /// Used by `build_screen_space()` when the panel width is dynamic
    /// (e.g. `Percent` or `Grow`) so that changing `panel.width`
    /// triggers correct reflow without a tree rebuild. Pass
    /// `Dimension { value: 0.0, unit: None }` for `min` and
    /// `Dimension { value: f32::MAX, unit: None }` for `max` to match
    /// the previous unbounded behavior.
    pub(super) fn set_root_grow_width(&mut self, min: Dimension, max: Dimension) {
        if let Some(root) = self.root
            && let Some(element) = self.elements.get_mut(root)
        {
            element.width = Sizing::Grow { min, max };
        }
    }

    /// Changes the root element's height sizing to `Grow { min, max }`.
    ///
    /// See [`set_root_grow_width`](Self::set_root_grow_width) for rationale.
    pub(super) fn set_root_grow_height(&mut self, min: Dimension, max: Dimension) {
        if let Some(root) = self.root
            && let Some(element) = self.elements.get_mut(root)
        {
            element.height = Sizing::Grow { min, max };
        }
    }

    /// Changes the root element's width sizing to `FIT { min, max }`.
    ///
    /// Paired with `DiegeticPanel::screen().size(Sizing::Fit { .. }, _)` so
    /// the two-pass layout (`propagate_fit_sizes` bottom-up +
    /// `size_along_axis` top-down) resolves root to its natural content
    /// width, clamped to `[min, max]`.
    pub(super) fn set_root_fit_width(&mut self, min: Dimension, max: Dimension) {
        if let Some(root) = self.root
            && let Some(element) = self.elements.get_mut(root)
        {
            element.width = Sizing::Fit { min, max };
        }
    }

    /// Changes the root element's height sizing to `FIT { min, max }`.
    ///
    /// See [`set_root_fit_width`](Self::set_root_fit_width) for rationale.
    pub(super) fn set_root_fit_height(&mut self, min: Dimension, max: Dimension) {
        if let Some(root) = self.root
            && let Some(element) = self.elements.get_mut(root)
        {
            element.height = Sizing::Fit { min, max };
        }
    }

    /// Clones `root_index` and its descendants into a standalone tree whose root
    /// has the supplied fixed size.
    ///
    /// Used by LDR precomposition: the cloned tree renders offscreen with the
    /// exact subtree content, while the source tree draws only the resulting
    /// image. Precompose flags are cleared in the clone so the helper panel does
    /// not recursively spawn another helper for the same subtree.
    #[must_use]
    pub(crate) fn precompose_subtree(
        &self,
        root_index: usize,
        width: Dimension,
        height: Dimension,
    ) -> Option<Self> {
        if root_index >= self.elements.len() {
            return None;
        }

        let mut tree = Self::new();
        let root = self.clone_subtree_into(root_index, &mut tree, width, height, true)?;
        tree.set_root(root);
        Some(tree)
    }

    /// Returns an iterator over child indices of the given element.
    #[must_use]
    pub(super) fn children_of(&self, index: usize) -> &[usize] {
        self.elements
            .get(index)
            .map_or(&[], |element| match &element.content {
                ElementContent::Children(children) => children.as_slice(),
                _ => &[],
            })
    }

    fn clone_subtree_into(
        &self,
        source_index: usize,
        target: &mut Self,
        root_width: Dimension,
        root_height: Dimension,
        is_root: bool,
    ) -> Option<usize> {
        let source = self.elements.get(source_index)?;
        let mut clone = source.clone();
        clone.precompose = PrecomposeMode::Direct;
        if let ElementContent::Text { config, .. } = &mut clone.content {
            config.clear_hdr_text_coverage_bias();
        }
        if is_root {
            clone.width = Sizing::Fixed(root_width);
            clone.height = Sizing::Fixed(root_height);
        }
        let children = match &source.content {
            ElementContent::Children(children) => Some(children.clone()),
            _ => None,
        };
        if children.is_some() {
            clone.content = ElementContent::Children(SmallVec::new());
        }

        let target_index = target.add(clone);
        if let Some(children) = children {
            for child in children {
                let child_index =
                    self.clone_subtree_into(child, target, root_width, root_height, false)?;
                if let Some(target_element) = target.elements.get_mut(target_index)
                    && let ElementContent::Children(target_children) = &mut target_element.content
                {
                    target_children.push(child_index);
                }
            }
        }

        Some(target_index)
    }

    /// Returns the number of elements in the tree.
    #[must_use]
    pub const fn len(&self) -> usize { self.elements.len() }

    /// Replaces one element's state appearance for a layout test fixture.
    #[cfg(test)]
    pub(crate) fn set_element_state_appearance(
        &mut self,
        index: usize,
        appearance: StateAppearance,
    ) -> bool {
        let Some(element) = self.elements.get_mut(index) else {
            return false;
        };
        element.appearance = Some(Box::new(appearance));
        true
    }

    /// Returns `true` if the tree has no elements.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.elements.is_empty() }

    /// Classifies whether `next` differs from this tree only in render-only
    /// fields.
    #[must_use]
    pub fn classify_change(&self, next: &Self) -> LayoutTreeChange {
        if self.root != next.root || self.elements.len() != next.elements.len() {
            return LayoutTreeChange::LayoutAffecting;
        }

        let mut change = LayoutTreeChange::Identical;
        for (element, next_element) in self.elements.iter().zip(&next.elements) {
            change = change.combine(classify_element_change(element, next_element));
            if change == LayoutTreeChange::LayoutAffecting {
                return change;
            }
        }
        change
    }

    /// Hashes only structural facts needed to safely reuse computed geometry.
    #[must_use]
    pub(super) fn structure_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.root.hash(&mut hasher);
        self.elements.len().hash(&mut hasher);
        for element in &self.elements {
            match &element.content {
                ElementContent::Children(children) => {
                    0_u8.hash(&mut hasher);
                    children.hash(&mut hasher);
                },
                ElementContent::Text { .. } => {
                    1_u8.hash(&mut hasher);
                },
                ElementContent::Image { .. } => {
                    2_u8.hash(&mut hasher);
                },
                ElementContent::Empty => {
                    3_u8.hash(&mut hasher);
                },
            }
        }
        hasher.finish()
    }

    /// Returns the PBR material authoring for the element at `index`.
    #[must_use]
    pub(crate) fn element_material(&self, index: usize) -> Cascade<&Handle<StandardMaterial>> {
        self.elements
            .get(index)
            .map_or(Cascade::Inherit, |element| element.material.as_ref())
    }

    /// Returns the corner radius for the element at `index`.
    #[must_use]
    pub fn element_corner_radius(&self, index: usize) -> CornerRadius {
        self.elements
            .get(index)
            .map_or(CornerRadius::ZERO, |e| e.corner_radius)
    }

    /// Returns the visual presentation authored on an editable field element.
    pub(crate) fn editable_field_presentation(
        &self,
        index: usize,
    ) -> Option<PanelFieldPresentation> {
        let element = self.elements.get(index)?;
        element.editable.as_ref()?;
        let text_style = self
            .first_text_descendant(index)
            .and_then(|text_index| self.elements.get(text_index))
            .and_then(|text_element| match &text_element.content {
                ElementContent::Text { config, .. } => Some(config.clone()),
                ElementContent::Children(_)
                | ElementContent::Empty
                | ElementContent::Image { .. } => None,
            });
        Some(PanelFieldPresentation {
            background: element.background,
            border: element.border,
            corner_radius: element.corner_radius,
            padding: element.padding,
            align_x: element.child_layout.align_x(),
            align_y: element.child_layout.align_y(),
            text_style,
            editor_text: element.editor_text.as_deref().cloned(),
            editor_selection: element.editor_selection.as_deref().cloned(),
            editor_caret: element.editor_caret.as_deref().cloned(),
            editor_validation: element.editor_validation.as_deref().cloned(),
        })
    }

    /// Returns editable field metadata for the element at `index`, if any.
    #[must_use]
    pub(crate) fn editable_field(&self, index: usize) -> Option<&ImePanelField> {
        self.elements.get(index).and_then(|e| e.editable.as_ref())
    }

    /// Returns the paint-only draw data for the element at `index`, if any.
    #[cfg(test)]
    #[must_use]
    fn element_draw(&self, index: usize) -> Option<&PanelDraw> {
        self.elements.get(index).and_then(|e| e.draw.as_ref())
    }

    /// Returns the anti-alias authoring for the element at `index`.
    #[must_use]
    pub(crate) fn element_anti_alias(&self, index: usize) -> Cascade<AntiAlias> {
        self.elements
            .get(index)
            .map_or(Cascade::Inherit, |element| element.anti_alias)
    }

    /// Returns the hairline fade authoring for the element at `index`.
    #[must_use]
    pub(crate) fn element_hairline_fade(&self, index: usize) -> Cascade<HairlineFade> {
        self.elements
            .get(index)
            .map_or(Cascade::Inherit, |element| element.hairline_fade)
    }

    /// Returns the shadow-casting authoring for the element at `index`.
    #[must_use]
    pub(crate) fn element_shadow_casting(&self, index: usize) -> Cascade<ShadowCasting> {
        self.elements
            .get(index)
            .map_or(Cascade::Inherit, |element| element.shadow_casting)
    }

    /// Returns the subtree precomposition mode for the element at `index`.
    #[cfg(test)]
    #[must_use]
    fn element_precompose(&self, index: usize) -> PrecomposeMode {
        self.elements
            .get(index)
            .map_or(PrecomposeMode::Direct, |element| element.precompose)
    }

    /// Returns text content for the element at `index`, if any.
    #[must_use]
    pub(crate) fn element_text(&self, index: usize) -> Option<&str> {
        self.elements
            .get(index)
            .and_then(|element| match &element.content {
                ElementContent::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
    }

    /// Overwrites the cached run string at `index`, returning whether it
    /// changed. `TextContent` on the reified child is the single source;
    /// this keeps the `El.text` layout cache (which the engine measures and
    /// word-wraps) current after an out-of-flow edit. Returns `false` (no write)
    /// when `index` is not a text leaf or the string already matches, so the
    /// caller can skip dirtying the panel.
    pub(crate) fn set_element_text(&mut self, index: usize, text: &str) -> bool {
        let Some(element) = self.elements.get_mut(index) else {
            return false;
        };
        let ElementContent::Text { text: existing, .. } = &mut element.content else {
            return false;
        };
        if existing == text {
            return false;
        }
        text.clone_into(existing);
        true
    }

    /// Returns the authored [`TextStyle`] of the text run at `index`, if that
    /// element is a text leaf. The tree config — not the run child's
    /// `for_shaping`-derived style — is authoritative for measurement, so a
    /// restyle reads it here, applies its edit, and writes back via
    /// [`set_element_style`](Self::set_element_style).
    #[must_use]
    pub(crate) fn element_style(&self, index: usize) -> Option<&TextStyle> {
        self.elements
            .get(index)
            .and_then(|element| match &element.content {
                ElementContent::Text { config, .. } => Some(config),
                _ => None,
            })
    }

    /// Overwrites the authored run style at `index`, returning whether it
    /// changed. The tree config is the single source the layout engine measures
    /// and reification re-derives the run from; writing it here is how a font /
    /// size restyle reaches both measurement and rendering. Returns `false` (no
    /// write) when `index` is not a text leaf or the style already matches, so
    /// the caller can skip dirtying the panel.
    pub(crate) fn set_element_style(&mut self, index: usize, style: TextStyle) -> bool {
        let Some(element) = self.elements.get_mut(index) else {
            return false;
        };
        let ElementContent::Text { config, .. } = &mut element.content else {
            return false;
        };
        if *config == style {
            return false;
        }
        *config = style;
        true
    }

    /// Returns the panel-local id of the element at `index`.
    #[must_use]
    pub(crate) fn element_id(&self, index: usize) -> Option<&PanelElementId> {
        self.elements
            .get(index)
            .and_then(|element| element.id.as_ref())
    }

    pub(crate) fn widget_interactivity(
        &self,
        id: &PanelElementId,
    ) -> Option<Cascade<WidgetInteractivity>> {
        self.widget_element_index(id)
            .map(|index| self.elements[index].interactivity)
    }

    pub(crate) fn set_widget_interactivity(
        &mut self,
        id: &PanelElementId,
        authored: Cascade<WidgetInteractivity>,
    ) -> bool {
        let Some(index) = self.widget_element_index(id) else {
            return false;
        };
        self.elements[index].interactivity = authored;
        true
    }

    fn widget_element_index(&self, id: &PanelElementId) -> Option<usize> {
        self.elements.iter().position(|element| {
            element.widget.is_some()
                && element
                    .id
                    .as_ref()
                    .is_some_and(|element_id| element_id == id)
        })
    }

    /// Returns the panel-local id of the text element at `index`, if that
    /// element is a text leaf. Reification reads this to key a child by its id instead of
    /// the former positional `(element_idx, command_index)` pair.
    #[must_use]
    pub(crate) fn text_element_id(&self, index: usize) -> Option<&PanelElementId> {
        self.elements
            .get(index)
            .filter(|element| matches!(element.content, ElementContent::Text { .. }))
            .and_then(|element| element.id.as_ref())
    }

    /// Whether any text element in the tree carries `id`.
    ///
    /// The tree is the authoritative list of valid run ids at build time,
    /// independent of reification timing, so a lookup that misses the panel's
    /// `text_index` consults this to tell a genuine typo (`id` absent here) from a
    /// run not yet reified into an entity (`id` present here, index just not
    /// rebuilt). See [`PanelText`](crate::PanelText).
    // The runtime caller is the `#[cfg(debug_assertions)]` typo-warn path in
    // `PanelTextReader::resolve`; test harnesses also compile this for coverage.
    #[cfg(any(debug_assertions, test))]
    #[must_use]
    pub(crate) fn contains_text_id(&self, id: &PanelElementId) -> bool {
        (0..self.elements.len()).any(|index| self.text_element_id(index) == Some(id))
    }

    /// Returns the first author-assigned [`PanelElementId::Named`] id that appears
    /// on more than one element, scanning element ids and editable-field ids
    /// together — they share one panel-local namespace. Auto ids are skipped
    /// (unforgeable and unique by construction), so this only flags a real
    /// author collision. `DiegeticPanelBuilder::build` calls this to reject a
    /// duplicate at build time.
    #[must_use]
    pub(crate) fn duplicate_named_element_id(&self) -> Option<&PanelElementId> {
        let mut seen: Vec<&PanelElementId> = Vec::new();
        for index in 0..self.elements.len() {
            let element_id = self.element_id(index);
            let field_id = self.editable_field(index).map(|field| &field.field_id);
            for id in [element_id, field_id].into_iter().flatten() {
                if id.is_named() {
                    if seen.contains(&id) {
                        return Some(id);
                    }
                    seen.push(id);
                }
            }
        }
        None
    }

    pub(crate) fn validate_widgets(&self) -> Result<(), PanelBuildError> {
        let Some(root) = self.root else {
            return Ok(());
        };
        let mut sliders_with_thumb = HashSet::new();
        let mut stack = vec![(
            root,
            Option::<(PanelElementId, WidgetKind)>::None,
            PrecomposeMode::Direct,
        )];
        while let Some((index, owning_widget, inherited_precompose)) = stack.pop() {
            let Some(element) = self.elements.get(index) else {
                continue;
            };
            let precompose = match (inherited_precompose, element.precompose) {
                (PrecomposeMode::Ldr, _) | (_, PrecomposeMode::Ldr) => PrecomposeMode::Ldr,
                (PrecomposeMode::Direct, PrecomposeMode::Direct) => PrecomposeMode::Direct,
            };

            if element.visual_slot == Some(VisualSlotId::SLIDER_THUMB) {
                if let Some((slider_id, WidgetKind::Slider)) = owning_widget.as_ref() {
                    if !sliders_with_thumb.insert(slider_id.clone()) {
                        return Err(PanelBuildError::SliderHasMultipleThumbs(slider_id.clone()));
                    }
                } else {
                    let thumb_id = element
                        .id
                        .clone()
                        .unwrap_or_else(|| PanelElementId::auto(u32::try_from(index).unwrap_or(0)));
                    return Err(PanelBuildError::SliderThumbOutsideSlider(thumb_id));
                }
            }

            validate_part_state_colors(element, index, owning_widget.as_ref())?;

            let next_owning_widget =
                validated_element_widget_owner(element, index, owning_widget, precompose)?;

            for &child in self.children_of(index).iter().rev() {
                stack.push((child, next_owning_widget.clone(), precompose));
            }
        }
        Ok(())
    }

    pub(crate) fn computed_widget_records(
        &self,
        result: &LayoutResult,
    ) -> Vec<ComputedWidgetRecord> {
        let Some(root) = self.root else {
            return Vec::new();
        };
        let Some(root_layout) = result.computed.get(root) else {
            return Vec::new();
        };
        let mut ranked_records = Vec::new();
        let mut stack = vec![(root, Cascade::Inherit, root_layout.bounds, None::<usize>)];
        let mut preorder = 0;
        while let Some((index, inherited_interactivity, inherited_clip, inherited_owner)) =
            stack.pop()
        {
            let (Some(element), Some(computed)) =
                (self.elements.get(index), result.computed.get(index))
            else {
                continue;
            };
            let interactivity = match element.interactivity {
                Cascade::Inherit => inherited_interactivity,
                Cascade::Override(value) => Cascade::Override(value),
            };
            let widget_declaration = element
                .id
                .as_ref()
                .zip(element.widget.as_ref())
                .map(|(id, widget)| (id.clone(), widget.clone()))
                .or_else(|| {
                    element.editable.as_ref().map(|field| {
                        (
                            field.field_id.clone(),
                            WidgetSpec::EditableField(field.clone()),
                        )
                    })
                });
            let is_widget_root = widget_declaration.is_some();
            let owning_record = if let Some((id, widget)) = widget_declaration {
                ranked_records.push((
                    element.z_index,
                    ComputedWidgetRecord::new(
                        id,
                        preorder,
                        widget,
                        element.appearance.as_deref().cloned().unwrap_or_default(),
                        interactivity,
                        computed.bounds,
                        computed.bounds.intersect(&inherited_clip),
                    ),
                ));
                Some(ranked_records.len() - 1)
            } else {
                inherited_owner
            };
            if let Some(record_index) = owning_record {
                record_owned_widget_element(
                    &mut ranked_records[record_index].1,
                    element,
                    index,
                    is_widget_root,
                );
            }
            if let (Some(slot), Some(record_index)) = (element.visual_slot, owning_record) {
                let border_box = computed.bounds;
                let content_box = engine::content_box_bounds(element, border_box);
                ranked_records[record_index]
                    .1
                    .push_visual_slot(ComputedVisualSlot {
                        slot,
                        element_index: index,
                        border_box,
                        content_box,
                    });
            }
            preorder += 1;
            let child_clip = if matches!(element.overflow, ChildOverflow::Clipped) {
                inherited_clip
                    .intersect(&element.child_clip_bounds(computed.bounds))
                    .unwrap_or_default()
            } else {
                inherited_clip
            };
            for &child in self.children_of(index).iter().rev() {
                stack.push((child, interactivity, child_clip, owning_record));
            }
        }

        let mut rank_order: Vec<usize> = (0..ranked_records.len()).collect();
        rank_order.sort_by_key(|&index| {
            let (z_index, record) = &ranked_records[index];
            (*z_index, record.preorder())
        });
        for (interaction_rank, record_index) in rank_order.into_iter().enumerate() {
            ranked_records[record_index]
                .1
                .set_interaction_rank(interaction_rank);
        }
        ranked_records
            .into_iter()
            .map(|(_, record)| record)
            .collect()
    }

    pub(crate) fn computed_tooltip_records(&self) -> Vec<ComputedTooltipRecord> {
        self.elements
            .iter()
            .filter_map(|element| {
                let widget_id = element
                    .id
                    .as_ref()
                    .filter(|_| element.widget.is_some())
                    .or_else(|| element.editable.as_ref().map(|field| &field.field_id))?;
                let tooltip = element.tooltip.as_ref()?;
                Some(crate::widgets::ComputedTooltipRecord::new(
                    widget_id.clone(),
                    tooltip.clone(),
                ))
            })
            .collect()
    }

    /// Returns the first text string owned by `index` or one of its descendants.
    #[must_use]
    pub(crate) fn field_display_text(&self, index: usize) -> Option<&str> {
        let mut stack = vec![index];
        while let Some(current) = stack.pop() {
            if let Some(text) = self.element_text(current) {
                return Some(text);
            }
            for &child in self.children_of(current).iter().rev() {
                stack.push(child);
            }
        }
        None
    }

    pub(crate) fn set_field_display_text(
        &mut self,
        field_id: &PanelElementId,
        text: impl Into<String>,
    ) -> FieldDisplayTextUpdate {
        let matches: Vec<usize> = self
            .elements
            .iter()
            .enumerate()
            .filter_map(|(index, element)| {
                element
                    .editable
                    .as_ref()
                    .is_some_and(|field| field.field_id == *field_id)
                    .then_some(index)
            })
            .collect();

        let [field_index] = matches.as_slice() else {
            return if matches.is_empty() {
                FieldDisplayTextUpdate::MissingField
            } else {
                FieldDisplayTextUpdate::DuplicateField
            };
        };

        let Some(text_index) = self.first_text_descendant(*field_index) else {
            return FieldDisplayTextUpdate::MissingText;
        };
        if let ElementContent::Text {
            text: existing_text,
            ..
        } = &mut self.elements[text_index].content
        {
            *existing_text = text.into();
        }
        FieldDisplayTextUpdate::Updated
    }

    /// Replaces one editable field's descendants with `replacement` while
    /// preserving the editable field element itself.
    ///
    /// The result is rebuilt as a compact arena so descendants hidden by the
    /// replacement cannot keep duplicate ids or participate in later scans.
    /// Auto-generated text ids in `replacement` are reminted after the source
    /// tree's ids because each tree was originally built with its own counter.
    pub(crate) fn set_field_editing_content(
        &mut self,
        field_id: &PanelElementId,
        replacement: &Self,
    ) -> FieldDisplayTextUpdate {
        let matches: Vec<usize> = self
            .elements
            .iter()
            .enumerate()
            .filter_map(|(index, element)| {
                element
                    .editable
                    .as_ref()
                    .is_some_and(|field| field.field_id == *field_id)
                    .then_some(index)
            })
            .collect();
        let [field_index] = matches.as_slice() else {
            return if matches.is_empty() {
                FieldDisplayTextUpdate::MissingField
            } else {
                FieldDisplayTextUpdate::DuplicateField
            };
        };
        let (Some(source_root), Some(replacement_root)) = (self.root, replacement.root) else {
            return FieldDisplayTextUpdate::MissingText;
        };

        let mut next = Self::with_capacity(self.len() + replacement.len());
        let mut next_auto_id = self.next_auto_element_id();
        let Some(root) = self.clone_with_field_editing_content(
            source_root,
            *field_index,
            replacement,
            replacement_root,
            &mut next,
            &mut next_auto_id,
        ) else {
            return FieldDisplayTextUpdate::MissingText;
        };
        next.set_root(root);
        *self = next;
        FieldDisplayTextUpdate::Updated
    }

    fn clone_with_field_editing_content(
        &self,
        source_index: usize,
        field_index: usize,
        replacement: &Self,
        replacement_root: usize,
        target: &mut Self,
        next_auto_id: &mut u32,
    ) -> Option<usize> {
        let source = self.elements.get(source_index)?;
        let children = match &source.content {
            ElementContent::Children(children) => children.clone(),
            ElementContent::Empty | ElementContent::Text { .. } | ElementContent::Image { .. } => {
                SmallVec::new()
            },
        };
        let mut clone = source.clone();
        if source_index == field_index || !children.is_empty() {
            clone.content = ElementContent::Children(SmallVec::new());
        }
        let target_index = target.add(clone);

        if source_index == field_index {
            let replacement_index = replacement.clone_reminting_auto_ids_into(
                replacement_root,
                target,
                next_auto_id,
            )?;
            if let ElementContent::Children(target_children) =
                &mut target.elements.get_mut(target_index)?.content
            {
                target_children.push(replacement_index);
            }
            return Some(target_index);
        }

        for child in children {
            let child_index = self.clone_with_field_editing_content(
                child,
                field_index,
                replacement,
                replacement_root,
                target,
                next_auto_id,
            )?;
            if let ElementContent::Children(target_children) =
                &mut target.elements.get_mut(target_index)?.content
            {
                target_children.push(child_index);
            }
        }
        Some(target_index)
    }

    fn clone_reminting_auto_ids_into(
        &self,
        source_index: usize,
        target: &mut Self,
        next_auto_id: &mut u32,
    ) -> Option<usize> {
        let source = self.elements.get(source_index)?;
        let children = match &source.content {
            ElementContent::Children(children) => children.clone(),
            ElementContent::Empty | ElementContent::Text { .. } | ElementContent::Image { .. } => {
                SmallVec::new()
            },
        };
        let mut clone = source.clone();
        if matches!(clone.id, Some(PanelElementId::Auto(_))) {
            clone.id = Some(PanelElementId::auto(*next_auto_id));
            *next_auto_id = next_auto_id.saturating_add(1);
        }
        if !children.is_empty() {
            clone.content = ElementContent::Children(SmallVec::new());
        }
        let target_index = target.add(clone);
        for child in children {
            let child_index = self.clone_reminting_auto_ids_into(child, target, next_auto_id)?;
            if let ElementContent::Children(target_children) =
                &mut target.elements.get_mut(target_index)?.content
            {
                target_children.push(child_index);
            }
        }
        Some(target_index)
    }

    fn next_auto_element_id(&self) -> u32 {
        self.elements
            .iter()
            .filter_map(|element| match element.id.as_ref() {
                Some(PanelElementId::Auto(id)) => Some(id.value()),
                Some(PanelElementId::Named(_)) | None => None,
            })
            .max()
            .map_or(0, |value| value.saturating_add(1))
    }

    fn first_text_descendant(&self, index: usize) -> Option<usize> {
        let mut stack = vec![index];
        while let Some(current) = stack.pop() {
            if matches!(
                self.elements.get(current).map(|element| &element.content),
                Some(ElementContent::Text { .. })
            ) {
                return Some(current);
            }
            for &child in self.children_of(current).iter().rev() {
                stack.push(child);
            }
        }
        None
    }

    /// Returns a copy of this tree with all dimensions converted to points.
    ///
    /// `layout_scale` multiplies spatial values (padding, gaps, borders, fixed sizes).
    /// `font_scale` multiplies font-related values (size, line height, letter/word spacing).
    ///
    /// Used by the panel layout system to ensure the layout engine and parley
    /// always operate in points, avoiding parley's integer quantization at small sizes.
    #[must_use]
    pub fn scaled(&self, layout_scale: f32, font_scale: f32) -> Self {
        let mut tree = self.clone();
        for element in &mut tree.elements {
            element.width = element.width.resolved(layout_scale);
            element.height = element.height.resolved(layout_scale);
            element.padding = element.padding.resolved(layout_scale);
            element.child_layout = element.child_layout.to_points(layout_scale);
            if let Some(ref mut border) = element.border {
                *border = border.resolved(layout_scale);
            }
            element.corner_radius = element.corner_radius.resolved(layout_scale);
            if let Some(ref mut panel_draw) = element.draw {
                *panel_draw = panel_draw.scaled(layout_scale);
            }
            element.scroll_offset *= layout_scale;
            scale_editor_part(&mut element.editor_text, layout_scale);
            scale_editor_part(&mut element.editor_selection, layout_scale);
            scale_editor_part(&mut element.editor_caret, layout_scale);
            scale_editor_part(&mut element.editor_validation, layout_scale);
            if let ElementContent::Text { ref mut config, .. } = element.content {
                // If this text element carries an explicit unit (e.g., from
                // `TextStyle::new(Mm(6.0))`), convert from that unit to
                // points directly. Otherwise use the panel-wide font_scale.
                let scale = config.unit().map_or(font_scale, Unit::to_points);
                *config = config.scaled(scale);
            }
        }
        tree
    }

    /// Returns a copy of this tree authored in screen-pixel source values.
    ///
    /// The first pass resolves the existing authored dimensions into layout
    /// points. The second pass converts those resolved point values into pixel
    /// source values so a screen-space panel with `Unit::Pixels` renders the
    /// same relative content size as the original world panel projection.
    #[must_use]
    pub(crate) fn screen_source_scaled(
        &self,
        layout_to_points: f32,
        font_to_points: f32,
        points_to_pixels: f32,
    ) -> Self {
        let mut tree = self.scaled(layout_to_points, font_to_points);
        for element in &mut tree.elements {
            element.width = element.width.resolved(points_to_pixels);
            element.height = element.height.resolved(points_to_pixels);
            element.padding = element.padding.resolved(points_to_pixels);
            element.child_layout = element.child_layout.to_points(points_to_pixels);
            if let Some(ref mut border) = element.border {
                *border = border.resolved(points_to_pixels);
            }
            element.corner_radius = element.corner_radius.resolved(points_to_pixels);
            if let Some(ref mut panel_draw) = element.draw {
                *panel_draw = panel_draw.scaled(points_to_pixels);
            }
            element.scroll_offset *= points_to_pixels;
            scale_editor_part(&mut element.editor_text, points_to_pixels);
            scale_editor_part(&mut element.editor_selection, points_to_pixels);
            scale_editor_part(&mut element.editor_caret, points_to_pixels);
            scale_editor_part(&mut element.editor_validation, points_to_pixels);
            if let ElementContent::Text { ref mut config, .. } = element.content {
                *config = config.scaled_as_unit(points_to_pixels, Unit::Pixels);
            }
        }
        tree
    }
}

fn scale_editor_part(part: &mut Option<Box<EditorPart>>, default_scale: f32) {
    if let Some(part) = part {
        let scaled = part.scaled(default_scale);
        **part = scaled;
    }
}

fn validated_widget_id(
    id: PanelElementId,
    precompose: PrecomposeMode,
) -> Result<PanelElementId, PanelBuildError> {
    if !id.is_named() {
        return Err(PanelBuildError::WidgetRequiresNamedId(id));
    }
    if precompose == PrecomposeMode::Ldr {
        return Err(PanelBuildError::WidgetInsidePrecomposedSubtree(id));
    }
    Ok(id)
}

fn validated_element_widget_owner(
    element: &Element,
    index: usize,
    owning_widget: Option<(PanelElementId, WidgetKind)>,
    precompose: PrecomposeMode,
) -> Result<Option<(PanelElementId, WidgetKind)>, PanelBuildError> {
    if let Some(widget) = &element.widget {
        let id = element
            .id
            .clone()
            .unwrap_or_else(|| PanelElementId::auto(u32::try_from(index).unwrap_or(0)));
        let id = validated_widget_id(id, precompose)?;
        if let WidgetSpec::Slider(slider) = widget {
            slider
                .validated()
                .map_err(|error| PanelBuildError::SliderConfig(id.clone(), error))?;
        }
        return Ok(Some((id, widget.kind())));
    }
    if let Some(field) = &element.editable {
        let id = validated_widget_id(field.field_id.clone(), precompose)?;
        return Ok(Some((id, WidgetKind::EditableField)));
    }
    Ok(owning_widget)
}

fn element_visual_capabilities(element: &Element) -> VisualElementCapabilities {
    let mut capabilities = VisualElementCapabilities::empty();
    if element.background.is_some() {
        capabilities |= VisualElementCapabilities::SDF_FILL;
    }
    if element.border.is_some() {
        capabilities |= VisualElementCapabilities::SDF_BORDER;
    }
    let has_text = matches!(element.content, ElementContent::Text { .. });
    let has_image = matches!(element.content, ElementContent::Image { .. });
    let has_draw = element
        .draw
        .as_ref()
        .is_some_and(|draw| !draw.shapes_ref().is_empty());
    if has_text {
        capabilities |= VisualElementCapabilities::TEXT;
    }
    if has_image {
        capabilities |= VisualElementCapabilities::IMAGE;
    }
    if has_draw {
        capabilities |= VisualElementCapabilities::DRAW;
    }
    if element.background.is_some() || element.border.is_some() || has_text || has_draw {
        capabilities |= VisualElementCapabilities::SDF_MATERIAL;
    }
    capabilities
}

fn validate_part_state_colors(
    element: &Element,
    index: usize,
    owning_widget: Option<&(PanelElementId, WidgetKind)>,
) -> Result<(), PanelBuildError> {
    if owning_widget.is_none() || element.widget.is_some() || element.editable.is_some() {
        return Ok(());
    }
    let Some(appearance) = element.appearance.as_deref() else {
        return Ok(());
    };
    let capabilities = element_visual_capabilities(element);
    let element_id = || {
        element
            .id
            .clone()
            .unwrap_or_else(|| PanelElementId::auto(u32::try_from(index).unwrap_or(0)))
    };
    let cascades = appearance.cascades();
    if cascades.any(|layer| layer.text_color.is_authored())
        && !capabilities.contains(VisualElementCapabilities::TEXT)
    {
        return Err(PanelBuildError::StateTextColorRequiresText(element_id()));
    }
    if cascades.any(|layer| layer.path_color.is_authored())
        && !capabilities.contains(VisualElementCapabilities::DRAW)
    {
        return Err(PanelBuildError::StatePathColorRequiresDraw(element_id()));
    }
    if cascades.any(|layer| layer.tint.is_authored())
        && !capabilities.contains(VisualElementCapabilities::IMAGE)
    {
        return Err(PanelBuildError::StateTintRequiresImage(element_id()));
    }
    Ok(())
}

fn record_owned_widget_element(
    record: &mut ComputedWidgetRecord,
    element: &Element,
    element_index: usize,
    is_widget_root: bool,
) {
    if element.editor_origin == EditorElementOrigin::Generated {
        record.push_generated_editor_element(element_index);
    }
    let visual_element_capabilities = element_visual_capabilities(element);
    if !visual_element_capabilities.is_empty() {
        record.push_visual_element(element_index, visual_element_capabilities);
    }
    if !is_widget_root
        && let Some(appearance) = element.appearance.as_deref()
        && appearance.cascades().any_overridden()
    {
        record.push_part_appearance(element_index, appearance.clone());
    }
}

fn classify_element_change(element: &Element, next: &Element) -> LayoutTreeChange {
    let Element {
        id: _,
        width,
        height,
        padding,
        child_layout,
        background: _,
        border,
        corner_radius: _,
        overflow,
        scroll_offset,
        scroll_anchor_x,
        scroll_anchor_y,
        material: _,
        interactivity: _,
        editable,
        widget: _,
        appearance: _,
        editor_origin: _,
        editor_text: _,
        editor_selection: _,
        editor_caret: _,
        editor_validation: _,
        tooltip: _,
        visual_slot: _,
        draw: _,
        z_index: _,
        anti_alias: _,
        hairline_fade: _,
        shadow_casting: _,
        precompose: _,
        content,
    } = element;
    let Element {
        id: _,
        width: n_width,
        height: n_height,
        padding: n_padding,
        child_layout: n_child_layout,
        background: _,
        border: n_border,
        corner_radius: _,
        overflow: n_overflow,
        scroll_offset: n_scroll_offset,
        scroll_anchor_x: n_scroll_anchor_x,
        scroll_anchor_y: n_scroll_anchor_y,
        material: _,
        interactivity: _,
        editable: n_editable,
        widget: _,
        appearance: _,
        editor_origin: _,
        editor_text: _,
        editor_selection: _,
        editor_caret: _,
        editor_validation: _,
        tooltip: _,
        visual_slot: _,
        draw: _,
        z_index: _,
        anti_alias: _,
        hairline_fade: _,
        shadow_casting: _,
        precompose: _,
        content: n_content,
    } = next;

    let child_layout_change = classify_child_layout_change(child_layout, n_child_layout);
    if width != n_width
        || height != n_height
        || padding != n_padding
        || child_layout_change == LayoutTreeChange::LayoutAffecting
        || overflow != n_overflow
        || scroll_offset != n_scroll_offset
        || scroll_anchor_x != n_scroll_anchor_x
        || scroll_anchor_y != n_scroll_anchor_y
    {
        return LayoutTreeChange::LayoutAffecting;
    }

    if editable != n_editable {
        return LayoutTreeChange::LayoutAffecting;
    }

    let border_change = classify_border_change(*border, *n_border);
    if border_change == LayoutTreeChange::LayoutAffecting {
        return LayoutTreeChange::LayoutAffecting;
    }

    let mut change = border_change.combine(child_layout_change);
    if visual_only_properties_changed(element, next) {
        change = change.combine(LayoutTreeChange::VisualOnly);
    }

    change.combine(classify_content_change(content, n_content))
}

fn visual_only_properties_changed(element: &Element, next: &Element) -> bool {
    element.id != next.id
        || element.widget != next.widget
        || element.appearance != next.appearance
        || editor_parts_changed(
            (
                element.editor_text.as_deref(),
                element.editor_selection.as_deref(),
                element.editor_caret.as_deref(),
                element.editor_validation.as_deref(),
            ),
            (
                next.editor_text.as_deref(),
                next.editor_selection.as_deref(),
                next.editor_caret.as_deref(),
                next.editor_validation.as_deref(),
            ),
        )
        || element.tooltip != next.tooltip
        || element.visual_slot != next.visual_slot
        || element.interactivity != next.interactivity
        || element.background != next.background
        || element.corner_radius != next.corner_radius
        || element.draw != next.draw
        || element.z_index != next.z_index
        || element.anti_alias != next.anti_alias
        || element.hairline_fade != next.hairline_fade
        || element.shadow_casting != next.shadow_casting
        || element.precompose != next.precompose
        || element.material != next.material
}

fn editor_parts_changed(
    editor_parts: (
        Option<&EditorPart>,
        Option<&EditorPart>,
        Option<&EditorPart>,
        Option<&EditorPart>,
    ),
    next_editor_parts: (
        Option<&EditorPart>,
        Option<&EditorPart>,
        Option<&EditorPart>,
        Option<&EditorPart>,
    ),
) -> bool {
    editor_parts != next_editor_parts
}

fn classify_child_layout_change(old: &ChildLayout, next: &ChildLayout) -> LayoutTreeChange {
    match (old, next) {
        (
            ChildLayout::Row {
                gap,
                align_x,
                align_y,
                divider,
            },
            ChildLayout::Row {
                gap: n_gap,
                align_x: n_align_x,
                align_y: n_align_y,
                divider: n_divider,
            },
        )
        | (
            ChildLayout::Column {
                gap,
                align_x,
                align_y,
                divider,
            },
            ChildLayout::Column {
                gap: n_gap,
                align_x: n_align_x,
                align_y: n_align_y,
                divider: n_divider,
            },
        ) => {
            if gap != n_gap || align_x != n_align_x || align_y != n_align_y {
                LayoutTreeChange::LayoutAffecting
            } else {
                classify_child_divider_change(*divider, *n_divider)
            }
        },
        (
            ChildLayout::Overlay { align_x, align_y },
            ChildLayout::Overlay {
                align_x: n_align_x,
                align_y: n_align_y,
            },
        ) => {
            if align_x != n_align_x || align_y != n_align_y {
                LayoutTreeChange::LayoutAffecting
            } else {
                LayoutTreeChange::Identical
            }
        },
        _ => LayoutTreeChange::LayoutAffecting,
    }
}

fn classify_child_divider_change(
    divider: Option<ChildDivider>,
    next: Option<ChildDivider>,
) -> LayoutTreeChange {
    match (divider, next) {
        (None, None) => LayoutTreeChange::Identical,
        (Some(divider), Some(next)) => {
            if divider.width() != next.width() {
                LayoutTreeChange::LayoutAffecting
            } else if divider.color() != next.color() {
                LayoutTreeChange::VisualOnly
            } else {
                LayoutTreeChange::Identical
            }
        },
        (None, Some(_)) | (Some(_), None) => LayoutTreeChange::LayoutAffecting,
    }
}

fn classify_border_change(border: Option<Border>, next: Option<Border>) -> LayoutTreeChange {
    match (border, next) {
        (None, None) => LayoutTreeChange::Identical,
        (Some(border), Some(next)) => {
            let Border {
                color,
                left,
                right,
                top,
                bottom,
            } = border;
            let Border {
                color: n_color,
                left: n_left,
                right: n_right,
                top: n_top,
                bottom: n_bottom,
            } = next;
            if left != n_left || right != n_right || top != n_top || bottom != n_bottom {
                LayoutTreeChange::LayoutAffecting
            } else if color != n_color {
                LayoutTreeChange::VisualOnly
            } else {
                LayoutTreeChange::Identical
            }
        },
        (None, Some(_)) | (Some(_), None) => LayoutTreeChange::LayoutAffecting,
    }
}

fn classify_content_change(content: &ElementContent, next: &ElementContent) -> LayoutTreeChange {
    match (content, next) {
        (ElementContent::Children(children), ElementContent::Children(next_children)) => {
            if children == next_children {
                LayoutTreeChange::Identical
            } else {
                LayoutTreeChange::LayoutAffecting
            }
        },
        (
            ElementContent::Text {
                text,
                config,
                sizing,
                ..
            },
            ElementContent::Text {
                text: next_text,
                config: next_config,
                sizing: next_sizing,
                ..
            },
        ) => {
            if sizing != next_sizing
                || (!config.layout_eq_excluding_visuals(next_config))
                || (sizing.visible_text_affects_layout() && text != next_text)
            {
                LayoutTreeChange::LayoutAffecting
            } else if text != next_text || config != next_config {
                LayoutTreeChange::VisualOnly
            } else {
                LayoutTreeChange::Identical
            }
        },
        (
            ElementContent::Image { handle, tint },
            ElementContent::Image {
                handle: next_handle,
                tint: next_tint,
            },
        ) => {
            if handle == next_handle && tint == next_tint {
                LayoutTreeChange::Identical
            } else {
                LayoutTreeChange::VisualOnly
            }
        },
        (ElementContent::Empty, ElementContent::Empty) => LayoutTreeChange::Identical,
        _ => LayoutTreeChange::LayoutAffecting,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use bevy::asset::Assets;
    use bevy::asset::Handle;
    use bevy::color::Color;
    use bevy::image::Image;
    use bevy::pbr::StandardMaterial;
    use bevy::prelude::default;

    use super::ElementContent;
    use super::FieldDisplayTextUpdate;
    use super::LayoutTree;
    use super::LayoutTreeChange;
    use super::PrecomposeMode;
    use super::element_visual_capabilities;
    use crate::Appearance;
    use crate::CalloutCap;
    use crate::DiegeticPanel;
    use crate::EditorStateColors;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::Mm;
    use crate::PanelBuildError;
    use crate::PanelElementId;
    use crate::Slider;
    use crate::WidgetHoveredAppearance;
    use crate::WidgetInteractivity;
    use crate::cascade::Cascade;
    use crate::layout::AlignX;
    use crate::layout::AlignY;
    use crate::layout::Border;
    use crate::layout::ChildDivider;
    use crate::layout::ChildLayoutState;
    use crate::layout::Dimension;
    use crate::layout::El;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutEngine;
    use crate::layout::LayoutOnly;
    use crate::layout::Padding;
    use crate::layout::PanelCircle;
    use crate::layout::PanelCoord;
    use crate::layout::PanelDraw;
    use crate::layout::PanelLine;
    use crate::layout::PanelShape;
    use crate::layout::Sizing;
    use crate::layout::Text;
    use crate::layout::TextDimensions;
    use crate::layout::TextStyle;
    use crate::layout::Unit;
    use crate::layout::WidgetElement;
    use crate::layout::child_layout::ChildLayout;
    use crate::widgets::StateAppearance;
    use crate::widgets::VisualElementCapabilities;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetState;

    const LARGE_CHILD_GAP: f32 = 2.0;
    const SMALL_CHILD_GAP: f32 = 1.0;
    const FLOAT_TOLERANCE: f32 = 0.001;

    fn text_tree(text: &str, style: TextStyle) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text((text, style));
        builder.build()
    }

    fn build_world_panel(tree: LayoutTree) -> Result<DiegeticPanel, PanelBuildError> {
        DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build()
    }

    #[test]
    fn contains_text_id_discriminates_named_typo_from_present() {
        let present = PanelElementId::named("title");
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(Text::new("Hi", TextStyle::new(10.0)).id(present.clone()));
        let tree = builder.build();

        // A present named id is found; a typo is not — this is the discriminator a
        // `PanelText` lookup miss consults to tell a real typo from a not-yet-built
        // run, so it only warns on the former.
        assert!(tree.contains_text_id(&present));
        assert!(!tree.contains_text_id(&PanelElementId::named("typo")));

        // An auto-id run (plain `.text`) is not name-addressable, so a named query
        // never matches it.
        let auto = text_tree("Hi", TextStyle::new(10.0));
        assert!(!auto.contains_text_id(&PanelElementId::named("Hi")));
    }

    #[test]
    fn text_id_survives_layout_without_its_own_id() {
        let id = PanelElementId::named("title");
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("Hi", TextStyle::new(10.0))
                .id(id.clone())
                .layout(El::column()),
        );
        let tree = builder.build();

        assert_eq!(tree.text_element_id(1), Some(&id));
    }

    #[test]
    fn text_layout_id_overrides_prior_text_id() {
        let id = PanelElementId::named("layout-id");
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("Hi", TextStyle::new(10.0))
                .id("text-id")
                .layout(El::column().id(id.clone())),
        );
        let tree = builder.build();

        assert_eq!(tree.text_element_id(1), Some(&id));
    }

    fn root_tree<L: ChildLayoutState>(root: El<L, LayoutOnly>) -> LayoutTree {
        let mut builder = LayoutBuilder::with_root(root);
        builder.text(("child", TextStyle::new(10.0)));
        builder.build()
    }

    fn widget_root_tree<L: ChildLayoutState, W: crate::WidgetOwner>(
        root: El<L, WidgetElement<W>>,
    ) -> LayoutTree {
        let mut builder = LayoutBuilder::with_widget_root(root);
        builder.text(("child", TextStyle::new(10.0)));
        builder.build()
    }

    fn hovered_background(color: Color) -> StateAppearance {
        StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(
                Appearance::new().background(color),
            )),
            ..default()
        }
    }

    #[test]
    fn precompose_ldr_sets_element_mode() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::column().precompose_ldr(), |builder| {
            builder.text(("child", TextStyle::new(10.0)));
        });
        let tree = builder.build();

        assert_eq!(tree.element_precompose(1), PrecomposeMode::Ldr);
        assert_eq!(tree.element_precompose(2), PrecomposeMode::Direct);
    }

    #[test]
    fn text_precompose_ldr_sets_text_element_mode() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(Text::new("child", TextStyle::new(10.0)).precompose_ldr());
        let tree = builder.build();

        assert_eq!(tree.element_precompose(1), PrecomposeMode::Ldr);
    }

    #[test]
    fn precompose_subtree_clears_modes_and_fixes_root_size() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::column().precompose_ldr(), |builder| {
            builder.text((
                "child",
                TextStyle::new(10.0).with_hdr_text_coverage_bias(2.0),
            ));
        });
        let tree = builder.build();

        let subtree = tree.precompose_subtree(
            1,
            Dimension {
                value: 30.0,
                unit:  None,
            },
            Dimension {
                value: 20.0,
                unit:  None,
            },
        );
        assert!(subtree.is_some());
        let Some(subtree) = subtree else {
            return;
        };
        assert!(subtree.root.is_some());
        let Some(root) = subtree.root else {
            return;
        };

        assert_eq!(subtree.element_precompose(root), PrecomposeMode::Direct);
        assert_eq!(subtree.element_precompose(1), PrecomposeMode::Direct);
        assert_eq!(
            subtree
                .element_style(1)
                .map_or(Cascade::Inherit, TextStyle::hdr_text_coverage_bias),
            Cascade::Inherit
        );
        assert!(matches!(subtree.elements[root].width, Sizing::Fixed(_)));
        let Sizing::Fixed(width) = subtree.elements[root].width else {
            return;
        };
        assert!((width.value - 30.0).abs() < FLOAT_TOLERANCE);
        assert!(matches!(subtree.elements[root].height, Sizing::Fixed(_)));
        let Sizing::Fixed(height) = subtree.elements[root].height else {
            return;
        };
        assert!((height.value - 20.0).abs() < FLOAT_TOLERANCE);
    }

    #[test]
    fn screen_source_scaled_resolves_world_units_to_pixel_source_values() {
        let mut builder = LayoutBuilder::with_root(
            El::column()
                .width(Sizing::fixed(Mm(20.0)))
                .height(Sizing::fixed(Mm(10.0)))
                .padding(Padding::all(Mm(2.0))),
        );
        builder.text(("child", TextStyle::new(3.0)));
        let tree = builder.build();

        let millimeters_to_points = Unit::Millimeters.to_points();
        let points_to_pixels = 2.0;
        let scaled = tree.screen_source_scaled(
            millimeters_to_points,
            millimeters_to_points,
            points_to_pixels,
        );

        assert!(matches!(scaled.elements[0].width, Sizing::Fixed(_)));
        let Sizing::Fixed(width) = scaled.elements[0].width else {
            return;
        };
        let expected_width = 20.0 * millimeters_to_points * points_to_pixels;
        assert!((width.value - expected_width).abs() < FLOAT_TOLERANCE);
        assert_eq!(width.unit, None);

        let padding = scaled.elements[0].padding;
        let expected_padding = 2.0 * millimeters_to_points * points_to_pixels;
        assert!((padding.left.value - expected_padding).abs() < FLOAT_TOLERANCE);
        assert_eq!(padding.left.unit, None);

        assert!(matches!(
            &scaled.elements[1].content,
            ElementContent::Text { .. }
        ));
        let ElementContent::Text { config, .. } = &scaled.elements[1].content else {
            return;
        };
        let expected_text_size = 3.0 * millimeters_to_points * points_to_pixels;
        assert!((config.size() - expected_text_size).abs() < FLOAT_TOLERANCE);
        assert_eq!(config.unit(), Cascade::Override(Unit::Pixels));
    }

    #[test]
    fn screen_source_scaled_preserves_editor_caret_state_colors() {
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(
            El::new()
                .editable_field("field", field_spec())
                .editor_caret(EditorStateColors::new().hovered(Color::srgb(0.2, 0.4, 0.6))),
            |_| {},
        );
        let tree = builder.build();
        let millimeters_to_points = Unit::Millimeters.to_points();
        let points_to_pixels = 2.0;
        let scaled = tree.screen_source_scaled(
            millimeters_to_points,
            millimeters_to_points,
            points_to_pixels,
        );
        let declaration = scaled.elements[1]
            .editor_caret
            .as_deref()
            .cloned()
            .expect("editor caret declaration should be retained");

        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.text(declaration.into_text("caret", &TextStyle::new(3.0)));
        let generated = builder.build();
        let appearance = generated.elements[1]
            .appearance
            .as_deref()
            .expect("editor caret state colors should be retained");
        assert_eq!(
            appearance
                .cascades()
                .resolve(&[Some(WidgetState::Hovered)], None),
            VisualSlotOverride {
                fill_color: Some(Color::srgb(0.2, 0.4, 0.6)),
                ..VisualSlotOverride::default()
            },
        );
    }

    fn assert_default_leaf_child_layout(child_layout: ChildLayout) {
        assert_eq!(child_layout, ChildLayout::default());
    }

    fn field_spec() -> ImeEditableFieldSpec {
        ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text))
    }

    fn panel_line() -> PanelLine {
        PanelLine::new((1.0, 2.0), (PanelCoord::end(3.0), PanelCoord::percent(0.5)))
            .width(0.25)
            .cap_size(4.0)
            .start_inset(0.5)
            .end_inset(0.75)
            .start_cap(CalloutCap::arrow().length_dimension(Mm(2.0)))
            .end_cap(CalloutCap::circle().radius(1.5))
    }

    #[test]
    fn text_leaf_normalizes_authored_child_layout() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("child", TextStyle::new(10.0)).layout(
                El::column()
                    .gap(4.0)
                    .alignment(AlignX::Right, AlignY::Bottom)
                    .child_divider(ChildDivider::new(1.0, Color::WHITE)),
            ),
        );
        let tree = builder.build();

        assert_default_leaf_child_layout(tree.elements[1].child_layout);
    }

    #[test]
    fn image_leaf_normalizes_authored_child_layout() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.image(
            El::column()
                .gap(4.0)
                .alignment(AlignX::Right, AlignY::Bottom)
                .child_divider(ChildDivider::new(1.0, Color::WHITE)),
            Handle::<Image>::default(),
            Color::WHITE,
        );
        let tree = builder.build();

        assert_default_leaf_child_layout(tree.elements[1].child_layout);
    }

    #[test]
    fn text_leaf_normalizes_authored_overlay_child_layout() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("child", TextStyle::new(10.0))
                .layout(El::overlay().alignment(AlignX::Right, AlignY::Bottom)),
        );
        let tree = builder.build();

        assert_default_leaf_child_layout(tree.elements[1].child_layout);
    }

    #[test]
    fn image_leaf_normalizes_authored_overlay_child_layout() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.image(
            El::overlay().alignment(AlignX::Right, AlignY::Bottom),
            Handle::<Image>::default(),
            Color::WHITE,
        );
        let tree = builder.build();

        assert_default_leaf_child_layout(tree.elements[1].child_layout);
    }

    fn approx_eq(a: f32, b: f32) -> bool { (a - b).abs() < f32::EPSILON }

    fn assert_some_approx(actual: Option<f32>, expected: f32) {
        assert!(actual.is_some_and(|value| approx_eq(value, expected)));
    }

    #[test]
    fn identical_tree_classifies_as_identical() {
        let tree = text_tree("same", TextStyle::new(10.0));

        assert_eq!(
            tree.classify_change(&tree.clone()),
            LayoutTreeChange::Identical
        );
    }

    #[test]
    fn text_color_only_classifies_as_visual_only() {
        let tree = text_tree("same", TextStyle::new(10.0).with_color(Color::WHITE));
        let next = text_tree("same", TextStyle::new(10.0).with_color(Color::BLACK));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn background_add_remove_classifies_as_visual_only() {
        let tree = root_tree(El::new().width(Sizing::GROW).height(Sizing::GROW));
        let next = root_tree(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::srgb(0.2, 0.3, 0.4)),
        );

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn button_record_add_remove_classifies_as_visual_only() {
        let tree = root_tree(El::new());
        let next = widget_root_tree(El::new().button("action"));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
        assert_eq!(next.classify_change(&tree), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn slider_record_edit_classifies_as_visual_only() {
        let slider = Slider::new(0.0..=10.0).value(2.0);
        let next_slider = Slider::new(0.0..=10.0).value(8.0);
        let tree = widget_root_tree(El::new().widget("level", slider));
        let next = widget_root_tree(El::new().widget("level", next_slider));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn interactivity_only_classifies_as_visual_only() {
        let tree = widget_root_tree(El::new().button("action"));
        let next = widget_root_tree(
            El::new()
                .button("action")
                .widget_interactivity(WidgetInteractivity::Disabled),
        );

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn visual_slot_add_remove_classifies_as_visual_only() {
        let tree = widget_root_tree(El::new().button("action"));
        let next = widget_root_tree(El::new().button("action").visual_slot(VisualSlotId::new(1)));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
        assert_eq!(next.classify_change(&tree), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn button_state_value_change_classifies_as_visual_only() {
        let tree = widget_root_tree(
            El::new()
                .background(Color::WHITE)
                .button("action")
                .hovered(Appearance::new().background(Color::srgb(0.2, 0.4, 0.8))),
        );
        let next = widget_root_tree(
            El::new()
                .background(Color::WHITE)
                .button("action")
                .hovered(Appearance::new().background(Color::srgb(0.8, 0.4, 0.2))),
        );

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn button_elements_carry_the_root_visual_slot() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("action"), |_| {});
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions::default()));
        let result = engine.compute(&tree, 100.0, 50.0, 1.0);
        let records = tree.computed_widget_records(&result);

        assert_eq!(records.len(), 1);
        let slots = records[0].visual_slots();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].slot, VisualSlotId::BUTTON_ROOT);
    }

    #[test]
    fn computed_widgets_carry_subtree_visual_slot_references() {
        const WIDGET_SLOT: VisualSlotId = VisualSlotId::new(1);
        const CHILD_SLOT: VisualSlotId = VisualSlotId::new(2);
        const ORPHAN_SLOT: VisualSlotId = VisualSlotId::new(3);

        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().button("styled").visual_slot(WIDGET_SLOT),
            |builder| {
                builder.with(El::new().visual_slot(CHILD_SLOT), |_| {});
            },
        );
        builder.with(El::new().visual_slot(ORPHAN_SLOT), |_| {});
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions::default()));
        let result = engine.compute(&tree, 100.0, 50.0, 1.0);
        let records = tree.computed_widget_records(&result);

        assert_eq!(records.len(), 1);
        let slots = records[0].visual_slots();
        assert_eq!(slots.len(), 2);
        let widget_slot = slots.iter().find(|slot| slot.slot == WIDGET_SLOT);
        let child_slot = slots.iter().find(|slot| slot.slot == CHILD_SLOT);
        assert!(widget_slot.is_some());
        assert!(child_slot.is_some());
        assert_ne!(
            widget_slot.map(|slot| slot.element_index),
            child_slot.map(|slot| slot.element_index),
            "each slot references its own element's records",
        );
        assert!(
            slots.iter().all(|slot| slot.slot != ORPHAN_SLOT),
            "slots outside a widget subtree belong to no widget",
        );
    }

    #[test]
    fn computed_widget_part_appearances_are_sparse_sorted_and_exclude_the_root() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(Color::WHITE)
                .button("action")
                .hovered(Appearance::new().background(Color::BLACK)),
            |builder| {
                builder.with(El::new(), |_| {});
                builder.with(El::new(), |_| {});
                builder.with(El::new(), |_| {});
                builder.with(El::new(), |_| {});
            },
        );
        let mut tree = builder.build();
        let Some(root_index) = tree.root else {
            return;
        };
        let Some(&widget_index) = tree.children_of(root_index).first() else {
            return;
        };
        let owned = tree.children_of(widget_index).to_vec();
        let Some(&first_child) = owned.first() else {
            return;
        };
        let Some(&second_child) = owned.get(1) else {
            return;
        };
        let Some(&empty_child) = owned.get(2) else {
            return;
        };
        let Some(&overridden_empty_child) = owned.get(3) else {
            return;
        };
        tree.elements[first_child].appearance =
            Some(Box::new(hovered_background(Color::srgb(1.0, 0.0, 0.0))));
        tree.elements[second_child].appearance =
            Some(Box::new(hovered_background(Color::srgb(0.0, 0.0, 1.0))));
        tree.elements[empty_child].appearance = Some(Box::new(StateAppearance::default()));
        tree.elements[overridden_empty_child].appearance = Some(Box::new(StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(Appearance::new())),
            ..default()
        }));

        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions::default()));
        let before_result = engine.compute(&tree, 100.0, 50.0, 1.0);
        let before_records = tree.computed_widget_records(&before_result);
        assert_eq!(before_records.len(), 1);
        let before_parts = before_records[0].part_appearances().to_vec();
        if let ElementContent::Children(children) = &mut tree.elements[widget_index].content {
            children.swap(0, 1);
        } else {
            return;
        }

        let after_result = engine.compute(&tree, 100.0, 50.0, 1.0);
        let records = tree.computed_widget_records(&after_result);

        assert_eq!(records.len(), 1);
        let mut expected = vec![first_child, second_child, overridden_empty_child];
        expected.sort_unstable();
        let parts = records[0].part_appearances();
        assert_eq!(parts, before_parts.as_slice());
        assert_eq!(
            parts
                .iter()
                .map(|(element_index, _)| *element_index)
                .collect::<Vec<_>>(),
            expected,
        );
        assert!(
            parts
                .iter()
                .all(|(element_index, _)| *element_index != widget_index),
            "the widget root remains in ComputedWidgetRecord::appearance",
        );
        assert!(
            parts
                .iter()
                .all(|(element_index, _)| *element_index != empty_child),
            "an appearance with four inherited channels has no sparse-map entry",
        );
        assert!(
            parts
                .iter()
                .find(|(element_index, _)| *element_index == overridden_empty_child)
                .is_some_and(|(_, appearance)| appearance.hovered.as_override().is_some()),
            "an overridden empty appearance keeps its hovered cascade channel",
        );
    }

    #[test]
    fn authored_slider_parts_reach_sorted_visual_recipients() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().slider("level", 0.0..=1.0), |builder| {
            builder.with(
                builder
                    .child(El::new().background(Color::WHITE))
                    .disabled(Appearance::new().background(Color::BLACK)),
                |_| {},
            );
            builder.with(
                builder
                    .child(
                        El::new()
                            .background(Color::WHITE)
                            .border(Border::all(1.0, Color::BLACK))
                            .slider_thumb(),
                    )
                    .disabled(Appearance::new().background(Color::BLACK)),
                |_| {},
            );
            builder.text(
                Text::new("LEVEL", TextStyle::new(10.0)).layout(
                    builder
                        .child(El::new().background(Color::NONE))
                        .disabled(Appearance::new().background(Color::BLACK)),
                ),
            );
        });
        let tree = builder.build();

        assert!(tree.validate_widgets().is_ok());

        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions::default()));
        let result = engine.compute(&tree, 100.0, 50.0, 1.0);
        let records = tree.computed_widget_records(&result);
        assert_eq!(records.len(), 1);

        let record = &records[0];
        let parts = record.part_appearances();
        assert_eq!(parts.len(), 3);
        assert!(parts.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(parts.iter().all(|(element_index, _)| {
            record
                .visual_elements()
                .iter()
                .any(|(visual_index, _)| visual_index == element_index)
        }));
    }

    #[test]
    fn computed_widget_recipients_exclude_example_slider_containers() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::column().widget("level", Slider::new(0.0..=1.0)),
            |builder| {
                builder.with(El::overlay(), |builder| {
                    builder.text(("Level", TextStyle::new(10.0)));
                });
                builder.with(El::overlay(), |builder| {
                    builder.with(El::overlay(), |builder| {
                        builder.with(El::new().background(Color::WHITE), |_| {});
                    });
                    builder.with(El::overlay(), |builder| {
                        builder.with(
                            El::new()
                                .background(Color::WHITE)
                                .border(Border::all(1.0, Color::BLACK))
                                .slider_thumb(),
                            |_| {},
                        );
                    });
                });
            },
        );
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions::default()));
        let result = engine.compute(&tree, 100.0, 50.0, 1.0);
        let records = tree.computed_widget_records(&result);

        assert_eq!(
            tree.elements.len(),
            9,
            "one layout root plus eight slider nodes"
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].visual_elements().len(), 3);
        assert_eq!(
            records[0]
                .visual_elements()
                .iter()
                .map(|(_, capabilities)| *capabilities)
                .collect::<Vec<_>>(),
            vec![
                VisualElementCapabilities::TEXT | VisualElementCapabilities::SDF_MATERIAL,
                VisualElementCapabilities::SDF_FILL | VisualElementCapabilities::SDF_MATERIAL,
                VisualElementCapabilities::SDF_FILL
                    | VisualElementCapabilities::SDF_BORDER
                    | VisualElementCapabilities::SDF_MATERIAL,
            ],
        );
    }

    #[test]
    fn part_state_text_color_requires_a_text_recipient() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("button"), |children| {
            children.with(
                children
                    .child(El::new().id("container"))
                    .disabled(Appearance::new().text_color(Color::BLACK)),
                |_| {},
            );
        });

        assert!(matches!(
            build_world_panel(builder.build()),
            Err(PanelBuildError::StateTextColorRequiresText(id))
                if id == PanelElementId::named("container")
        ));
    }

    #[test]
    fn part_state_path_color_requires_a_draw_recipient() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("button"), |children| {
            children.with(
                children
                    .child(El::new().id("container"))
                    .disabled(Appearance::new().path_color(Color::BLACK)),
                |_| {},
            );
        });

        assert!(matches!(
            build_world_panel(builder.build()),
            Err(PanelBuildError::StatePathColorRequiresDraw(id))
                if id == PanelElementId::named("container")
        ));
    }

    #[test]
    fn part_state_tint_requires_an_image_recipient() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("button"), |children| {
            children.with(
                children
                    .child(El::new().id("container"))
                    .disabled(Appearance::new().tint(Color::BLACK)),
                |_| {},
            );
        });

        assert!(matches!(
            build_world_panel(builder.build()),
            Err(PanelBuildError::StateTintRequiresImage(id))
                if id == PanelElementId::named("container")
        ));
    }

    #[test]
    fn panel_text_material_default_stays_dormant_on_textless_widget_part() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("button"), |children| {
            children.with(El::new().id("container"), |_| {});
        });

        assert!(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .text_material(Handle::default())
                .with_tree(builder.build())
                .build()
                .is_ok()
        );
    }

    #[test]
    fn circle_part_state_material_and_path_color_need_no_default_fill() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("button"), |children| {
            children.with(
                children
                    .child(
                        El::new()
                            .id("circle")
                            .draw(PanelDraw::shapes([PanelCircle::new((10.0, 10.0), 5.0)])),
                    )
                    .disabled(
                        Appearance::new()
                            .material(Handle::default())
                            .path_color(Color::BLACK),
                    ),
                |_| {},
            );
        });
        let tree = builder.build();

        assert!(build_world_panel(tree.clone()).is_ok());
        let circle = tree
            .elements
            .iter()
            .find(|element| element.id.as_ref() == Some(&PanelElementId::named("circle")))
            .expect("circle recipient should remain in the layout tree");
        assert_eq!(circle.background, None);
        assert_eq!(
            element_visual_capabilities(circle),
            VisualElementCapabilities::DRAW | VisualElementCapabilities::SDF_MATERIAL,
        );
    }

    #[test]
    fn editable_tree_replacement_rekeys_part_appearance_entries() {
        let mut display_builder = LayoutBuilder::new(100.0, 50.0);
        display_builder.with(El::new().editable_field("gain", field_spec()), |builder| {
            builder.with(El::new(), |_| {});
        });
        let mut display = display_builder.build();
        let Some(display_index) = display
            .elements
            .iter()
            .position(|element| matches!(element.content, ElementContent::Empty))
            .filter(|index| *index != 0)
        else {
            return;
        };
        display.elements[display_index].appearance =
            Some(Box::new(hovered_background(Color::srgb(1.0, 0.0, 0.0))));

        let mut editor_builder = LayoutBuilder::with_root(El::new());
        editor_builder.with(El::new(), |_| {});
        let mut editor_content = editor_builder.build();
        let Some(editor_root) = editor_content.root else {
            return;
        };
        let Some(&editor_child) = editor_content.children_of(editor_root).first() else {
            return;
        };
        editor_content.elements[editor_child].appearance =
            Some(Box::new(hovered_background(Color::srgb(0.0, 0.0, 1.0))));

        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions::default()));
        let display_result = engine.compute(&display, 100.0, 50.0, 1.0);
        let display_parts = display.computed_widget_records(&display_result)[0]
            .part_appearances()
            .iter()
            .map(|(element_index, _)| *element_index)
            .collect::<Vec<_>>();

        let mut editor = display.clone();
        assert_eq!(
            editor.set_field_editing_content(&PanelElementId::named("gain"), &editor_content),
            FieldDisplayTextUpdate::Updated,
        );
        let editor_result = engine.compute(&editor, 100.0, 50.0, 1.0);
        let editor_parts = editor.computed_widget_records(&editor_result)[0]
            .part_appearances()
            .iter()
            .map(|(element_index, _)| *element_index)
            .collect::<Vec<_>>();

        let display_again_result = engine.compute(&display, 100.0, 50.0, 1.0);
        let display_again_parts = display.computed_widget_records(&display_again_result)[0]
            .part_appearances()
            .iter()
            .map(|(element_index, _)| *element_index)
            .collect::<Vec<_>>();

        assert_eq!(display_parts, vec![display_index]);
        assert_eq!(editor_parts.len(), 1);
        assert_ne!(editor_parts, display_parts);
        assert_eq!(display_again_parts, display_parts);
    }

    #[test]
    fn computed_widgets_fold_nearest_layout_interactivity() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::column().widget_interactivity(WidgetInteractivity::Disabled),
            |builder| {
                builder.with(El::new().button("disabled"), |_| {});
                builder.with(
                    El::row().widget_interactivity(WidgetInteractivity::Enabled),
                    |builder| {
                        builder.with(El::new().button("enabled"), |_| {});
                    },
                );
            },
        );
        builder.with(El::new().button("inherited"), |_| {});
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions::default()));
        let result = engine.compute(&tree, 100.0, 50.0, 1.0);
        let records = tree.computed_widget_records(&result);

        assert_eq!(records.len(), 3);
        assert_eq!(
            records[0].interactivity(),
            Cascade::Override(WidgetInteractivity::Disabled)
        );
        assert_eq!(
            records[1].interactivity(),
            Cascade::Override(WidgetInteractivity::Enabled)
        );
        assert_eq!(records[2].interactivity(), Cascade::Inherit);
    }

    #[test]
    fn computed_widgets_store_clipped_rects_and_visual_interaction_rank() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::row().size(20.0, 20.0).clip(), |builder| {
            builder.with(El::new().size(30.0, 10.0).button("partial"), |_| {});
            builder.with(El::new().size(10.0, 10.0).button("clipped"), |_| {});
        });
        builder.with(El::overlay().size(20.0, 20.0), |builder| {
            builder.with(
                El::new().size(20.0, 20.0).button("back").z_index(-1),
                |_| {},
            );
            builder.with(
                El::new().size(20.0, 20.0).button("front-first").z_index(2),
                |_| {},
            );
            builder.with(
                El::new().size(20.0, 20.0).button("front-last").z_index(2),
                |_| {},
            );
        });
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(|_, _| TextDimensions::default()));
        let result = engine.compute(&tree, 100.0, 50.0, 1.0);
        let records = tree.computed_widget_records(&result);

        let partial = records
            .iter()
            .find(|record| record.id() == &PanelElementId::named("partial"))
            .expect("partially clipped widget should have a record");
        assert!((partial.rect().width - 30.0).abs() < FLOAT_TOLERANCE);
        assert!(
            partial
                .clipped_rect()
                .is_some_and(|rect| (rect.width - 20.0).abs() < FLOAT_TOLERANCE)
        );
        assert_eq!(
            records
                .iter()
                .find(|record| record.id() == &PanelElementId::named("clipped"))
                .and_then(super::ComputedWidgetRecord::clipped_rect),
            None
        );

        let rank = |id| {
            records
                .iter()
                .find(|record| record.id() == &PanelElementId::named(id))
                .map(super::ComputedWidgetRecord::interaction_rank)
        };
        assert!(rank("back") < rank("front-first"));
        assert!(rank("front-first") < rank("front-last"));
    }

    #[test]
    fn z_index_only_classifies_as_visual_only() {
        let tree = root_tree(El::new().width(Sizing::GROW).height(Sizing::GROW));
        let next = root_tree(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .z_index(1),
        );

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn builder_stores_panel_draw_on_element() {
        let tree = root_tree(El::new().draw(PanelDraw::lines([panel_line()])));

        assert_eq!(
            tree.element_draw(0).map(|draw| draw.shapes_ref().len()),
            Some(1)
        );
    }

    #[test]
    fn draw_only_change_classifies_as_visual_only() {
        let tree = root_tree(El::new());
        let next = root_tree(El::new().draw(PanelDraw::lines([panel_line()])));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn draw_only_change_is_excluded_from_structure_hash() {
        let tree = root_tree(El::new());
        let next = root_tree(El::new().draw(PanelDraw::lines([panel_line()])));

        assert_eq!(tree.structure_hash(), next.structure_hash());
    }

    #[test]
    fn scaled_tree_scales_panel_draw_dimensions() {
        let scale = 3.0;
        let tree = root_tree(El::new().draw(PanelDraw::lines([panel_line()])));
        let scaled = tree.scaled(scale, scale);
        let line = scaled
            .element_draw(0)
            .and_then(|draw| draw.shapes_ref().first())
            .and_then(PanelShape::as_line);

        assert_some_approx(
            line.and_then(|line| line.start().x().start_dimension())
                .map(|dimension| dimension.value),
            3.0,
        );
        assert_some_approx(
            line.and_then(|line| line.start().y().start_dimension())
                .map(|dimension| dimension.value),
            6.0,
        );
        assert_some_approx(
            line.and_then(|line| line.end().x().end_dimension())
                .map(|dimension| dimension.value),
            9.0,
        );
        assert_eq!(
            line.and_then(|line| line.end().y().percent_value()),
            Some(0.5)
        );
        assert_some_approx(
            line.map(|line| line.line_style().width_dimension().value),
            0.75,
        );
        assert_some_approx(
            line.map(|line| line.line_style().cap_size_dimension().value),
            12.0,
        );
        assert_some_approx(
            line.map(PanelLine::start_inset_dimension)
                .map(|dimension| dimension.value),
            1.5,
        );
        assert_some_approx(
            line.map(PanelLine::end_inset_dimension)
                .map(|dimension| dimension.value),
            2.25,
        );

        let expected_arrow_inset = Dimension::from(Mm(2.0)).to_points(scale);
        assert_some_approx(
            line.map(|line| {
                line.line_style()
                    .start_cap_value()
                    .resolved_primitives(99.0, Color::WHITE, |dimension| dimension.value)
                    .shaft_inset
            }),
            expected_arrow_inset,
        );
        assert_some_approx(
            line.map(|line| {
                line.line_style()
                    .end_cap_value()
                    .resolved_primitives(99.0, Color::WHITE, |dimension| dimension.value)
                    .shaft_inset
            }),
            4.5,
        );
    }

    #[test]
    fn text_content_change_classifies_as_layout_affecting() {
        let tree = text_tree("before", TextStyle::new(10.0));
        let next = text_tree("after", TextStyle::new(10.0));

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn text_measurement_change_classifies_as_layout_affecting() {
        let tree = text_tree("same", TextStyle::new(10.0));
        let next = text_tree("same", TextStyle::new(11.0));

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn border_color_only_classifies_as_visual_only() {
        let tree = root_tree(El::new().border(Border::all(2.0, Color::WHITE)));
        let next = root_tree(El::new().border(Border::all(2.0, Color::BLACK)));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn border_width_change_classifies_as_layout_affecting() {
        let tree = root_tree(El::new().border(Border::all(2.0, Color::WHITE)));
        let next = root_tree(El::new().border(Border::all(3.0, Color::WHITE)));

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn divider_color_only_classifies_as_visual_only() {
        let tree = root_tree(El::row().child_divider(ChildDivider::new(2.0, Color::WHITE)));
        let next = root_tree(El::row().child_divider(ChildDivider::new(2.0, Color::BLACK)));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn divider_width_change_classifies_as_layout_affecting() {
        let tree = root_tree(El::row().child_divider(ChildDivider::new(2.0, Color::WHITE)));
        let next = root_tree(El::row().child_divider(ChildDivider::new(3.0, Color::WHITE)));

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn combined_visual_and_layout_change_classifies_as_layout_affecting() {
        let tree = root_tree(
            El::new()
                .padding(Padding::all(4.0))
                .background(Color::WHITE),
        );
        let next = root_tree(
            El::new()
                .padding(Padding::all(8.0))
                .background(Color::BLACK),
        );

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn row_gap_change_classifies_as_layout_affecting() {
        let tree = root_tree(El::row().gap(SMALL_CHILD_GAP));
        let next = root_tree(El::row().gap(LARGE_CHILD_GAP));

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn column_alignment_change_classifies_as_layout_affecting() {
        let tree = root_tree(El::column().align_x(AlignX::Left));
        let next = root_tree(El::column().align_x(AlignX::Right));

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn row_to_column_change_classifies_as_layout_affecting() {
        let tree = root_tree(El::row());
        let next = root_tree(El::column());

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn overlay_alignment_change_classifies_as_layout_affecting() {
        let tree = root_tree(El::overlay().align_x(AlignX::Left));
        let next = root_tree(El::overlay().align_x(AlignX::Right));

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn row_to_overlay_change_classifies_as_layout_affecting() {
        let tree = root_tree(El::row());
        let next = root_tree(El::overlay());

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn empty_to_populated_tree_classifies_as_layout_affecting() {
        let tree = LayoutTree::new();
        let next = text_tree("child", TextStyle::new(10.0));

        assert_eq!(
            tree.classify_change(&next),
            LayoutTreeChange::LayoutAffecting
        );
        assert_eq!(
            next.classify_change(&tree),
            LayoutTreeChange::LayoutAffecting
        );
    }

    #[test]
    fn material_handle_add_remove_and_swap_are_visual_only() {
        let mut materials = Assets::<StandardMaterial>::default();
        let first = materials.add(StandardMaterial::default());
        let second = materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.0, 0.0),
            ..default()
        });

        let tree = root_tree(El::new());
        let next = root_tree(El::new().material(first));
        assert_eq!(tree.classify_change(&next), LayoutTreeChange::VisualOnly);
        assert_eq!(next.classify_change(&tree), LayoutTreeChange::VisualOnly);

        let swapped = root_tree(El::new().material(second));
        assert_eq!(next.classify_change(&swapped), LayoutTreeChange::VisualOnly);
    }

    #[test]
    fn identical_material_handle_is_identical() {
        let mut materials = Assets::<StandardMaterial>::default();
        let material = materials.add(StandardMaterial::default());
        let tree = root_tree(El::new().material(material.clone()));
        let next = root_tree(El::new().material(material));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::Identical);
    }

    #[test]
    fn layout_text_ignores_standalone_only_world_scale() {
        let tree = text_tree("same", TextStyle::new(10.0));
        let next = text_tree("same", TextStyle::new(10.0));

        assert_eq!(tree.classify_change(&next), LayoutTreeChange::Identical);
    }

    #[test]
    fn updates_field_display_text_descendant() {
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(El::new().editable_field("gain", field_spec()), |builder| {
            builder.text(("10", TextStyle::new(10.0)));
        });
        let mut tree = builder.build();

        let update = tree.set_field_display_text(&"gain".into(), "11");

        assert_eq!(update, FieldDisplayTextUpdate::Updated);
        assert_eq!(tree.field_display_text(1), Some("11"));
    }

    #[test]
    fn editing_content_replaces_only_field_descendants_and_remints_text_ids() {
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.text(("before", TextStyle::new(10.0)));
        builder.with(
            El::new()
                .padding(Padding::all(3.0))
                .background(Color::BLACK)
                .editable_field("gain", field_spec()),
            |builder| {
                builder.text(("10", TextStyle::new(10.0)));
            },
        );
        builder.text(("after", TextStyle::new(10.0)));
        let mut tree = builder.build();

        let mut replacement = LayoutBuilder::with_root(El::row());
        replacement.text(("left", TextStyle::new(10.0)));
        replacement.text(("right", TextStyle::new(10.0)));
        let update = tree.set_field_editing_content(&"gain".into(), &replacement.build());

        assert_eq!(update, FieldDisplayTextUpdate::Updated);
        let field_index = tree
            .elements
            .iter()
            .position(|element| element.editable.is_some())
            .expect("the editable field remains in the derived tree");
        assert_eq!(tree.field_display_text(field_index), Some("left"));
        assert_eq!(tree.elements[field_index].padding, Padding::all(3.0));
        assert_eq!(tree.elements[field_index].background, Some(Color::BLACK));
        assert!(
            tree.elements.iter().all(|element| {
                !matches!(&element.content, ElementContent::Text { text, .. } if text == "10")
            }),
            "the authored display descendant must not remain in the compact derived arena",
        );
        let auto_ids = tree
            .elements
            .iter()
            .filter_map(|element| match element.id.as_ref() {
                Some(PanelElementId::Auto(id)) => Some(id.value()),
                Some(PanelElementId::Named(_)) | None => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            auto_ids.iter().copied().collect::<HashSet<_>>().len(),
            auto_ids.len(),
            "replacement text ids must not collide with source-tree text ids",
        );
    }

    #[test]
    fn rejects_duplicate_field_display_update() {
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(El::new().editable_field("gain", field_spec()), |builder| {
            builder.text(("10", TextStyle::new(10.0)));
        });
        builder.with(El::new().editable_field("gain", field_spec()), |builder| {
            builder.text(("12", TextStyle::new(10.0)));
        });
        let mut tree = builder.build();

        let update = tree.set_field_display_text(&"gain".into(), "11");

        assert_eq!(update, FieldDisplayTextUpdate::DuplicateField);
    }
}
