use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::ecs::system::SystemParam;
use bevy::prelude::Camera;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use bevy::window::PrimaryWindow;

use super::slider_state::SliderDirection;
use super::slider_state::SliderProposalValue;
use super::slider_state::SliderRange;
use super::slider_state::SliderState;
use crate::DiegeticPanel;
use crate::layout::BoundingBox;
use crate::render::CapturedCameraRay;
use crate::render::{self};
use crate::widgets::VisualSlotId;
use crate::widgets::WidgetVisualSlots;

/// A pointer position cannot be projected onto the slider's directed travel.
///
/// A marked thumb whose extent meets or exceeds the content extent has no
/// visible travel, and a headless slider with zero active-axis content extent
/// has no directed interval; both leave nothing to propose. Non-finite
/// active-axis geometry and a negative or non-finite thumb extent are
/// rejected the same way. A missing slider-root content box also prevents
/// projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ProjectionUnavailable;

/// Active-axis extent of a slider's thumb, which insets the directed travel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ThumbExtent {
    /// The slider marks no thumb; travel spans the full directed content extent.
    Headless,
    /// The marked thumb's solved border-box extent on the active axis.
    Marked(f32),
}

/// Why a slider slot has no value-derived translation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SlotTranslationUnavailable {
    /// The slider root, the placed slot, or the travel thumb solved no box:
    /// the slider marks no such slot.
    Unmarked,
    /// The active-axis content geometry is non-finite or zero-extent, the
    /// thumb extent is non-finite or negative, or the solved delta is non-finite.
    Degenerate,
}

/// Projects a panel-layout pointer position into a raw slider-domain target.
///
/// The directed interval follows the thumb center's path inside `content` —
/// the slider root's content box, excluding border and padding: a marked
/// thumb insets each directed endpoint by half `thumb_extent` on the active
/// axis, while a headless slider ([`ThumbExtent::Headless`]) travels the full
/// directed content extent. The pointer's active-axis coordinate is clamped
/// into that interval, normalized to `[0, 1]`, and mapped through the raw
/// `range`. The result carries no snap or step normalization; application
/// acceptance through [`SliderState::set_proposed_value`] performs the only snap and
/// clamp.
pub(super) fn project_pointer_value(
    pointer: Vec2,
    content: BoundingBox,
    thumb_extent: ThumbExtent,
    range: SliderRange,
    direction: SliderDirection,
) -> Result<f32, ProjectionUnavailable> {
    let travel = directed_thumb_travel(content, thumb_extent, direction)?;
    let pointer_coordinate = match direction {
        SliderDirection::LeftToRight | SliderDirection::RightToLeft => pointer.x,
        SliderDirection::BottomToTop | SliderDirection::TopToBottom => pointer.y,
    };
    let clamped =
        pointer_coordinate.clamp(travel.start.min(travel.end), travel.start.max(travel.end));
    let fraction = (clamped - travel.start) / (travel.end - travel.start);
    let target = range.fraction_to_value(fraction);
    target
        .is_finite()
        .then_some(target)
        .ok_or(ProjectionUnavailable)
}

/// Active-axis coordinates the thumb center reaches at the directed range
/// start and end.
///
/// `start` maps to the normalized value `0` and `end` to `1`, so both pointer
/// projection and value presentation share one directed interval.
#[derive(Clone, Copy)]
struct DirectedThumbTravel {
    start: f32,
    end:   f32,
}

/// Computes the directed thumb-center interval inside `content`.
///
/// A marked thumb insets each directed endpoint by half `thumb_extent`; a
/// headless slider ([`ThumbExtent::Headless`]) travels the full directed content
/// extent. Non-finite active-axis geometry, a negative or non-finite thumb
/// extent, and a thumb extent that meets or exceeds the content extent all
/// leave no directed travel and return [`ProjectionUnavailable`].
fn directed_thumb_travel(
    content: BoundingBox,
    thumb_extent: ThumbExtent,
    direction: SliderDirection,
) -> Result<DirectedThumbTravel, ProjectionUnavailable> {
    let (axis_start, axis_extent) = match direction {
        SliderDirection::LeftToRight | SliderDirection::RightToLeft => (content.x, content.width),
        SliderDirection::BottomToTop | SliderDirection::TopToBottom => (content.y, content.height),
    };
    if !axis_start.is_finite() || !axis_extent.is_finite() || axis_extent <= 0.0 {
        return Err(ProjectionUnavailable);
    }
    let endpoint_inset = match thumb_extent {
        ThumbExtent::Headless => 0.0,
        ThumbExtent::Marked(extent) if extent.is_finite() && extent >= 0.0 => extent * 0.5,
        ThumbExtent::Marked(_) => return Err(ProjectionUnavailable),
    };
    if endpoint_inset.mul_add(-2.0, axis_extent) <= 0.0 {
        return Err(ProjectionUnavailable);
    }
    let range_start_coordinate = axis_start + endpoint_inset;
    let range_end_coordinate = axis_start + axis_extent - endpoint_inset;
    let (start, end) = match direction {
        SliderDirection::LeftToRight | SliderDirection::TopToBottom => {
            (range_start_coordinate, range_end_coordinate)
        },
        SliderDirection::RightToLeft | SliderDirection::BottomToTop => {
            (range_end_coordinate, range_start_coordinate)
        },
    };
    Ok(DirectedThumbTravel { start, end })
}

/// Live camera, panel, and window state for reprojecting a slider pointer.
#[derive(SystemParam)]
pub(in crate::widgets) struct SliderProjection<'w, 's> {
    pub(super) fine_adjustment: Res<'w, crate::FineAdjustment>,
    cameras: Query<
        'w,
        's,
        (
            &'static Camera,
            &'static GlobalTransform,
            Option<&'static RenderTarget>,
        ),
    >,
    panels:                     Query<'w, 's, (&'static DiegeticPanel, &'static GlobalTransform)>,
    primary:                    Query<'w, 's, Entity, With<PrimaryWindow>>,
}

impl SliderProjection<'_, '_> {
    /// Projects `viewport_position` through the live captured camera and panel
    /// onto the slider's raw-domain travel.
    ///
    /// Flat-panel ray failures and unavailable directed travel return
    /// [`ProjectionUnavailable`].
    fn project(
        &self,
        camera_entity: Entity,
        captured_target: &NormalizedRenderTarget,
        viewport_position: Vec2,
        panel_entity: Entity,
        travel: SliderTravel,
    ) -> Result<(Vec2, f32), ProjectionUnavailable> {
        let Ok((camera, camera_transform, render_target)) = self.cameras.get(camera_entity) else {
            return Err(ProjectionUnavailable);
        };
        let Ok((panel, panel_transform)) = self.panels.get(panel_entity) else {
            return Err(ProjectionUnavailable);
        };
        let primary_window = self.primary.single().ok();
        let captured_camera_ray = CapturedCameraRay {
            camera,
            camera_transform,
            render_target,
            primary_window,
            captured_target,
            viewport_position,
        };
        let pointer_local =
            render::project_flat_panel_ray_hit(&captured_camera_ray, panel, panel_transform)
                .map_err(|_| ProjectionUnavailable)?;
        let value = project_pointer_value(
            pointer_local,
            travel.content,
            travel.thumb_extent,
            travel.range,
            travel.direction,
        )
        .map_err(|_| ProjectionUnavailable)?;
        Ok((pointer_local, value))
    }
}

/// The solved geometry one slider projection reads: the slider root's content
/// box, the marked thumb's active-axis extent, and the value range and
/// direction the pointer coordinate maps through.
#[derive(Clone, Copy)]
struct SliderTravel {
    content:      BoundingBox,
    thumb_extent: ThumbExtent,
    range:        SliderRange,
    direction:    SliderDirection,
}

impl SliderTravel {
    /// Reads the geometry from the widget's solved slots.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionUnavailable`] when the slider root solved no content box.
    fn new(state: &SliderState, slots: &WidgetVisualSlots) -> Result<Self, ProjectionUnavailable> {
        let direction = state.direction();
        Ok(Self {
            content: slots
                .content_box(VisualSlotId::SLIDER_ROOT)
                .ok_or(ProjectionUnavailable)?,
            thumb_extent: thumb_extent(slots, direction),
            range: state.range(),
            direction,
        })
    }
}

/// Active-axis extent of the marked thumb's solved border box.
fn thumb_extent(slots: &WidgetVisualSlots, direction: SliderDirection) -> ThumbExtent {
    let Ok(thumb) = travel_thumb_bounds(slots) else {
        return ThumbExtent::Headless;
    };
    ThumbExtent::Marked(match direction {
        SliderDirection::LeftToRight | SliderDirection::RightToLeft => thumb.width,
        SliderDirection::BottomToTop | SliderDirection::TopToBottom => thumb.height,
    })
}

/// Solved bounds that inset thumb travel, preferring the preserved hit envelope.
/// A slider with neither thumb slot is unmarked and projects as headless.
fn travel_thumb_bounds(
    slots: &WidgetVisualSlots,
) -> Result<BoundingBox, SlotTranslationUnavailable> {
    slots
        .border_box(VisualSlotId::SLIDER_THUMB_HIT)
        .or_else(|| slots.border_box(VisualSlotId::SLIDER_THUMB))
        .ok_or(SlotTranslationUnavailable::Unmarked)
}

pub(super) fn project_widget_hit(
    projection: &SliderProjection<'_, '_>,
    camera: Entity,
    captured_target: &NormalizedRenderTarget,
    viewport_position: Vec2,
    panel: Entity,
    state: &SliderState,
    slots: &WidgetVisualSlots,
) -> Result<(Vec2, SliderProposalValue), ProjectionUnavailable> {
    let travel = SliderTravel::new(state, slots)?;
    projection
        .project(camera, captured_target, viewport_position, panel, travel)
        .map(|(position, value)| (position, SliderProposalValue::snapping(value)))
}

/// A pointer's possibly off-track position, separate from the real value it proposes.
#[derive(Clone, Copy)]
pub(super) struct SliderPointerPosition {
    pub(super) fraction: f32,
    range:               SliderRange,
}

impl SliderPointerPosition {
    pub(super) fn clamped_value(self) -> SliderProposalValue {
        SliderProposalValue::snapping(self.range.fraction_to_value(self.fraction.clamp(0.0, 1.0)))
    }
}

/// Relative motion keeps measuring pointer travel beyond either end of the
/// track; the applied slider value still goes through its usual acceptance.
pub(super) fn project_widget_motion(
    projection: &SliderProjection<'_, '_>,
    camera: Entity,
    captured_target: &NormalizedRenderTarget,
    viewport_position: Vec2,
    panel: Entity,
    state: &SliderState,
    slots: &WidgetVisualSlots,
) -> Result<(Vec2, SliderPointerPosition), ProjectionUnavailable> {
    let (pointer, _) = project_widget_hit(
        projection,
        camera,
        captured_target,
        viewport_position,
        panel,
        state,
        slots,
    )?;
    let geometry = SliderTravel::new(state, slots)?;
    let travel =
        directed_thumb_travel(geometry.content, geometry.thumb_extent, geometry.direction)?;
    let coordinate = match geometry.direction {
        SliderDirection::LeftToRight | SliderDirection::RightToLeft => pointer.x,
        SliderDirection::BottomToTop | SliderDirection::TopToBottom => pointer.y,
    };
    let fraction = (coordinate - travel.start) / (travel.end - travel.start);
    if !fraction.is_finite() {
        return Err(ProjectionUnavailable);
    }
    Ok((
        pointer,
        SliderPointerPosition {
            fraction,
            range: geometry.range,
        },
    ))
}

/// Layout-frame translation delta — layout points with Y increasing downward —
/// that moves the marked thumb so its center sits at the applied value.
/// [`SlotTranslationUnavailable::Unmarked`] means a required slot solved no box;
/// [`SlotTranslationUnavailable::Degenerate`] means invalid active-axis geometry.
///
/// The delta stays in layout coordinates so it shares the solved slot geometry
/// with pointer projection; [`present_slider_state`] converts it once through
/// the owning panel to the panel-local render frame
/// [`VisualSlotOverride::offset`] is consumed in.
///
/// A finite oversized thumb — active-axis extent at or beyond the content extent
/// — centers on the content box's active axis; every other finite case solves
/// the value's center from the same directed endpoint interval pointer
/// projection uses, then subtracts the thumb's solved authored center. The
/// returned delta preserves the thumb's cross axis and authored draw depth.
pub(super) fn thumb_translation(
    slots: &WidgetVisualSlots,
    state: &SliderState,
) -> Result<Vec2, SlotTranslationUnavailable> {
    slot_translation(slots, state, VisualSlotId::SLIDER_THUMB, state.value())
}

/// Fraction of the fill border box from its directed leading edge to the
/// rendered thumb center in the layout frame.
pub(super) fn fill_extent_fraction(
    slots: &WidgetVisualSlots,
    state: &SliderState,
) -> Result<f32, SlotTranslationUnavailable> {
    fill_extent_fraction_at_value(slots, state, VisualSlotId::SLIDER_FILL, state.value())
}

/// Places a value on a surface using the thumb's solved travel, rather than
/// assuming the track and thumb have identical endpoints.
pub(super) fn fill_extent_fraction_at_value(
    slots: &WidgetVisualSlots,
    state: &SliderState,
    surface: VisualSlotId,
    value: f32,
) -> Result<f32, SlotTranslationUnavailable> {
    let fill = slots
        .border_box(surface)
        .ok_or(SlotTranslationUnavailable::Unmarked)?;
    let thumb = slots
        .border_box(VisualSlotId::SLIDER_THUMB)
        .ok_or(SlotTranslationUnavailable::Unmarked)?;
    let (center_x, center_y) = thumb.center();
    let translated_center = Vec2::new(center_x, center_y)
        + slot_translation(slots, state, VisualSlotId::SLIDER_THUMB, value)?;
    let (distance, length) = match state.direction() {
        SliderDirection::LeftToRight => (translated_center.x - fill.x, fill.width),
        SliderDirection::RightToLeft => (fill.x + fill.width - translated_center.x, fill.width),
        SliderDirection::BottomToTop => (fill.y + fill.height - translated_center.y, fill.height),
        SliderDirection::TopToBottom => (translated_center.y - fill.y, fill.height),
    };
    if !distance.is_finite() || !length.is_finite() || length <= 0.0 {
        return Err(SlotTranslationUnavailable::Degenerate);
    }
    Ok((distance / length).clamp(0.0, 1.0))
}

pub(super) fn slot_translation(
    slots: &WidgetVisualSlots,
    state: &SliderState,
    slot: VisualSlotId,
    value: f32,
) -> Result<Vec2, SlotTranslationUnavailable> {
    let content = slots
        .content_box(VisualSlotId::SLIDER_ROOT)
        .ok_or(SlotTranslationUnavailable::Unmarked)?;
    let thumb = slots
        .border_box(slot)
        .ok_or(SlotTranslationUnavailable::Unmarked)?;
    let travel_thumb = travel_thumb_bounds(slots)?;
    let direction = state.direction();
    let range = state.range();
    let (thumb_extent, authored_center, axis_start, axis_extent) = match direction {
        SliderDirection::LeftToRight | SliderDirection::RightToLeft => (
            travel_thumb.width,
            thumb.center().0,
            content.x,
            content.width,
        ),
        SliderDirection::BottomToTop | SliderDirection::TopToBottom => (
            travel_thumb.height,
            thumb.center().1,
            content.y,
            content.height,
        ),
    };
    // Only finite `T >= C` geometry centers the thumb. Non-finite
    // active-axis geometry, a zero-extent content box, and a non-finite or
    // negative thumb extent stay unavailable and manufacture no translation, so
    // that `directed_thumb_travel`'s only remaining error is the oversized-thumb
    // case handled below.
    if !axis_start.is_finite() || !axis_extent.is_finite() || axis_extent <= 0.0 {
        return Err(SlotTranslationUnavailable::Degenerate);
    }
    if !thumb_extent.is_finite() || thumb_extent < 0.0 {
        return Err(SlotTranslationUnavailable::Degenerate);
    }
    let content_center = axis_extent.mul_add(0.5, axis_start);
    let fraction = range.value_to_fraction(value).clamp(0.0, 1.0);
    let desired_center =
        match directed_thumb_travel(content, ThumbExtent::Marked(thumb_extent), direction) {
            Ok(travel) => fraction.mul_add(travel.end - travel.start, travel.start),
            Err(ProjectionUnavailable) => content_center,
        };
    let delta = desired_center - authored_center;
    if !delta.is_finite() {
        return Err(SlotTranslationUnavailable::Degenerate);
    }
    Ok(match direction {
        SliderDirection::LeftToRight | SliderDirection::RightToLeft => Vec2::new(delta, 0.0),
        SliderDirection::BottomToTop | SliderDirection::TopToBottom => Vec2::new(0.0, delta),
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::prelude::Vec2;

    use super::super::*;
    use super::*;
    use crate::layout::BoundingBox;

    fn range(start: f32, end: f32) -> SliderRange {
        SliderRange::new(start, end).expect("range should validate")
    }

    fn step(interval: f32) -> SliderStep {
        SliderStep::new(interval).expect("step should validate")
    }

    fn content_box(x: f32, y: f32, width: f32, height: f32) -> BoundingBox {
        BoundingBox {
            x,
            y,
            width,
            height,
        }
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn logarithmic_slider_equal_pointer_distances_propose_equal_value_ratios() {
        let content = content_box(10.0, 20.0, 40.0, 16.0);
        let range = SliderRange::logarithmic(0.01, 10.0).expect("positive log range");
        for direction in [
            SliderDirection::LeftToRight,
            SliderDirection::RightToLeft,
            SliderDirection::TopToBottom,
            SliderDirection::BottomToTop,
        ] {
            let project = |fraction: f32| {
                let point = match direction {
                    SliderDirection::LeftToRight => {
                        Vec2::new(32.0_f32.mul_add(fraction, 14.0), 0.0)
                    },
                    SliderDirection::RightToLeft => {
                        Vec2::new((-32.0_f32).mul_add(fraction, 46.0), 0.0)
                    },
                    SliderDirection::TopToBottom => Vec2::new(0.0, 8.0_f32.mul_add(fraction, 24.0)),
                    SliderDirection::BottomToTop => {
                        Vec2::new(0.0, (-8.0_f32).mul_add(fraction, 32.0))
                    },
                };
                project_pointer_value(point, content, ThumbExtent::Marked(8.0), range, direction)
                    .expect("pointer inside nondegenerate travel")
            };
            for (fraction, value) in [(0.0, 0.01), (1.0 / 3.0, 0.1), (2.0 / 3.0, 1.0), (1.0, 10.0)]
            {
                assert!(
                    (project(fraction) / value - 1.0).abs() < 1e-5,
                    "{direction:?}: each decade occupies one third of the travel"
                );
            }
            let low_ratio = project(0.3) / project(0.2);
            let high_ratio = project(0.7) / project(0.6);
            assert!(
                (low_ratio / high_ratio - 1.0).abs() < 1e-5,
                "{direction:?}: equal pointer travel has equal ratios"
            );
            assert!((low_ratio / 1000.0_f32.powf(0.1) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn thumb_center_endpoints_project_exact_range_endpoints_in_all_directions() {
        // Content box (10, 20) sized 40x16 with an 8-point thumb: the thumb
        // center travels x 14..46 horizontally and y 24..32 vertically. The
        // cross-axis pointer coordinate is ignored.
        let content = content_box(10.0, 20.0, 40.0, 16.0);
        let projection_range = range(0.25, 0.75);
        let cases = [
            (
                SliderDirection::LeftToRight,
                Vec2::new(14.0, -3.0),
                Vec2::new(46.0, 99.0),
            ),
            (
                SliderDirection::RightToLeft,
                Vec2::new(46.0, -3.0),
                Vec2::new(14.0, 99.0),
            ),
            (
                SliderDirection::TopToBottom,
                Vec2::new(-3.0, 24.0),
                Vec2::new(99.0, 32.0),
            ),
            (
                SliderDirection::BottomToTop,
                Vec2::new(-3.0, 32.0),
                Vec2::new(99.0, 24.0),
            ),
        ];

        for (direction, start_center, end_center) in cases {
            let at_start = project_pointer_value(
                start_center,
                content,
                ThumbExtent::Marked(8.0),
                projection_range,
                direction,
            );
            assert_eq!(
                at_start.map(f32::to_bits),
                Ok(projection_range.start().to_bits()),
                "{direction:?} range-start endpoint",
            );
            let at_end = project_pointer_value(
                end_center,
                content,
                ThumbExtent::Marked(8.0),
                projection_range,
                direction,
            );
            assert_eq!(
                at_end.map(f32::to_bits),
                Ok(projection_range.end().to_bits()),
                "{direction:?} range-end endpoint",
            );
        }
    }

    #[test]
    fn pointer_outside_directed_travel_clamps_to_range_endpoints() {
        let content = content_box(10.0, 20.0, 40.0, 16.0);
        let projection_range = range(-2.0, 6.0);

        let before_start = project_pointer_value(
            Vec2::new(-100.0, 0.0),
            content,
            ThumbExtent::Marked(8.0),
            projection_range,
            SliderDirection::LeftToRight,
        );
        assert_eq!(
            before_start.map(f32::to_bits),
            Ok(projection_range.start().to_bits()),
        );
        let past_end = project_pointer_value(
            Vec2::new(500.0, 0.0),
            content,
            ThumbExtent::Marked(8.0),
            projection_range,
            SliderDirection::LeftToRight,
        );
        assert_eq!(
            past_end.map(f32::to_bits),
            Ok(projection_range.end().to_bits()),
        );

        // A reversed direction clamps the same way: below the content box is
        // before the range start for BottomToTop.
        let reversed_before = project_pointer_value(
            Vec2::new(0.0, 500.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::BottomToTop,
        );
        assert_eq!(
            reversed_before.map(f32::to_bits),
            Ok(projection_range.start().to_bits()),
        );
        let reversed_past = project_pointer_value(
            Vec2::new(0.0, -500.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::BottomToTop,
        );
        assert_eq!(
            reversed_past.map(f32::to_bits),
            Ok(projection_range.end().to_bits()),
        );
    }

    #[test]
    fn headless_projection_travels_the_full_directed_content_extent() {
        let content = content_box(10.0, 20.0, 40.0, 16.0);
        let projection_range = range(0.0, 8.0);

        let at_left = project_pointer_value(
            Vec2::new(10.0, 0.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::LeftToRight,
        );
        assert_eq!(
            at_left.map(f32::to_bits),
            Ok(projection_range.start().to_bits()),
        );
        let at_right = project_pointer_value(
            Vec2::new(50.0, 0.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::LeftToRight,
        );
        assert_eq!(
            at_right.map(f32::to_bits),
            Ok(projection_range.end().to_bits()),
        );
        let at_quarter = project_pointer_value(
            Vec2::new(20.0, 0.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::LeftToRight,
        )
        .expect("interior positions should project");
        assert_close(at_quarter, 2.0);
    }

    #[test]
    fn off_lattice_slider_targets_stay_raw_until_proposal_acceptance() {
        // The helper takes no step: an interior pointer projects the raw
        // interpolant, and only application acceptance through
        // `SliderState::set_proposed_value` snaps it onto the lattice.
        let content = content_box(0.0, 0.0, 100.0, 10.0);
        let projection_range = range(0.0, 1.0);
        let raw = project_pointer_value(
            Vec2::new(33.0, 0.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::LeftToRight,
        )
        .expect("interior positions should project");
        assert_close(raw, 0.33);

        let mut state = SliderState::new(
            projection_range,
            0.0,
            SliderStepping::Stepped(step(0.25)),
            SliderDirection::LeftToRight,
        )
        .expect("state should validate");
        assert!(
            state.set_proposed_value(
                SliderProposalValue::new(raw).expect("finite projected target")
            )
        );
        assert_close(state.value(), 0.25);
    }

    #[test]
    fn projection_reads_the_border_and_padding_excluded_content_box() {
        // Synthetic slider root at (0, 0) sized 60x20 with an asymmetric
        // border (4 left, 2 right) and 3-point padding; the caller passes the
        // remaining content box.
        let root_width = 60.0;
        let border_left = 4.0;
        let border_right = 2.0;
        let padding = 3.0;
        let content_left = border_left + padding;
        let content_width = root_width - content_left - (border_right + padding);
        let content = content_box(content_left, 5.0, content_width, 10.0);
        let projection_range = range(0.0, 1.0);

        // The content box's left edge is the exact range-start endpoint; the
        // root's left edge lies outside the interval and clamps to the same
        // value.
        let at_content_edge = project_pointer_value(
            Vec2::new(content_left, 10.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::LeftToRight,
        );
        assert_eq!(
            at_content_edge.map(f32::to_bits),
            Ok(projection_range.start().to_bits()),
        );
        let at_root_edge = project_pointer_value(
            Vec2::new(0.0, 10.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::LeftToRight,
        );
        assert_eq!(
            at_root_edge.map(f32::to_bits),
            Ok(projection_range.start().to_bits()),
        );

        // The interval midpoint is the content box's center, not the root
        // box's center: the asymmetric border shifts them apart.
        let (content_center_x, _) = content.center();
        let at_content_center = project_pointer_value(
            Vec2::new(content_center_x, 10.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::LeftToRight,
        )
        .expect("interior positions should project");
        assert_close(at_content_center, 0.5);
        let at_root_center = project_pointer_value(
            Vec2::new(root_width * 0.5, 10.0),
            content,
            ThumbExtent::Headless,
            projection_range,
            SliderDirection::LeftToRight,
        )
        .expect("interior positions should project");
        assert_close(
            at_root_center,
            f32::mul_add(root_width, 0.5, -content_left) / content_width,
        );
    }

    #[test]
    fn empty_directed_travel_is_unavailable() {
        let projection_range = range(0.0, 1.0);

        // Zero active-axis content extent for a headless slider.
        let zero_width = content_box(10.0, 20.0, 0.0, 16.0);
        assert_eq!(
            project_pointer_value(
                Vec2::new(10.0, 0.0),
                zero_width,
                ThumbExtent::Headless,
                projection_range,
                SliderDirection::LeftToRight,
            ),
            Err(ProjectionUnavailable),
        );
        let zero_height = content_box(10.0, 20.0, 40.0, 0.0);
        assert_eq!(
            project_pointer_value(
                Vec2::new(0.0, 20.0),
                zero_height,
                ThumbExtent::Headless,
                projection_range,
                SliderDirection::TopToBottom,
            ),
            Err(ProjectionUnavailable),
        );

        // A marked thumb meeting or exceeding the content extent has no
        // visible travel.
        let content = content_box(10.0, 20.0, 40.0, 16.0);
        for oversized_thumb in [40.0, 64.0] {
            assert_eq!(
                project_pointer_value(
                    Vec2::new(30.0, 0.0),
                    content,
                    ThumbExtent::Marked(oversized_thumb),
                    projection_range,
                    SliderDirection::LeftToRight,
                ),
                Err(ProjectionUnavailable),
            );
        }
        assert_eq!(
            project_pointer_value(
                Vec2::new(0.0, 28.0),
                content,
                ThumbExtent::Marked(16.0),
                projection_range,
                SliderDirection::BottomToTop,
            ),
            Err(ProjectionUnavailable),
        );
    }

    #[test]
    fn non_finite_pointer_projects_no_raw_target() {
        let content = content_box(0.0, 0.0, 100.0, 10.0);
        let projection_range = range(0.0, 1.0);
        assert_eq!(
            project_pointer_value(
                Vec2::new(f32::NAN, 0.0),
                content,
                ThumbExtent::Headless,
                projection_range,
                SliderDirection::LeftToRight,
            ),
            Err(ProjectionUnavailable),
        );
    }

    #[test]
    fn negative_or_non_finite_thumb_extent_is_unavailable() {
        let content = content_box(10.0, 20.0, 40.0, 16.0);
        let projection_range = range(0.0, 1.0);
        for thumb_extent in [-8.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                project_pointer_value(
                    Vec2::new(30.0, 0.0),
                    content,
                    ThumbExtent::Marked(thumb_extent),
                    projection_range,
                    SliderDirection::LeftToRight,
                ),
                Err(ProjectionUnavailable),
                "thumb extent {thumb_extent}",
            );
        }
    }

    #[test]
    fn non_finite_active_axis_geometry_is_unavailable() {
        let projection_range = range(0.0, 1.0);
        let cases = [
            (
                content_box(f32::NAN, 20.0, 40.0, 16.0),
                SliderDirection::LeftToRight,
            ),
            (
                content_box(10.0, 20.0, f32::INFINITY, 16.0),
                SliderDirection::LeftToRight,
            ),
            (
                content_box(10.0, f32::NEG_INFINITY, 40.0, 16.0),
                SliderDirection::TopToBottom,
            ),
            (
                content_box(10.0, 20.0, 40.0, f32::NAN),
                SliderDirection::BottomToTop,
            ),
        ];
        for (content, direction) in cases {
            assert_eq!(
                project_pointer_value(
                    Vec2::new(30.0, 25.0),
                    content,
                    ThumbExtent::Headless,
                    projection_range,
                    direction,
                ),
                Err(ProjectionUnavailable),
            );
        }
    }

    // ---- pointer capture lifecycle ----
}
