use bevy::math::Vec2;
use ttf_parser::Face;
use ttf_parser::GlyphId;
use ttf_parser::OutlineBuilder;

use crate::render::Bounds;
use crate::render::ContourRole;
use crate::render::PathContour;
use crate::render::QuadraticSegment;
use crate::text::TextOutlineError;

const CUBIC_TO_QUADRATIC_TOLERANCE: f32 = 0.25;
const CUBIC_TO_QUADRATIC_MAX_DEPTH: u8 = 10;
const HALF: f32 = 0.5;
const ERROR_SAMPLE_A: f32 = 0.25;
const ERROR_SAMPLE_B: f32 = 0.75;
const TANGENT_INTERSECTION_EPSILON: f32 = 0.000_001;
const ISLAND_CLEARANCE_FONT_UNITS: f32 = 0.5;

/// One closed glyph contour expressed as quadratic segments.
pub(in crate::text) type Contour = PathContour;

/// Quadratic-only outline representation for one glyph.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::text) struct Glyph {
    /// Unicode scalar used to request this glyph.
    pub character: char,
    /// Font glyph ID selected for `character`.
    pub id:        u16,
    /// Glyph bounds in font design-space units.
    pub bounds:    Bounds,
    /// Quadratic contours extracted from the font.
    pub contours:  Vec<Contour>,
}

/// Errors produced by the outline loader.
#[derive(thiserror::Error, Clone, Debug, PartialEq, Eq)]
pub(crate) enum OutlineError {
    /// Font bytes could not be parsed.
    #[error("font bytes could not be parsed")]
    InvalidFont,
    /// The requested glyph ID has no outline entry in the font.
    #[error("font does not contain glyph id {0}")]
    MissingGlyphId(u16),
}

impl From<OutlineError> for TextOutlineError {
    fn from(error: OutlineError) -> Self {
        match error {
            OutlineError::InvalidFont => Self::UnreadableFace,
            OutlineError::MissingGlyphId(glyph_id) => Self::MissingGlyphId(glyph_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;

    use super::Contour;
    use super::OutlineError;
    use super::load_glyph_by_id_from_face;
    use super::mark_islands;
    use crate::render::ContourRole;
    use crate::render::QuadraticSegment;

    fn line(start: (f32, f32), end: (f32, f32)) -> QuadraticSegment {
        let start = Vec2::from(start);
        let end = Vec2::from(end);
        QuadraticSegment {
            start,
            control: start.midpoint(end),
            end,
        }
    }

    fn curve(start: (f32, f32), control: (f32, f32), end: (f32, f32)) -> QuadraticSegment {
        QuadraticSegment {
            start:   Vec2::from(start),
            control: Vec2::from(control),
            end:     Vec2::from(end),
        }
    }

    fn contour(segments: Vec<QuadraticSegment>) -> Contour {
        Contour {
            segments,
            minimum_feature_size: 0.0,
            fade_exponent: 0.0,
            role: ContourRole::Ink,
        }
    }

    fn square(min: (f32, f32), max: (f32, f32), clockwise: bool) -> Contour {
        let (left, bottom) = min;
        let (right, top) = max;
        let points = if clockwise {
            [(left, bottom), (left, top), (right, top), (right, bottom)]
        } else {
            [(left, bottom), (right, bottom), (right, top), (left, top)]
        };
        contour(
            (0..4)
                .map(|index| line(points[index], points[(index + 1) % 4]))
                .collect(),
        )
    }

    fn ring_with(candidate: Contour) -> Vec<Contour> {
        vec![
            square((0.0, 0.0), (100.0, 100.0), false),
            square((20.0, 20.0), (80.0, 80.0), true),
            candidate,
        ]
    }

    #[test]
    fn curved_lens_inside_counter_is_an_island() {
        let lens = contour(vec![
            curve((40.0, 50.0), (50.0, 30.0), (60.0, 50.0)),
            curve((60.0, 50.0), (50.0, 70.0), (40.0, 50.0)),
        ]);
        let mut contours = ring_with(lens);
        mark_islands(&mut contours, 100.0);
        assert!(matches!(contours[2].role, ContourRole::Island { .. }));
    }

    #[test]
    fn island_touching_another_contour_at_a_point_stays_ink() {
        let mut contours = ring_with(square((40.0, 40.0), (50.0, 50.0), false));
        contours.push(square((50.0, 50.0), (60.0, 60.0), false));
        mark_islands(&mut contours, 100.0);
        assert_eq!(contours[2].role, ContourRole::Ink);
    }

    #[test]
    fn island_tangent_to_another_contour_stays_ink() {
        let mut contours = ring_with(square((40.0, 40.0), (60.0, 60.0), false));
        contours.push(contour(vec![
            curve((65.0, 45.0), (55.0, 50.0), (65.0, 55.0)),
            line((65.0, 55.0), (65.0, 45.0)),
        ]));
        mark_islands(&mut contours, 100.0);
        assert_eq!(contours[2].role, ContourRole::Ink);
    }

    #[test]
    fn island_crossing_another_contour_stays_ink() {
        let mut contours = ring_with(square((40.0, 40.0), (50.0, 50.0), false));
        contours.push(square((45.0, 45.0), (55.0, 55.0), false));
        mark_islands(&mut contours, 100.0);
        assert_eq!(contours[2].role, ContourRole::Ink);
    }

    #[test]
    fn curve_bulging_out_between_samples_stays_ink() {
        let counter = contour(vec![
            line((20.0, 20.0), (20.0, 80.0)),
            curve((20.0, 80.0), (50.0, 120.0), (80.0, 80.0)),
            line((80.0, 80.0), (80.0, 20.0)),
            line((80.0, 20.0), (20.0, 20.0)),
        ]);
        let candidate = contour(vec![
            line((49.5, 99.06), (45.0, 30.0)),
            line((45.0, 30.0), (55.0, 30.0)),
            curve((55.0, 30.0), (52.25, 110.57), (49.5, 99.06)),
        ]);
        let mut contours = vec![
            square((0.0, 0.0), (100.0, 110.0), false),
            counter,
            candidate,
        ];
        mark_islands(&mut contours, 100.0);
        assert_eq!(contours[2].role, ContourRole::Ink);
    }

    #[test]
    #[allow(clippy::expect_used, reason = "the embedded mono fixture must exist")]
    fn real_isolated_glyph_features_keep_their_roles() {
        const FONT: &[u8] = include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf");
        let face = ttf_parser::Face::parse(FONT, 0).expect("embedded mono face");
        for character in ['0', 'Θ'] {
            let id = face
                .glyph_index(character)
                .expect("glyph in embedded mono face");
            let glyph =
                load_glyph_by_id_from_face(FONT, 0, id.0, character).expect("glyph outline");
            assert_eq!(
                glyph
                    .contours
                    .iter()
                    .filter(|contour| matches!(contour.role, ContourRole::Island { .. }))
                    .count(),
                1,
                "{character} needs one island",
            );
        }
        let id = face
            .glyph_index('@')
            .expect("at sign in embedded mono face");
        let glyph = load_glyph_by_id_from_face(FONT, 0, id.0, '@').expect("at sign outline");
        assert!(
            glyph
                .contours
                .iter()
                .all(|contour| contour.role == ContourRole::Ink)
        );
    }

    #[test]
    fn outline_error_messages_are_stable() {
        let cases = [
            (OutlineError::InvalidFont, "font bytes could not be parsed"),
            (
                OutlineError::MissingGlyphId(42),
                "font does not contain glyph id 42",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }
}

/// Loads one glyph by resolved font face and glyph ID.
pub(in crate::text) fn load_glyph_by_id_from_face(
    font_data: &[u8],
    face_index: u32,
    glyph_id: u16,
    character: char,
) -> Result<Glyph, OutlineError> {
    let face = Face::parse(font_data, face_index).map_err(|_| OutlineError::InvalidFont)?;
    let glyph_id = GlyphId(glyph_id);
    load_glyph_from_face(&face, glyph_id, character)
}

/// Visibility established while extracting an outline from a parsed face.
#[derive(Debug)]
pub(in crate::text::slug) enum GlyphOutline {
    /// Extracted contours for a glyph that draws.
    Visible(Glyph),
    /// A blank or out-of-range glyph has no outline.
    Invisible,
}

/// Extracts a glyph once; blank and out-of-range glyphs retain explicit visibility.
pub(in crate::text::slug) fn prepare_glyph_outline(
    face: &Face<'_>,
    glyph_id: u16,
    character: char,
) -> Result<GlyphOutline, OutlineError> {
    match load_glyph_from_face(face, GlyphId(glyph_id), character) {
        Ok(glyph) => Ok(GlyphOutline::Visible(glyph)),
        Err(OutlineError::MissingGlyphId(_)) => Ok(GlyphOutline::Invisible),
        Err(error) => Err(error),
    }
}

/// Returns whether `glyph_id` can produce visible curve data.
#[must_use]
fn glyph_id_has_visible_outline(face: &Face<'_>, glyph_id: u16) -> bool {
    glyph_id < face.number_of_glyphs() && face.glyph_bounding_box(GlyphId(glyph_id)).is_some()
}

/// Parses the face and returns whether `glyph_id` can produce visible curve
/// data, for callers without an already-parsed [`Face`].
pub(in crate::text) fn font_glyph_id_has_visible_outline(
    font_data: &[u8],
    face_index: u32,
    glyph_id: u16,
) -> Result<bool, OutlineError> {
    let face = Face::parse(font_data, face_index).map_err(|_| OutlineError::InvalidFont)?;
    Ok(glyph_id_has_visible_outline(&face, glyph_id))
}

/// Loads an outline from an already parsed face.
pub(in crate::text::slug) fn load_glyph_from_face(
    face: &Face<'_>,
    glyph_id: GlyphId,
    character: char,
) -> Result<Glyph, OutlineError> {
    let mut builder = QuadraticOutlineBuilder::new(character);
    let rect = face
        .outline_glyph(glyph_id, &mut builder)
        .ok_or(OutlineError::MissingGlyphId(glyph_id.0))?;
    let mut glyph = builder.finish(
        glyph_id,
        Bounds {
            min: Vec2::new(f32::from(rect.x_min), f32::from(rect.y_min)),
            max: Vec2::new(f32::from(rect.x_max), f32::from(rect.y_max)),
        },
    );
    mark_islands(&mut glyph.contours, f32::from(face.units_per_em()));
    Ok(glyph)
}

fn contour_bounds(contour: &Contour) -> Bounds {
    let mut bounds = Bounds {
        min: Vec2::splat(f32::INFINITY),
        max: Vec2::splat(f32::NEG_INFINITY),
    };
    for segment in &contour.segments {
        for point in [segment.start, segment.control, segment.end] {
            bounds.min = bounds.min.min(point);
            bounds.max = bounds.max.max(point);
        }
    }
    bounds
}

fn contour_area(contour: &Contour) -> f32 {
    contour
        .segments
        .iter()
        .map(|segment| {
            (2.0_f32 / 3.0).mul_add(
                cross(segment.control - segment.start, segment.end - segment.start),
                cross(segment.start, segment.end),
            )
        })
        .sum()
}

fn segment_bounds(segment: QuadraticSegment) -> Bounds {
    Bounds {
        min: segment.start.min(segment.control).min(segment.end),
        max: segment.start.max(segment.control).max(segment.end),
    }
}

fn bounds_may_touch(left: Bounds, right: Bounds) -> bool {
    let gap = (left.min - right.max)
        .max(right.min - left.max)
        .max(Vec2::ZERO);
    gap.length_squared() <= ISLAND_CLEARANCE_FONT_UNITS * ISLAND_CLEARANCE_FONT_UNITS
}

fn split_quadratic(segment: QuadraticSegment) -> (QuadraticSegment, QuadraticSegment) {
    let start_control = segment.start.midpoint(segment.control);
    let control_end = segment.control.midpoint(segment.end);
    let middle = start_control.midpoint(control_end);
    (
        QuadraticSegment {
            start:   segment.start,
            control: start_control,
            end:     middle,
        },
        QuadraticSegment {
            start:   middle,
            control: control_end,
            end:     segment.end,
        },
    )
}

fn curves_may_touch(left: QuadraticSegment, right: QuadraticSegment) -> bool {
    let left_bounds = segment_bounds(left);
    let right_bounds = segment_bounds(right);
    if !bounds_may_touch(left_bounds, right_bounds) {
        return false;
    }
    let left_size = left_bounds.width().max(left_bounds.height());
    let right_size = right_bounds.width().max(right_bounds.height());
    if left_size <= ISLAND_CLEARANCE_FONT_UNITS * HALF
        && right_size <= ISLAND_CLEARANCE_FONT_UNITS * HALF
    {
        return true;
    }
    if left_size >= right_size {
        let (first, second) = split_quadratic(left);
        curves_may_touch(first, right) || curves_may_touch(second, right)
    } else {
        let (first, second) = split_quadratic(right);
        curves_may_touch(left, first) || curves_may_touch(left, second)
    }
}

fn contours_may_touch(
    left: &Contour,
    left_bounds: Bounds,
    right: &Contour,
    right_bounds: Bounds,
) -> bool {
    bounds_may_touch(left_bounds, right_bounds)
        && left.segments.iter().any(|left_segment| {
            right
                .segments
                .iter()
                .any(|right_segment| curves_may_touch(*left_segment, *right_segment))
        })
}

fn contour_contains(contour: &Contour, point: Vec2) -> bool {
    let mut crossings = 0;
    for segment in &contour.segments {
        let mut before = segment.start;
        for step in 1..=16_u8 {
            let t = f32::from(step) / 16.0;
            let after = sample_quadratic(*segment, t);
            if (before.y > point.y) != (after.y > point.y)
                && point.x
                    < before.x + (point.y - before.y) * (after.x - before.x) / (after.y - before.y)
            {
                crossings += 1;
            }
            before = after;
        }
    }
    crossings % 2 != 0
}

fn mark_islands(contours: &mut [Contour], em_size: f32) {
    if contours.len() < 3 || em_size <= 0.0 {
        return;
    }
    let bounds: Vec<_> = contours.iter().map(contour_bounds).collect();
    let areas: Vec<_> = contours.iter().map(contour_area).collect();
    let outer_sign = areas
        .iter()
        .copied()
        .max_by(|a, b| a.abs().total_cmp(&b.abs()))
        .unwrap_or(0.0)
        .signum();
    for index in 0..contours.len() {
        if areas[index] * outer_sign <= 0.0 || contours[index].segments.is_empty() {
            continue;
        }
        let island = bounds[index];
        if contours.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && contours_may_touch(&contours[index], island, other, bounds[other_index])
        }) {
            continue;
        }
        let point = contours[index].segments[0].start;
        let inside_counter = contours.iter().enumerate().any(|(parent_index, parent)| {
            areas[parent_index] * outer_sign < 0.0
                && point.x >= bounds[parent_index].min.x
                && point.y >= bounds[parent_index].min.y
                && point.x <= bounds[parent_index].max.x
                && point.y <= bounds[parent_index].max.y
                && contour_contains(parent, point)
        });
        if inside_counter {
            contours[index].role = ContourRole::Island {
                extent_em: Vec2::new(island.width(), island.height()) / em_size,
            };
        }
    }
}

#[derive(Debug)]
struct QuadraticOutlineBuilder {
    character:     char,
    contours:      Vec<Contour>,
    current:       Vec<QuadraticSegment>,
    contour_start: Option<Vec2>,
    current_point: Option<Vec2>,
}

impl QuadraticOutlineBuilder {
    const fn new(character: char) -> Self {
        Self {
            character,
            contours: Vec::new(),
            current: Vec::new(),
            contour_start: None,
            current_point: None,
        }
    }

    fn finish(mut self, glyph_id: GlyphId, bounds: Bounds) -> Glyph {
        self.finish_contour();
        Glyph {
            character: self.character,
            id: glyph_id.0,
            bounds,
            contours: self.contours,
        }
    }

    fn finish_contour(&mut self) {
        if self.current.is_empty() {
            self.contour_start = None;
            self.current_point = None;
            return;
        }
        self.contours.push(Contour {
            segments:             std::mem::take(&mut self.current),
            minimum_feature_size: 0.0,
            fade_exponent:        0.0,
            role:                 ContourRole::Ink,
        });
        self.contour_start = None;
        self.current_point = None;
    }

    fn push_line(&mut self, end: Vec2) {
        if let Some(start) = self.current_point {
            let control = start.midpoint(end);
            self.current.push(QuadraticSegment {
                start,
                control,
                end,
            });
        }
        self.current_point = Some(end);
    }

    fn push_quadratic(&mut self, control: Vec2, end: Vec2) {
        if let Some(start) = self.current_point {
            self.current.push(QuadraticSegment {
                start,
                control,
                end,
            });
        }
        self.current_point = Some(end);
    }

    fn push_cubic_as_quadratics(&mut self, control_a: Vec2, control_b: Vec2, end: Vec2) {
        if let Some(start) = self.current_point {
            append_cubic_quadratics(
                CubicSegment {
                    start,
                    control_a,
                    control_b,
                    end,
                },
                0,
                &mut self.current,
            );
        }
        self.current_point = Some(end);
    }
}

impl OutlineBuilder for QuadraticOutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        let point = Vec2::new(x, y);
        self.contour_start = Some(point);
        self.current_point = Some(point);
    }

    fn line_to(&mut self, x: f32, y: f32) { self.push_line(Vec2::new(x, y)); }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.push_quadratic(Vec2::new(x1, y1), Vec2::new(x, y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.push_cubic_as_quadratics(Vec2::new(x1, y1), Vec2::new(x2, y2), Vec2::new(x, y));
    }

    fn close(&mut self) {
        if let (Some(start), Some(current_point)) = (self.contour_start, self.current_point)
            && current_point != start
        {
            self.push_line(start);
        }
        self.finish_contour();
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CubicSegment {
    start:     Vec2,
    control_a: Vec2,
    control_b: Vec2,
    end:       Vec2,
}

fn append_cubic_quadratics(cubic: CubicSegment, depth: u8, segments: &mut Vec<QuadraticSegment>) {
    let quadratic = approximate_cubic_with_quadratic(cubic);
    let below_tolerance = cubic_quadratic_error(cubic, quadratic) <= CUBIC_TO_QUADRATIC_TOLERANCE;
    if depth >= CUBIC_TO_QUADRATIC_MAX_DEPTH || below_tolerance {
        segments.push(quadratic);
        return;
    }

    let (left, right) = split_cubic(cubic);
    append_cubic_quadratics(left, depth + 1, segments);
    append_cubic_quadratics(right, depth + 1, segments);
}

fn approximate_cubic_with_quadratic(cubic: CubicSegment) -> QuadraticSegment {
    let midpoint_control = || {
        let midpoint = sample_cubic(cubic, HALF);
        midpoint * 2.0 - (cubic.start + cubic.end) * HALF
    };
    QuadraticSegment {
        start:   cubic.start,
        control: tangent_intersection_control(cubic).unwrap_or_else(midpoint_control),
        end:     cubic.end,
    }
}

fn tangent_intersection_control(cubic: CubicSegment) -> Option<Vec2> {
    let start_tangent = cubic.control_a - cubic.start;
    let end_tangent = cubic.end - cubic.control_b;
    let denominator = cross(start_tangent, end_tangent);
    if denominator.abs() < TANGENT_INTERSECTION_EPSILON {
        return None;
    }

    let scale = cross(cubic.end - cubic.start, end_tangent) / denominator;
    Some(cubic.start + start_tangent * scale)
}

fn cross(left: Vec2, right: Vec2) -> f32 { left.x.mul_add(right.y, -(left.y * right.x)) }

fn cubic_quadratic_error(cubic: CubicSegment, quadratic: QuadraticSegment) -> f32 {
    [ERROR_SAMPLE_A, ERROR_SAMPLE_B]
        .into_iter()
        .map(|t| sample_cubic(cubic, t).distance(sample_quadratic(quadratic, t)))
        .fold(0.0, f32::max)
}

fn split_cubic(cubic: CubicSegment) -> (CubicSegment, CubicSegment) {
    let start_control = cubic.start.lerp(cubic.control_a, HALF);
    let control_midpoint = cubic.control_a.lerp(cubic.control_b, HALF);
    let end_control = cubic.control_b.lerp(cubic.end, HALF);
    let left_control_b = start_control.lerp(control_midpoint, HALF);
    let right_control_a = control_midpoint.lerp(end_control, HALF);
    let midpoint = left_control_b.lerp(right_control_a, HALF);

    (
        CubicSegment {
            start:     cubic.start,
            control_a: start_control,
            control_b: left_control_b,
            end:       midpoint,
        },
        CubicSegment {
            start:     midpoint,
            control_a: right_control_a,
            control_b: end_control,
            end:       cubic.end,
        },
    )
}

fn sample_cubic(cubic: CubicSegment, t: f32) -> Vec2 {
    let start = cubic.start.lerp(cubic.control_a, t);
    let control = cubic.control_a.lerp(cubic.control_b, t);
    let end = cubic.control_b.lerp(cubic.end, t);
    let start_control = start.lerp(control, t);
    let control_end = control.lerp(end, t);
    start_control.lerp(control_end, t)
}

fn sample_quadratic(quadratic: QuadraticSegment, t: f32) -> Vec2 {
    let start = quadratic.start.lerp(quadratic.control, t);
    let end = quadratic.control.lerp(quadratic.end, t);
    start.lerp(end, t)
}
