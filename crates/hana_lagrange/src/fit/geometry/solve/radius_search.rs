use bevy::prelude::Camera;
use bevy::prelude::EulerRot;
use bevy::prelude::GlobalTransform;
use bevy::prelude::OrthographicProjection;
use bevy::prelude::Projection;
use bevy::prelude::Quat;
use bevy::prelude::Rect;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::debug;
use bevy::prelude::warn;
use hana_kana::Position;

use super::fit_solution::FitError;
use super::fit_solution::FitSolution;
use super::focus;
use super::margins;
use crate::fit::constants::INITIAL_RADIUS_MULTIPLIER;
use crate::fit::constants::MAX_ITERATIONS;
use crate::fit::constants::MAX_MARGIN;
use crate::fit::constants::MAX_RADIUS_MULTIPLIER;
use crate::fit::constants::MIN_MARGIN;
use crate::fit::constants::MIN_RADIUS_MULTIPLIER;
use crate::fit::constants::TOLERANCE;
use crate::fit::geometry::anchor::FitAnchor;
use crate::fit::geometry::projection;
use crate::fit::geometry::projection::ProjectionMode;
use crate::fit::geometry::projection::ScreenSpaceBounds;

/// Tracks whether the binary search ever saw projectable bounds.
#[derive(Debug, Clone, Copy)]
enum BoundsSearch {
    NeverProjectable,
    Projectable,
}

/// Pre-computed parameters for the fit binary search.
pub(super) struct FitParameters {
    pub(super) rotation:                    Quat,
    pub(super) aspect_ratio:                f32,
    pub(super) orthographic_fixed_distance: Option<f32>,
    pub(super) projection_mode:             ProjectionMode,
    pub(super) zoom_multiplier:             f32,
    pub(super) viewport_size:               Option<Vec2>,
}

/// Calculates the optimal radius and centered focus to fit pre-extracted vertices in the camera
/// view. The focus is adjusted so the projected mesh silhouette is centered in the viewport.
///
/// For each candidate radius, computes the focus that centers the projected silhouette in the
/// viewport (since the geometric center doesn't project to screen center from off-axis angles),
/// then evaluates margins at that centered position. Returns the fit solution where
/// the constraining margin equals the target and the silhouette is centered.
///
/// A lateral camera shift does not change point depths, so the centering is
/// geometrically exact for the constraining margin check.
pub fn calculate_fit(
    points: &[Vec3],
    geometric_center: Vec3,
    yaw: f32,
    pitch: f32,
    margin: f32,
    anchor: FitAnchor,
    offset_px: Vec2,
    projection: &Projection,
    camera: &Camera,
) -> Result<FitSolution, FitError> {
    let clamped_margin = if margin.is_nan() {
        MIN_MARGIN
    } else {
        margin.clamp(MIN_MARGIN, MAX_MARGIN)
    };
    #[allow(
        clippy::float_cmp,
        reason = "clamp returns input unchanged when in bounds — bitwise identical"
    )]
    if clamped_margin != margin {
        warn!(
            "calculate_fit: clamped margin from {margin} to {clamped_margin} (expected [{MIN_MARGIN}, {MAX_MARGIN}])"
        );
    }

    let mode_and_distance = match projection {
        Projection::Perspective(_) => Some((ProjectionMode::Perspective, None)),
        Projection::Orthographic(o) => Some((
            ProjectionMode::Orthographic,
            Some(f32::midpoint(o.near, o.far)),
        )),
        Projection::Custom(_) => None,
    };
    let Some((projection_mode, orthographic_fixed_distance)) = mode_and_distance else {
        return Err(FitError::UnsupportedProjection);
    };

    let viewport_size = camera.logical_viewport_size();
    if has_pixel_offset(offset_px) && !focus::viewport_can_map_pixels(viewport_size) {
        return Err(FitError::NoViewport);
    }

    let aspect_ratio = projection::projection_aspect_ratio(projection, viewport_size)
        .ok_or(FitError::NoViewport)?;

    let parameters = FitParameters {
        rotation: Quat::from_euler(EulerRot::YXZ, yaw, -pitch, 0.0),
        aspect_ratio,
        orthographic_fixed_distance,
        projection_mode,
        zoom_multiplier: margins::zoom_margin_multiplier(clamped_margin),
        viewport_size,
    };

    let object_radius = points
        .iter()
        .map(|c| (*c - geometric_center).length())
        .fold(0.0_f32, f32::max);

    binary_search_for_fit(
        points,
        geometric_center,
        object_radius,
        projection,
        &parameters,
        anchor,
        offset_px,
    )
}

fn has_pixel_offset(offset_px: Vec2) -> bool { offset_px.length_squared() > f32::EPSILON }

/// Binary search for the camera radius that produces the target margin.
///
/// For perspective: radius = camera distance (changes apparent size).
/// For ortho: `OrbitCam` maps radius → `OrthographicProjection::scale`,
/// so searching over radius effectively searches over scale.
fn binary_search_for_fit(
    points: &[Vec3],
    geometric_center: Vec3,
    object_radius: f32,
    projection: &Projection,
    parameters: &FitParameters,
    anchor: FitAnchor,
    offset_px: Vec2,
) -> Result<FitSolution, FitError> {
    let mut min_radius = object_radius * MIN_RADIUS_MULTIPLIER;
    let mut max_radius = object_radius * MAX_RADIUS_MULTIPLIER;
    let mut best_radius = object_radius * INITIAL_RADIUS_MULTIPLIER;
    let mut best_focus = Position(geometric_center);
    let mut best_error = f32::INFINITY;
    let mut bounds_search = BoundsSearch::NeverProjectable;
    let mut converged = false;

    debug!("Binary search starting: range [{min_radius:.1}, {max_radius:.1}]");

    for iteration in 0..MAX_ITERATIONS {
        let test_radius = f32::midpoint(min_radius, max_radius);
        let test_projection = build_test_projection(projection, test_radius);

        let centered_focus = focus::refine_focus_centering(
            points,
            geometric_center,
            test_radius,
            &test_projection,
            parameters,
        );

        let camera_distance = parameters
            .orthographic_fixed_distance
            .unwrap_or(test_radius);
        let camera_position =
            centered_focus + parameters.rotation * Vec3::new(0.0, 0.0, camera_distance);
        let camera_global = GlobalTransform::from(
            Transform::from_translation(camera_position).with_rotation(parameters.rotation),
        );

        let Some((bounds, _)) = ScreenSpaceBounds::from_points(
            points,
            &camera_global,
            &test_projection,
            parameters.aspect_ratio,
        ) else {
            debug!(
                "Iteration {iteration}: Points behind camera at radius {test_radius:.1}, searching higher"
            );
            min_radius = test_radius;
            continue;
        };
        bounds_search = BoundsSearch::Projectable;

        let (target_margin_x, target_margin_y) =
            margins::calculate_target_margins(&bounds, parameters.zoom_multiplier);
        let (current_margin, target_margin, dimension) =
            margins::find_constraining_margin(&bounds, target_margin_x, target_margin_y);

        debug!(
            "Iteration {iteration}: radius={test_radius:.1} | {dimension} margin={current_margin:.3} \
             target={target_margin:.3} | L={:.3} R={:.3} T={:.3} B={:.3} | range=[{min_radius:.1}, {max_radius:.1}]",
            bounds.left_margin, bounds.right_margin, bounds.top_margin, bounds.bottom_margin
        );

        let margin_error = (current_margin - target_margin).abs();
        if margin_error < best_error {
            best_error = margin_error;
            best_radius = test_radius;
            best_focus = Position(centered_focus);
        }

        if current_margin > target_margin {
            max_radius = test_radius;
        } else {
            min_radius = test_radius;
        }

        if (max_radius - min_radius) < TOLERANCE {
            debug!(
                "Iteration {iteration}: Converged to best radius {best_radius:.3} error={best_error:.5}"
            );
            converged = true;
            break;
        }
    }

    if matches!(bounds_search, BoundsSearch::NeverProjectable) {
        return Err(FitError::PointsBehindCamera);
    }

    if !converged {
        warn!(
            "Binary search did not converge in {MAX_ITERATIONS} iterations. Using best radius {best_radius:.1}"
        );
    }

    let focus = focus::refine_focus_anchoring(
        points,
        *best_focus,
        best_radius,
        projection,
        parameters,
        anchor,
        offset_px,
    );

    Ok(FitSolution {
        radius: best_radius,
        focus:  Position(focus),
    })
}

/// Builds a test projection with the given radius/scale for binary search iterations.
///
/// For perspective, returns the original projection unchanged.
/// For orthographic, creates a modified projection with `area` recomputed for the test scale,
/// since `OrbitCam` maps `radius` → `OrthographicProjection::scale`.
fn build_test_projection(projection: &Projection, test_radius: f32) -> Projection {
    match projection {
        Projection::Orthographic(ortho) => {
            // Compute what the area would be at this scale.
            // The current area is `base_size * current_scale`, so base_size = area / scale.
            // At test scale: new_area = base_size * test_radius.
            let current_scale = ortho.scale;
            let scale_ratio = if current_scale.abs() > f32::EPSILON {
                test_radius / current_scale
            } else {
                1.0
            };
            let new_area = Rect::new(
                ortho.area.min.x * scale_ratio,
                ortho.area.min.y * scale_ratio,
                ortho.area.max.x * scale_ratio,
                ortho.area.max.y * scale_ratio,
            );
            Projection::Orthographic(OrthographicProjection {
                scale: test_radius,
                area: new_area,
                ..*ortho
            })
        },
        Projection::Perspective(_) | Projection::Custom(_) => projection.clone(),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "expect is idiomatic for test assertions"
)]
mod tests {
    use bevy::prelude::Camera;
    use bevy::prelude::EulerRot;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::OrthographicProjection;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::Projection;
    use bevy::prelude::Quat;
    use bevy::prelude::Rect;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;

    use super::calculate_fit;
    use crate::fit::FitAnchor;
    use crate::fit::constants::CENTERING_TOLERANCE;
    use crate::fit::constants::DEFAULT_FIT_MARGIN;
    use crate::fit::geometry::projection;
    use crate::fit::geometry::projection::ScreenSpaceBounds;
    use crate::fit::geometry::solve::fit_solution::FitError;
    use crate::fit::geometry::solve::fit_solution::FitSolution;

    fn default_perspective() -> Projection {
        Projection::Perspective(PerspectiveProjection::default())
    }

    fn projected_bounds(
        points: &[Vec3],
        fit: FitSolution,
        yaw: f32,
        pitch: f32,
        projection: &Projection,
        camera: &Camera,
    ) -> ScreenSpaceBounds {
        let aspect_ratio =
            projection::projection_aspect_ratio(projection, camera.logical_viewport_size())
                .expect("test projection should have an aspect ratio");
        let rotation = Quat::from_euler(EulerRot::YXZ, yaw, -pitch, 0.0);
        let camera_position = *fit.focus + rotation * Vec3::new(0.0, 0.0, fit.radius);
        let camera_global = GlobalTransform::from(
            Transform::from_translation(camera_position).with_rotation(rotation),
        );
        let (bounds, _) =
            ScreenSpaceBounds::from_points(points, &camera_global, projection, aspect_ratio)
                .expect("test fit should project its points");
        bounds
    }

    #[test]
    fn calculate_fit_returns_no_viewport_for_invalid_ortho_area() {
        let projection = Projection::Orthographic(OrthographicProjection {
            area: Rect::new(-1.0, 0.0, 1.0, 0.0),
            ..OrthographicProjection::default_3d()
        });
        let camera = Camera::default();

        let result = calculate_fit(
            &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)],
            Vec3::ZERO,
            0.0,
            0.0,
            DEFAULT_FIT_MARGIN,
            FitAnchor::Center,
            Vec2::ZERO,
            &projection,
            &camera,
        );

        assert!(matches!(result, Err(FitError::NoViewport)));
    }

    #[test]
    fn calculate_fit_returns_points_behind_camera_for_degenerate_point_cloud() {
        let projection = default_perspective();
        let camera = Camera::default();
        let points = [Vec3::ZERO, Vec3::ZERO, Vec3::ZERO];

        let result = calculate_fit(
            &points,
            Vec3::ZERO,
            0.0,
            0.0,
            DEFAULT_FIT_MARGIN,
            FitAnchor::Center,
            Vec2::ZERO,
            &projection,
            &camera,
        );

        assert!(matches!(result, Err(FitError::PointsBehindCamera)));
    }

    #[test]
    fn calculate_fit_clamps_out_of_range_margin_and_still_returns_solution() {
        let projection = default_perspective();
        let camera = Camera::default();
        let points = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];

        let result = calculate_fit(
            &points,
            Vec3::ZERO,
            0.0,
            0.0,
            5.0,
            FitAnchor::Center,
            Vec2::ZERO,
            &projection,
            &camera,
        );

        let fit = result.expect("fit should succeed with clamped margin");
        assert!(fit.radius.is_finite());
        assert!(fit.focus.is_finite());
    }

    #[test]
    fn calculate_fit_handles_nan_margin_by_clamping_to_zero() {
        let projection = default_perspective();
        let camera = Camera::default();
        let points = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];

        let result = calculate_fit(
            &points,
            Vec3::ZERO,
            0.0,
            0.0,
            f32::NAN,
            FitAnchor::Center,
            Vec2::ZERO,
            &projection,
            &camera,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn calculate_fit_can_anchor_bounds_top_left() {
        let projection = default_perspective();
        let camera = Camera::default();
        let points = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];

        let fit = calculate_fit(
            &points,
            Vec3::ZERO,
            0.0,
            0.0,
            DEFAULT_FIT_MARGIN,
            FitAnchor::TopLeft,
            Vec2::ZERO,
            &projection,
            &camera,
        )
        .expect("top-left anchored fit should succeed");
        let bounds = projected_bounds(&points, fit, 0.0, 0.0, &projection, &camera);

        assert!(
            (bounds.min_normalized_x + bounds.half_extent_x).abs() < CENTERING_TOLERANCE,
            "left edge should land on viewport left edge: {bounds:?}",
        );
        assert!(
            (bounds.max_normalized_y - bounds.half_extent_y).abs() < CENTERING_TOLERANCE,
            "top edge should land on viewport top edge: {bounds:?}",
        );
    }

    #[test]
    fn calculate_fit_requires_viewport_for_pixel_offset() {
        let projection = default_perspective();
        let camera = Camera::default();
        let points = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];

        let result = calculate_fit(
            &points,
            Vec3::ZERO,
            0.0,
            0.0,
            DEFAULT_FIT_MARGIN,
            FitAnchor::TopLeft,
            Vec2::new(16.0, 16.0),
            &projection,
            &camera,
        );

        assert!(matches!(result, Err(FitError::NoViewport)));
    }

    /// Flat quad in XZ at Y=0, camera at pitch=0 (edge-on). The vertical screen
    /// extent is zero; without the degenerate-extent guard in
    /// `calculate_target_margins` that divides by zero and the binary search
    /// converges on an absurd radius.
    #[test]
    fn edge_on_flat_plane_produces_reasonable_radius() {
        let projection = default_perspective();
        let camera = Camera::default();
        let points = [
            Vec3::new(-0.5, 0.0, -0.5),
            Vec3::new(0.5, 0.0, -0.5),
            Vec3::new(-0.5, 0.0, 0.5),
            Vec3::new(0.5, 0.0, 0.5),
        ];
        let object_radius = points
            .iter()
            .copied()
            .map(Vec3::length)
            .fold(0.0_f32, f32::max);

        let fit = calculate_fit(
            &points,
            Vec3::ZERO,
            0.0,
            0.0,
            DEFAULT_FIT_MARGIN,
            FitAnchor::Center,
            Vec2::ZERO,
            &projection,
            &camera,
        )
        .expect("edge-on flat plane should produce a valid fit");

        assert!(
            fit.radius < object_radius * 10.0,
            "radius {:.1} should be less than 10x object_radius {:.3}",
            fit.radius,
            object_radius,
        );
    }

    /// Same flat quad but with a tiny pitch (near-degenerate). Should converge
    /// to a similar radius as a non-degenerate case.
    #[test]
    fn near_edge_on_flat_plane_still_converges() {
        let projection = default_perspective();
        let camera = Camera::default();
        let points = [
            Vec3::new(-0.5, 0.0, -0.5),
            Vec3::new(0.5, 0.0, -0.5),
            Vec3::new(-0.5, 0.0, 0.5),
            Vec3::new(0.5, 0.0, 0.5),
        ];
        let object_radius = points
            .iter()
            .copied()
            .map(Vec3::length)
            .fold(0.0_f32, f32::max);

        let fit = calculate_fit(
            &points,
            Vec3::ZERO,
            0.0,
            0.001,
            DEFAULT_FIT_MARGIN,
            FitAnchor::Center,
            Vec2::ZERO,
            &projection,
            &camera,
        )
        .expect("near-edge-on flat plane should produce a valid fit");

        assert!(
            fit.radius < object_radius * 10.0,
            "radius {:.1} should be less than 10x object_radius {:.3}",
            fit.radius,
            object_radius,
        );
    }

    /// Vertical line segment (zero horizontal extent) viewed head-on.
    /// Mirror of the edge-on plane case — ensures the degenerate guard is symmetric.
    #[test]
    fn vertical_line_zero_horizontal_extent_produces_reasonable_radius() {
        let projection = default_perspective();
        let camera = Camera::default();
        let points = [Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)];
        let object_radius = 1.0;

        let fit = calculate_fit(
            &points,
            Vec3::ZERO,
            0.0,
            0.0,
            DEFAULT_FIT_MARGIN,
            FitAnchor::Center,
            Vec2::ZERO,
            &projection,
            &camera,
        )
        .expect("vertical line should produce a valid fit");

        assert!(
            fit.radius < object_radius * 10.0,
            "radius {:.1} should be less than 10x object_radius {:.1}",
            fit.radius,
            object_radius,
        );
    }
}
