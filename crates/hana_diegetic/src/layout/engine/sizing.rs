use bevy::math::Vec2;
use bevy_kana::ToF32;

use super::layout_engine::ComputedLayout;
use crate::layout::BoundingBox;
use crate::layout::Sizing;
use crate::layout::TextSizing;
use crate::layout::TextWrap;
use crate::layout::child_layout::AxisRole;
use crate::layout::child_layout::ChildLayout;
use crate::layout::constants::LAYOUT_EPSILON;
use crate::layout::element::ChildOverflow;
use crate::layout::element::Element;
use crate::layout::element::ElementContent;
use crate::layout::element::LayoutTree;

/// Selects which layout axis a sizing or positioning operation targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Axis {
    X,
    Y,
}

/// Parent content box after padding and border are removed.
#[derive(Clone, Copy, Debug)]
pub(super) struct ContentBox {
    pub(super) origin: Vec2,
    pub(super) size:   Vec2,
}

impl ContentBox {
    pub(super) const fn axis_size(self, axis: Axis) -> f32 {
        match axis {
            Axis::X => self.size.x,
            Axis::Y => self.size.y,
        }
    }
}

/// Returns the total border inset for `element` along `axis`.
pub(super) fn border_inset(element: &Element, axis: Axis) -> f32 {
    element.border.as_ref().map_or(0.0, |b| match axis {
        Axis::X => b.horizontal(),
        Axis::Y => b.vertical(),
    })
}

/// Returns the leading border width (left for X, top for Y).
pub(super) fn border_leading(element: &Element, axis: Axis) -> f32 {
    element.border.as_ref().map_or(0.0, |b| match axis {
        Axis::X => b.left.value,
        Axis::Y => b.top.value,
    })
}

/// Returns `element`'s solved content box in the same coordinate space as
/// `border_box`, excluding its padding and border.
///
/// `border_box` is the element's solved border box (its outer bounds). This
/// reuses the [`content_box`] formula so widget reify records the exact
/// padding/border-excluded interior the layout engine positions children in.
pub(crate) fn content_box_bounds(element: &Element, border_box: BoundingBox) -> BoundingBox {
    let content = content_box(element, Vec2::new(border_box.width, border_box.height));
    BoundingBox {
        x:      border_box.x + content.origin.x,
        y:      border_box.y + content.origin.y,
        width:  content.size.x,
        height: content.size.y,
    }
}

/// Returns the parent content box for a resolved element size.
pub(super) fn content_box(element: &Element, parent_size: Vec2) -> ContentBox {
    let border_left = border_leading(element, Axis::X);
    let border_top = border_leading(element, Axis::Y);
    ContentBox {
        origin: Vec2::new(
            element.padding.left.value + border_left,
            element.padding.top.value + border_top,
        ),
        size:   Vec2::new(
            (parent_size.x - element.padding.horizontal() - border_inset(element, Axis::X))
                .max(0.0),
            (parent_size.y - element.padding.vertical() - border_inset(element, Axis::Y)).max(0.0),
        ),
    }
}

/// Bottom-up pass: set `Fit` container sizes and propagate `minDimensions`.
///
/// This runs before the BFS so that when a parent processes its children,
/// `Fit` containers already have a content-based initial size and every element
/// has its `min_width`/`min_height` floor computed.
///
/// Returns the content size of the element so that parent `Fit` elements can
/// account for it — even if this element is `Grow` (whose actual size is
/// determined later by `size_along_axis`). Without this, a `Fit` parent with
/// `Grow` children would see 0 and compute a collapsed height.
pub(super) fn propagate_fit_sizes(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    index: usize,
    axis: Axis,
) -> f32 {
    let element = &tree.elements[index];
    let children = tree.children_of(index);
    let sizing = get_sizing(element, axis);

    // Leaf node: set `minDimensions` from the element's content.
    //
    // For text elements, Clay sets `minDimensions = { minWidth, height }`:
    // - height: measured height — text can't be compressed vertically.
    // - width: widest single word for `TextWrap::Words` (stored in
    //   `ComputedLayout::min_text_width`) — wrapped text compresses to its longest unbreakable word
    //   and no further. Zero for other wrap modes, leaving the sizing floor in charge.
    if children.is_empty() {
        let current_size = get_size(computed[index], axis);
        let content_min = match axis {
            Axis::X => computed[index].min_text_width,
            // Text min height = measured height (matches Clay line 2003).
            Axis::Y if matches!(element.content, ElementContent::Text { .. }) => current_size,
            Axis::Y => 0.0,
        };
        let leaf_min = content_min.clamp(sizing.min_size(), sizing.max_size());
        set_min_size(&mut computed[index], axis, leaf_min);
        // Floor a childless leaf to its `min`: a `grow_min`/`fit_min` leaf has no
        // content of its own, so its `min` is the size it must contribute to a
        // `Fit` ancestor. Returning `current_size` alone (0 for such a leaf) lets
        // the floor reach only `min_acc`, which a `Fit` parent ignores unless it
        // is already narrower than the floor — so the reserved space vanishes.
        let sized = current_size.max(leaf_min);
        set_size(&mut computed[index], axis, sized);
        return sized;
    }

    let axis_role = child_axis_role(&element.child_layout, axis);

    // Recurse into children first (post-order), accumulating sizes inline
    // to avoid per-call Vec allocation.
    let mut content_acc: f32 = 0.0;
    let mut min_acc: f32 = 0.0;
    for &child_idx in children {
        let child_size = propagate_fit_sizes(tree, computed, child_idx, axis);
        let child_min = get_min_size(computed[child_idx], axis);
        if matches!(axis_role, AxisRole::RowMain | AxisRole::ColumnMain) {
            content_acc += child_size;
            min_acc += child_min;
        } else {
            content_acc = content_acc.max(child_size);
            min_acc = min_acc.max(child_min);
        }
    }

    // Fixed elements already have their size — but still compute minDimensions.
    if let Sizing::Fixed(size) = sizing {
        let min = 0.0_f32.clamp(sizing.min_size(), sizing.max_size());
        set_min_size(&mut computed[index], axis, min);
        return size.value;
    }

    let padding = match axis {
        Axis::X => element.padding.horizontal(),
        Axis::Y => element.padding.vertical(),
    };
    let border = border_inset(element, axis);

    let gap_total =
        if matches!(axis_role, AxisRole::RowMain | AxisRole::ColumnMain) && children.len() > 1 {
            element
                .child_layout
                .main_gap()
                .map_or(0.0, |gap| gap.value * (children.len() - 1).to_f32())
        } else {
            0.0
        };

    let chrome = padding + border;

    // A clipping container does not inflate to contain its children — they
    // overflow (and may scroll), so it reports only its own chrome to ancestors.
    // Without this, a scrollable container's min size equals its full content,
    // forcing every ancestor to grow to the content instead of clipping it.
    let clipped = matches!(element.overflow, ChildOverflow::Clipped);
    let content_acc = if clipped { 0.0 } else { content_acc };
    let min_acc = if clipped { 0.0 } else { min_acc };
    let gap_for_size = if clipped || matches!(axis_role, AxisRole::Cross | AxisRole::Overlay) {
        0.0
    } else {
        gap_total
    };

    let content_size = content_acc + chrome + gap_for_size;
    let min_from_children = min_acc + chrome + gap_for_size;

    // Clamp minDimensions to [sizing.min, sizing.max] — matches Clay.
    let clamped_min = min_from_children.clamp(sizing.min_size(), sizing.max_size());
    set_min_size(&mut computed[index], axis, clamped_min);

    // Fit elements: set their computed size now.
    if sizing.is_fit() {
        let clamped = content_size.clamp(sizing.min_size(), sizing.max_size());
        set_size(&mut computed[index], axis, clamped);
        return clamped;
    }

    // Grow elements: set initial computed size from content, clamped to [min, max].
    // This matches Clay's `CloseElement` which initializes dimensions for all element
    // types from their children before `SizeContainersAlongAxis`. Without this, GROW
    // elements start at 0, masking overflow and preventing compression from triggering.
    if sizing.is_grow() {
        let clamped = content_size.clamp(sizing.min_size(), sizing.max_size());
        set_size(&mut computed[index], axis, clamped);
        return clamped;
    }

    // Percent elements: return content size so ancestor Fit elements
    // can account for it, but don't set the computed size yet.
    content_size
}

/// Bottom-up initial pass for both layout axes.
///
/// X and Y fit propagation are independent before wrapping, so the initial
/// solve can compute both axes in one post-order traversal. The single-axis
/// pass remains for the wrap-corrected Y-only propagation after text reflow.
pub(super) fn propagate_fit_sizes_xy(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    index: usize,
) -> Vec2 {
    let element = &tree.elements[index];
    let children = tree.children_of(index);
    let width_sizing = get_sizing(element, Axis::X);
    let height_sizing = get_sizing(element, Axis::Y);

    if children.is_empty() {
        let current_width = computed[index].width;
        let current_height = computed[index].height;
        // Widest single word for `TextWrap::Words` text, zero otherwise —
        // see the leaf `minDimensions` comment in `propagate_fit_sizes`.
        let min_width = computed[index]
            .min_text_width
            .clamp(width_sizing.min_size(), width_sizing.max_size());
        let min_height = if matches!(element.content, ElementContent::Text { .. }) {
            current_height.clamp(height_sizing.min_size(), height_sizing.max_size())
        } else {
            0.0_f32.clamp(height_sizing.min_size(), height_sizing.max_size())
        };
        computed[index].min_width = min_width;
        computed[index].min_height = min_height;
        // Floor a childless leaf to its `min` so a `grow_min`/`fit_min` leaf
        // reserves space in a `Fit` ancestor (see `propagate_fit_sizes`).
        let sized_width = current_width.max(min_width);
        let sized_height = current_height.max(min_height);
        computed[index].width = sized_width;
        computed[index].height = sized_height;
        return Vec2::new(sized_width, sized_height);
    }

    let x_axis_role = child_axis_role(&element.child_layout, Axis::X);
    let y_axis_role = child_axis_role(&element.child_layout, Axis::Y);
    let mut content_x = 0.0_f32;
    let mut content_y = 0.0_f32;
    let mut min_x = 0.0_f32;
    let mut min_y = 0.0_f32;

    for &child_idx in children {
        let child_size = propagate_fit_sizes_xy(tree, computed, child_idx);
        let child_min_x = computed[child_idx].min_width;
        let child_min_y = computed[child_idx].min_height;

        if matches!(x_axis_role, AxisRole::RowMain | AxisRole::ColumnMain) {
            content_x += child_size.x;
            min_x += child_min_x;
        } else {
            content_x = content_x.max(child_size.x);
            min_x = min_x.max(child_min_x);
        }

        if matches!(y_axis_role, AxisRole::RowMain | AxisRole::ColumnMain) {
            content_y += child_size.y;
            min_y += child_min_y;
        } else {
            content_y = content_y.max(child_size.y);
            min_y = min_y.max(child_min_y);
        }
    }

    let width = finish_propagated_axis(
        element,
        &mut computed[index],
        Axis::X,
        children.len(),
        content_x,
        min_x,
    );
    let height = finish_propagated_axis(
        element,
        &mut computed[index],
        Axis::Y,
        children.len(),
        content_y,
        min_y,
    );

    Vec2::new(width, height)
}

fn finish_propagated_axis(
    element: &Element,
    computed: &mut ComputedLayout,
    axis: Axis,
    child_count: usize,
    content_acc: f32,
    min_acc: f32,
) -> f32 {
    let sizing = get_sizing(element, axis);
    let axis_role = child_axis_role(&element.child_layout, axis);

    if let Sizing::Fixed(size) = sizing {
        let min = 0.0_f32.clamp(sizing.min_size(), sizing.max_size());
        set_min_size(computed, axis, min);
        return size.value;
    }

    let padding = match axis {
        Axis::X => element.padding.horizontal(),
        Axis::Y => element.padding.vertical(),
    };
    let border = border_inset(element, axis);

    let gap_total =
        if matches!(axis_role, AxisRole::RowMain | AxisRole::ColumnMain) && child_count > 1 {
            element
                .child_layout
                .main_gap()
                .map_or(0.0, |gap| gap.value * (child_count - 1).to_f32())
        } else {
            0.0
        };

    let chrome = padding + border;
    let clipped = matches!(element.overflow, ChildOverflow::Clipped);
    let content_acc = if clipped { 0.0 } else { content_acc };
    let min_acc = if clipped { 0.0 } else { min_acc };
    let gap_for_size = if clipped || matches!(axis_role, AxisRole::Cross | AxisRole::Overlay) {
        0.0
    } else {
        gap_total
    };

    let content_size = content_acc + chrome + gap_for_size;
    let min_from_children = min_acc + chrome + gap_for_size;
    let clamped_min = min_from_children.clamp(sizing.min_size(), sizing.max_size());
    set_min_size(computed, axis, clamped_min);

    if sizing.is_fit() {
        let clamped = content_size.clamp(sizing.min_size(), sizing.max_size());
        set_size(computed, axis, clamped);
        return clamped;
    }

    if sizing.is_grow() {
        let clamped = content_size.clamp(sizing.min_size(), sizing.max_size());
        set_size(computed, axis, clamped);
        return clamped;
    }

    content_size
}

/// BFS sizing pass along one axis.
///
/// When `axis` is `Axis::X`, sizes widths; when `Axis::Y`, sizes heights.
pub(super) fn size_along_axis(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    root: usize,
    axis: Axis,
    viewport_size: f32,
) {
    // Set root size from the viewport.
    // - Fixed roots keep their declared size (already set by initialize_leaf_sizes).
    // - Grow/Fit roots fill the viewport — this is re-applied unconditionally because
    //   propagate_fit_sizes may have set them to their content size, but the root should match the
    //   viewport, not its content.
    let root_element = &tree.elements[root];
    let new_root_size = match get_sizing(root_element, axis) {
        Sizing::Grow { min, max } => viewport_size.clamp(min.value, max.value),
        Sizing::Fit { min, max } => {
            let content = get_size(computed[root], axis);
            if content > 0.0 {
                content.clamp(min.value, max.value)
            } else {
                viewport_size.clamp(min.value, max.value)
            }
        },
        Sizing::Fixed(size) => size.value,
        Sizing::Percent(frac) => viewport_size * frac,
    };
    set_size(&mut computed[root], axis, new_root_size);

    // Top-down traversal using a stack (parents always processed before children).
    let mut queue = Vec::with_capacity(tree.len());
    queue.push(root);

    while let Some(parent_idx) = queue.pop() {
        let children = tree.children_of(parent_idx);
        if children.is_empty() {
            continue;
        }

        let parent_element = &tree.elements[parent_idx];
        let parent_size = get_size(computed[parent_idx], axis);
        let axis_role = child_axis_role(&parent_element.child_layout, axis);
        let parent_content_box = content_box(
            parent_element,
            Vec2::new(computed[parent_idx].width, computed[parent_idx].height),
        );
        let parent_content_size = parent_content_box.axis_size(axis);

        let padding = match axis {
            Axis::X => parent_element.padding.horizontal(),
            Axis::Y => parent_element.padding.vertical(),
        };
        let border = border_inset(parent_element, axis);

        let gap_total = if matches!(axis_role, AxisRole::RowMain | AxisRole::ColumnMain)
            && children.len() > 1
        {
            parent_element
                .child_layout
                .main_gap()
                .map_or(0.0, |gap| gap.value * (children.len() - 1).to_f32())
        } else {
            0.0
        };

        // Resolve Percent children first for row/column main-axis distribution.
        let available_for_percent = parent_content_size - gap_total;
        if matches!(axis_role, AxisRole::RowMain | AxisRole::ColumnMain) {
            for &child_idx in children {
                let child_sizing = get_sizing(&tree.elements[child_idx], axis);
                if let Sizing::Percent(frac) = child_sizing {
                    let size = (available_for_percent * frac).max(0.0);
                    set_size(&mut computed[child_idx], axis, size);
                }
            }
        }

        match axis_role {
            AxisRole::RowMain | AxisRole::ColumnMain => {
                size_children_along_axis(
                    tree,
                    computed,
                    parent_idx,
                    children,
                    axis,
                    AxisMetrics {
                        parent_size,
                        padding,
                        border,
                        gap_total,
                    },
                );
            },
            AxisRole::Cross => {
                size_children_cross_axis(
                    tree,
                    computed,
                    children,
                    axis,
                    parent_size,
                    parent_content_size,
                );
            },
            AxisRole::Overlay => {
                size_overlay_children(tree, computed, children, axis, parent_content_size);
            },
        }

        // Enqueue children (reverse order so first child is popped first from stack).
        for &child_idx in children.iter().rev() {
            if !tree.children_of(child_idx).is_empty() {
                queue.push(child_idx);
            }
        }
    }
}

/// Size overlay children independently against the parent's content box.
fn size_overlay_children(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    children: &[usize],
    axis: Axis,
    content_size: f32,
) {
    for &child_idx in children {
        let child_element = &tree.elements[child_idx];
        let child_sizing = get_sizing(child_element, axis);
        let current = get_size(computed[child_idx], axis);
        let min_dim = get_min_size(computed[child_idx], axis);

        let new_size = match child_sizing {
            Sizing::Grow { min, max } => content_size.clamp(min.value, max.value),
            Sizing::Fit { min, max } => {
                if current > f32::EPSILON {
                    current.clamp(min.value, max.value)
                } else {
                    min.value
                }
            },
            Sizing::Fixed(size) => size.value,
            Sizing::Percent(frac) => (content_size * frac).max(0.0),
        };

        set_size(&mut computed[child_idx], axis, new_size.max(min_dim));
    }
}

/// Parent-axis size budget: the parent's full size along the axis and the
/// three deductions (padding, border, gap totals) that reduce what children
/// can consume.
struct AxisMetrics {
    parent_size: f32,
    padding:     f32,
    border:      f32,
    gap_total:   f32,
}

/// Size children that are laid out ALONG the parent's layout axis.
fn size_children_along_axis(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    parent_idx: usize,
    children: &[usize],
    axis: Axis,
    metrics: AxisMetrics,
) {
    let parent_element = &tree.elements[parent_idx];

    // Sum current child sizes, count grow children.
    let mut content_size: f32 = 0.0;
    let mut grow_count = 0_u32;
    for &child_idx in children {
        let child_sizing = get_sizing(&tree.elements[child_idx], axis);
        let child_size = get_size(computed[child_idx], axis);
        content_size += child_size;
        if child_sizing.is_grow() {
            grow_count += 1;
        }
    }

    let available = metrics.parent_size - metrics.padding - metrics.border - metrics.gap_total;
    let mut to_distribute = available - content_size;

    // Overflow compression: largest-first heuristic.
    if to_distribute < 0.0 && matches!(parent_element.overflow, ChildOverflow::Visible) {
        compress_children(tree, computed, children, axis, &mut to_distribute);
    }

    // Growth expansion: smallest-first heuristic.
    if to_distribute > 0.0 && grow_count > 0 {
        expand_children(tree, computed, children, axis, &mut to_distribute);
    }
}

/// Size children that are laid out ACROSS (perpendicular to) the parent's layout axis.
///
/// Applies `MAX(minDimensions, MIN(childSize, maxSize))` -- Clay's cross-axis rule
/// that prevents children from shrinking below their propagated content minimum.
fn size_children_cross_axis(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    children: &[usize],
    axis: Axis,
    parent_size: f32,
    content_size: f32,
) {
    for &child_idx in children {
        let child_element = &tree.elements[child_idx];
        let child_sizing = get_sizing(child_element, axis);
        let current = get_size(computed[child_idx], axis);
        let min_dim = get_min_size(computed[child_idx], axis);

        let new_size = match child_sizing {
            Sizing::Grow { min, max } => content_size.clamp(min.value, max.value),
            Sizing::Fit { min, max } => {
                // Fit elements keep their propagated content size, clamped to
                // the parent's content size when Clay treats them as resizable
                // (clay.h `Clay__SizeContainersAlongAxis` off-axis branch).
                // Without the parent clamp, a Fit chain inside a bounded
                // ancestor keeps its natural width and wrapped text inside it
                // never reflows.
                if current > f32::EPSILON {
                    let clamped = current.clamp(min.value, max.value);
                    if cross_axis_resizable(child_element) {
                        clamped.min(content_size)
                    } else {
                        clamped
                    }
                } else {
                    min.value
                }
            },
            Sizing::Fixed(size) => size.value,
            Sizing::Percent(frac) => (parent_size * frac).max(0.0),
        };

        // Apply minDimensions floor: MAX(minDimensions, MIN(childSize, maxSize)).
        let floored = new_size.max(min_dim);
        set_size(&mut computed[child_idx], axis, floored);
    }
}

/// Returns whether Clay's off-axis sizing clamps this element to the parent's
/// content size.
///
/// Clay includes an element in its resizable buffer unless it is a text
/// element whose wrap mode is not `CLAY_TEXT_WRAP_WORDS` — text that cannot
/// reflow keeps its natural box and overflows instead.
const fn cross_axis_resizable(element: &Element) -> bool {
    match &element.content {
        ElementContent::Text { sizing, .. } => matches!(
            sizing,
            TextSizing::Natural {
                wrap: TextWrap::Words,
            }
        ),
        _ => true,
    }
}

/// Compresses children using the largest-first heuristic.
///
/// Iterates `children` directly each pass to avoid per-call Vec allocations.
fn compress_children(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    children: &[usize],
    axis: Axis,
    to_distribute: &mut f32,
) {
    loop {
        if *to_distribute >= -LAYOUT_EPSILON {
            break;
        }

        // Single pass: find largest, second-largest, and count at largest
        // among resizable children still above their minimum.
        let mut largest = f32::NEG_INFINITY;
        let mut second_largest = f32::NEG_INFINITY;
        let mut at_largest_count = 0_u32;

        for &idx in children {
            if !get_sizing(&tree.elements[idx], axis).is_resizable() {
                continue;
            }
            let size = get_size(computed[idx], axis);
            let min = get_min_size(computed[idx], axis);
            if size <= min + LAYOUT_EPSILON {
                continue;
            }
            if size > largest + LAYOUT_EPSILON {
                second_largest = largest;
                largest = size;
                at_largest_count = 1;
            } else if (size - largest).abs() <= LAYOUT_EPSILON {
                at_largest_count += 1;
            } else if size > second_largest {
                second_largest = size;
            }
        }

        if at_largest_count == 0 || largest <= LAYOUT_EPSILON {
            break;
        }

        let count = at_largest_count.to_f32();
        let delta_even = (-*to_distribute) / count;

        // If all at same size (no second largest), just distribute evenly.
        let shrink_per_child = if second_largest > f32::NEG_INFINITY {
            let delta_to_second = largest - second_largest;
            delta_to_second.min(delta_even)
        } else {
            delta_even
        };

        if shrink_per_child <= LAYOUT_EPSILON {
            break;
        }

        // Apply shrink to resizable children at the largest size.
        let mut total_shrink = 0.0_f32;
        for &idx in children {
            if !get_sizing(&tree.elements[idx], axis).is_resizable() {
                continue;
            }
            let current = get_size(computed[idx], axis);
            if (current - largest).abs() > LAYOUT_EPSILON {
                continue;
            }
            let min = get_min_size(computed[idx], axis);
            let new_size = (current - shrink_per_child).max(min);
            let actual_shrink = current - new_size;
            set_size(&mut computed[idx], axis, new_size);
            *to_distribute += actual_shrink;
            total_shrink += actual_shrink;
        }

        if total_shrink <= LAYOUT_EPSILON {
            break;
        }
    }
}

/// Expands `Grow` children using the smallest-first heuristic.
///
/// Iterates `children` directly each pass to avoid per-call Vec allocations.
fn expand_children(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    children: &[usize],
    axis: Axis,
    to_distribute: &mut f32,
) {
    loop {
        if *to_distribute <= LAYOUT_EPSILON {
            break;
        }

        // Single pass: find smallest, second-smallest, and count at smallest
        // among growable children still below their maximum.
        let mut smallest = f32::INFINITY;
        let mut second_smallest = f32::INFINITY;
        let mut at_smallest_count = 0_u32;

        for &idx in children {
            if !get_sizing(&tree.elements[idx], axis).is_grow() {
                continue;
            }
            let size = get_size(computed[idx], axis);
            let max = get_sizing(&tree.elements[idx], axis).max_size();
            if size >= max - LAYOUT_EPSILON {
                continue;
            }
            if size < smallest - LAYOUT_EPSILON {
                second_smallest = smallest;
                smallest = size;
                at_smallest_count = 1;
            } else if (size - smallest).abs() <= LAYOUT_EPSILON {
                at_smallest_count += 1;
            } else if size < second_smallest {
                second_smallest = size;
            }
        }

        if at_smallest_count == 0 {
            break;
        }

        let count = at_smallest_count.to_f32();
        let delta_even = *to_distribute / count;

        // If all at same size (no second smallest), just distribute evenly.
        let grow_per_child = if second_smallest < f32::INFINITY {
            let delta_to_second = second_smallest - smallest;
            delta_to_second.min(delta_even)
        } else {
            delta_even
        };

        if grow_per_child <= LAYOUT_EPSILON {
            break;
        }

        // Apply growth to growable children at the smallest size.
        let mut total_grow = 0.0_f32;
        for &idx in children {
            if !get_sizing(&tree.elements[idx], axis).is_grow() {
                continue;
            }
            let current = get_size(computed[idx], axis);
            if (current - smallest).abs() > LAYOUT_EPSILON {
                continue;
            }
            let max = get_sizing(&tree.elements[idx], axis).max_size();
            let new_size = (current + grow_per_child).min(max);
            let actual_grow = new_size - current;
            set_size(&mut computed[idx], axis, new_size);
            *to_distribute -= actual_grow;
            total_grow += actual_grow;
        }

        if total_grow <= LAYOUT_EPSILON {
            break;
        }
    }
}

/// Returns the sizing rule for the given element along the specified axis.
const fn get_sizing(element: &Element, axis: Axis) -> Sizing {
    match axis {
        Axis::X => element.width,
        Axis::Y => element.height,
    }
}

/// Returns the computed size for the given element along the specified axis.
const fn get_size(computed: ComputedLayout, axis: Axis) -> f32 {
    match axis {
        Axis::X => computed.width,
        Axis::Y => computed.height,
    }
}

/// Sets the computed size for the given element along the specified axis.
const fn set_size(computed: &mut ComputedLayout, axis: Axis, value: f32) {
    match axis {
        Axis::X => computed.width = value,
        Axis::Y => computed.height = value,
    }
}

/// Returns the propagated minimum content size along the specified axis.
const fn get_min_size(computed: ComputedLayout, axis: Axis) -> f32 {
    match axis {
        Axis::X => computed.min_width,
        Axis::Y => computed.min_height,
    }
}

/// Sets the propagated minimum content size along the specified axis.
const fn set_min_size(computed: &mut ComputedLayout, axis: Axis, value: f32) {
    match axis {
        Axis::X => computed.min_width = value,
        Axis::Y => computed.min_height = value,
    }
}

/// Returns how `child_layout` treats `axis`.
pub(super) const fn child_axis_role(child_layout: &ChildLayout, axis: Axis) -> AxisRole {
    match axis {
        Axis::X => child_layout.x_axis_role(),
        Axis::Y => child_layout.y_axis_role(),
    }
}
