use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Formatter;
use std::sync::Arc;

use bevy::math::Vec2;

use super::positioning;
use super::positioning::CaretOffsetCorrection;
use super::sizing;
use super::sizing::Axis;
use super::wrapping;
use super::wrapping::WrappedText;
use crate::PanelElementId;
use crate::layout::BoundingBox;
use crate::layout::MeasureRichTextFn;
use crate::layout::RichShapedCacheKey;
use crate::layout::Sizing;
use crate::layout::TextDimensions;
use crate::layout::TextMeasure;
use crate::layout::TextSizing;
use crate::layout::TextWrap;
use crate::layout::element::ChildOverflow;
use crate::layout::element::Element;
use crate::layout::element::ElementContent;
use crate::layout::element::LayoutTree;
use crate::layout::render::RenderCommand;

/// Previous caret offsets and library view state, keyed by element identity.
#[derive(Clone, Default)]
pub(crate) struct ScrollSolveInput {
    caret_x:          HashMap<PanelElementId, f32>,
    pub(crate) views: HashMap<PanelElementId, ScrollViewSolve>,
}

/// A library view's requested position, in solved layout points.
#[derive(Clone, Copy)]
pub(crate) struct ScrollViewSolve {
    pub(crate) offset_points:     f32,
    pub(crate) overscroll_points: f32,
    pub(crate) indicator_opacity: f32,
}

impl ScrollSolveInput {
    pub(crate) fn is_empty(&self) -> bool { self.caret_x.is_empty() && self.views.is_empty() }

    pub(crate) fn insert(&mut self, id: PanelElementId, offset_points: f32) {
        self.caret_x.insert(id, offset_points);
    }

    pub(super) fn previous(&self, id: &PanelElementId) -> CaretOffsetState {
        self.caret_x
            .get(id)
            .copied()
            .map_or(CaretOffsetState::FirstSolve, CaretOffsetState::Previous)
    }
}

/// The starting point for a caret-follow field's solve.
pub(super) enum CaretOffsetState {
    FirstSolve,
    Previous(f32),
}

/// Callback type for measuring text dimensions.
///
/// Given a text string and its measurement properties, returns the measured
/// dimensions in layout units. The layout engine calls this during sizing to
/// determine how much space text elements need.
///
/// Takes [`TextMeasure`] — the measurement-relevant subset of
/// [`TextStyle`](crate::TextStyle) — so the callback does not depend on that
/// type's render and cascade fields.
///
/// Uses `Arc` so the function can be shared across threads and cloned cheaply
/// (e.g. stored in a Bevy `Resource` and cloned to create `LayoutEngine` instances).
pub type MeasureTextFn = Arc<dyn Fn(&str, &TextMeasure) -> TextDimensions + Send + Sync>;

/// Font-relative distances for centring a single line without counting descenders.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CapHeightMetrics {
    pub(crate) baseline:          f32,
    pub(crate) cap_height:        f32,
    pub(crate) shaped_line_count: usize,
}

pub(crate) type MeasureCapHeightFn =
    Arc<dyn Fn(&str, &TextMeasure) -> CapHeightMeasurement + Send + Sync>;

/// Missing font metrics preserve the existing line-box placement.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum CapHeightMeasurement {
    FontMetrics(CapHeightMetrics),
    #[default]
    LineBoxFallback,
}

#[derive(Clone, Default)]
pub(crate) enum CapHeightMeasurer {
    FontMetrics(MeasureCapHeightFn),
    #[default]
    LineBoxFallback,
}

impl std::fmt::Debug for CapHeightMeasurer {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::FontMetrics(_) => "FontMetrics",
            Self::LineBoxFallback => "LineBoxFallback",
        })
    }
}

#[derive(Clone)]
pub(crate) enum RichMeasurement {
    ShapedParagraph(MeasureRichTextFn),
    PlainTextFallback,
}

/// Alignment carried through Fit wrappers and retained for visual refresh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum LabelAlignment {
    #[default]
    OwnContent,
    ParentCapCenter,
}

/// Computed layout data for a single element.
#[derive(Clone, Copy, Debug, Default)]
pub struct ComputedLayout {
    pub(super) cap_height_metrics: CapHeightMeasurement,
    /// Parent centring carried through Fit wrappers, reused on visual refresh.
    pub(super) label_alignment:    LabelAlignment,
    /// Final bounding box in layout coordinates.
    pub bounds:                    BoundingBox,
    /// Width after sizing, before positioning.
    pub width:                     f32,
    /// Height after sizing, before positioning.
    pub height:                    f32,
    /// Propagated minimum width from children's content.
    ///
    /// Computed bottom-up alongside `propagate_fit_sizes`. Used as a hard
    /// floor during overflow compression and cross-axis sizing — an element
    /// must never shrink below its children's irreducible content size.
    pub(super) min_width:          f32,
    /// Propagated minimum height from children's content.
    pub(super) min_height:         f32,
    /// Cached natural (unwrapped) text width from `initialize_leaf_sizes`.
    ///
    /// Stored once during initial measurement so that `rewrap_text_elements`
    /// can check whether wrapping is needed without re-calling the measure
    /// function. Zero for non-text elements.
    pub(super) natural_text_width: f32,
    /// Width of the visible, unwrapped text, even when layout reserves a
    /// surrogate string's width through `measure_as` or `wrap_at_measure`.
    pub(super) drawn_text_width:   f32,
    /// Height of the visible, unwrapped text inside the element's content box.
    pub(super) drawn_text_height:  f32,
    /// Width of the widest single word for `TextWrap::Words` elements.
    ///
    /// Measured once during `initialize_leaf_sizes` and propagated into
    /// `min_width` so overflow compression stops at the longest unbreakable
    /// word instead of collapsing wrapped text to zero. Zero for non-text
    /// elements and other wrap modes.
    pub(super) min_text_width:     f32,
}

/// The layout engine. Thread-safe, no global state.
///
/// # Usage
///
/// ```ignore
/// let engine = LayoutEngine::new(measure_fn);
/// let result = engine.compute(&tree, 800.0, 600.0, 1.0);
/// ```
///
/// The layout result keeps the complete render-command stream, including
/// commands whose bounds fall outside the viewport. Clipping happens
/// render-side, from the scissor commands in the stream.
pub struct LayoutEngine {
    measurement: TextMeasurement,
}

enum TextMeasurement {
    Plain(MeasureTextFn),
    Extended(Box<TextMeasurements>),
}

struct TextMeasurements {
    plain: MeasureTextFn,
    rich:  RichMeasurement,
    caps:  CapHeightMeasurer,
}

impl TextMeasurement {
    fn cap_height_measurer(&self) -> CapHeightMeasurer {
        match self {
            Self::Extended(measure) => measure.caps.clone(),
            Self::Plain(_) => CapHeightMeasurer::LineBoxFallback,
        }
    }

    fn cap_height(&self, text: &str, style: &TextMeasure) -> CapHeightMeasurement {
        match self {
            Self::Extended(measure) => match &measure.caps {
                CapHeightMeasurer::FontMetrics(caps) => caps(text, style),
                CapHeightMeasurer::LineBoxFallback => CapHeightMeasurement::LineBoxFallback,
            },
            Self::Plain(_) => CapHeightMeasurement::LineBoxFallback,
        }
    }

    fn plain(&self) -> &MeasureTextFn {
        match self {
            Self::Plain(measure) => measure,
            Self::Extended(measure) => &measure.plain,
        }
    }
}

impl LayoutEngine {
    /// Creates a new layout engine with the given text measurement callback.
    #[must_use]
    pub fn new(measure_text: MeasureTextFn) -> Self {
        Self {
            measurement: TextMeasurement::Plain(measure_text),
        }
    }

    pub(crate) fn with_rich_measure(
        measure_text: MeasureTextFn,
        measure_rich: MeasureRichTextFn,
    ) -> Self {
        Self {
            measurement: TextMeasurement::Extended(Box::new(TextMeasurements {
                plain: measure_text,
                rich:  RichMeasurement::ShapedParagraph(measure_rich),
                caps:  CapHeightMeasurer::LineBoxFallback,
            })),
        }
    }

    pub(crate) fn with_cap_height_measure(mut self, measure: CapHeightMeasurer) -> Self {
        if let CapHeightMeasurer::FontMetrics(_) = &measure {
            self.measurement = match self.measurement {
                TextMeasurement::Plain(plain) => {
                    TextMeasurement::Extended(Box::new(TextMeasurements {
                        plain,
                        rich: RichMeasurement::PlainTextFallback,
                        caps: measure,
                    }))
                },
                TextMeasurement::Extended(mut measurements) => {
                    measurements.caps = measure;
                    TextMeasurement::Extended(measurements)
                },
            };
        }
        self
    }

    #[inline(never)]
    fn rich_dimensions(&self, paragraph: &crate::RichParagraph, scale: f32) -> TextDimensions {
        if let TextMeasurement::Extended(measure) = &self.measurement
            && let RichMeasurement::ShapedParagraph(rich) = &measure.rich
        {
            return rich(paragraph, scale);
        }
        let measure = self.measurement.plain();
        let base = paragraph.base.as_measure().scaled(scale);
        let mut dimensions = measure(&paragraph.text, &base);
        for run in &paragraph.runs {
            let spaces = paragraph.text[run.range.clone()]
                .chars()
                .filter(|&c| c == ' ')
                .count();
            dimensions.width = run
                .measure
                .word_spacing
                .mul_add(scale, -base.word_spacing)
                .mul_add(hana_kana::ToF32::to_f32(spaces), dimensions.width);
        }
        dimensions
    }

    /// Computes layout for the given tree within the specified viewport dimensions.
    ///
    /// Returns a list of render commands in draw order, and the computed layout
    /// for each element (indexed by element index).
    #[must_use]
    ///
    /// `font_scale` converts font sizes from font units to layout units.
    /// When font and layout units are the same, pass `1.0`.
    pub fn compute(
        &self,
        tree: &LayoutTree,
        viewport_width: f32,
        viewport_height: f32,
        font_scale: f32,
    ) -> LayoutResult {
        self.compute_with_caret_offsets(
            tree,
            viewport_width,
            viewport_height,
            font_scale,
            &ScrollSolveInput::default(),
        )
    }

    /// Computes a panel while retaining caret-follow offsets across editor-tree rebuilds.
    pub(crate) fn compute_with_caret_offsets(
        &self,
        tree: &LayoutTree,
        viewport_width: f32,
        viewport_height: f32,
        font_scale: f32,
        previous_offsets: &ScrollSolveInput,
    ) -> LayoutResult {
        let Some(root) = tree.root else {
            return LayoutResult::default();
        };

        let element_count = tree.len();
        let mut computed = vec![ComputedLayout::default(); element_count];

        let mut sizing_tree = field_sizing_tree(tree);

        // Initialize leaf sizes (text measurement, fixed values).
        let rich_geometry = if sizing_tree.contains_rich_text() {
            sizing_tree = paragraph_sizing_tree(sizing_tree);
            let mut keys =
                self.initialize_leaf_sizes::<true>(&sizing_tree, &mut computed, font_scale);
            for (index, element) in tree.elements.iter().enumerate() {
                if let ElementContent::RichText(paragraph) = &element.content
                    && paragraph.uses_base_geometry()
                {
                    keys.push((index, RichShapedCacheKey::new(paragraph, font_scale)));
                }
            }
            keys
        } else {
            self.initialize_leaf_sizes::<false>(&sizing_tree, &mut computed, font_scale)
        };
        let sizing_tree = sizing_tree.as_ref();
        let measure_text = self.measurement.plain();

        // Propagate initial Fit container sizes bottom-up from their children.
        sizing::propagate_fit_sizes_xy(sizing_tree, &mut computed, root);

        // Phase 1: Size along X axis (BFS top-down).
        sizing::size_along_axis(sizing_tree, &mut computed, root, Axis::X, viewport_width);

        // Phase 2: Re-wrap text elements within their resolved widths.
        // This may change text heights (more lines), so Y is re-propagated and
        // re-sized afterwards — but only when wrapping actually changed sizes.
        let (wrapped, text_sizes_changed) =
            wrapping::rewrap_text_elements(sizing_tree, &mut computed, measure_text, font_scale);
        if text_sizes_changed {
            sizing::propagate_fit_sizes(sizing_tree, &mut computed, root, Axis::Y);
        }

        // Phase 3: Size along Y axis (BFS top-down) with wrap-corrected heights.
        sizing::size_along_axis(sizing_tree, &mut computed, root, Axis::Y, viewport_height);

        // Phase 4: Position elements and generate the complete render
        // command stream (DFS).
        let mut scrolls =
            positioning::ScrollSolver::new(measure_text, font_scale, previous_offsets);
        let mut commands = positioning::position_and_render(
            tree,
            &mut computed,
            root,
            &wrapped,
            font_scale,
            &mut scrolls,
        );
        let mut scrolls = scrolls.into_solved();
        if let CaretOffsetCorrection::Corrected(corrected_offsets) =
            positioning::corrected_caret_offsets(tree, &computed, &scrolls, previous_offsets)
        {
            let mut corrected_scrolls =
                positioning::ScrollSolver::new(measure_text, font_scale, &corrected_offsets);
            commands = positioning::position_and_render(
                tree,
                &mut computed,
                root,
                &wrapped,
                font_scale,
                &mut corrected_scrolls,
            );
            scrolls = corrected_scrolls.into_solved();
        }

        LayoutResult {
            computed,
            commands,
            scrolls,
            text_geometry: TextGeometry::new(
                wrapped,
                rich_geometry,
                self.measurement.cap_height_measurer(),
            ),
            viewport_width,
            viewport_height,
            font_scale,
            structure_hash: tree.structure_hash(),
        }
    }

    /// Initializes leaf element dimensions from text measurement and fixed sizing rules.
    fn initialize_leaf_sizes<const RICH: bool>(
        &self,
        tree: &LayoutTree,
        computed: &mut [ComputedLayout],
        font_scale: f32,
    ) -> Vec<(usize, RichShapedCacheKey)> {
        let mut rich_geometry = Vec::new();
        let measure_text = self.measurement.plain();
        for (index, element) in tree.elements.iter().enumerate() {
            // Set initial size from Fixed rules.
            computed[index].width = match element.width {
                Sizing::Fixed(size) => size.value,
                _ => 0.0,
            };
            computed[index].height = match element.height {
                Sizing::Fixed(size) => size.value,
                _ => 0.0,
            };

            // Measure text content and cache the natural width for the
            // rewrap fast-path (avoids re-measuring every text element later).
            if let ElementContent::Text {
                ref text,
                ref config,
                ref sizing,
                ..
            } = element.content
            {
                let measurement_text = sizing.measure_text(text);
                let text_measure = config.as_measure().scaled(font_scale);
                computed[index].cap_height_metrics =
                    self.measurement.cap_height(text, &text_measure);
                let dims = measure_text(measurement_text, &text_measure);
                computed[index].natural_text_width = dims.width;
                let drawn = if measurement_text == text {
                    dims
                } else {
                    measure_text(text, &text_measure)
                };
                computed[index].drawn_text_width = drawn.width;
                computed[index].drawn_text_height = drawn.height;
                if matches!(
                    sizing,
                    TextSizing::Natural {
                        wrap: TextWrap::Words,
                    }
                ) {
                    computed[index].min_text_width =
                        wrapping::min_content_width(measurement_text, &text_measure, measure_text)
                            + element.padding.horizontal()
                            + sizing::border_inset(element, Axis::X);
                }
                if element.width.is_fit() {
                    computed[index].width = (dims.width
                        + element.padding.horizontal()
                        + sizing::border_inset(element, Axis::X))
                    .clamp(element.width.min_size(), element.width.max_size());
                }
                if element.height.is_fit() {
                    computed[index].height = (dims.height
                        + element.padding.vertical()
                        + sizing::border_inset(element, Axis::Y))
                    .clamp(element.height.min_size(), element.height.max_size());
                }
            } else if RICH && let ElementContent::RichText(paragraph) = &element.content {
                self.initialize_rich_leaf_size(
                    paragraph,
                    element,
                    &mut computed[index],
                    font_scale,
                    index,
                    &mut rich_geometry,
                );
            }
        }
        rich_geometry
    }

    // Key ownership and paragraph measurement stay outside the plain leaf loop.
    #[inline(never)]
    fn initialize_rich_leaf_size(
        &self,
        paragraph: &crate::RichParagraph,
        element: &Element,
        computed: &mut ComputedLayout,
        font_scale: f32,
        index: usize,
        rich_geometry: &mut Vec<(usize, RichShapedCacheKey)>,
    ) {
        computed.cap_height_metrics = self.measurement.cap_height(
            &paragraph.text,
            &paragraph.base.as_measure().scaled(font_scale),
        );
        let dims = self.rich_dimensions(paragraph, font_scale);
        rich_geometry.push((index, RichShapedCacheKey::new(paragraph, font_scale)));
        computed.natural_text_width = dims.width;
        computed.drawn_text_width = dims.width;
        computed.drawn_text_height = dims.height;
        if element.width.is_fit() {
            computed.width = (dims.width
                + element.padding.horizontal()
                + sizing::border_inset(element, Axis::X))
            .clamp(element.width.min_size(), element.width.max_size());
        }
        if element.height.is_fit() {
            computed.height =
                (dims.height + element.padding.vertical() + sizing::border_inset(element, Axis::Y))
                    .clamp(element.height.min_size(), element.height.max_size());
        }
    }
}

/// Colour-only paragraphs share the plain unwrapped sizing rules.
/// Only a solve already known to contain rich leaves owns this projection.
#[inline(never)]
fn paragraph_sizing_tree(tree: Cow<'_, LayoutTree>) -> Cow<'_, LayoutTree> {
    let mut tree = tree.into_owned();
    for element in &mut tree.elements {
        if let ElementContent::RichText(paragraph) = &element.content
            && paragraph.uses_base_geometry()
        {
            element.content = ElementContent::Text {
                text:   paragraph.text.clone(),
                config: paragraph.base.clone(),
                sizing: paragraph.sizing.clone(),
            };
        }
    }
    Cow::Owned(tree)
}

/// A clipped Fit field sizes from text, with room for the visual caret.
/// Its original tree still clips and positions the content.
fn field_sizing_tree(tree: &LayoutTree) -> Cow<'_, LayoutTree> {
    if !tree
        .elements
        .iter()
        .any(clipped_fit_field_needs_content_size)
    {
        return Cow::Borrowed(tree);
    }
    let mut sizing_tree = tree.clone();
    for (index, element) in sizing_tree.elements.iter_mut().enumerate() {
        if clipped_fit_field_needs_content_size(element) {
            element.overflow = ChildOverflow::Visible;
            if element.caret_follow && element.width.is_fit() {
                element.padding.right.value += caret_visual_width(tree, index);
            }
        }
    }
    Cow::Owned(sizing_tree)
}

const fn clipped_fit_field_needs_content_size(element: &Element) -> bool {
    if !element.overflow.clips() {
        return false;
    }
    element.editable.is_some() && (element.width.is_fit() || element.height.is_fit())
}

fn caret_visual_width(tree: &LayoutTree, field_index: usize) -> f32 {
    let mut pending = tree.children_of(field_index).to_vec();
    while let Some(index) = pending.pop() {
        if tree.elements[index].caret_marker {
            return tree.children_of(index).first().map_or(0.0, |&child| {
                match tree.elements[child].width {
                    Sizing::Fixed(width) => width.value,
                    _ => 0.0,
                }
            });
        }
        pending.extend_from_slice(tree.children_of(index));
    }
    0.0
}

/// Solved scroll state of one scrolling parent, in layout points.
///
/// The engine solves in the tree's own units; a panel scales its tree to layout
/// points before the solve, so for a panel every length here is in points.
/// Author-driven scrolls and library scroll views produce one; a clip-only
/// element does not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolvedScroll {
    /// Signed elastic displacement beyond the clamped offset, in points.
    pub overscroll_points: f32,
    /// Overlay thumb geometry and activity fade.
    pub indicator:         SolvedScrollIndicator,
    /// Index of the scrolling parent in the [`LayoutTree`].
    pub element_index:     usize,
    /// Size of the parent's children, gaps included, in layout points.
    pub content_points:    Vec2,
    /// Size of the parent's content box, the visible window onto the children,
    /// in layout points.
    pub viewport_points:   Vec2,
    /// Offset from the start the engine applied, clamped to
    /// `[0, content_points - viewport_points]` per axis, in layout points.
    pub offset_points:     Vec2,
    /// Part of the parent left visible by its clipping ancestors and the
    /// layout viewport, in layout points.
    pub visible_region:    VisibleScrollRegion,
    /// Distance one wheel line moves the content: the measured height of one
    /// line of the parent's scroll line style plus its child gap, in layout
    /// points.
    pub line_pitch_points: f32,
}

/// Indicator geometry emitted inside a scrolling parent's clip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SolvedScrollIndicator {
    /// No visible indicator.
    #[default]
    Hidden,
    /// Rounded thumb in panel-local layout points.
    Shown {
        /// Thumb rectangle in points.
        thumb_points: BoundingBox,
        /// Activity fade between zero and one.
        opacity:      f32,
    },
}

/// Part of a scrolling element that its clipping ancestors leave visible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VisibleScrollRegion {
    /// The visible rectangle, in the same layout-point space as the element's
    /// bounds.
    Visible(BoundingBox),
    /// Clipping ancestors hide the whole element, so no point lies over it.
    ClippedOut,
}

impl VisibleScrollRegion {
    /// Returns `true` when `point` lies in the visible part of the element.
    #[must_use]
    pub fn contains(self, point: Vec2) -> bool {
        match self {
            Self::Visible(bounds) => bounds.contains(point),
            Self::ClippedOut => false,
        }
    }
}

/// Plain geometry stays inline; font-backed solves retain their text measurer
/// so command refresh can update cap metrics after a geometry-stable text edit.
#[derive(Clone, Debug)]
enum TextGeometry {
    Plain(Vec<Option<WrappedText>>),
    Measured(Box<MeasuredTextGeometry>),
}

#[derive(Clone, Debug)]
struct MeasuredTextGeometry {
    wrapped: Vec<Option<WrappedText>>,
    keys:    Vec<(usize, RichShapedCacheKey)>,
    caps:    CapHeightMeasurer,
}

impl Default for TextGeometry {
    fn default() -> Self { Self::Plain(Vec::new()) }
}

impl TextGeometry {
    fn new(
        wrapped: Vec<Option<WrappedText>>,
        keys: Vec<(usize, RichShapedCacheKey)>,
        caps: CapHeightMeasurer,
    ) -> Self {
        if keys.is_empty() && matches!(caps, CapHeightMeasurer::LineBoxFallback) {
            Self::Plain(wrapped)
        } else {
            Self::Measured(Box::new(MeasuredTextGeometry {
                wrapped,
                keys,
                caps,
            }))
        }
    }

    fn wrapped(&self) -> &[Option<WrappedText>] {
        match self {
            Self::Plain(wrapped) => wrapped,
            Self::Measured(geometry) => &geometry.wrapped,
        }
    }

    fn rich_keys(&self) -> &[(usize, RichShapedCacheKey)] {
        match self {
            Self::Plain(_) => &[],
            Self::Measured(geometry) => &geometry.keys,
        }
    }

    fn cap_height(&self, text: &str, style: &TextMeasure) -> CapHeightMeasurement {
        match self {
            Self::Measured(geometry) => match &geometry.caps {
                CapHeightMeasurer::FontMetrics(measure) => measure(text, style),
                CapHeightMeasurer::LineBoxFallback => CapHeightMeasurement::LineBoxFallback,
            },
            Self::Plain(_) => CapHeightMeasurement::LineBoxFallback,
        }
    }
}

/// Result of a layout computation.
#[derive(Clone, Debug, Default)]
pub struct LayoutResult {
    /// Computed layout for each `Element`, indexed by element index.
    pub computed:    Vec<ComputedLayout>,
    /// Render commands in draw order.
    pub commands:    Vec<RenderCommand>,
    /// Solved scroll state of every scrolling parent with children, in
    /// positioning order (outer parents before the parents they contain).
    pub scrolls:     Vec<SolvedScroll>,
    text_geometry:   TextGeometry,
    viewport_width:  f32,
    viewport_height: f32,
    font_scale:      f32,
    structure_hash:  u64,
}

impl LayoutResult {
    /// Returns the bounding box of the root's first child.
    ///
    /// For trees built with [`crate::layout::LayoutBuilder::new`], element 0 is
    /// the implicit root and element 1 is the first user-defined element. For
    /// trees built with [`crate::layout::LayoutBuilder::with_root`], element 0
    /// is the authored root and element 1 is its first child. Use element 0's
    /// bounds when a caller needs the physical root surface including root
    /// padding and border.
    ///
    /// Returns `None` if the root has no child element.
    #[must_use = "callers use the content bounds to resolve fit-sized panels"]
    pub fn content_bounds(&self) -> Option<BoundingBox> { self.computed.get(1).map(|c| c.bounds) }

    /// Whether this result was solved against exactly this viewport.
    #[must_use]
    pub const fn solved_for_viewport(&self, viewport_width: f32, viewport_height: f32) -> bool {
        self.viewport_width.to_bits() == viewport_width.to_bits()
            && self.viewport_height.to_bits() == viewport_height.to_bits()
    }

    /// Regenerates render commands from cached geometry and wrapped text data.
    ///
    /// `measure_text` re-measures the visible width of surrogate-sized text;
    /// pass the measurer that [`can_reuse_geometry`](Self::can_reuse_geometry)
    /// validated the reuse against.
    pub fn regenerate_commands(&mut self, tree: &LayoutTree, measure_text: &MeasureTextFn) {
        let Some(root) = tree.root else {
            debug_assert!(self.computed.is_empty());
            self.commands.clear();
            return;
        };

        debug_assert_eq!(self.computed.len(), tree.len());
        let wrapped = self.text_geometry.wrapped();
        let wrapped_len = wrapped.len();
        debug_assert!(wrapped_len == 0 || wrapped_len == tree.len());
        debug_assert_eq!(self.structure_hash, tree.structure_hash());

        if self.computed.len() != tree.len() || (wrapped_len != 0 && wrapped_len != tree.len()) {
            return;
        }

        let font_scale = self.font_scale;
        let computed = self.computed.as_mut_slice();
        Self::refresh_text_metrics(
            computed,
            tree,
            &self.text_geometry,
            font_scale,
            measure_text,
        );
        self.commands = if tree.contains_scroll_views() {
            positioning::render_commands_from_geometry::<true>(
                tree,
                computed,
                root,
                wrapped,
                font_scale,
                &self.scrolls,
            )
        } else {
            positioning::render_commands_from_geometry::<false>(
                tree,
                computed,
                root,
                wrapped,
                font_scale,
                &self.scrolls,
            )
        };
    }

    /// Refreshes cap metrics for the actual text, including shaped line count,
    /// before reused commands decide between cap and line-box centring.
    /// Surrogate-sized text also refreshes its visible width and height;
    /// natural text already matched its measured width for geometry reuse.
    #[inline(never)]
    fn refresh_text_metrics(
        computed: &mut [ComputedLayout],
        tree: &LayoutTree,
        geometry: &TextGeometry,
        font_scale: f32,
        measure_text: &MeasureTextFn,
    ) {
        for (index, element) in tree.elements.iter().enumerate() {
            let ElementContent::Text {
                text,
                config,
                sizing,
            } = &element.content
            else {
                continue;
            };
            let style = config.as_measure().scaled(font_scale);
            computed[index].cap_height_metrics = geometry.cap_height(text, &style);
            if !matches!(
                sizing,
                TextSizing::MeasureAs { .. } | TextSizing::WrapAtMeasure { .. }
            ) || geometry.wrapped().get(index).is_some_and(Option::is_some)
            {
                continue;
            }
            let drawn = measure_text(text, &style);
            computed[index].drawn_text_width = drawn.width;
            computed[index].drawn_text_height = drawn.height;
        }
    }

    /// Returns whether reusing this result's cached geometry via
    /// [`regenerate_commands`](Self::regenerate_commands) for `tree` would
    /// produce the same layout a fresh [`compute`](LayoutEngine::compute) would.
    ///
    /// This is the geometry-stable skip's correctness gate. After a per-frame
    /// text edit, the caller re-measures the changed leaves: if their box did
    /// not move, only the glyphs changed, so the engine solve can be skipped and
    /// the render commands regenerated from the unchanged geometry. The command
    /// stream re-reads text and visual-only config (color, alignment) directly
    /// from `tree`, so those flow through without a solve.
    ///
    /// Conservative — any of these returns `false`, so the caller runs a full
    /// solve: a structural change (element count / kinds / nesting), a viewport
    /// or `font_scale` change, a text leaf that was word-wrapped in this result
    /// (its cached line breaks belong to the old string and cannot carry a new
    /// one), a newline in the new text (which would wrap), or a changed measured
    /// width.
    #[must_use]
    pub fn can_reuse_geometry(
        &self,
        tree: &LayoutTree,
        measure: &MeasureTextFn,
        viewport_width: f32,
        viewport_height: f32,
        font_scale: f32,
    ) -> bool {
        if tree.contains_rich_text() {
            self.geometry_matches::<true>(
                tree,
                measure,
                viewport_width,
                viewport_height,
                font_scale,
            )
        } else {
            self.geometry_matches::<false>(
                tree,
                measure,
                viewport_width,
                viewport_height,
                font_scale,
            )
        }
    }

    fn geometry_matches<const RICH: bool>(
        &self,
        tree: &LayoutTree,
        measure: &MeasureTextFn,
        viewport_width: f32,
        viewport_height: f32,
        font_scale: f32,
    ) -> bool {
        let wrapped = self.text_geometry.wrapped();
        let rich_keys = self.text_geometry.rich_keys();
        if self.computed.len() != tree.len()
            || (!wrapped.is_empty() && wrapped.len() != tree.len())
            || self.structure_hash != tree.structure_hash()
            || self.viewport_width.to_bits() != viewport_width.to_bits()
            || self.viewport_height.to_bits() != viewport_height.to_bits()
            || self.font_scale.to_bits() != font_scale.to_bits()
        {
            return false;
        }

        tree.elements.iter().enumerate().all(|(index, element)| {
            let ElementContent::Text {
                text,
                config,
                sizing,
                ..
            } = &element.content
            else {
                if RICH && let ElementContent::RichText(paragraph) = &element.content {
                    let next = RichShapedCacheKey::new(paragraph, font_scale);
                    return rich_keys
                        .iter()
                        .any(|(source, key)| *source == index && *key == next);
                }
                return true;
            };
            // A wrapped leaf's cached line-break offsets index into the old string;
            // the same offsets in a new string point to the wrong positions, so the
            // reused command stream would emit the wrong glyphs. A newline in the
            // new text also forces re-wrapping. Either case requires a full solve.
            if wrapped.get(index).is_some_and(Option::is_some) || text.contains('\n') {
                return false;
            }
            // Bit-exact: the same cache-backed measure over the same text and
            // measure inputs returns the same value the prior solve stored, so an
            // unchanged box compares equal without an epsilon.
            let dims = measure(
                sizing.measure_text(text),
                &config.as_measure().scaled(font_scale),
            );
            dims.width.to_bits() == self.computed[index].natural_text_width.to_bits()
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::expect_used,
    clippy::float_cmp,
    reason = "layout tests use short strings, exact solved offsets, and explicit failure context"
)]
mod tests {
    use std::sync::Arc;

    use super::LayoutEngine;
    use super::MeasureTextFn;
    use super::ScrollSolveInput;
    use crate::El;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::PanelElementId;
    use crate::Sizing;
    use crate::layout::LayoutBuilder;
    use crate::layout::RenderCommandKind;
    use crate::layout::Text;
    use crate::layout::TextAlign;
    use crate::layout::TextDimensions;
    use crate::layout::TextStyle;

    #[test]
    fn rich_text_keeps_plain_engine_and_result_storage_budgets() {
        assert_eq!(
            std::mem::size_of::<LayoutEngine>(),
            std::mem::size_of::<MeasureTextFn>()
        );
        assert_eq!(
            std::mem::size_of::<super::TextGeometry>(),
            std::mem::size_of::<Vec<()>>()
        );
        // The original result owns four vectors, three dimensions, and a hash.
        assert_eq!(
            std::mem::size_of::<super::LayoutResult>(),
            std::mem::size_of::<([Vec<()>; 4], [f32; 3], u64)>()
        );
    }

    #[test]
    fn caret_follow_scrolls_on_the_current_solve_and_retains_previous_offset() {
        let measure: MeasureTextFn = Arc::new(|text, style| TextDimensions {
            width:       text.chars().count() as f32 * style.size,
            height:      style.size,
            line_height: style.size,
        });
        let style = TextStyle::new(10.0);
        let mut authored = LayoutBuilder::new(100.0, 30.0);
        authored.with(
            El::new()
                .width(Sizing::fixed(100.0))
                .height(Sizing::fixed(30.0))
                .editable_field(
                    "field",
                    ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                        ImeBuiltInFieldKind::Text,
                    )),
                    style.clone(),
                ),
            |builder| {
                builder.text(("", style.clone()));
            },
        );
        let authored = authored.build();
        let make_tree = |prefix: &str, suffix: &str| {
            let mut tree = authored.clone();
            let mut replacement =
                LayoutBuilder::with_root(El::column().width(Sizing::GROW).height(Sizing::GROW));
            replacement.with(
                El::row().width(Sizing::GROW).height(Sizing::GROW),
                |builder| {
                    builder.text((prefix, style.clone()));
                    builder.with(
                        El::column()
                            .caret_marker()
                            .width(Sizing::fixed(0.0))
                            .height(Sizing::GROW),
                        |builder| {
                            builder.with(
                                El::new()
                                    .width(Sizing::fixed(2.0))
                                    .height(Sizing::fixed(10.0)),
                                |_| {},
                            );
                        },
                    );
                    builder.text((suffix, style.clone()));
                },
            );
            tree.set_field_editing_content(&PanelElementId::named("field"), &replacement.build());
            tree
        };
        let engine = LayoutEngine::new(measure);
        let first = engine.compute(&make_tree("abcdefghijk", ""), 100.0, 30.0, 1.0);
        let first_scroll = first
            .scrolls
            .iter()
            .find(|scroll| scroll.element_index == 1)
            .expect("field scroll is solved");
        assert!(
            first_scroll.offset_points.x > 0.0,
            "scroll={first_scroll:?}; computed={:?}",
            first.computed
        );
        let mut previous = ScrollSolveInput::default();
        previous.insert(PanelElementId::named("field"), first_scroll.offset_points.x);

        let next = engine.compute_with_caret_offsets(
            &make_tree("abcdefghijk", "lmnopqrst"),
            100.0,
            30.0,
            1.0,
            &previous,
        );
        let next_scroll = next
            .scrolls
            .iter()
            .find(|scroll| scroll.element_index == 1)
            .expect("rebuilt field scroll is solved");
        assert_eq!(
            next_scroll.offset_points.x, first_scroll.offset_points.x,
            "next={:?}",
            next.computed
        );

        let back = engine.compute_with_caret_offsets(
            &make_tree("", "abcdefghijklmnopqrstu"),
            100.0,
            30.0,
            1.0,
            &previous,
        );
        let back_scroll = back
            .scrolls
            .iter()
            .find(|scroll| scroll.element_index == 1)
            .expect("field scroll back to the start is solved");
        assert_eq!(back_scroll.offset_points.x, 0.0);
    }

    #[test]
    fn fit_sized_field_retains_editor_text_geometry() {
        let measure: MeasureTextFn = Arc::new(|text, style| TextDimensions {
            width:       text.chars().count() as f32 * style.size,
            height:      style.size,
            line_height: style.size,
        });
        let style = TextStyle::new(10.0);
        let mut authored = LayoutBuilder::new(100.0, 40.0);
        authored.with(
            El::new().editable_field(
                "field",
                ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text)),
                style.clone(),
            ),
            |builder| {
                builder.text(("display", style.clone()));
            },
        );
        let mut tree = authored.build();
        let mut replacement =
            LayoutBuilder::with_root(El::column().width(Sizing::GROW).height(Sizing::GROW));
        replacement.with(
            El::row().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                builder.text(Text::new("display", style).generated_editor_part());
            },
        );
        tree.set_field_editing_content(&PanelElementId::named("field"), &replacement.build());
        let result = LayoutEngine::new(measure).compute(&tree, 100.0, 40.0, 1.0);
        assert!(
            result.computed[1].width > 0.0 && result.computed[1].height > 0.0,
            "{:#?}",
            result.computed
        );
    }

    #[test]
    fn rejected_field_keeps_validation_below_the_scrolling_row_visible() {
        let measure: MeasureTextFn = Arc::new(|text, style| TextDimensions {
            width:       text.chars().count() as f32 * style.size,
            height:      style.size,
            line_height: style.size,
        });
        let style = TextStyle::new(10.0);
        let mut authored = LayoutBuilder::new(100.0, 40.0);
        authored.with(
            El::new()
                .width(Sizing::fixed(80.0))
                .height(Sizing::fixed(12.0))
                .editable_field(
                    "field",
                    ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                        ImeBuiltInFieldKind::Text,
                    )),
                    style.clone(),
                ),
            |builder| {
                builder.text(("value", style.clone()));
            },
        );
        let mut tree = authored.build();
        let mut replacement =
            LayoutBuilder::with_root(El::column().width(Sizing::GROW).height(Sizing::GROW));
        replacement.with(
            El::row().width(Sizing::GROW).height(Sizing::GROW),
            |builder| {
                builder.text(("a long value beyond the field", style.clone()));
                builder.with(
                    El::column()
                        .caret_marker()
                        .width(Sizing::fixed(0.0))
                        .height(Sizing::GROW),
                    |builder| {
                        builder.with(El::new().size(1.0, 10.0), |_| {});
                    },
                );
            },
        );
        replacement.text(("rejected", style));
        tree.set_field_editing_content(&PanelElementId::named("field"), &replacement.build());
        let result = LayoutEngine::new(measure).compute(&tree, 100.0, 40.0, 1.0);
        let field = result.computed[1].bounds;
        let field_scroll = result
            .scrolls
            .iter()
            .find(|scroll| scroll.element_index == 1)
            .expect("edited field has solved scroll");
        assert!(
            field_scroll.offset_points.x > 0.0,
            "caret must scroll right"
        );
        let validation = result
            .commands
            .iter()
            .find(|command| {
                matches!(
                    &command.kind, RenderCommandKind::Text { text, .. } if text == "rejected"
                )
            })
            .expect("validation text is rendered");
        assert!(field.height >= 20.0);
        assert!(validation.bounds.x >= field.x);
        assert!(validation.bounds.x + validation.bounds.width <= field.x + field.width);
        assert!(validation.bounds.y + validation.bounds.height <= field.y + field.height);
    }

    #[test]
    fn reused_surrogate_text_aligns_with_the_validating_measurer() {
        let measure_a: MeasureTextFn = Arc::new(|text, style| TextDimensions {
            width:       text.chars().fold(0.0, |width, _| width + style.size),
            height:      style.size,
            line_height: style.size,
        });
        let measure_b: MeasureTextFn = Arc::new(|text, style| TextDimensions {
            width:       if text == "5%" {
                3.0 * style.size
            } else {
                text.chars().fold(0.0, |width, _| width + style.size)
            },
            height:      style.size,
            line_height: style.size,
        });
        let tree = |visible: &str| {
            let mut builder = LayoutBuilder::new(100.0, 100.0);
            builder.text(
                Text::new(visible, TextStyle::new(10.0).with_align(TextAlign::Right))
                    .measure_as("100%"),
            );
            builder.build()
        };
        let mut result = LayoutEngine::new(measure_a).compute(&tree("50%"), 100.0, 100.0, 1.0);
        let updated_tree = tree("5%");
        assert!(result.can_reuse_geometry(&updated_tree, &measure_b, 100.0, 100.0, 1.0));
        result.regenerate_commands(&updated_tree, &measure_b);
        let text = result
            .commands
            .iter()
            .find(|command| matches!(command.kind, RenderCommandKind::Text { .. }));
        assert_eq!(
            text.map(|command| (command.bounds.x, command.bounds.width)),
            Some((10.0, 30.0))
        );
    }
}
