//! Analytic path conversion for resolved panel-line primitives.

use std::f32::consts::TAU;

use bevy::math::Vec2;
use bevy_kana::ToF32;

use crate::layout::BoundingBox;
use crate::layout::PanelShapePrimitiveGeometry;
use crate::layout::PanelShapePrimitiveKind;
use crate::layout::ResolvedPanelShapePrimitive;
use crate::layout::Unit;
use crate::render::Bounds;
use crate::render::PathContour;
use crate::render::PathOutline;
use crate::render::QuadraticSegment;

const CIRCLE_SEGMENT_COUNT: usize = 8;
const MIN_EXTENT: f32 = f32::EPSILON;
const PANEL_LINE_REFERENCE_EM_POINTS: f32 = 8.0;
const PANEL_LINE_DESIGN_UNITS_PER_EM: f32 = 1000.0;
const PANEL_LINE_REFERENCE_DESIGN_UNITS_PER_METER: f32 =
    Unit::Meters.to_points() * (PANEL_LINE_DESIGN_UNITS_PER_EM / PANEL_LINE_REFERENCE_EM_POINTS);
const PANEL_LINE_MIN_STROKE_DESIGN_UNITS: f32 = 96.0;
const PANEL_LINE_PADDING_DESIGN_UNITS: f32 = 16.0;
/// World-unit floor for the instance-quad padding — the head-on baseline fringe
/// around every panel-line quad. The screen-space coverage ramp at a grazing
/// tilt (its world width grows ~1/cos(grazing), which a fixed-world pad would
/// clip into a staircase) is handled separately and camera-correctly by the
/// vertex stage: it expands each line corner by a fixed device-pixel margin
/// (see `LINE_AA_MARGIN_PX` in `analytic_path_vertex_pull.wgsl`). That expansion
/// runs only in the main pass, so this floor still covers the prepass/shadow
/// quad and the design-unit proportional term for thick strokes. It inflates
/// the transparent quad of every panel line (those fragments cover no stroke and
/// `alpha_discard` before OIT, so the cost is fill, not OIT slots).
const PANEL_LINE_AA_PADDING_WORLD: f32 = 0.01;

/// Panel-local conversion values shared by every primitive from one panel.
pub(super) struct PanelShapePathContext {
    pub points_to_world: f32,
    pub anchor_x:        f32,
    pub anchor_y:        f32,
}

/// Renderer-owned path data and the clipped instance quad for one primitive.
pub(super) struct PanelShapePath {
    pub outline:     PathOutline,
    pub rect_min:    Vec2,
    pub rect_size:   Vec2,
    pub uv_min:      Vec2,
    pub uv_size:     Vec2,
    pub box_uv_min:  Vec2,
    pub box_uv_size: Vec2,
}

/// One merge-group member: a resolved primitive plus its resolved hairline
/// fade exponent (line override else element/panel resolution, encoded by
/// [`HairlineFade::fade_exponent`](crate::HairlineFade::fade_exponent)).
pub(super) struct PanelShapeMember<'a> {
    pub primitive:     &'a ResolvedPanelShapePrimitive,
    pub fade_exponent: f32,
}

/// One member's panel-space contour with its per-contour packing inputs.
struct MemberContour {
    segments:      Vec<QuadraticSegment>,
    /// World-unit narrowest stroke (hairline dilation input).
    min_feature:   f32,
    fade_exponent: f32,
}

/// Converts one mergeable group of resolved panel-line primitives (same
/// color, clip, and layering) into a single multi-contour filled analytic
/// path. One winding field covers the whole group, so abutting or
/// overlapping members — a tick butted against a ruler spine — render as one
/// silhouette with no anti-aliasing junction line between them. Each contour
/// keeps its own stroke width for per-curve hairline dilation and its own
/// fade exponent, so fading and non-fading members coexist in the merged
/// path.
pub(super) fn build_panel_shape_path(
    members: &[PanelShapeMember<'_>],
    owner_bounds: BoundingBox,
    clip: Option<BoundingBox>,
    context: &PanelShapePathContext,
) -> Option<PanelShapePath> {
    let contours: Vec<MemberContour> = members
        .iter()
        .filter_map(|member| member_contour(member, context))
        .collect();
    let source_bounds = segment_bounds(contours.iter().flat_map(|contour| &contour.segments))?;
    let translation = PathTranslation::new(source_bounds);
    let (rect_min, rect_size, source_uv_min, source_uv_size) =
        clipped_instance(translation, clip, owner_bounds, context)?;
    let outline = local_design_outline(contours, translation);
    Some(PanelShapePath {
        outline,
        rect_min,
        rect_size,
        uv_min: source_uv_min,
        uv_size: source_uv_size,
        box_uv_min: source_uv_min,
        box_uv_size: source_uv_size,
    })
}

/// One member's panel-space contour, world-unit narrowest stroke, and fade
/// exponent.
fn member_contour(
    member: &PanelShapeMember<'_>,
    context: &PanelShapePathContext,
) -> Option<MemberContour> {
    let (segments, min_feature) = match member.primitive.geometry() {
        PanelShapePrimitiveGeometry::Segment { start, end, width } => (
            segment_contour(start, end, width, context)?,
            width * context.points_to_world,
        ),
        PanelShapePrimitiveGeometry::Form {
            center,
            axis,
            half_size,
        } => {
            let world_half_size = half_size * context.points_to_world;
            (
                form_contour(
                    member.primitive.kind(),
                    center,
                    axis,
                    world_half_size,
                    context,
                )?,
                2.0 * world_half_size.x.min(world_half_size.y),
            )
        },
    };
    Some(MemberContour {
        segments,
        min_feature,
        fade_exponent: member.fade_exponent,
    })
}

fn segment_contour(
    start: Vec2,
    end: Vec2,
    width: f32,
    context: &PanelShapePathContext,
) -> Option<Vec<QuadraticSegment>> {
    let start = layout_point_to_panel(start, context);
    let end = layout_point_to_panel(end, context);
    let axis = (end - start).try_normalize()?;
    let half_width = width * context.points_to_world * 0.5;
    if half_width <= 0.0 {
        return None;
    }
    let normal = perp(axis) * half_width;
    Some(polygon_segments(&[
        start - normal,
        end - normal,
        end + normal,
        start + normal,
    ]))
}

fn form_contour(
    kind: PanelShapePrimitiveKind,
    center: Vec2,
    axis: Vec2,
    half_size: Vec2,
    context: &PanelShapePathContext,
) -> Option<Vec<QuadraticSegment>> {
    if half_size.x <= 0.0 || half_size.y <= 0.0 {
        return None;
    }
    let center = layout_point_to_panel(center, context);
    let axis = layout_axis_to_panel(axis)?;
    let normal = perp(axis);
    match kind {
        PanelShapePrimitiveKind::Segment => None,
        PanelShapePrimitiveKind::Triangle => Some(polygon_segments(&[
            center + axis * half_size.x,
            center - axis * half_size.x + normal * half_size.y,
            center - axis * half_size.x - normal * half_size.y,
        ])),
        PanelShapePrimitiveKind::Circle => Some(ellipse_segments(center, axis, normal, half_size)),
        PanelShapePrimitiveKind::Square => Some(polygon_segments(&[
            center - axis * half_size.x - normal * half_size.y,
            center + axis * half_size.x - normal * half_size.y,
            center + axis * half_size.x + normal * half_size.y,
            center - axis * half_size.x + normal * half_size.y,
        ])),
        PanelShapePrimitiveKind::Diamond => Some(polygon_segments(&[
            center + axis * half_size.x,
            center + normal * half_size.y,
            center - axis * half_size.x,
            center - normal * half_size.y,
        ])),
    }
}

fn polygon_segments(points: &[Vec2]) -> Vec<QuadraticSegment> {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .map(line_segment)
        .collect()
}

fn ellipse_segments(
    center: Vec2,
    axis: Vec2,
    normal: Vec2,
    half_size: Vec2,
) -> Vec<QuadraticSegment> {
    let step = TAU / CIRCLE_SEGMENT_COUNT.to_f32();
    let control_scale = (step * 0.5).cos().recip();
    let ellipse_point = |angle: f32, scale: f32| {
        center
            + axis * (angle.cos() * half_size.x * scale)
            + normal * (angle.sin() * half_size.y * scale)
    };

    (0..CIRCLE_SEGMENT_COUNT)
        .map(|index| {
            let start_angle = index.to_f32() * step;
            let end_angle = start_angle + step;
            let control_angle = step.mul_add(0.5, start_angle);
            QuadraticSegment {
                start:   ellipse_point(start_angle, 1.0),
                control: ellipse_point(control_angle, control_scale),
                end:     ellipse_point(end_angle, 1.0),
            }
        })
        .collect()
}

fn line_segment((start, end): (Vec2, Vec2)) -> QuadraticSegment {
    QuadraticSegment {
        start,
        control: (start + end) * 0.5,
        end,
    }
}

fn local_design_outline(contours: Vec<MemberContour>, translation: PathTranslation) -> PathOutline {
    let contours = contours
        .into_iter()
        .map(|mut contour| {
            for segment in &mut contour.segments {
                segment.start = translation.to_design(segment.start);
                segment.control = translation.to_design(segment.control);
                segment.end = translation.to_design(segment.end);
            }
            PathContour {
                segments:      contour.segments,
                min_feature:   contour.min_feature * translation.design_units_per_world,
                fade_exponent: contour.fade_exponent,
            }
        })
        .collect();
    PathOutline {
        bounds: Bounds {
            min: Vec2::ZERO,
            max: translation.source_size() * translation.design_units_per_world,
        },
        contours,
    }
}

#[derive(Clone, Copy)]
struct PathTranslation {
    source_bounds:          Bounds,
    padded_bounds:          Bounds,
    design_units_per_world: f32,
}

impl PathTranslation {
    fn new(source_bounds: Bounds) -> Self {
        let design_units_per_world = design_units_per_world(source_bounds);
        Self {
            source_bounds,
            padded_bounds: padded_bounds(source_bounds, design_units_per_world),
            design_units_per_world,
        }
    }

    fn source_size(self) -> Vec2 { self.source_bounds.max - self.source_bounds.min }

    fn to_design(self, point: Vec2) -> Vec2 {
        (point - self.source_bounds.min) * self.design_units_per_world
    }
}

fn design_units_per_world(source_bounds: Bounds) -> f32 {
    let source_size = source_bounds.max - source_bounds.min;
    let min_feature = source_size.x.min(source_size.y);
    if min_feature <= MIN_EXTENT {
        return PANEL_LINE_REFERENCE_DESIGN_UNITS_PER_METER;
    }
    PANEL_LINE_REFERENCE_DESIGN_UNITS_PER_METER
        .max(PANEL_LINE_MIN_STROKE_DESIGN_UNITS / min_feature)
}

fn segment_bounds<'a, I>(segments: I) -> Option<Bounds>
where
    I: IntoIterator<Item = &'a QuadraticSegment>,
{
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let mut any = false;
    for segment in segments {
        for point in [segment.start, segment.control, segment.end] {
            min = min.min(point);
            max = max.max(point);
            any = true;
        }
    }
    any.then_some(Bounds { min, max })
}

/// Quad padding around the path in world units. The design-unit term keeps the
/// proportional fringe for thick strokes; `PANEL_LINE_AA_PADDING_WORLD` floors
/// it so a foreshortened screen-space AA ramp stays inside the quad instead of
/// clipping to a hard edge (see the constant). `SDF_AA_PADDING` is subsumed by
/// the larger floor.
fn world_padding(design_units_per_world: f32) -> f32 {
    (PANEL_LINE_PADDING_DESIGN_UNITS / design_units_per_world).max(PANEL_LINE_AA_PADDING_WORLD)
}

fn padded_bounds(bounds: Bounds, design_units_per_world: f32) -> Bounds {
    let padding = world_padding(design_units_per_world);
    Bounds {
        min: bounds.min - Vec2::splat(padding),
        max: bounds.max + Vec2::splat(padding),
    }
}

fn clipped_instance(
    translation: PathTranslation,
    clip: Option<BoundingBox>,
    owner_bounds: BoundingBox,
    context: &PanelShapePathContext,
) -> Option<(Vec2, Vec2, Vec2, Vec2)> {
    let mut rect = PanelRect {
        min: translation.padded_bounds.min,
        max: translation.padded_bounds.max,
    };
    if let Some(clip) = clip {
        rect = rect.intersect(inflated_clip_rect(clip, owner_bounds, translation, context))?;
    }
    let source_size = translation.source_size();
    if rect.width() <= MIN_EXTENT
        || rect.height() <= MIN_EXTENT
        || source_size.x <= MIN_EXTENT
        || source_size.y <= MIN_EXTENT
    {
        return None;
    }

    let uv_left = (rect.min.x - translation.source_bounds.min.x) / source_size.x;
    let uv_right = (rect.max.x - translation.source_bounds.min.x) / source_size.x;
    let uv_top = (translation.source_bounds.max.y - rect.max.y) / source_size.y;
    let uv_bottom = (translation.source_bounds.max.y - rect.min.y) / source_size.y;
    Some((
        rect.min,
        rect.max - rect.min,
        Vec2::new(uv_left, uv_top),
        Vec2::new(uv_right - uv_left, uv_bottom - uv_top),
    ))
}

/// Layout-point tolerance for deciding whether a clip edge came from the
/// owner element. `BoundingBox::intersect` recomputes width/height, so a
/// far edge that the owner contributed can drift by float rounding.
const CLIP_EDGE_EPSILON: f32 = 0.001;

/// Converts the resolved clip to panel space, granting AA fringe room on the
/// edges the owner element contributed. `PanelShapeClipPolicy::OwnerBounds`
/// clips every draw line to its owner element, and a tightly-fitting element
/// (the ruler spine's 0.2mm column) leaves the instance quad no padding — the
/// anti-aliasing ramp is cut at the stroke edge and subpixel strokes stop
/// covering pixel centers. Edges contributed by an inherited (scroll/panel)
/// clip stay exact.
fn inflated_clip_rect(
    clip: BoundingBox,
    owner_bounds: BoundingBox,
    translation: PathTranslation,
    context: &PanelShapePathContext,
) -> PanelRect {
    let mut rect = clip_rect_to_panel(clip, context);
    let padding = world_padding(translation.design_units_per_world);
    let owner_edge =
        |clip_edge: f32, owner_edge: f32| (clip_edge - owner_edge).abs() <= CLIP_EDGE_EPSILON;

    if owner_edge(clip.x, owner_bounds.x) {
        rect.min.x -= padding;
    }
    if owner_edge(clip.x + clip.width, owner_bounds.x + owner_bounds.width) {
        rect.max.x += padding;
    }
    // Layout y grows down, panel y grows up: layout top edge is panel max.y.
    if owner_edge(clip.y, owner_bounds.y) {
        rect.max.y += padding;
    }
    if owner_edge(clip.y + clip.height, owner_bounds.y + owner_bounds.height) {
        rect.min.y -= padding;
    }
    rect
}

fn clip_rect_to_panel(clip: BoundingBox, context: &PanelShapePathContext) -> PanelRect {
    let left = clip.x.mul_add(context.points_to_world, -context.anchor_x);
    let right = (clip.x + clip.width).mul_add(context.points_to_world, -context.anchor_x);
    let top = -(clip.y.mul_add(context.points_to_world, -context.anchor_y));
    let bottom = -((clip.y + clip.height).mul_add(context.points_to_world, -context.anchor_y));
    PanelRect {
        min: Vec2::new(left.min(right), bottom.min(top)),
        max: Vec2::new(left.max(right), bottom.max(top)),
    }
}

fn layout_point_to_panel(point: Vec2, context: &PanelShapePathContext) -> Vec2 {
    Vec2::new(
        point.x.mul_add(context.points_to_world, -context.anchor_x),
        -(point.y.mul_add(context.points_to_world, -context.anchor_y)),
    )
}

fn layout_axis_to_panel(axis: Vec2) -> Option<Vec2> { Vec2::new(axis.x, -axis.y).try_normalize() }

const fn perp(axis: Vec2) -> Vec2 { Vec2::new(-axis.y, axis.x) }

#[derive(Clone, Copy)]
struct PanelRect {
    min: Vec2,
    max: Vec2,
}

impl PanelRect {
    fn width(self) -> f32 { self.max.x - self.min.x }

    fn height(self) -> f32 { self.max.y - self.min.y }

    fn intersect(self, other: Self) -> Option<Self> {
        let min = self.min.max(other.min);
        let max = self.max.min(other.max);
        (max.x > min.x && max.y > min.y).then_some(Self { min, max })
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::sync::Arc;

    use bevy::color::Color;
    use bevy_kana::ToF32;

    use super::*;
    use crate::cascade::Cascade;
    use crate::layout::El;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutEngine;
    use crate::layout::PanelDraw;
    use crate::layout::PanelLine;
    use crate::layout::PanelPoint;
    use crate::layout::PanelShapePrimitiveKey;
    use crate::layout::PanelShapeSourceKey;
    use crate::layout::RenderCommandKind;
    use crate::layout::Sizing;
    use crate::layout::TextDimensions;
    use crate::layout::Unit;
    use crate::render::CurveRecord;

    const TEST_VEC2_EPSILON_SQUARED: f32 = 0.000_001;

    const fn member(primitive: &ResolvedPanelShapePrimitive) -> PanelShapeMember<'_> {
        PanelShapeMember {
            primitive,
            fade_exponent: 0.0,
        }
    }

    #[test]
    fn segment_emits_closed_rectangle_with_midpoint_quadratics() {
        let primitive = segment_primitive(None);
        let Some(path) = build_panel_shape_path(
            &[member(&primitive)],
            wide_owner_bounds(),
            None,
            &test_context(),
        ) else {
            panic!("segment should build a path");
        };

        let contour = &path.outline.contours[0];
        let scale = PANEL_LINE_REFERENCE_DESIGN_UNITS_PER_METER;

        assert_eq!(contour.segments.len(), 4);
        assert_eq!(contour.segments[0].start, Vec2::ZERO);
        assert_eq!(contour.segments[0].control, Vec2::new(5.0, 0.0) * scale);
        assert_eq!(contour.segments[0].end, Vec2::new(10.0, 0.0) * scale);
        let padding = (PANEL_LINE_PADDING_DESIGN_UNITS / scale).max(PANEL_LINE_AA_PADDING_WORLD);
        assert_vec2_near(path.rect_min, Vec2::new(-padding, -1.0 - padding));
        assert_vec2_near(
            path.rect_size,
            Vec2::new(padding.mul_add(2.0, 10.0), padding.mul_add(2.0, 2.0)),
        );
        assert_vec2_near(path.outline.bounds.max / scale, Vec2::new(10.0, 2.0));
        assert!(path.uv_min.x < 0.0);
        assert!(path.uv_min.y < 0.0);
        assert!(path.uv_size.x > 1.0);
        assert!(path.uv_size.y > 1.0);
    }

    #[test]
    fn clipping_trims_instance_rect_and_uvs_without_changing_outline() {
        let clip = BoundingBox {
            x:      2.0,
            y:      -1.0,
            width:  4.0,
            height: 2.0,
        };
        let primitive = segment_primitive(Some(clip));
        let Some(path) = build_panel_shape_path(
            &[member(&primitive)],
            wide_owner_bounds(),
            Some(clip),
            &test_context(),
        ) else {
            panic!("clipped segment should build a path");
        };

        assert_eq!(path.rect_min, Vec2::new(2.0, -1.0));
        assert_eq!(path.rect_size, Vec2::new(4.0, 2.0));
        assert!(path.uv_min.x > 0.0);
        assert!(path.uv_size.x < 1.0);
        assert_eq!(path.outline.contours[0].segments.len(), 4);
    }

    /// A clip whose edges come from the owner element keeps the quad's AA
    /// padding: the spine's tightly-fitting column must not cut the
    /// anti-aliasing ramp at the stroke edge.
    #[test]
    fn owner_edge_clip_keeps_aa_padding() {
        let clip = BoundingBox {
            x:      2.0,
            y:      -1.0,
            width:  4.0,
            height: 2.0,
        };
        let primitive = segment_primitive(Some(clip));
        let Some(path) =
            build_panel_shape_path(&[member(&primitive)], clip, Some(clip), &test_context())
        else {
            panic!("owner-clipped segment should build a path");
        };

        let scale = PANEL_LINE_REFERENCE_DESIGN_UNITS_PER_METER;
        let padding = (PANEL_LINE_PADDING_DESIGN_UNITS / scale).max(PANEL_LINE_AA_PADDING_WORLD);
        assert_vec2_near(path.rect_min, Vec2::new(2.0 - padding, -1.0 - padding));
        assert_vec2_near(
            path.rect_size,
            Vec2::new(padding.mul_add(2.0, 4.0), padding.mul_add(2.0, 2.0)),
        );
    }

    #[test]
    fn circle_form_emits_quadratic_arc_segments() {
        let primitive = ResolvedPanelShapePrimitive {
            source_key: PanelShapePrimitiveKey::new(PanelShapeSourceKey::element(0, 0, 0), 0),
            kind:       PanelShapePrimitiveKind::Circle,
            geometry:   PanelShapePrimitiveGeometry::Form {
                center:    Vec2::ZERO,
                axis:      Vec2::X,
                half_size: Vec2::new(2.0, 1.0),
            },
            color:      Color::WHITE,
            material:   Cascade::Inherit,
            bounds:     BoundingBox {
                x:      -2.0,
                y:      -1.0,
                width:  4.0,
                height: 2.0,
            },
            clip:       None,
            part_order: 0,
        };

        let Some(path) = build_panel_shape_path(
            &[member(&primitive)],
            wide_owner_bounds(),
            None,
            &test_context(),
        ) else {
            panic!("circle should build a path");
        };

        assert_eq!(
            path.outline.contours[0].segments.len(),
            CIRCLE_SEGMENT_COUNT
        );
        assert_ne!(
            path.outline.contours[0].segments[0].control,
            (path.outline.contours[0].segments[0].start + path.outline.contours[0].segments[0].end)
                * 0.5,
        );
    }

    fn segment_primitive(clip: Option<BoundingBox>) -> ResolvedPanelShapePrimitive {
        ResolvedPanelShapePrimitive {
            source_key: PanelShapePrimitiveKey::new(PanelShapeSourceKey::element(0, 0, 0), 0),
            kind: PanelShapePrimitiveKind::Segment,
            geometry: PanelShapePrimitiveGeometry::Segment {
                start: Vec2::ZERO,
                end:   Vec2::new(10.0, 0.0),
                width: 2.0,
            },
            color: Color::WHITE,
            material: Cascade::Inherit,
            bounds: BoundingBox {
                x:      0.0,
                y:      -1.0,
                width:  10.0,
                height: 2.0,
            },
            clip,
            part_order: 0,
        }
    }

    const fn test_context() -> PanelShapePathContext {
        PanelShapePathContext {
            points_to_world: 1.0,
            anchor_x:        0.0,
            anchor_y:        0.0,
        }
    }

    /// Owner bounds far outside every test clip, so no clip edge reads as
    /// owner-contributed and the exact-trim assertions stay exact.
    const fn wide_owner_bounds() -> BoundingBox {
        BoundingBox {
            x:      -1000.0,
            y:      -1000.0,
            width:  2000.0,
            height: 2000.0,
        }
    }

    /// End-to-end reproduction of the `analytic_line_probe` example's
    /// panel-route ruler: 102 lines through tree scaling, engine compute, and
    /// primitive conversion. Prints the spine's resolved extent so a
    /// conversion truncation shows up CPU-side.
    #[test]
    fn probe_example_ruler_spine_spans_full_panel_height() {
        const RULER_MARKS: i32 = 100;
        const RULER_HEIGHT_MM: f32 = 100.0;
        let spine_x = 8.0;
        let mut lines = vec![
            PanelLine::new(
                PanelPoint::new(spine_x, 0.0),
                PanelPoint::new(spine_x, RULER_HEIGHT_MM),
            )
            .width(0.2),
        ];
        for mark in 0..=RULER_MARKS {
            let (length_mm, stroke_mm) = if mark % 10 == 0 {
                (5.0, 0.3)
            } else if mark % 5 == 0 {
                (3.5, 0.1)
            } else {
                (2.0, 0.1)
            };
            let y = (RULER_MARKS - mark).to_f32();
            lines.push(
                PanelLine::new(
                    PanelPoint::new(spine_x - length_mm, y),
                    PanelPoint::new(spine_x, y),
                )
                .width(stroke_mm),
            );
        }
        let tree = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .draw(PanelDraw::lines(lines)),
        )
        .build();

        let layout_to_points = Unit::Millimeters.to_points();
        let scaled = tree.scaled(layout_to_points, layout_to_points);
        let engine = LayoutEngine::new(Arc::new(|_: &str, _: &_| TextDimensions {
            width:       0.0,
            height:      0.0,
            line_height: 0.0,
        }));
        let viewport_width = 10.0 * layout_to_points;
        let viewport_height = 100.0 * layout_to_points;
        let result = engine.compute(&scaled, viewport_width, viewport_height, 1.0);

        // world panel Mm(10) x Mm(100), anchor BottomLeft
        let world_height = 0.1_f32;
        let points_to_world = world_height / viewport_height;
        let context = PanelShapePathContext {
            points_to_world,
            anchor_x: 0.0,
            anchor_y: world_height,
        };

        let mut spine_seen = false;
        for command in &result.commands {
            let RenderCommandKind::PanelShapes { shapes: lines } = &command.kind else {
                continue;
            };
            for (line_index, line) in lines.iter().enumerate() {
                for primitive in line.primitives() {
                    let Some(path) = build_panel_shape_path(
                        &[member(primitive)],
                        line.owner_bounds(),
                        primitive.clip(),
                        &context,
                    ) else {
                        println!("line {line_index}: primitive DROPPED");
                        continue;
                    };
                    let rect_max = path.rect_min + path.rect_size;
                    if line_index == 0 {
                        spine_seen = true;
                        println!(
                            "spine: rect y {:.5}..{:.5} (m), rect x {:.5}..{:.5}, uv_min {:?} uv_size {:?}",
                            path.rect_min.y,
                            rect_max.y,
                            path.rect_min.x,
                            rect_max.x,
                            path.uv_min,
                            path.uv_size,
                        );
                        println!(
                            "spine geometry: {:?} clip {:?} bounds {:?}",
                            primitive.geometry(),
                            primitive.clip(),
                            primitive.bounds(),
                        );
                        assert!(
                            rect_max.y >= world_height - 0.0005,
                            "spine rect top {:.5} short of panel top {world_height}",
                            rect_max.y,
                        );
                        assert!(
                            path.rect_min.y <= 0.0005,
                            "spine rect bottom {:.5} short of panel bottom 0",
                            path.rect_min.y,
                        );
                    }
                }
            }
        }
        assert!(spine_seen, "spine line produced no primitives");
    }

    /// Unit scaling rounds each coordinate independently, so a
    /// midpoint-control line segment's quadratic second difference is not
    /// inherently zero; packing must snap it, or the shader's winding root
    /// catastrophically cancels and long spines lose fragments (regression:
    /// the 100mm panel-ruler spine rendered only its bottom 72.5%).
    #[test]
    fn live_conversion_segments_pack_exactly_linear() {
        for height_mm in [25.0_f32, 50.0, 75.0, 100.0] {
            let primitive = ResolvedPanelShapePrimitive {
                source_key: PanelShapePrimitiveKey::new(PanelShapeSourceKey::element(0, 0, 0), 0),
                kind:       PanelShapePrimitiveKind::Segment,
                geometry:   PanelShapePrimitiveGeometry::Segment {
                    start: Vec2::new(22.677_166, 0.0),
                    end:   Vec2::new(22.677_166, height_mm * 2.834_645_7),
                    width: 0.566_929_16,
                },
                color:      bevy::color::Color::WHITE,
                material:   Cascade::Inherit,
                bounds:     BoundingBox {
                    x:      22.0,
                    y:      0.0,
                    width:  1.0,
                    height: height_mm * 2.834_645_7,
                },
                clip:       None,
                part_order: 0,
            };
            let context = PanelShapePathContext {
                points_to_world: 0.1 / 283.464_57,
                anchor_x:        0.0,
                anchor_y:        0.1,
            };
            let Some(path) =
                build_panel_shape_path(&[member(&primitive)], wide_owner_bounds(), None, &context)
            else {
                panic!("path should build");
            };
            for segment in &path.outline.contours[0].segments {
                let record = CurveRecord::from(segment);
                assert_eq!(
                    (record.curve_end.x, record.curve_end.y),
                    (0.0, 0.0),
                    "height {height_mm}mm: segment second difference must pack to zero",
                );
            }
        }
    }

    fn assert_vec2_near(left: Vec2, right: Vec2) {
        let difference = left - right;
        assert!(
            difference.length_squared() <= TEST_VEC2_EPSILON_SQUARED,
            "left {left:?} should be near right {right:?}"
        );
    }
}
