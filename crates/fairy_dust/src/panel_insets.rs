//! Window space covered by screen panels, as insets for the camera home fit.

use bevy::prelude::Vec2;
use hana_diegetic::Anchor;
use hana_lagrange::FitViewportInsets;

/// One screen panel's placement in the window, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CoveringPanel {
    pub anchor:   Anchor,
    pub top_left: Vec2,
    pub size:     Vec2,
}

/// The window edge a panel's space is reserved from, with the reserved depth.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ReservedEdge {
    Left(f32),
    Top(f32),
    Right(f32),
    Bottom(f32),
}

/// Insets that keep the home framing out of every edge-anchored panel.
///
/// A panel touching the edge named by its anchor reserves its depth from that
/// edge. A corner panel reserves from whichever touched edge leaves the larger free
/// region once every panel is placed. A centered panel, such as a help
/// overlay, reserves nothing.
#[cfg(test)]
pub(crate) fn covering_panel_insets(
    window_size: Vec2,
    panels: impl IntoIterator<Item = CoveringPanel>,
) -> FitViewportInsets {
    panel_insets(window_size, panels, 0.0)
}

/// Keeps the free window edge as far from Home as the opposite panel edge.
pub(crate) fn balanced_horizontal_panel_insets(
    window_size: Vec2,
    panels: impl IntoIterator<Item = CoveringPanel>,
    margin: f32,
) -> FitViewportInsets {
    panel_insets(window_size, panels, margin)
}

fn panel_insets(
    window_size: Vec2,
    panels: impl IntoIterator<Item = CoveringPanel>,
    horizontal_window_margin: f32,
) -> FitViewportInsets {
    let choices: Vec<Vec<ReservedEdge>> = panels
        .into_iter()
        .map(|panel| reserved_edge_choices(window_size, panel))
        .filter(|edges| !edges.is_empty())
        .collect();

    let mut best = [0.0_f32; 4];
    let mut best_area = f32::NEG_INFINITY;
    let combinations: usize = choices.iter().map(Vec::len).product();
    for combination in 0..combinations {
        let mut remaining = combination;
        let mut sides = [0.0_f32; 4];
        for edges in &choices {
            let edge = edges[remaining % edges.len()];
            remaining /= edges.len();
            match edge {
                ReservedEdge::Left(depth) => sides[0] = sides[0].max(depth),
                ReservedEdge::Top(depth) => sides[1] = sides[1].max(depth),
                ReservedEdge::Right(depth) => sides[2] = sides[2].max(depth),
                ReservedEdge::Bottom(depth) => sides[3] = sides[3].max(depth),
            }
        }
        let free_width = (window_size.x - sides[0] - sides[2]).max(0.0);
        let free_height = (window_size.y - sides[1] - sides[3]).max(0.0);
        let area = free_width * free_height;
        if area > best_area {
            best_area = area;
            best = sides;
        }
    }
    if best[0] > 0.0 && best[2] == 0.0 {
        best[2] = horizontal_window_margin;
    } else if best[2] > 0.0 && best[0] == 0.0 {
        best[0] = horizontal_window_margin;
    }
    FitViewportInsets::new(best[0], best[1], best[2], best[3])
}

fn reserved_edge_choices(window_size: Vec2, panel: CoveringPanel) -> Vec<ReservedEdge> {
    let bottom_right = panel.top_left + panel.size;
    let left = (panel.top_left.x <= 0.0).then_some(ReservedEdge::Left(bottom_right.x));
    let top = (panel.top_left.y <= 0.0).then_some(ReservedEdge::Top(bottom_right.y));
    let right = (bottom_right.x >= window_size.x)
        .then_some(ReservedEdge::Right(window_size.x - panel.top_left.x));
    let bottom = (bottom_right.y >= window_size.y)
        .then_some(ReservedEdge::Bottom(window_size.y - panel.top_left.y));
    match panel.anchor {
        Anchor::TopLeft => [left, top].into_iter().flatten().collect(),
        Anchor::TopCenter => top.into_iter().collect(),
        Anchor::TopRight => [right, top].into_iter().flatten().collect(),
        Anchor::CenterLeft => left.into_iter().collect(),
        Anchor::Center => Vec::new(),
        Anchor::CenterRight => right.into_iter().collect(),
        Anchor::BottomLeft => [left, bottom].into_iter().flatten().collect(),
        Anchor::BottomCenter => bottom.into_iter().collect(),
        Anchor::BottomRight => [right, bottom].into_iter().flatten().collect(),
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::Vec2;
    use hana_diegetic::Anchor;
    use hana_lagrange::FitViewportInsets;

    use super::CoveringPanel;
    use super::balanced_horizontal_panel_insets;
    use super::covering_panel_insets;

    const fn panel(anchor: Anchor, top_left: Vec2, size: Vec2) -> CoveringPanel {
        CoveringPanel {
            anchor,
            top_left,
            size,
        }
    }

    #[test]
    fn widgets_example_panels_reserve_bottom_and_right() {
        let window = Vec2::new(1269.0, 1006.0);
        let help = panel(
            Anchor::BottomLeft,
            Vec2::new(0.0, 744.0),
            Vec2::new(482.0, 262.0),
        );
        let camera = panel(
            Anchor::BottomRight,
            Vec2::new(952.0, 543.0),
            Vec2::new(317.0, 463.0),
        );
        let title = panel(
            Anchor::TopCenter,
            Vec2::new(0.0, 0.0),
            Vec2::new(1269.0, 40.0),
        );
        let insets = covering_panel_insets(window, [help, camera, title]);
        assert_eq!(insets, FitViewportInsets::new(0.0, 40.0, 317.0, 262.0));
    }

    #[test]
    fn centered_panels_reserve_nothing() {
        let window = Vec2::new(1280.0, 720.0);
        let overlay = panel(
            Anchor::Center,
            Vec2::new(340.0, 160.0),
            Vec2::new(600.0, 400.0),
        );
        assert_eq!(
            covering_panel_insets(window, [overlay]),
            FitViewportInsets::NONE
        );
    }

    #[test]
    fn transient_panel_away_from_its_named_edge_reserves_nothing() {
        let window = Vec2::new(1280.0, 720.0);
        let tooltip = panel(
            Anchor::CenterLeft,
            Vec2::new(480.0, 300.0),
            Vec2::new(150.0, 50.0),
        );
        assert_eq!(
            covering_panel_insets(window, [tooltip]),
            FitViewportInsets::NONE
        );
    }

    #[test]
    fn balancing_does_not_move_camera_panel_reservation_to_bottom() {
        let window = Vec2::new(1440.0, 900.0);
        let title = panel(Anchor::TopCenter, Vec2::ZERO, Vec2::new(1440.0, 40.0));
        let help = panel(
            Anchor::BottomLeft,
            Vec2::new(0.0, 780.0),
            Vec2::new(250.0, 120.0),
        );
        let orbit_camera = panel(
            Anchor::BottomRight,
            Vec2::new(1123.0, 437.0),
            Vec2::new(317.0, 463.0),
        );
        let free_camera = panel(
            Anchor::BottomRight,
            Vec2::new(1160.0, 630.0),
            Vec2::new(280.0, 270.0),
        );
        for camera in [orbit_camera, free_camera] {
            let insets = balanced_horizontal_panel_insets(window, [title, help, camera], 12.0);
            assert_eq!(
                insets,
                FitViewportInsets::new(12.0, 40.0, camera.size.x, 120.0)
            );
        }
    }
}
