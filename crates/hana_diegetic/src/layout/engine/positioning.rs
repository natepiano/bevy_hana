use bevy::math::Vec2;
use bevy_kana::ToF32;

use super::layout_engine::ComputedLayout;
use super::sizing;
use super::sizing::ContentBox;
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
    index:        usize,
    x:            f32,
    y:            f32,
    visited:      bool,
    clip_context: ClipContext,
}

#[derive(Clone, Copy)]
struct GeometryStackEntry {
    index:        usize,
    visited:      bool,
    clip_context: ClipContext,
}

/// Clip state threaded down the DFS.
/// `inherited` = viewport ∩ every [`ChildOverflow::Clipped`] ancestor; applied to owner-bound
/// content. `scissor`   = only explicit [`ChildOverflow::Clipped`] ancestors (no viewport); line
/// geometry that               overflows a panel is not clipped to the panel viewport, but still
/// respects these regions.
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

    fn child(self, element: &Element, bounds: BoundingBox) -> Self {
        if matches!(element.overflow, ChildOverflow::Clipped) {
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

    fn child_position(&self, origin: Vec2, reverse_cursor: f32, child_size: Vec2) -> Vec2 {
        let parent = self.parent;
        let base = origin + self.content_box.origin - self.scroll_offset;
        match self.flow {
            ChildFlow::Row { .. } => {
                let cross_offset = match parent.child_layout.align_y() {
                    AlignY::Top => 0.0,
                    AlignY::Center => (self.content_box.size.y - child_size.y).max(0.0) * 0.5,
                    AlignY::Bottom => (self.content_box.size.y - child_size.y).max(0.0),
                };
                Vec2::new(base.x + reverse_cursor, base.y + cross_offset)
            },
            ChildFlow::Column { .. } => {
                let cross_offset = match parent.child_layout.align_x() {
                    AlignX::Left => 0.0,
                    AlignX::Center => (self.content_box.size.x - child_size.x).max(0.0) * 0.5,
                    AlignX::Right => (self.content_box.size.x - child_size.x).max(0.0),
                };
                Vec2::new(base.x + cross_offset, base.y + reverse_cursor)
            },
            ChildFlow::Overlay => {
                let x_offset = match parent.child_layout.align_x() {
                    AlignX::Left => 0.0,
                    AlignX::Center => (self.content_box.size.x - child_size.x).max(0.0) * 0.5,
                    AlignX::Right => (self.content_box.size.x - child_size.x).max(0.0),
                };
                let y_offset = match parent.child_layout.align_y() {
                    AlignY::Top => 0.0,
                    AlignY::Center => (self.content_box.size.y - child_size.y).max(0.0) * 0.5,
                    AlignY::Bottom => (self.content_box.size.y - child_size.y).max(0.0),
                };
                Vec2::new(base.x + x_offset, base.y + y_offset)
            },
        }
    }
}

/// Emits border and scissor-end commands during the up-traversal (second visit)
/// of the DFS positioning pass.
fn emit_up_traversal_commands(
    tree: &LayoutTree,
    computed: &[ComputedLayout],
    commands: &mut Vec<RenderCommand>,
    element: &Element,
    bounds: BoundingBox,
    index: usize,
) {
    if let Some(ref border) = element.border {
        push_command(
            commands,
            bounds,
            RenderCommandKind::Border { border: *border },
            index,
            element.z_index,
        );
    }

    if let Some(divider) = element.child_layout.divider()
        && divider.width().value > 0.0
    {
        emit_child_dividers(tree, computed, commands, index, divider);
    }

    if matches!(element.overflow, ChildOverflow::Clipped) {
        push_command(
            commands,
            bounds,
            RenderCommandKind::ScissorEnd,
            index,
            element.z_index,
        );
    }
}

fn needs_up_traversal(element: &Element) -> bool {
    // Keep this predicate in sync with `emit_up_traversal_commands`: skipping a
    // required second visit can drop borders/dividers or unbalance scissors.
    element.border.is_some()
        || element
            .child_layout
            .divider()
            .is_some_and(|divider| divider.width().value > 0.0)
        || matches!(element.overflow, ChildOverflow::Clipped)
}

/// Emits background, scissor-start, and text render commands during the
/// down-traversal (first visit) of the DFS positioning pass.
fn emit_down_traversal_commands(
    commands: &mut Vec<RenderCommand>,
    element: &Element,
    wrapped: Option<&WrappedText>,
    bounds: BoundingBox,
    index: usize,
    font_scale: f32,
    clip_context: ClipContext,
) {
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
    if matches!(element.overflow, ChildOverflow::Clipped) {
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
            bounds,
            index,
            font_scale,
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
    bounds: BoundingBox,
    index: usize,
    font_scale: f32,
    z_index: DrawZIndex,
) {
    // Render commands store font sizes in layout units so downstream
    // renderers don't need to know about the font unit conversion.
    // `TextStyle::scaled(1.0)` is intentionally an identity transform; avoid
    // the multiply path in per-text command emission after trees are pre-scaled.
    let scaled_config = if font_scale.to_bits() == 1.0_f32.to_bits() {
        config.clone()
    } else {
        config.scaled(font_scale)
    };

    if let Some(wrap_result) = wrapped {
        // Wrapped text: emit one command per line.
        for (line_idx, line) in wrap_result.lines.iter().enumerate() {
            let line_y = wrap_result.line_height.mul_add(line_idx.to_f32(), bounds.y);
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
        // Unwrapped text (`TextWrap::None`): single command.
        push_command(
            commands,
            bounds,
            RenderCommandKind::Text {
                text:   text.to_owned(),
                config: scaled_config,
            },
            index,
            z_index,
        );
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

/// Resolves the clamped per-axis scroll offset for a scrolling parent, returning
/// `Vec2::ZERO` for the common non-scrolling case. Each axis clamps to
/// `[0, content - viewport]`; `End`-anchored axes measure the offset from the
/// far edge so `0` pins to the bottom/right.
fn resolve_scroll_offset(parent_el: &Element, max_scroll: Vec2) -> Vec2 {
    // A zero `Start` offset is a no-op; a zero `End` offset still resolves
    // (scrollback 0 pins to the end), so don't short-circuit it.
    if parent_el.scroll_offset == Vec2::ZERO
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
        resolve(
            parent_el.scroll_offset.x,
            max_scroll.x,
            parent_el.scroll_anchor_x,
        ),
        resolve(
            parent_el.scroll_offset.y,
            max_scroll.y,
            parent_el.scroll_anchor_y,
        ),
    )
}

fn child_stack_context<'a>(
    parent: &'a Element,
    computed: &[ComputedLayout],
    children: &[usize],
    parent_size: Vec2,
    clip_context: ClipContext,
) -> ChildStackContext<'a> {
    let content_box = sizing::content_box(parent, parent_size);
    let child_content_size = children_content_size(&parent.child_layout, computed, children);
    let max_scroll = Vec2::new(
        (child_content_size.x - content_box.size.x).max(0.0),
        (child_content_size.y - content_box.size.y).max(0.0),
    );
    let scroll_offset = resolve_scroll_offset(parent, max_scroll);

    let flow = match parent.child_layout {
        ChildLayout::Row { .. } => {
            let extra_main = (content_box.size.x - child_content_size.x).max(0.0);
            let main_offset = match parent.child_layout.align_x() {
                AlignX::Left => 0.0,
                AlignX::Center => extra_main * 0.5,
                AlignX::Right => extra_main,
            };
            ChildFlow::Row {
                reverse_cursor_start: main_offset + child_content_size.x,
            }
        },
        ChildLayout::Column { .. } => {
            let extra_main = (content_box.size.y - child_content_size.y).max(0.0);
            let main_offset = match parent.child_layout.align_y() {
                AlignY::Top => 0.0,
                AlignY::Center => extra_main * 0.5,
                AlignY::Bottom => extra_main,
            };
            ChildFlow::Column {
                reverse_cursor_start: main_offset + child_content_size.y,
            }
        },
        ChildLayout::Overlay { .. } => ChildFlow::Overlay,
    };

    ChildStackContext {
        parent,
        content_box,
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

fn main_gap_total(child_layout: &ChildLayout, child_count: usize) -> f32 {
    if child_count > 1 {
        child_layout
            .main_gap()
            .map_or(0.0, |gap| gap.value * (child_count - 1).to_f32())
    } else {
        0.0
    }
}

/// Pushes children onto the DFS stack in reverse order with computed positions.
///
/// Children are pushed in reverse so the first child is processed first during
/// iteration. Uses a reverse cursor to compute positions without allocation.
fn push_children_to_stack(
    tree: &LayoutTree,
    computed: &[ComputedLayout],
    stack: &mut Vec<PositionStackEntry>,
    index: usize,
    x: f32,
    y: f32,
    clip_context: ClipContext,
) {
    let children = tree.children_of(index);
    if children.is_empty() {
        return;
    }

    let parent_el = &tree.elements[index];
    let child_context = child_stack_context(
        parent_el,
        computed,
        children,
        Vec2::new(computed[index].width, computed[index].height),
        clip_context,
    );

    let origin = Vec2::new(x, y);
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
                index:        child_idx,
                x:            child_position.x,
                y:            child_position.y,
                visited:      false,
                clip_context: child_context.clip_context,
            });
            reverse_cursor -= gap;
        }
    } else {
        for &child_idx in children.iter().rev() {
            let child_size = Vec2::new(computed[child_idx].width, computed[child_idx].height);
            let child_position = child_context.child_position(origin, 0.0, child_size);

            stack.push(PositionStackEntry {
                index:        child_idx,
                x:            child_position.x,
                y:            child_position.y,
                visited:      false,
                clip_context: child_context.clip_context,
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
) -> Vec<RenderCommand> {
    let mut commands = Vec::with_capacity(tree.len() * 2);
    let root_clip = root_box_clip(computed[root].width, computed[root].height);

    let mut stack = Vec::with_capacity(tree.len());
    stack.push(PositionStackEntry {
        index:        root,
        x:            0.0,
        y:            0.0,
        visited:      false,
        clip_context: ClipContext::root(root_clip),
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
            emit_up_traversal_commands(tree, computed, &mut commands, element, bounds, index);
        } else {
            // Store the final bounding box for render-side culling and clipping.
            computed[index].bounds = bounds;

            if matches!(element.precompose, PrecomposeMode::Ldr) {
                emit_precompose_command(&mut commands, bounds, index, element.z_index);
            } else {
                emit_down_traversal_commands(
                    &mut commands,
                    element,
                    wrapped.get(index).and_then(Option::as_ref),
                    bounds,
                    index,
                    font_scale,
                    entry.clip_context,
                );

                let child_clip = entry.clip_context.child(element, bounds);
                if needs_up_traversal(element) {
                    stack.push(PositionStackEntry {
                        visited: true,
                        ..entry
                    });
                }
                push_children_to_stack(
                    tree, computed, &mut stack, index, entry.x, entry.y, child_clip,
                );
            }
        }
    }
    commands
}

/// Generates render commands from already-computed element bounds.
pub(super) fn render_commands_from_geometry(
    tree: &LayoutTree,
    computed: &[ComputedLayout],
    root: usize,
    wrapped: &[Option<WrappedText>],
    font_scale: f32,
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
            emit_up_traversal_commands(tree, computed, &mut commands, element, bounds, index);
            continue;
        }

        if matches!(element.precompose, PrecomposeMode::Ldr) {
            emit_precompose_command(&mut commands, bounds, index, element.z_index);
        } else {
            emit_down_traversal_commands(
                &mut commands,
                element,
                wrapped.get(index).and_then(Option::as_ref),
                bounds,
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
