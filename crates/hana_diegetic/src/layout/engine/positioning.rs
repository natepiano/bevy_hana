use std::sync::Arc;

use bevy::math::Vec2;
use bevy::prelude::Color;
use hana_kana::ToF32;

use super::layout_engine::CapHeightMeasurement;
use super::layout_engine::CaretOffsetState;
use super::layout_engine::ComputedLayout;
use super::layout_engine::LabelAlignment;
use super::layout_engine::MeasureTextFn;
use super::layout_engine::ScrollSolveInput;
use super::layout_engine::SolvedScroll;
use super::layout_engine::SolvedScrollIndicator;
use super::layout_engine::VisibleScrollRegion;
use super::sizing;
use super::sizing::ContentBox;
use super::wrapping::WrappedLine;
use super::wrapping::WrappedText;
use crate::layout::AlignX;
use crate::layout::AlignY;
use crate::layout::BoundingBox;
use crate::layout::ChildDivider;
use crate::layout::DrawOverflow;
use crate::layout::DrawZIndex;
use crate::layout::PanelShape;
use crate::layout::PanelShapeSourceKey;
use crate::layout::ResolvedPanelShape;
use crate::layout::ScrollDrive;
use crate::layout::ScrollRestEdge;
use crate::layout::TextAlign;
use crate::layout::TextStyle;
use crate::layout::child_layout::ChildLayout;
use crate::layout::element::ChildOverflow;
use crate::layout::element::Element;
use crate::layout::element::ElementContent;
use crate::layout::element::LayoutTree;
use crate::layout::element::PrecomposeMode;
use crate::layout::element::ScrollAnchor;
use crate::layout::line;
use crate::layout::line::PanelShapeClipPolicy;
use crate::layout::line::PanelShapeResolveContext;
use crate::layout::render::RectangleSource;
use crate::layout::render::RenderCommand;
use crate::layout::render::RenderCommandKind;
use crate::panel::scroll::ScrollIndicatorState;
use crate::panel::scroll_view;

/// Pushes one command with its [`RenderCommand::z_index`].
fn push_command(
    commands: &mut Vec<RenderCommand>,
    bounds: BoundingBox,
    kind: RenderCommandKind,
    element_idx: usize,
    z_index: DrawZIndex,
) {
    commands.push(RenderCommand {
        bounds,
        kind,
        element_idx,
        z_index,
    });
}

#[derive(Clone, Copy)]
struct PositionStackEntry {
    index:             usize,
    x:                 f32,
    y:                 f32,
    visited:           bool,
    clip_context:      ClipContext,
    editor_row_scroll: EditorRowScroll,
    label_alignment:   LabelAlignment,
}

#[derive(Clone, Copy)]
enum EditorRowScroll {
    Inactive,
    AtReplacementRoot(f32),
}

#[derive(Clone, Copy)]
struct GeometryStackEntry {
    index:        usize,
    visited:      bool,
    clip_context: ClipContext,
}

/// Clip state threaded down the DFS.
///
/// `inherited` is the viewport ∩ every clipping ([`ChildOverflow::clips`]) ancestor, and applies to
/// owner-bound content. `scissor` is only the explicit clipping ancestors, with
/// no viewport term: line geometry that overflows a panel is not clipped to the panel viewport,
/// but is still clipped to these regions.
#[derive(Clone, Copy)]
struct ClipContext {
    inherited: BoundingBox,
    scissor:   Option<BoundingBox>,
}

impl ClipContext {
    const fn root(viewport: BoundingBox) -> Self {
        Self {
            inherited: viewport,
            scissor:   None,
        }
    }

    // Keep clip propagation out of the traversal loops, as in the plain solver.
    #[inline(never)]
    fn child(self, element: &Element, bounds: BoundingBox) -> Self {
        if element.overflow.clips() {
            let scissor_bounds = element.child_clip_bounds(bounds);
            Self {
                inherited: self
                    .inherited
                    .intersect(&scissor_bounds)
                    .unwrap_or_else(empty_clip),
                scissor:   Some(match self.scissor {
                    Some(scissor) => scissor
                        .intersect(&scissor_bounds)
                        .unwrap_or_else(empty_clip),
                    None => scissor_bounds,
                }),
            }
        } else {
            self
        }
    }
}

#[derive(Clone, Copy)]
struct ChildStackContext<'a> {
    parent:        &'a Element,
    content_box:   ContentBox,
    content_size:  Vec2,
    flow:          ChildFlow,
    scroll_offset: Vec2,
    clip_context:  ClipContext,
}

#[derive(Clone, Copy)]
enum ChildFlow {
    Row { reverse_cursor_start: f32 },
    Column { reverse_cursor_start: f32 },
    Overlay,
}

impl ChildStackContext<'_> {
    const fn child_main_size(&self, child_size: Vec2) -> f32 {
        match self.flow {
            ChildFlow::Row { .. } => child_size.x,
            ChildFlow::Column { .. } => child_size.y,
            ChildFlow::Overlay => 0.0,
        }
    }

    const fn reverse_cursor_start(&self) -> Option<f32> {
        match self.flow {
            ChildFlow::Row {
                reverse_cursor_start,
            }
            | ChildFlow::Column {
                reverse_cursor_start,
            } => Some(reverse_cursor_start),
            ChildFlow::Overlay => None,
        }
    }

    fn editor_row_x(&self, first_child: bool, x: f32, inherited: EditorRowScroll) -> f32 {
        if self.parent.caret_follow {
            // The field owns the record, but its replacement column and
            // validation stay fixed. Only the first editor row moves.
            x + self.scroll_offset.x
        } else if first_child {
            match inherited {
                EditorRowScroll::AtReplacementRoot(offset) => x - offset,
                EditorRowScroll::Inactive => x,
            }
        } else {
            x
        }
    }

    fn child_position(&self, origin: Vec2, reverse_cursor: f32, child_size: Vec2) -> Vec2 {
        let parent = self.parent;
        let base = origin + self.content_box.origin - self.scroll_offset;
        let aligned_width = if parent.caret_follow {
            self.content_size.x.max(child_size.x)
        } else {
            child_size.x
        };
        // Centre alignment keeps the anchors coincident even when a child
        // overflows. Empty icon bearings can exceed a smaller control slot.
        match self.flow {
            ChildFlow::Row { .. } => {
                let cross_offset = match parent.child_layout.align_y() {
                    AlignY::Top => 0.0,
                    AlignY::Center => (self.content_box.size.y - child_size.y) * 0.5,
                    AlignY::Bottom => (self.content_box.size.y - child_size.y).max(0.0),
                };
                Vec2::new(base.x + reverse_cursor, base.y + cross_offset)
            },
            ChildFlow::Column { .. } => {
                let cross_offset = match parent.child_layout.align_x() {
                    AlignX::Left => 0.0,
                    AlignX::Center => (self.content_box.size.x - aligned_width) * 0.5,
                    AlignX::Right => (self.content_box.size.x - aligned_width).max(0.0),
                };
                Vec2::new(base.x + cross_offset, base.y + reverse_cursor)
            },
            ChildFlow::Overlay => {
                let x_offset = match parent.child_layout.align_x() {
                    AlignX::Left => 0.0,
                    AlignX::Center => (self.content_box.size.x - aligned_width) * 0.5,
                    AlignX::Right => (self.content_box.size.x - aligned_width).max(0.0),
                };
                let y_offset = match parent.child_layout.align_y() {
                    AlignY::Top => 0.0,
                    AlignY::Center => (self.content_box.size.y - child_size.y) * 0.5,
                    AlignY::Bottom => (self.content_box.size.y - child_size.y).max(0.0),
                };
                Vec2::new(base.x + x_offset, base.y + y_offset)
            },
        }
    }
}

/// Emits border and scissor-end commands during the up-traversal (second visit)
/// of the DFS positioning pass.
fn emit_up_traversal_commands<const VIEWS: bool>(
    tree: &LayoutTree,
    computed: &[ComputedLayout],
    commands: &mut Vec<RenderCommand>,
    element: &Element,
    bounds: BoundingBox,
    index: usize,
    scrolls: &[SolvedScroll],
) {
    if let Some(divider) = element.child_layout.divider()
        && divider.width().value > 0.0
    {
        emit_child_dividers(tree, computed, commands, index, divider);
    }

    if element.overflow.clips() {
        if VIEWS && matches!(element.scroll_drive, ScrollDrive::Library(_)) {
            emit_scroll_indicator(tree, commands, scrolls, index, element.z_index);
        }
        push_command(
            commands,
            bounds,
            RenderCommandKind::ScissorEnd,
            index,
            element.z_index,
        );
    }
    // The content scissor stops at the border's inner edge. The outline
    // belongs outside that scissor, while still respecting ancestor clips.
    if let Some(ref border) = element.border {
        push_command(
            commands,
            bounds,
            RenderCommandKind::Border { border: *border },
            index,
            element.z_index,
        );
    }
}

fn emit_scroll_indicator(
    tree: &LayoutTree,
    commands: &mut Vec<RenderCommand>,
    scrolls: &[SolvedScroll],
    index: usize,
    z_index: DrawZIndex,
) {
    if let Some(SolvedScroll {
        indicator:
            SolvedScrollIndicator::Shown {
                thumb_points,
                opacity,
            },
        ..
    }) = scrolls.iter().find(|scroll| scroll.element_index == index)
    {
        push_command(
            commands,
            *thumb_points,
            RenderCommandKind::Rectangle {
                color:  scroll_indicator_color(tree, index, *opacity),
                source: RectangleSource::ScrollIndicator,
            },
            index,
            z_index,
        );
    }
}

/// Resolve only for a visible thumb, while generating its layout command.
/// No fill means light ink; the root background is the panel's fill.
fn scroll_indicator_color(tree: &LayoutTree, mut index: usize, opacity: f32) -> Color {
    let (ink, alpha) = loop {
        if let Some(fill) = tree.elements[index].background {
            let linear = fill.to_linear();
            let luminance = linear
                .red
                .mul_add(0.2126, linear.green.mul_add(0.7152, linear.blue * 0.0722));
            // Linear-light blending needs more black coverage for 3:1 contrast.
            // Keep light ink below the HDR compensation table's white endpoint
            // so its fade darkens immediately instead of staying saturated.
            break if luminance > 0.5 {
                (0.0, 0.75)
            } else {
                (0.8, 0.6)
            };
        }
        let Some(parent) =
            (0..tree.len()).find(|&parent| tree.children_of(parent).contains(&index))
        else {
            break (0.8, 0.6);
        };
        index = parent;
    };
    Color::srgba(ink, ink, ink, alpha * opacity)
}

fn needs_up_traversal(element: &Element) -> bool {
    // Keep this predicate in sync with `emit_up_traversal_commands`: skipping a
    // required second visit can drop borders/dividers or unbalance scissors.
    element.border.is_some()
        || element
            .child_layout
            .divider()
            .is_some_and(|divider| divider.width().value > 0.0)
        || element.overflow.clips()
}

/// Emits background, scissor-start, and text render commands during the
/// down-traversal (first visit) of the DFS positioning pass.
fn emit_down_traversal_commands(
    commands: &mut Vec<RenderCommand>,
    element: &Element,
    wrapped: Option<&WrappedText>,
    layout: &ComputedLayout,
    index: usize,
    font_scale: f32,
    clip_context: ClipContext,
) {
    let bounds = layout.bounds;
    // Emit rectangle if background is set.
    if let Some(color) = element.background {
        push_command(
            commands,
            bounds,
            RenderCommandKind::Rectangle {
                color,
                source: RectangleSource::Background,
            },
            index,
            element.z_index,
        );
    }

    // Emit scissor start if clipping (always emit — scissor regions
    // must be balanced even when the parent is off-screen).
    // Clip to the border's inner edge — content can fill up to (but
    // not into) the border. Padding is inside this region.
    if element.overflow.clips() {
        let clip_bounds = element.child_clip_bounds(bounds);
        push_command(
            commands,
            clip_bounds,
            RenderCommandKind::ScissorStart,
            index,
            element.z_index,
        );
    }

    emit_shape_commands(commands, element, bounds, index, clip_context);

    // Emit text render commands.
    if let ElementContent::Text {
        ref config,
        ref text,
        ..
    } = element.content
    {
        emit_text_commands(
            commands,
            wrapped,
            config,
            text,
            element,
            layout,
            index,
            font_scale,
            element.z_index,
        );
    } else if let ElementContent::RichText(paragraph) = &element.content {
        emit_rich_text_commands(commands, paragraph, element, layout, index, font_scale);
    }

    if let ElementContent::Icon(ref icon) = element.content {
        push_command(
            commands,
            bounds,
            RenderCommandKind::Icon(icon.clone()),
            index,
            element.z_index,
        );
    }

    // Emit image render commands.
    if let ElementContent::Image { ref handle, tint } = element.content {
        push_command(
            commands,
            bounds,
            RenderCommandKind::Image {
                handle: handle.clone(),
                tint,
            },
            index,
            element.z_index,
        );
    }
}

// Rich paragraph scaling and Arc ownership never enter the plain emission body.
#[inline(never)]
fn emit_rich_text_commands(
    commands: &mut Vec<RenderCommand>,
    paragraph: &Arc<crate::RichParagraph>,
    element: &Element,
    layout: &ComputedLayout,
    index: usize,
    font_scale: f32,
) {
    let start = commands.len();
    let hard_lines;
    let emission_lines = if paragraph.text.contains('\n') {
        let count = paragraph.text.split('\n').count();
        hard_lines = WrappedText {
            lines:       paragraph
                .text
                .split('\n')
                .map(|text| WrappedLine {
                    text:  text.to_owned(),
                    width: layout.drawn_text_width,
                })
                .collect(),
            line_height: layout.drawn_text_height / count.to_f32(),
        };
        Some(&hard_lines)
    } else {
        None
    };
    emit_text_commands(
        commands,
        emission_lines,
        &paragraph.base,
        &paragraph.text,
        element,
        layout,
        index,
        font_scale,
        element.z_index,
    );
    let paragraph = if font_scale.to_bits() == 1.0_f32.to_bits() {
        Arc::clone(paragraph)
    } else {
        Arc::new(paragraph.scaled(font_scale))
    };
    for (line_index, command) in commands[start..].iter_mut().enumerate() {
        command.kind = RenderCommandKind::RichText {
            paragraph: Arc::clone(&paragraph),
            line_index,
        };
    }
}

fn emit_precompose_command(
    commands: &mut Vec<RenderCommand>,
    bounds: BoundingBox,
    index: usize,
    z_index: DrawZIndex,
) {
    push_command(
        commands,
        bounds,
        RenderCommandKind::PrecomposeLdr,
        index,
        z_index,
    );
}

fn emit_shape_commands(
    commands: &mut Vec<RenderCommand>,
    element: &Element,
    bounds: BoundingBox,
    index: usize,
    clip_context: ClipContext,
) {
    let Some(panel_draw) = element.draw.as_ref() else {
        return;
    };
    if panel_draw.shapes_ref().is_empty() {
        return;
    }

    let source_command_index = commands.len();
    let (clip, clip_policy) = match panel_draw.overflow_policy() {
        DrawOverflow::Clipped => (
            Some(clip_context.inherited),
            PanelShapeClipPolicy::OwnerBounds,
        ),
        DrawOverflow::Visible => (clip_context.scissor, PanelShapeClipPolicy::Inherited),
    };

    let mut shapes = Vec::new();
    for (shape_ordinal, shape) in panel_draw.shapes_ref().iter().enumerate() {
        let source_key = PanelShapeSourceKey::element(index, 0, shape_ordinal);
        let context = PanelShapeResolveContext::new(
            bounds,
            clip,
            clip_policy,
            source_command_index,
            source_key,
        );
        let resolved = match shape {
            PanelShape::Line(line) => line::resolve_panel_line(line, context),
            PanelShape::Circle(circle) => line::resolve_panel_circle(circle, context),
        };
        if let Some(resolved) = resolved {
            shapes.push(resolved);
        }
    }

    let Some(command_bounds) = line_command_bounds(&shapes) else {
        return;
    };
    push_command(
        commands,
        command_bounds,
        RenderCommandKind::PanelShapes { shapes },
        index,
        element.z_index,
    );
}

/// Emits render commands for text content (both wrapped and unwrapped).
fn emit_text_commands(
    commands: &mut Vec<RenderCommand>,
    wrapped: Option<&WrappedText>,
    config: &TextStyle,
    text: &str,
    element: &Element,
    layout: &ComputedLayout,
    index: usize,
    font_scale: f32,
    z_index: DrawZIndex,
) {
    let bounds = sizing::content_box_bounds(element, layout.bounds);
    let single_line = match layout.cap_height_metrics {
        CapHeightMeasurement::FontMetrics(metrics) => {
            metrics.shaped_line_count == 1 && wrapped.is_none_or(|wrapped| wrapped.lines.len() == 1)
        },
        CapHeightMeasurement::LineBoxFallback => false,
    };
    let cap_metrics = if single_line {
        layout.cap_height_metrics
    } else {
        CapHeightMeasurement::LineBoxFallback
    };
    // A parent centres the leaf's outer box, including any authored cap-box
    // trims. Use that same centre through Fit wrappers; fixed boxes instead
    // centre their own padded content. Multi-line blocks keep line-box rules.
    let (vertical_align, vertical_bounds) =
        if matches!(cap_metrics, CapHeightMeasurement::FontMetrics(_))
            && layout.label_alignment == LabelAlignment::ParentCapCenter
        {
            (AlignY::Center, layout.bounds)
        } else {
            (element.child_layout.align_y(), bounds)
        };
    // Render commands store font sizes in layout units, so downstream renderers
    // do not repeat the font-unit conversion. `TextStyle::scaled(1.0)` is an
    // identity transform, so a pre-scaled tree (`font_scale == 1.0`) clones the
    // config and skips the multiply.
    let scaled_config = if font_scale.to_bits() == 1.0_f32.to_bits() {
        config.clone()
    } else {
        config.scaled(font_scale)
    };

    if let Some(wrap_result) = wrapped {
        // Wrapped text: emit one command per line.
        let text_height = wrap_result.line_height * wrap_result.lines.len().to_f32();
        let first_line_y =
            line_y_for_alignment(vertical_align, vertical_bounds, text_height, cap_metrics);
        for (line_idx, line) in wrap_result.lines.iter().enumerate() {
            let line_y = wrap_result
                .line_height
                .mul_add(line_idx.to_f32(), first_line_y);
            let line_x = line_x_for_alignment(config.text_align(), bounds, line.width);
            push_command(
                commands,
                BoundingBox {
                    x:      line_x,
                    y:      line_y,
                    width:  line.width,
                    height: wrap_result.line_height,
                },
                RenderCommandKind::Text {
                    text:   line.text.clone(),
                    config: scaled_config.clone(),
                },
                index,
                z_index,
            );
        }
    } else {
        // Unwrapped text keeps the content box's alignment while drawing at
        // the visible string's own measured width. `measure_as` can reserve a
        // wider box without drawing the surrogate text.
        let drawn_width = layout.drawn_text_width;
        let drawn_height = layout.drawn_text_height;
        push_command(
            commands,
            BoundingBox {
                x:      line_x_for_alignment(config.text_align(), bounds, drawn_width),
                y:      line_y_for_alignment(
                    vertical_align,
                    vertical_bounds,
                    drawn_height,
                    cap_metrics,
                ),
                width:  drawn_width,
                height: drawn_height,
            },
            RenderCommandKind::Text {
                text:   text.to_owned(),
                config: scaled_config,
            },
            index,
            z_index,
        );
    }
}

fn line_y_for_alignment(
    align: AlignY,
    bounds: BoundingBox,
    text_height: f32,
    cap_metrics: CapHeightMeasurement,
) -> f32 {
    let extra = (bounds.height - text_height).max(0.0);
    match align {
        AlignY::Top => bounds.y,
        AlignY::Center => match cap_metrics {
            CapHeightMeasurement::LineBoxFallback => extra.mul_add(0.5, bounds.y),
            CapHeightMeasurement::FontMetrics(metrics) => f32::mul_add(
                metrics.cap_height,
                0.5,
                bounds.height.mul_add(0.5, bounds.y) - metrics.baseline,
            ),
        },
        AlignY::Bottom => bounds.y + extra,
    }
}

fn line_x_for_alignment(align: TextAlign, bounds: BoundingBox, line_width: f32) -> f32 {
    match align {
        TextAlign::Left => bounds.x,
        TextAlign::Center => (bounds.width - line_width).mul_add(0.5, bounds.x),
        TextAlign::Right => bounds.x + bounds.width - line_width,
    }
}

fn line_command_bounds(lines: &[ResolvedPanelShape]) -> Option<BoundingBox> {
    let mut iter = lines.iter();
    let first = iter.next()?.visual_bounds;
    Some(iter.fold(first, |bounds, line| {
        union_bounds(bounds, line.visual_bounds)
    }))
}

fn union_bounds(a: BoundingBox, b: BoundingBox) -> BoundingBox {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    BoundingBox {
        x:      x0,
        y:      y0,
        width:  x1 - x0,
        height: y1 - y0,
    }
}

/// Root DFS clip: the panel's own laid-out box. `size_along_axis` resolves the
/// root element to its real size during the sizing pass — content size for a
/// `Fit` panel, viewport-derived for `Grow`/`Percent`, declared size for
/// `Fixed` — so this is the correct owner-bounds clip on the very first solve,
/// before the panel's stored dimensions (`panel.width()`/`height()`) are written
/// back. A `Fit` screen panel's stored size lags one solve, but its root element
/// is already content-sized here.
const fn root_box_clip(width: f32, height: f32) -> BoundingBox {
    BoundingBox {
        x: 0.0,
        y: 0.0,
        width,
        height,
    }
}

const fn empty_clip() -> BoundingBox {
    BoundingBox {
        x:      0.0,
        y:      0.0,
        width:  0.0,
        height: 0.0,
    }
}

/// Text measured to find one line's height for a scrolling element's wheel
/// line pitch; any single-line glyph run yields the style's line height.
const SCROLL_LINE_PROBE_TEXT: &str = "M";

/// Collects the solved state of each scrolling parent during positioning,
/// measuring its wheel line pitch with the engine's text measurer.
pub(super) struct ScrollSolver<'a> {
    measure_text:     &'a MeasureTextFn,
    font_scale:       f32,
    solved:           Vec<SolvedScroll>,
    previous_offsets: &'a ScrollSolveInput,
}

impl<'a> ScrollSolver<'a> {
    pub(super) const fn new(
        measure_text: &'a MeasureTextFn,
        font_scale: f32,
        previous_offsets: &'a ScrollSolveInput,
    ) -> Self {
        Self {
            measure_text,
            font_scale,
            solved: Vec::new(),
            previous_offsets,
        }
    }

    pub(super) fn into_solved(self) -> Vec<SolvedScroll> { self.solved }

    fn view_indicator(
        &self,
        parent: &Element,
        child_context: &ChildStackContext<'_>,
        border_box: BoundingBox,
    ) -> (f32, SolvedScrollIndicator) {
        let view = match parent.scroll_drive {
            ScrollDrive::Library(_) => parent
                .id
                .as_ref()
                .and_then(|id| self.previous_offsets.views.get(id)),
            ScrollDrive::Author => None,
        };
        let overscroll = if child_context.content_size.y > child_context.content_box.size.y {
            view.map_or(0.0, |view| view.overscroll_points)
        } else {
            0.0
        };
        let indicator = if matches!(parent.scroll_drive, ScrollDrive::Library(_)) {
            scroll_view::solve_indicator(
                parent.child_clip_bounds(border_box),
                child_context.content_box.size.y,
                child_context.content_size.y,
                child_context.scroll_offset.y - overscroll,
                overscroll,
                view.map_or(0.0, |view| view.indicator_opacity),
            )
        } else {
            ScrollIndicatorState::Hidden
        };
        let indicator = match indicator {
            ScrollIndicatorState::Hidden => SolvedScrollIndicator::Hidden,
            ScrollIndicatorState::Shown { thumb, opacity } => SolvedScrollIndicator::Shown {
                thumb_points: thumb,
                opacity,
            },
        };
        (overscroll, indicator)
    }

    fn record<const VIEWS: bool>(
        &mut self,
        parent: &Element,
        element_index: usize,
        child_context: &ChildStackContext<'_>,
        visible_region: VisibleScrollRegion,
        border_box: BoundingBox,
    ) {
        let line = (self.measure_text)(
            SCROLL_LINE_PROBE_TEXT,
            &parent.scroll_line.measure().scaled(self.font_scale),
        );
        let gap = parent.child_layout.main_gap().map_or(0.0, |gap| gap.value);
        let (overscroll, indicator) = if VIEWS {
            self.view_indicator(parent, child_context, border_box)
        } else {
            (0.0, SolvedScrollIndicator::Hidden)
        };
        self.solved.push(SolvedScroll {
            overscroll_points: overscroll,
            indicator,
            element_index,
            content_points: child_context.content_size,
            viewport_points: child_context.content_box.size,
            offset_points: child_context.scroll_offset - Vec2::new(0.0, overscroll),
            visible_region,
            line_pitch_points: line.line_height + gap,
        });
    }
}

/// Resolves the clamped per-axis scroll offset for a scrolling parent, returning
/// `Vec2::ZERO` for the common non-scrolling case. Each axis clamps to
/// `[0, content - viewport]`; `End`-anchored axes measure the offset from the
/// far edge so `0` pins to the bottom/right.
fn resolve_scroll_offset<const VIEWS: bool>(
    parent_el: &Element,
    max_scroll: Vec2,
    previous_offsets: &ScrollSolveInput,
) -> Vec2 {
    if VIEWS && let ScrollDrive::Library(edge) = parent_el.scroll_drive {
        let view = parent_el
            .id
            .as_ref()
            .and_then(|id| previous_offsets.views.get(id));
        let rest = match edge {
            ScrollRestEdge::Start => 0.0,
            ScrollRestEdge::End => max_scroll.y,
        };
        return Vec2::new(
            0.0,
            view.map_or(rest, |view| {
                view.offset_points.clamp(0.0, max_scroll.y)
                    + if max_scroll.y > 0.0 {
                        view.overscroll_points
                    } else {
                        0.0
                    }
            }),
        );
    }
    let caret_offset = if parent_el.caret_follow {
        match parent_el
            .editable
            .as_ref()
            .map(|field| previous_offsets.previous(&field.field_id))
        {
            Some(CaretOffsetState::Previous(offset)) => offset,
            Some(CaretOffsetState::FirstSolve) | None => 0.0,
        }
    } else {
        parent_el.scroll_offset.x
    };
    // A zero `Start` offset is a no-op; a zero `End` offset still resolves
    // (scrollback 0 pins to the end), so don't short-circuit it.
    if caret_offset == 0.0
        && parent_el.scroll_offset.y == 0.0
        && parent_el.scroll_anchor_x == ScrollAnchor::Start
        && parent_el.scroll_anchor_y == ScrollAnchor::Start
    {
        return Vec2::ZERO;
    }

    let resolve = |offset: f32, max: f32, anchor: ScrollAnchor| match anchor {
        ScrollAnchor::Start => offset.clamp(0.0, max),
        ScrollAnchor::End => (max - offset).clamp(0.0, max),
    };
    Vec2::new(
        resolve(caret_offset, max_scroll.x, parent_el.scroll_anchor_x),
        resolve(
            parent_el.scroll_offset.y,
            max_scroll.y,
            parent_el.scroll_anchor_y,
        ),
    )
}

fn child_stack_context<'a, const VIEWS: bool>(
    tree: &LayoutTree,
    parent: &'a Element,
    computed: &[ComputedLayout],
    children: &[usize],
    parent_size: Vec2,
    clip_context: ClipContext,
    previous_offsets: &ScrollSolveInput,
) -> ChildStackContext<'a> {
    let content_box = sizing::content_box(parent, parent_size);
    let flow_content_size = children_content_size(&parent.child_layout, computed, children);
    let mut child_content_size = flow_content_size;
    if parent.caret_follow {
        let mut pending = children.to_vec();
        while let Some(index) = pending.pop() {
            if tree.elements[index].caret_marker {
                child_content_size.x += tree
                    .children_of(index)
                    .first()
                    .map_or(0.0, |&child| computed[child].width);
                break;
            }
            pending.extend_from_slice(tree.children_of(index));
        }
    }
    let max_scroll = Vec2::new(
        (child_content_size.x - content_box.size.x).max(0.0),
        (child_content_size.y - content_box.size.y).max(0.0),
    );
    let scroll_offset = resolve_scroll_offset::<VIEWS>(parent, max_scroll, previous_offsets);

    let flow = match parent.child_layout {
        ChildLayout::Row { .. } => {
            let extra_main = (content_box.size.x - child_content_size.x).max(0.0);
            let main_offset = match parent.child_layout.align_x() {
                AlignX::Left => 0.0,
                AlignX::Center => extra_main * 0.5,
                AlignX::Right => extra_main,
            };
            ChildFlow::Row {
                reverse_cursor_start: main_offset + flow_content_size.x,
            }
        },
        ChildLayout::Column { .. } => {
            let extra_main = (content_box.size.y - flow_content_size.y).max(0.0);
            let main_offset = match parent.child_layout.align_y() {
                AlignY::Top => 0.0,
                AlignY::Center => extra_main * 0.5,
                AlignY::Bottom => extra_main,
            };
            ChildFlow::Column {
                reverse_cursor_start: main_offset + flow_content_size.y,
            }
        },
        ChildLayout::Overlay { .. } => ChildFlow::Overlay,
    };

    ChildStackContext {
        parent,
        content_box,
        content_size: child_content_size,
        flow,
        scroll_offset,
        clip_context,
    }
}

fn children_content_size(
    child_layout: &ChildLayout,
    computed: &[ComputedLayout],
    children: &[usize],
) -> Vec2 {
    let mut content_size = Vec2::ZERO;
    for &idx in children {
        let child_size = Vec2::new(computed[idx].width, computed[idx].height);
        match child_layout {
            ChildLayout::Row { .. } => {
                content_size.x += child_size.x;
                content_size.y = content_size.y.max(child_size.y);
            },
            ChildLayout::Column { .. } => {
                content_size.x = content_size.x.max(child_size.x);
                content_size.y += child_size.y;
            },
            ChildLayout::Overlay { .. } => {
                content_size.x = content_size.x.max(child_size.x);
                content_size.y = content_size.y.max(child_size.y);
            },
        }
    }

    match child_layout {
        ChildLayout::Row { .. } => {
            content_size.x += main_gap_total(child_layout, children.len());
        },
        ChildLayout::Column { .. } => {
            content_size.y += main_gap_total(child_layout, children.len());
        },
        ChildLayout::Overlay { .. } => {},
    }
    content_size
}

/// Whether caret visibility requires a second positioning pass in this solve.
pub(super) enum CaretOffsetCorrection {
    Unchanged,
    Corrected(ScrollSolveInput),
}

/// Compares the marked caret's solved span with its field's content box.
/// The second positioning pass applies any correction before rendering.
pub(super) fn corrected_caret_offsets(
    tree: &LayoutTree,
    computed: &[ComputedLayout],
    scrolls: &[SolvedScroll],
    previous_offsets: &ScrollSolveInput,
) -> CaretOffsetCorrection {
    let mut corrected = previous_offsets.clone();
    let mut changed = false;
    for scroll in scrolls {
        let field_index = scroll.element_index;
        let field = &tree.elements[field_index];
        if !field.caret_follow {
            continue;
        }
        let Some(id) = field.editable.as_ref().map(|editable| &editable.field_id) else {
            continue;
        };
        let mut pending = tree.children_of(field_index).to_vec();
        let mut caret = None;
        while let Some(index) = pending.pop() {
            if tree.elements[index].caret_marker {
                caret = Some(index);
                break;
            }
            pending.extend_from_slice(tree.children_of(index));
        }
        let Some(caret_index) = caret else { continue };
        let caret_index = tree
            .children_of(caret_index)
            .first()
            .copied()
            .unwrap_or(caret_index);
        let caret_bounds = computed[caret_index].bounds;
        let viewport = sizing::content_box_bounds(field, computed[field_index].bounds);
        let old = scroll.offset_points.x;
        let desired = if caret_bounds.x + caret_bounds.width > viewport.x + viewport.width {
            old + caret_bounds.x + caret_bounds.width - viewport.x - viewport.width
        } else if caret_bounds.x < viewport.x {
            old + caret_bounds.x - viewport.x
        } else {
            old
        };
        let max = (scroll.content_points.x - scroll.viewport_points.x).max(0.0);
        let desired = desired.clamp(0.0, max);
        if (desired - old).abs() > 0.001 {
            corrected.insert(id.clone(), desired);
            changed = true;
        }
    }
    if changed {
        CaretOffsetCorrection::Corrected(corrected)
    } else {
        CaretOffsetCorrection::Unchanged
    }
}

fn main_gap_total(child_layout: &ChildLayout, child_count: usize) -> f32 {
    if child_count > 1 {
        child_layout
            .main_gap()
            .map_or(0.0, |gap| gap.value * (child_count - 1).to_f32())
    } else {
        0.0
    }
}

/// Whether this Fit child belongs to a vertically centred label chain.
/// A fixed-height child owns its own alignment; a Fit wrapper carries its
/// parent's choice only while it contains a single vertical group.
fn child_label_alignment(
    tree: &LayoutTree,
    parent: usize,
    inherited: LabelAlignment,
    child: usize,
) -> LabelAlignment {
    let parent_element = &tree.elements[parent];
    if tree.elements[child].height.is_fit()
        && (parent_element.child_layout.align_y() == AlignY::Center
            || (inherited == LabelAlignment::ParentCapCenter
                && parent_element.height.is_fit()
                && (!matches!(parent_element.child_layout, ChildLayout::Column { .. })
                    || tree.children_of(parent).len() == 1)))
    {
        LabelAlignment::ParentCapCenter
    } else {
        LabelAlignment::OwnContent
    }
}

/// Pushes children onto the DFS stack in reverse order with computed positions.
///
/// Children are pushed in reverse so the first child is processed first during
/// iteration. Uses a reverse cursor to compute positions without allocation.
/// A scrolling parent also records its solved scroll state. `clip_context` is
/// the parent's own clip; its children's clip is derived here.
fn push_children_to_stack<const VIEWS: bool>(
    tree: &LayoutTree,
    computed: &[ComputedLayout],
    stack: &mut Vec<PositionStackEntry>,
    scrolls: &mut ScrollSolver<'_>,
    index: usize,
    bounds: BoundingBox,
    clip_context: ClipContext,
    editor_row_scroll: EditorRowScroll,
    label_alignment: LabelAlignment,
) {
    let children = tree.children_of(index);
    if children.is_empty()
        && !(VIEWS && matches!(tree.elements[index].scroll_drive, ScrollDrive::Library(_)))
    {
        return;
    }

    let parent_el = &tree.elements[index];
    let child_context = child_stack_context::<VIEWS>(
        tree,
        parent_el,
        computed,
        children,
        Vec2::new(computed[index].width, computed[index].height),
        clip_context.child(parent_el, bounds),
        scrolls.previous_offsets,
    );

    if matches!(parent_el.overflow, ChildOverflow::Scrolled) {
        scrolls.record::<VIEWS>(
            parent_el,
            index,
            &child_context,
            bounds.intersect(&clip_context.inherited).map_or(
                VisibleScrollRegion::ClippedOut,
                VisibleScrollRegion::Visible,
            ),
            bounds,
        );
    }

    if children.is_empty() {
        return;
    }
    let origin = Vec2::new(bounds.x, bounds.y);
    let first_child = children[0];
    let child_scroll = if parent_el.caret_follow {
        EditorRowScroll::AtReplacementRoot(child_context.scroll_offset.x)
    } else {
        EditorRowScroll::Inactive
    };
    let adjusted_x = |child_idx: usize, x: f32| {
        child_context.editor_row_x(child_idx == first_child, x, editor_row_scroll)
    };
    if let Some(mut reverse_cursor) = child_context.reverse_cursor_start() {
        // Walk children in reverse, subtracting each child's main size from the
        // cursor to produce positions in stack-push order.
        let gap = parent_el
            .child_layout
            .main_gap()
            .map_or(0.0, |gap| gap.value);
        for &child_idx in children.iter().rev() {
            let child_size = Vec2::new(computed[child_idx].width, computed[child_idx].height);
            let child_main = child_context.child_main_size(child_size);
            reverse_cursor -= child_main;
            let child_position = child_context.child_position(origin, reverse_cursor, child_size);

            stack.push(PositionStackEntry {
                index:             child_idx,
                x:                 adjusted_x(child_idx, child_position.x),
                y:                 child_position.y,
                visited:           false,
                clip_context:      child_context.clip_context,
                editor_row_scroll: child_scroll,
                label_alignment:   child_label_alignment(tree, index, label_alignment, child_idx),
            });
            reverse_cursor -= gap;
        }
    } else {
        for &child_idx in children.iter().rev() {
            let child_size = Vec2::new(computed[child_idx].width, computed[child_idx].height);
            let child_position = child_context.child_position(origin, 0.0, child_size);

            stack.push(PositionStackEntry {
                index:             child_idx,
                x:                 adjusted_x(child_idx, child_position.x),
                y:                 child_position.y,
                visited:           false,
                clip_context:      child_context.clip_context,
                editor_row_scroll: child_scroll,
                label_alignment:   child_label_alignment(tree, index, label_alignment, child_idx),
            });
        }
    }
}

/// DFS positioning pass. Computes final bounding boxes and emits render commands.
pub(super) fn position_and_render(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    root: usize,
    wrapped: &[Option<WrappedText>],
    font_scale: f32,
    scrolls: &mut ScrollSolver<'_>,
) -> Vec<RenderCommand> {
    if tree.contains_scroll_views() {
        position_tree::<true>(tree, computed, root, wrapped, font_scale, scrolls)
    } else {
        position_tree::<false>(tree, computed, root, wrapped, font_scale, scrolls)
    }
}

fn position_tree<const VIEWS: bool>(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    root: usize,
    wrapped: &[Option<WrappedText>],
    font_scale: f32,
    scrolls: &mut ScrollSolver<'_>,
) -> Vec<RenderCommand> {
    let mut commands = Vec::with_capacity(tree.len() * 2);
    let root_clip = root_box_clip(computed[root].width, computed[root].height);

    let mut stack = Vec::with_capacity(tree.len());
    stack.push(PositionStackEntry {
        index:             root,
        x:                 0.0,
        y:                 0.0,
        visited:           false,
        clip_context:      ClipContext::root(root_clip),
        editor_row_scroll: EditorRowScroll::Inactive,
        label_alignment:   LabelAlignment::OwnContent,
    });

    while let Some(entry) = stack.pop() {
        let index = entry.index;
        let element = &tree.elements[index];
        let bounds = BoundingBox {
            x:      entry.x,
            y:      entry.y,
            width:  computed[index].width,
            height: computed[index].height,
        };

        if entry.visited {
            emit_up_traversal_commands::<VIEWS>(
                tree,
                computed,
                &mut commands,
                element,
                bounds,
                index,
                &scrolls.solved,
            );
        } else {
            // Store the final bounding box for render-side culling and clipping.
            computed[index].bounds = bounds;
            computed[index].label_alignment = entry.label_alignment;

            if matches!(element.precompose, PrecomposeMode::Ldr) {
                emit_precompose_command(&mut commands, bounds, index, element.z_index);
            } else {
                emit_down_traversal_commands(
                    &mut commands,
                    element,
                    wrapped.get(index).and_then(Option::as_ref),
                    &computed[index],
                    index,
                    font_scale,
                    entry.clip_context,
                );

                if needs_up_traversal(element) {
                    stack.push(PositionStackEntry {
                        visited: true,
                        ..entry
                    });
                }
                push_children_to_stack::<VIEWS>(
                    tree,
                    computed,
                    &mut stack,
                    scrolls,
                    index,
                    bounds,
                    entry.clip_context,
                    entry.editor_row_scroll,
                    entry.label_alignment,
                );
            }
        }
    }
    commands
}

/// Generates render commands from already-computed element bounds.
// Keep command refresh in its caller, as on the plain layout path.
#[allow(
    clippy::inline_always,
    reason = "fixed-work profiling shows outlining this traversal adds instructions to plain command refresh"
)]
#[inline(always)]
pub(super) fn render_commands_from_geometry<const VIEWS: bool>(
    tree: &LayoutTree,
    computed: &[ComputedLayout],
    root: usize,
    wrapped: &[Option<WrappedText>],
    font_scale: f32,
    scrolls: &[SolvedScroll],
) -> Vec<RenderCommand> {
    let mut commands = Vec::with_capacity(tree.len() * 2);
    let root_clip = root_box_clip(computed[root].bounds.width, computed[root].bounds.height);

    let mut stack = Vec::with_capacity(tree.len());
    stack.push(GeometryStackEntry {
        index:        root,
        visited:      false,
        clip_context: ClipContext::root(root_clip),
    });

    while let Some(entry) = stack.pop() {
        let index = entry.index;
        let element = &tree.elements[index];
        let bounds = computed[index].bounds;

        if entry.visited {
            emit_up_traversal_commands::<VIEWS>(
                tree,
                computed,
                &mut commands,
                element,
                bounds,
                index,
                scrolls,
            );
            continue;
        }

        if matches!(element.precompose, PrecomposeMode::Ldr) {
            emit_precompose_command(&mut commands, bounds, index, element.z_index);
        } else {
            emit_down_traversal_commands(
                &mut commands,
                element,
                wrapped.get(index).and_then(Option::as_ref),
                &computed[index],
                index,
                font_scale,
                entry.clip_context,
            );

            let child_clip = entry.clip_context.child(element, bounds);
            if needs_up_traversal(element) {
                stack.push(GeometryStackEntry {
                    visited: true,
                    ..entry
                });
            }
            for &child_idx in tree.children_of(index).iter().rev() {
                stack.push(GeometryStackEntry {
                    index:        child_idx,
                    visited:      false,
                    clip_context: child_clip,
                });
            }
        }
    }

    commands
}

/// Emits child-divider rectangles.
///
/// Uses children's already-computed bounds (set during DFS first visit)
/// to avoid re-computing positions.
fn emit_child_dividers(
    tree: &LayoutTree,
    computed: &[ComputedLayout],
    commands: &mut Vec<RenderCommand>,
    parent_idx: usize,
    divider: ChildDivider,
) {
    let parent = &tree.elements[parent_idx];
    let parent_bounds = computed[parent_idx].bounds;
    let children = tree.children_of(parent_idx);

    if children.len() < 2 {
        return;
    }

    let is_horizontal = match parent.child_layout {
        ChildLayout::Row { .. } => true,
        ChildLayout::Column { .. } => false,
        ChildLayout::Overlay { .. } => return,
    };
    let width = divider.width().value;
    let color = divider.color();

    // Draw a line between each pair of adjacent children.
    for (ordinal, pair) in children.windows(2).enumerate() {
        let a_bounds = computed[pair[0]].bounds;
        let b_bounds = computed[pair[1]].bounds;

        if is_horizontal {
            let midpoint = (b_bounds.x - (a_bounds.x + a_bounds.width))
                .mul_add(0.5, a_bounds.x + a_bounds.width);
            let line_x = width.mul_add(-0.5, midpoint);
            push_command(
                commands,
                BoundingBox {
                    x: line_x,
                    y: parent_bounds.y + parent.padding.top.value,
                    width,
                    height: parent_bounds.height - parent.padding.vertical(),
                },
                RenderCommandKind::Rectangle {
                    color,
                    source: RectangleSource::ChildDivider { ordinal },
                },
                parent_idx,
                parent.z_index,
            );
        } else {
            let midpoint = (b_bounds.y - (a_bounds.y + a_bounds.height))
                .mul_add(0.5, a_bounds.y + a_bounds.height);
            let line_y = width.mul_add(-0.5, midpoint);
            push_command(
                commands,
                BoundingBox {
                    x:      parent_bounds.x + parent.padding.left.value,
                    y:      line_y,
                    width:  parent_bounds.width - parent.padding.horizontal(),
                    height: width,
                },
                RenderCommandKind::Rectangle {
                    color,
                    source: RectangleSource::ChildDivider { ordinal },
                },
                parent_idx,
                parent.z_index,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::color::Color;

    use crate::PanelElementId;
    use crate::layout::AlignX;
    use crate::layout::AlignY;
    use crate::layout::Border;
    use crate::layout::BoundingBox;
    use crate::layout::El;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutEngine;
    use crate::layout::LayoutResult;
    use crate::layout::MeasureTextFn;
    use crate::layout::Padding;
    use crate::layout::RenderCommandKind;
    use crate::layout::Sizing;
    use crate::layout::Text;
    use crate::layout::TextAlign;
    use crate::layout::TextDimensions;
    use crate::layout::TextStyle;
    use crate::layout::TextWrap;
    use crate::layout::engine::layout_engine::ScrollSolveInput;
    use crate::layout::engine::layout_engine::ScrollViewSolve;
    use crate::layout::render::RectangleSource;

    #[test]
    fn scroll_indicator_uses_the_nearest_fill_and_defaults_to_light_ink() -> Result<(), String> {
        for (root, ancestor, view, expected) in vec![
            (
                El::column().background(Color::WHITE),
                El::column(),
                El::column(),
                0.0,
            ),
            (
                El::column().background(Color::WHITE),
                El::column().background(Color::BLACK),
                El::column(),
                0.8,
            ),
            (
                El::column().background(Color::BLACK),
                El::column(),
                El::column().background(Color::WHITE),
                0.0,
            ),
            (El::column(), El::column(), El::column(), 0.8),
            (
                El::column().background(Color::linear_rgb(0.5, 0.5, 0.5)),
                El::column(),
                El::column(),
                0.8,
            ),
            (
                El::column().background(Color::linear_rgb(0.501, 0.501, 0.501)),
                El::column(),
                El::column(),
                0.0,
            ),
        ]
        .into_boxed_slice()
        {
            let mut builder = LayoutBuilder::with_root(root.size(100.0, 100.0));
            builder.with(ancestor.size(100.0, 100.0), |builder| {
                builder.with(view.size(100.0, 100.0).scroll_view("view"), |builder| {
                    builder.with(El::new().size(100.0, 400.0), |_| {});
                });
            });
            let measure: MeasureTextFn = Arc::new(|_, _| TextDimensions {
                width:       10.0,
                height:      10.0,
                line_height: 10.0,
            });
            let mut previous = ScrollSolveInput::default();
            previous.views.insert(
                PanelElementId::named("view"),
                ScrollViewSolve {
                    offset_points:     0.0,
                    overscroll_points: 0.0,
                    indicator_opacity: 0.5,
                },
            );
            let result = LayoutEngine::new(measure).compute_with_caret_offsets(
                &builder.build(),
                100.0,
                100.0,
                1.0,
                &previous,
            );
            let color = result
                .commands
                .iter()
                .find_map(|command| match command.kind {
                    RenderCommandKind::Rectangle {
                        source: RectangleSource::ScrollIndicator,
                        color,
                    } => Some(color.to_srgba()),
                    _ => None,
                })
                .ok_or("an active overflowing view draws its indicator")?;
            assert_near(color.red, expected);
            assert_near(color.green, expected);
            assert_near(color.blue, expected);
            assert_near(color.alpha, if expected < 0.5 { 0.375 } else { 0.3 });
        }
        Ok(())
    }

    fn text_bounds(result: &LayoutResult) -> BoundingBox {
        let text_commands: Vec<_> = result
            .commands
            .iter()
            .filter(|command| matches!(command.kind, RenderCommandKind::Text { .. }))
            .collect();
        assert_eq!(text_commands.len(), 1);
        text_commands[0].bounds
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn fixed_text_uses_padded_content_box_and_vertical_alignment() {
        let measure: MeasureTextFn = Arc::new(|_, _| TextDimensions {
            width:       20.0,
            height:      10.0,
            line_height: 10.0,
        });
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("value", TextStyle::new(10.0)).layout(
                El::new()
                    .size(80.0, 30.0)
                    .border(Border::all(1.0, Color::WHITE))
                    .padding(Padding::new(6.0, 8.0, 3.0, 5.0))
                    .alignment(AlignX::Left, AlignY::Center),
            ),
        );
        let result = LayoutEngine::new(measure).compute(&builder.build(), 100.0, 50.0, 1.0);
        let outer = result.computed[1].bounds;
        let glyph = text_bounds(&result);
        assert_near(glyph.x, outer.x + 1.0 + 6.0);
        assert_near(
            glyph.y,
            (30.0_f32 - 2.0 - 3.0 - 5.0 - 10.0).mul_add(0.5, outer.y + 1.0 + 3.0),
        );
        assert_near(glyph.width, 20.0);
        assert_near(glyph.height, 10.0);
    }

    #[test]
    fn fit_text_includes_padding_in_its_measured_size() {
        let measure: MeasureTextFn = Arc::new(|_, _| TextDimensions {
            width:       20.0,
            height:      10.0,
            line_height: 10.0,
        });
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("value", TextStyle::new(10.0)).layout(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .padding(Padding::new(6.0, 8.0, 3.0, 5.0)),
            ),
        );
        let result = LayoutEngine::new(measure).compute(&builder.build(), 100.0, 50.0, 1.0);
        let outer = result.computed[1].bounds;
        let glyph = text_bounds(&result);
        assert_near(outer.width, 34.0);
        assert_near(outer.height, 18.0);
        assert_near(glyph.x, outer.x + 6.0);
        assert_near(glyph.y, outer.y + 3.0);
    }

    #[test]
    fn wrapped_text_uses_padded_content_box_and_vertical_alignment() {
        let measure: MeasureTextFn = Arc::new(|text, _| TextDimensions {
            width:       text.chars().fold(0.0, |width, _| width + 5.0),
            height:      10.0,
            line_height: 10.0,
        });
        let mut builder = LayoutBuilder::new(100.0, 60.0);
        builder.text(
            Text::new("one two three", TextStyle::new(10.0))
                .wrap(TextWrap::Words)
                .layout(
                    El::new()
                        .size(40.0, 50.0)
                        .padding(Padding::all(5.0))
                        .alignment(AlignX::Left, AlignY::Center),
                ),
        );
        let result = LayoutEngine::new(measure).compute(&builder.build(), 100.0, 60.0, 1.0);
        let outer = result.computed[1].bounds;
        let lines = result
            .commands
            .iter()
            .filter(|command| matches!(command.kind, RenderCommandKind::Text { .. }))
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert_near(lines[0].bounds.x, outer.x + 5.0);
        assert_near(lines[0].bounds.y, outer.y + 10.0);
        assert_near(lines[2].bounds.y + lines[2].bounds.height, outer.y + 40.0);
    }

    #[test]
    fn measure_as_aligns_visible_text_after_initial_measure_and_regeneration() {
        let measure: MeasureTextFn = Arc::new(|text, style| TextDimensions {
            width:       text.chars().fold(0.0, |width, _| width + style.size),
            height:      style.size,
            line_height: style.size,
        });
        let engine = LayoutEngine::new(Arc::clone(&measure));
        for (align, initial_x, updated_x) in [
            (TextAlign::Right, 10.0, 20.0),
            (TextAlign::Center, 5.0, 10.0),
        ] {
            let tree = |visible: &str| {
                let mut builder = LayoutBuilder::new(100.0, 100.0);
                builder.text(
                    Text::new(visible, TextStyle::new(10.0).with_align(align)).measure_as("100%"),
                );
                builder.build()
            };
            let mut result = engine.compute(&tree("50%"), 100.0, 100.0, 1.0);
            let initial = text_bounds(&result);
            assert_near(initial.x, initial_x);
            assert_near(initial.width, 30.0);

            let updated_tree = tree("5%");
            assert!(result.can_reuse_geometry(&updated_tree, &measure, 100.0, 100.0, 1.0));
            result.regenerate_commands(&updated_tree, &measure);
            let updated = text_bounds(&result);
            assert_near(updated.x, updated_x);
            assert_near(updated.width, 20.0);
            assert_near(result.computed[1].bounds.width, 40.0);
        }
    }
}
