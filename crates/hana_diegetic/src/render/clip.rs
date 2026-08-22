//! Clip rect computation from layout render commands.
//!
//! Walks a `&[RenderCommand]` slice once, maintaining a stack of nested
//! clip regions from [`ScissorStart`]/[`ScissorEnd`] pairs. Returns a
//! parallel `Vec` where each entry is the active clip rect (or `None`
//! if the command is outside any clip region).

use crate::layout::BoundingBox;
use crate::layout::RenderCommand;
use crate::layout::RenderCommandKind;
use crate::panel::DiegeticPanel;

/// Returns the panel's layout-space viewport.
pub(super) const fn panel_viewport(panel: &DiegeticPanel) -> BoundingBox {
    let layout_to_points = panel.layout_unit().to_points();
    BoundingBox {
        x:      0.0,
        y:      0.0,
        width:  panel.width() * layout_to_points,
        height: panel.height() * layout_to_points,
    }
}

/// Returns the visible clip rect for a command after applying both the
/// panel viewport and the active scissor clip.
///
/// Returns `None` when the command is fully outside the effective clip.
pub(super) fn effective_clip(
    command_bounds: BoundingBox,
    scissor_clip: Option<BoundingBox>,
    viewport: BoundingBox,
) -> Option<BoundingBox> {
    let clip = match scissor_clip {
        Some(scissor) => scissor.intersect(&viewport)?,
        None => viewport,
    };
    command_bounds.intersect(&clip).map(|_| clip)
}

/// A zero-area clip rect used for retained children that exist in layout
/// but are fully outside the current viewport.
pub(super) const fn empty_clip() -> BoundingBox {
    BoundingBox {
        x:      0.0,
        y:      0.0,
        width:  0.0,
        height: 0.0,
    }
}

/// Computes the active clip rect for each render command.
///
/// Returns a `Vec` parallel to `commands` where each entry is the
/// active clip rect at that command's position in the stream, or
/// `None` if the command is not inside any `ScissorStart` region.
///
/// Nested clips are intersected — a child clip region is the overlap
/// of its bounds with the parent's active clip.
pub(super) fn compute_clip_rects(commands: &[RenderCommand]) -> Vec<Option<BoundingBox>> {
    let mut result = Vec::with_capacity(commands.len());
    let mut stack: Vec<BoundingBox> = Vec::new();

    for cmd in commands {
        match cmd.kind {
            RenderCommandKind::ScissorStart => {
                let new_clip = stack
                    .last()
                    .and_then(|current| current.intersect(&cmd.bounds))
                    .unwrap_or(cmd.bounds);
                stack.push(new_clip);
            },
            RenderCommandKind::ScissorEnd => {
                stack.pop();
            },
            _ => {},
        }
        result.push(stack.last().copied());
    }

    result
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests use unwrap for clearer failure messages"
)]
mod tests {
    use bevy::color::Color;

    use super::*;
    use crate::layout::DrawZIndex;
    use crate::layout::Mm;
    use crate::layout::RectangleSource;
    use crate::layout::Unit;
    use crate::panel::DiegeticPanel;

    fn bbox(x: f32, y: f32, width: f32, height: f32) -> BoundingBox {
        BoundingBox {
            x,
            y,
            width,
            height,
        }
    }

    fn rect_cmd(bounds: BoundingBox) -> RenderCommand {
        RenderCommand {
            bounds,
            kind: RenderCommandKind::Rectangle {
                color:  Color::WHITE,
                source: RectangleSource::Background,
            },
            element_idx: 0,
            z_index: DrawZIndex::default(),
        }
    }

    fn scissor_start(bounds: BoundingBox) -> RenderCommand {
        RenderCommand {
            bounds,
            kind: RenderCommandKind::ScissorStart,
            element_idx: 0,
            z_index: DrawZIndex::default(),
        }
    }

    fn scissor_end(bounds: BoundingBox) -> RenderCommand {
        RenderCommand {
            bounds,
            kind: RenderCommandKind::ScissorEnd,
            element_idx: 0,
            z_index: DrawZIndex::default(),
        }
    }

    #[test]
    fn empty_commands() {
        let result = compute_clip_rects(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn no_scissor_commands() {
        let commands = vec![
            rect_cmd(bbox(0.0, 0.0, 100.0, 100.0)),
            rect_cmd(bbox(10.0, 10.0, 50.0, 50.0)),
        ];
        let result = compute_clip_rects(&commands);
        assert!(result.iter().all(Option::is_none));
    }

    #[test]
    fn single_scissor_pair() {
        let clip = bbox(0.0, 0.0, 100.0, 100.0);
        let commands = vec![
            rect_cmd(bbox(0.0, 0.0, 200.0, 200.0)), // before clip
            scissor_start(clip),
            rect_cmd(bbox(10.0, 10.0, 50.0, 50.0)), // inside clip
            scissor_end(clip),
            rect_cmd(bbox(0.0, 0.0, 200.0, 200.0)), // after clip
        ];
        let result = compute_clip_rects(&commands);
        assert!(result[0].is_none());
        assert!(result[1].is_some()); // ScissorStart itself is inside
        assert_eq!(result[2].unwrap(), clip);
        assert!(result[3].is_none()); // ScissorEnd pops
        assert!(result[4].is_none());
    }

    #[test]
    fn nested_clips_intersect() {
        let outer = bbox(0.0, 0.0, 100.0, 100.0);
        let inner = bbox(50.0, 50.0, 100.0, 100.0);
        // Intersection: (50, 50, 50, 50)
        let commands = vec![
            scissor_start(outer),
            scissor_start(inner),
            rect_cmd(bbox(60.0, 60.0, 10.0, 10.0)),
            scissor_end(inner),
            rect_cmd(bbox(10.0, 10.0, 10.0, 10.0)),
            scissor_end(outer),
        ];
        let result = compute_clip_rects(&commands);
        let nested_clip = result[2].unwrap();
        assert!((nested_clip.x - 50.0).abs() < f32::EPSILON);
        assert!((nested_clip.y - 50.0).abs() < f32::EPSILON);
        assert!((nested_clip.width - 50.0).abs() < f32::EPSILON);
        assert!((nested_clip.height - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn pop_restores_parent_clip() {
        let outer = bbox(0.0, 0.0, 100.0, 100.0);
        let inner = bbox(50.0, 50.0, 100.0, 100.0);
        let commands = vec![
            scissor_start(outer),
            scissor_start(inner),
            scissor_end(inner),
            rect_cmd(bbox(10.0, 10.0, 10.0, 10.0)), // should be outer clip
            scissor_end(outer),
        ];
        let result = compute_clip_rects(&commands);
        assert_eq!(result[3].unwrap(), outer);
    }

    #[test]
    fn deeply_nested_three_levels() {
        let a = bbox(0.0, 0.0, 200.0, 200.0);
        let b = bbox(50.0, 50.0, 200.0, 200.0);
        let c = bbox(80.0, 80.0, 200.0, 200.0);
        let commands = vec![
            scissor_start(a),
            scissor_start(b),
            scissor_start(c),
            rect_cmd(bbox(90.0, 90.0, 10.0, 10.0)),
            scissor_end(c),
            scissor_end(b),
            scissor_end(a),
        ];
        let result = compute_clip_rects(&commands);
        // a ∩ b = (50, 50, 150, 150), then ∩ c = (80, 80, 120, 120)
        let deepest = result[3].unwrap();
        assert!((deepest.x - 80.0).abs() < f32::EPSILON);
        assert!((deepest.y - 80.0).abs() < f32::EPSILON);
        assert!((deepest.width - 120.0).abs() < f32::EPSILON);
        assert!((deepest.height - 120.0).abs() < f32::EPSILON);
    }

    #[test]
    fn effective_clip_uses_viewport_without_scissor() {
        let viewport = bbox(0.0, 0.0, 100.0, 80.0);
        let command = bbox(10.0, 10.0, 20.0, 20.0);

        assert_eq!(effective_clip(command, None, viewport), Some(viewport));
    }

    #[test]
    fn effective_clip_intersects_scissor_with_viewport() {
        let viewport = bbox(0.0, 0.0, 100.0, 80.0);
        let scissor = bbox(50.0, 20.0, 100.0, 100.0);
        let command = bbox(60.0, 30.0, 10.0, 10.0);

        assert_eq!(
            effective_clip(command, Some(scissor), viewport),
            Some(bbox(50.0, 20.0, 50.0, 60.0))
        );
    }

    #[test]
    fn effective_clip_returns_none_when_command_outside_viewport() {
        let viewport = bbox(0.0, 0.0, 100.0, 80.0);
        let command = bbox(120.0, 10.0, 20.0, 20.0);

        assert_eq!(effective_clip(command, None, viewport), None);
    }

    #[test]
    fn effective_clip_returns_none_when_scissor_misses_viewport() {
        let viewport = bbox(0.0, 0.0, 100.0, 80.0);
        let scissor = bbox(150.0, 20.0, 20.0, 20.0);
        let command = bbox(10.0, 10.0, 20.0, 20.0);

        assert_eq!(effective_clip(command, Some(scissor), viewport), None);
    }

    #[test]
    fn panel_viewport_uses_layout_points_for_world_units() {
        let panel = DiegeticPanel::world()
            .size(Mm(210.0), Mm(297.0))
            .build()
            .unwrap();
        let viewport = panel_viewport(&panel);
        let mm_to_points = Unit::Millimeters.to_points();

        assert!(210.0f32.mul_add(-mm_to_points, viewport.width).abs() < 1e-3);
        assert!(297.0f32.mul_add(-mm_to_points, viewport.height).abs() < 1e-3);
    }
}
