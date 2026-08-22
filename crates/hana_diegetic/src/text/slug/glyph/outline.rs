use bevy::math::Vec2;
use ttf_parser::Face;
use ttf_parser::GlyphId;
use ttf_parser::OutlineBuilder;

use crate::render::Bounds;
use crate::render::PathContour;
use crate::render::QuadraticSegment;

const CUBIC_TO_QUADRATIC_TOLERANCE: f32 = 0.25;
const CUBIC_TO_QUADRATIC_MAX_DEPTH: u8 = 10;
const HALF: f32 = 0.5;
const ERROR_SAMPLE_A: f32 = 0.25;
const ERROR_SAMPLE_B: f32 = 0.75;
const TANGENT_INTERSECTION_EPSILON: f32 = 0.000_001;

/// One closed glyph contour expressed as quadratic segments.
pub(in crate::text::slug) type Contour = PathContour;

/// Quadratic-only outline representation for one glyph.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::text::slug) struct Glyph {
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

#[cfg(test)]
mod tests {
    use super::OutlineError;

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
pub(in crate::text::slug) fn load_glyph_by_id_from_face(
    font_data: &[u8],
    face_index: u32,
    glyph_id: u16,
    character: char,
) -> Result<Glyph, OutlineError> {
    let face = Face::parse(font_data, face_index).map_err(|_| OutlineError::InvalidFont)?;
    let glyph_id = GlyphId(glyph_id);
    load_glyph_from_face(&face, glyph_id, character)
}

/// Returns whether `glyph_id` can produce visible curve data.
#[must_use]
fn glyph_id_has_visible_outline(face: &Face<'_>, glyph_id: u16) -> bool {
    glyph_id < face.number_of_glyphs() && face.glyph_bounding_box(GlyphId(glyph_id)).is_some()
}

/// Parses the face and returns whether `glyph_id` can produce visible curve
/// data, for callers without an already-parsed [`Face`].
pub(in crate::text::slug) fn font_glyph_id_has_visible_outline(
    font_data: &[u8],
    face_index: u32,
    glyph_id: u16,
) -> Result<bool, OutlineError> {
    let face = Face::parse(font_data, face_index).map_err(|_| OutlineError::InvalidFont)?;
    Ok(glyph_id_has_visible_outline(&face, glyph_id))
}

fn load_glyph_from_face(
    face: &Face<'_>,
    glyph_id: GlyphId,
    character: char,
) -> Result<Glyph, OutlineError> {
    let mut builder = QuadraticOutlineBuilder::new(character);
    let rect = face
        .outline_glyph(glyph_id, &mut builder)
        .ok_or(OutlineError::MissingGlyphId(glyph_id.0))?;
    Ok(builder.finish(
        glyph_id,
        Bounds {
            min: Vec2::new(f32::from(rect.x_min), f32::from(rect.y_min)),
            max: Vec2::new(f32::from(rect.x_max), f32::from(rect.y_max)),
        },
    ))
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
            segments:      std::mem::take(&mut self.current),
            min_feature:   0.0,
            fade_exponent: 0.0,
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
