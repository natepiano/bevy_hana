//! Glyph outlines of shaped text, read on the CPU.
//!
//! [`FontRegistry::text_outline`](super::FontRegistry::text_outline) shapes text
//! the way the measurer and the panel renderer do and hands back each visible
//! glyph's contours, placed along the run. [`GlyphOutline::contains`] answers
//! whether a point is ink by the renderer's own fill rule.

use std::sync::Mutex;
use std::sync::PoisonError;

use bevy::math::Rect;
use bevy::math::Vec2;
use hana_kana::ToU16;
use parley::FontContext;
use parley::Layout;
use parley::LayoutContext;
use parley::layout::PositionedLayoutItem;
use ttf_parser::Face;

use super::FontLookupError;
use super::measurer;
use crate::layout::TextMeasure;
use crate::render::PathContour;
use crate::render::QuadraticSegment;
use crate::text;

/// Stands in for the character a glyph was shaped from: the loader records it
/// for diagnostics, and an outline never reads it.
const OUTLINE_DIAGNOSTIC_CHAR: char = '\u{FFFD}';

/// The outline a piece of text draws.
#[derive(Clone, Debug, PartialEq)]
pub enum TextOutlineMeasurement {
    /// No glyph draws ink: empty or blank text.
    NoVisibleInk,
    /// At least one glyph draws ink.
    Visible(TextOutline),
}

/// Every visible glyph's outline along shaped text, in text order.
///
/// Built only by [`FontRegistry::text_outline`](super::FontRegistry::text_outline):
/// it holds at least one glyph, and its bounds hold every contour.
#[derive(Clone, Debug, PartialEq)]
pub struct TextOutline {
    glyphs: Vec<GlyphOutline>,
    bounds: Rect,
}

impl TextOutline {
    /// The visible glyphs in text order.
    #[must_use]
    pub fn glyphs(&self) -> &[GlyphOutline] { &self.glyphs }

    /// The smallest rectangle that holds every glyph's ink.
    #[must_use]
    pub const fn bounds(&self) -> Rect { self.bounds }
}

/// One glyph's closed contours, placed at its pen position along the text.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphOutline {
    bounds:   Rect,
    contours: Vec<OutlineContour>,
}

impl GlyphOutline {
    /// The smallest rectangle that holds the glyph's ink.
    #[must_use]
    pub const fn bounds(&self) -> Rect { self.bounds }

    /// Whether `point` is ink.
    ///
    /// Nonzero winding with exact quadratic roots: the renderer's fill rule
    /// (`analytic_path.wesl`). A crossing counts the lower end of its segment
    /// and not the upper one, so a ray through a join or a grazed extremum
    /// counts once or not at all.
    #[must_use]
    pub fn contains(&self, point: Vec2) -> bool {
        self.bounds.contains(point)
            && self
                .contours
                .iter()
                .flat_map(|contour| &contour.segments)
                .map(|segment| segment.winding(point))
                .sum::<i32>()
                != 0
    }
}

/// Why [`FontRegistry::text_outline`](super::FontRegistry::text_outline) could
/// not outline the text.
#[derive(thiserror::Error, Clone, Debug, PartialEq, Eq)]
pub enum TextOutlineError {
    /// The measure names no registered family.
    #[error(transparent)]
    Lookup(#[from] FontLookupError),
    /// A shaped glyph's font face does not parse.
    #[error("a shaped glyph's font face is unreadable")]
    UnreadableFace,
    /// A shaped glyph's face has no outline entry for its glyph ID.
    #[error("the shaped font face has no outline for glyph id {0}")]
    MissingGlyphId(u16),
}

/// One closed contour.
#[derive(Clone, Debug, PartialEq)]
struct OutlineContour {
    segments: Vec<OutlineSegment>,
}

/// A quadratic Bézier; a straight edge has `control` at its midpoint.
#[derive(Clone, Copy, Debug, PartialEq)]
struct OutlineSegment {
    start:   Vec2,
    control: Vec2,
    end:     Vec2,
}

impl OutlineSegment {
    fn placed(segment: QuadraticSegment, scale: f32, pen: Vec2) -> Self {
        Self {
            start:   segment.start * scale + pen,
            control: segment.control * scale + pen,
            end:     segment.end * scale + pen,
        }
    }

    fn at(self, t: f32) -> Vec2 {
        self.start
            .lerp(self.control, t)
            .lerp(self.control.lerp(self.end, t), t)
    }

    /// The parameter where the curve turns back along each axis.
    fn turns(self) -> (Vec2, Vec2) {
        let curvature = (self.end - self.control) - (self.control - self.start);
        ((self.start - self.control) / curvature, curvature)
    }

    fn bounds(self) -> Rect {
        let (turn, curvature) = self.turns();
        let mut bounds = Rect::from_corners(self.start, self.end);
        for (turn, curvature) in [(turn.x, curvature.x), (turn.y, curvature.y)] {
            if curvature != 0.0 && turn > 0.0 && turn < 1.0 {
                bounds = bounds.union_point(self.at(turn));
            }
        }
        bounds
    }

    /// Signed crossings of a ray from `point` toward +x: +1 where the curve
    /// rises, −1 where it falls.
    fn winding(self, point: Vec2) -> i32 {
        let (turn, curvature) = self.turns();
        if curvature.y != 0.0 && turn.y > 0.0 && turn.y < 1.0 {
            let apex = (turn.y, self.at(turn.y));
            self.monotone_winding(point, (0.0, self.start), apex)
                + self.monotone_winding(point, apex, (1.0, self.end))
        } else {
            self.monotone_winding(point, (0.0, self.start), (1.0, self.end))
        }
    }

    /// Winding of the piece between two parameters along which y only rises or
    /// only falls. The piece holds its lower end and not its upper one.
    fn monotone_winding(self, point: Vec2, from: (f32, Vec2), to: (f32, Vec2)) -> i32 {
        let rising = to.1.y > from.1.y;
        let (low, high) = if rising {
            (from.1.y, to.1.y)
        } else {
            (to.1.y, from.1.y)
        };
        if point.y < low || point.y >= high {
            return 0;
        }
        let t = self.parameter_at_height(point.y, from.0, to.0);
        if self.at(t).x <= point.x {
            return 0;
        }
        if rising { 1 } else { -1 }
    }

    /// Solves y(t) = `height` for the root inside `[from, to]`, a piece the
    /// caller already knows holds exactly one.
    fn parameter_at_height(self, height: f32, from: f32, to: f32) -> f32 {
        let curvature = (self.end.y - self.control.y) - (self.control.y - self.start.y);
        let linear = 2.0 * (self.control.y - self.start.y);
        let constant = self.start.y - height;
        if curvature == 0.0 {
            return (-constant / linear).clamp(from, to);
        }
        // The stable form: `q / a` and `c / q` never subtract nearly equal values.
        let discriminant = linear.mul_add(linear, -4.0 * curvature * constant).max(0.0);
        let q = -0.5 * (linear + discriminant.sqrt().copysign(linear));
        let distance_outside = |t: f32| (from - t).max(t - to).max(0.0);
        [q / curvature, constant / q]
            .into_iter()
            .filter(|t| t.is_finite())
            .min_by(|left, right| distance_outside(*left).total_cmp(&distance_outside(*right)))
            .map_or(from, |t| t.clamp(from, to))
    }
}

/// Lays out `text` in `measure`'s style and outlines every visible glyph.
pub(super) fn text_outline(
    font_cx: &Mutex<FontContext>,
    family_name: &str,
    text: &str,
    measure: &TextMeasure,
) -> Result<TextOutlineMeasurement, TextOutlineError> {
    let mut layout: Layout<()> = Layout::new();
    let mut font_cx = font_cx.lock().unwrap_or_else(PoisonError::into_inner);
    measurer::build_layout(
        &mut font_cx,
        &mut LayoutContext::default(),
        &mut layout,
        family_name,
        text,
        measure,
    );
    drop(font_cx);

    let first_baseline = layout
        .lines()
        .next()
        .map_or(0.0, |line| line.metrics().baseline);
    let mut glyphs = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let font = run.run().font();
            let font_data = font.data.data();
            let units_per_em = Face::parse(font_data, font.index)
                .map_err(|_| TextOutlineError::UnreadableFace)?
                .units_per_em();
            let scale = run.run().font_size() / f32::from(units_per_em);
            let mut advance_x = 0.0;
            for cluster in run.run().clusters() {
                for glyph in cluster.glyphs() {
                    let pen = Vec2::new(
                        run.offset() + advance_x + glyph.x,
                        first_baseline - (run.baseline() + glyph.y),
                    );
                    advance_x += glyph.advance;
                    if let GlyphInk::Visible(outline) =
                        outline_glyph(font_data, font.index, glyph.id.to_u16(), scale, pen)?
                    {
                        glyphs.push(outline);
                    }
                }
            }
        }
    }

    let Some(bounds) = glyphs
        .iter()
        .map(GlyphOutline::bounds)
        .reduce(|covered, bounds| covered.union(bounds))
    else {
        return Ok(TextOutlineMeasurement::NoVisibleInk);
    };
    Ok(TextOutlineMeasurement::Visible(TextOutline {
        glyphs,
        bounds,
    }))
}

/// One shaped glyph's ink, or none for a space or an empty outline.
enum GlyphInk {
    Blank,
    Visible(GlyphOutline),
}

/// Loads a glyph's contours and places them: design units scale by `scale`,
/// flip nothing (both are y-up), and shift to `pen`.
fn outline_glyph(
    font_data: &[u8],
    face_index: u32,
    glyph_id: u16,
    scale: f32,
    pen: Vec2,
) -> Result<GlyphInk, TextOutlineError> {
    if !text::font_glyph_id_has_visible_outline(font_data, face_index, glyph_id)? {
        return Ok(GlyphInk::Blank);
    }
    let glyph =
        text::load_glyph_by_id_from_face(font_data, face_index, glyph_id, OUTLINE_DIAGNOSTIC_CHAR)?;
    let contours: Vec<OutlineContour> = glyph
        .contours
        .into_iter()
        .filter(|contour| !contour.segments.is_empty())
        .map(|PathContour { segments, .. }| OutlineContour {
            segments: segments
                .into_iter()
                .map(|segment| OutlineSegment::placed(segment, scale, pen))
                .collect(),
        })
        .collect();
    let bounds = contours
        .iter()
        .flat_map(|contour| &contour.segments)
        .map(|segment| segment.bounds())
        .reduce(|covered, bounds| covered.union(bounds));
    Ok(bounds.map_or(GlyphInk::Blank, |bounds| {
        GlyphInk::Visible(GlyphOutline { bounds, contours })
    }))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests use unwrap for clearer failure messages"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy::math::Rect;
    use bevy::math::Vec2;
    use ttf_parser::Face;
    use ttf_parser::OutlineBuilder;

    use super::GlyphOutline;
    use super::OutlineContour;
    use super::OutlineSegment;
    use super::TextOutline;
    use super::TextOutlineError;
    use super::TextOutlineMeasurement;
    use crate::TextInk;
    use crate::TextInkMeasurement;
    use crate::TextMeasure;
    use crate::TextStyle;
    use crate::text::FontId;
    use crate::text::FontLookupError;
    use crate::text::FontRegistry;

    /// Noto Sans CJK SC: a CFF font, so its outlines are cubic.
    const NOTO_CJK_DATA: &[u8] = include_bytes!("../../../assets/fonts/NotoSansCJKsc-Regular.otf");
    /// Font size every embedded-font outline here is measured at.
    const SIZE: f32 = 100.0;
    /// Samples across one axis of an ink grid or a scan line.
    const SAMPLES: u16 = 200;
    /// One design unit of the embedded font (1000 units per em) at [`SIZE`]: a
    /// font's bounding box is whole design units.
    const DESIGN_UNIT: f32 = SIZE / 1000.0;
    /// How far each probe sits from a point on the true CFF curve, in design
    /// units: well past the 0.25-unit cubic conversion tolerance.
    const CFF_PROBE_UNITS: f32 = 2.0;
    /// Samples along each cubic of the CFF glyph, ends excluded.
    const CUBIC_SAMPLES: u16 = 16;

    fn measure(size: f32) -> TextMeasure { TextStyle::new(size).as_measure() }

    fn visible(registry: &FontRegistry, text: &str, measure: &TextMeasure) -> TextOutline {
        match registry.text_outline(text, measure).unwrap() {
            TextOutlineMeasurement::Visible(outline) => outline,
            TextOutlineMeasurement::NoVisibleInk => panic!("{text:?} should draw ink"),
        }
    }

    fn visible_ink(registry: &FontRegistry, text: &str, measure: &TextMeasure) -> TextInk {
        match registry.measure_ink(text, measure).unwrap() {
            TextInkMeasurement::Visible(ink) => ink,
            TextInkMeasurement::NoVisibleInk => panic!("{text:?} should draw ink"),
        }
    }

    /// The point `step` of [`SAMPLES`] across `from..to`, at the step's centre.
    fn sample(from: f32, to: f32, step: u16) -> f32 {
        from + (to - from) * (f32::from(step) + 0.5) / f32::from(SAMPLES)
    }

    /// Grid samples inside `region` that some glyph of `outline` holds.
    fn ink_samples(outline: &TextOutline, region: Rect) -> usize {
        (0..SAMPLES)
            .flat_map(|row| (0..SAMPLES).map(move |column| (row, column)))
            .filter(|&(row, column)| {
                let point = Vec2::new(
                    sample(region.min.x, region.max.x, column),
                    sample(region.min.y, region.max.y, row),
                );
                outline.glyphs().iter().any(|glyph| glyph.contains(point))
            })
            .count()
    }

    fn line(start: Vec2, end: Vec2) -> OutlineSegment {
        OutlineSegment {
            start,
            control: start.lerp(end, 0.5),
            end,
        }
    }

    /// A closed contour through `corners` in order.
    fn polygon(corners: &[Vec2]) -> OutlineContour {
        OutlineContour {
            segments: corners
                .iter()
                .zip(corners.iter().cycle().skip(1))
                .map(|(&start, &end)| line(start, end))
                .collect(),
        }
    }

    /// A square from `min` to `max`, counter-clockwise or clockwise.
    fn square(min: Vec2, max: Vec2, counter_clockwise: bool) -> OutlineContour {
        let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
        if counter_clockwise {
            polygon(&corners)
        } else {
            polygon(&[corners[0], corners[3], corners[2], corners[1]])
        }
    }

    fn glyph(contours: Vec<OutlineContour>) -> GlyphOutline {
        let bounds = contours
            .iter()
            .flat_map(|contour| &contour.segments)
            .map(|segment| segment.bounds())
            .reduce(|covered, bounds| covered.union(bounds))
            .unwrap();
        GlyphOutline { bounds, contours }
    }

    /// Records a glyph's segments as the font stores them, cubics included.
    #[derive(Default)]
    struct RecordedOutline {
        pen:    Vec2,
        cubics: Vec<[Vec2; 4]>,
    }

    impl OutlineBuilder for RecordedOutline {
        fn move_to(&mut self, x: f32, y: f32) { self.pen = Vec2::new(x, y); }

        fn line_to(&mut self, x: f32, y: f32) { self.pen = Vec2::new(x, y); }

        fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) { self.pen = Vec2::new(x, y); }

        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            let end = Vec2::new(x, y);
            self.cubics
                .push([self.pen, Vec2::new(x1, y1), Vec2::new(x2, y2), end]);
            self.pen = end;
        }

        fn close(&mut self) {}
    }

    fn cubic_at([start, control_a, control_b, end]: [Vec2; 4], t: f32) -> Vec2 {
        let ab = start.lerp(control_a, t);
        let bc = control_a.lerp(control_b, t);
        let cd = control_b.lerp(end, t);
        ab.lerp(bc, t).lerp(bc.lerp(cd, t), t)
    }

    #[test]
    fn hana_has_four_glyph_outlines_in_text_order() {
        let registry = FontRegistry::new().unwrap();
        let outline = visible(&registry, "hana", &measure(SIZE));

        let [h, a, n, second_a] = outline.glyphs() else {
            panic!("expected four glyphs, got {}", outline.glyphs().len());
        };
        let lefts = [h, a, n, second_a].map(|glyph| glyph.bounds().min.x);
        assert!(
            lefts.is_sorted_by(|left, right| left < right),
            "glyphs should run left to right in text order: {lefts:?}"
        );
        assert!(
            h.bounds().height() > n.bounds().height(),
            "h's ascender should make it the tallest"
        );
        assert!(
            (a.bounds().size() - second_a.bounds().size())
                .abs()
                .max_element()
                < 1e-3,
            "both a's should have the same outline"
        );
        for glyph in outline.glyphs() {
            assert_eq!(
                outline.bounds().union(glyph.bounds()),
                outline.bounds(),
                "the text bounds should hold every glyph"
            );
        }
    }

    #[test]
    fn outline_bounds_match_measure_ink() {
        let registry = FontRegistry::new().unwrap();
        for text in ["hana", "Typography", "jumpy"] {
            let bounds = visible(&registry, text, &measure(SIZE)).bounds();
            let ink = visible_ink(&registry, text, &measure(SIZE));
            let expected = [ink.left, ink.right, ink.bottom - ink.top];
            let actual = [bounds.min.x, bounds.max.x, bounds.height()];
            for (expected, actual) in expected.into_iter().zip(actual) {
                assert!(
                    (expected - actual).abs() <= DESIGN_UNIT,
                    "{text:?}: outline bounds {actual:?} should match measure_ink {expected:?}"
                );
            }
        }
    }

    #[test]
    fn a_counter_is_outside_and_its_bowl_is_inside() {
        let registry = FontRegistry::new().unwrap();
        let outline = visible(&registry, "o", &measure(SIZE));
        let [o] = outline.glyphs() else {
            panic!("expected one glyph");
        };
        let bounds = o.bounds();

        assert!(
            !o.contains(bounds.center()),
            "the counter should be outside"
        );
        let mut runs: Vec<bool> = Vec::new();
        for step in 0..SAMPLES {
            let point = Vec2::new(sample(bounds.min.x, bounds.max.x, step), bounds.center().y);
            let ink = o.contains(point);
            if runs.last() != Some(&ink) {
                runs.push(ink);
            }
        }
        assert_eq!(
            runs,
            [true, false, true],
            "the centre row should cross the bowl, the counter and the bowl"
        );
    }

    #[test]
    fn contains_matches_nonzero_winding_on_overlapping_contours() {
        // Two counter-clockwise squares overlap: the overlap winds twice, which
        // nonzero fills and even-odd would leave empty.
        let same_way = glyph(vec![
            square(Vec2::ZERO, Vec2::splat(2.0), true),
            square(Vec2::ONE, Vec2::splat(3.0), true),
        ]);
        assert!(same_way.contains(Vec2::splat(1.5)), "the overlap is ink");
        assert!(
            same_way.contains(Vec2::new(0.5, 1.0)),
            "a ray through the second square's corner is counted once"
        );
        assert!(
            !same_way.contains(Vec2::new(0.5, 3.0)),
            "a point beside both squares is not ink"
        );

        // A clockwise square inside a counter-clockwise one cuts a hole.
        let holed = glyph(vec![
            square(Vec2::ZERO, Vec2::splat(4.0), true),
            square(Vec2::ONE, Vec2::splat(3.0), false),
        ]);
        assert!(!holed.contains(Vec2::splat(2.0)), "the hole is not ink");
        assert!(holed.contains(Vec2::splat(0.5)), "the rim is ink");

        // A notch whose two edges meet at (2, 2): a ray from inside the notch
        // through that join enters the ink once and leaves it once.
        let notched = glyph(vec![polygon(&[
            Vec2::ZERO,
            Vec2::new(4.0, 0.0),
            Vec2::splat(4.0),
            Vec2::new(0.0, 4.0),
            Vec2::splat(2.0),
        ])]);
        assert!(
            !notched.contains(Vec2::new(1.0, 2.0)),
            "the notch is not ink"
        );
        assert!(
            notched.contains(Vec2::new(3.0, 2.0)),
            "past the join is ink"
        );

        // An arch whose apex is (1, 1): a ray that grazes the apex crosses
        // nothing, and one just below it crosses once.
        let arch = glyph(vec![OutlineContour {
            segments: vec![
                OutlineSegment {
                    start:   Vec2::ZERO,
                    control: Vec2::new(1.0, 2.0),
                    end:     Vec2::new(2.0, 0.0),
                },
                line(Vec2::new(2.0, 0.0), Vec2::ZERO),
            ],
        }]);
        assert!(
            !arch.contains(Vec2::new(0.5, 1.0)),
            "the grazed apex is not ink beside it"
        );
        assert!(arch.contains(Vec2::new(1.0, 0.99)), "under the apex is ink");
        assert!(arch.contains(Vec2::new(0.5, 0.5)), "inside the arch is ink");
        assert!(
            !arch.contains(Vec2::new(0.1, 0.5)),
            "beside the arch is not ink"
        );
    }

    #[test]
    fn blank_text_has_no_visible_ink() {
        let registry = FontRegistry::new().unwrap();
        for text in ["", " ", "   "] {
            assert_eq!(
                registry.text_outline(text, &measure(SIZE)),
                Ok(TextOutlineMeasurement::NoVisibleInk),
                "{text:?} should draw no ink"
            );
        }
    }

    #[test]
    fn unknown_font_id_is_an_error() {
        let registry = FontRegistry::new().unwrap();
        let mut unknown = measure(SIZE);
        unknown.font_id = u16::MAX;
        assert_eq!(
            registry.text_outline("hana", &unknown),
            Err(TextOutlineError::Lookup(FontLookupError::UnknownFamilyId(
                FontId(u16::MAX)
            )))
        );
    }

    #[test]
    fn bold_weight_uses_the_bold_face() {
        let registry = FontRegistry::new().unwrap();
        let regular = visible(&registry, "hana", &measure(SIZE));
        let bold = visible(&registry, "hana", &TextStyle::new(SIZE).bold().as_measure());
        let region = regular.bounds().union(bold.bounds());

        let regular_ink = ink_samples(&regular, region);
        let bold_ink = ink_samples(&bold, region);
        assert!(
            bold_ink * 10 > regular_ink * 11,
            "bold should draw at least a tenth more ink: regular {regular_ink}, bold {bold_ink}"
        );
    }

    #[test]
    fn cff_font_outlines_convert_within_tolerance() {
        let mut registry = FontRegistry::new().unwrap();
        let font_id = registry
            .register_font("Noto Sans CJK SC", NOTO_CJK_DATA)
            .unwrap();
        let face = Face::parse(NOTO_CJK_DATA, 0).unwrap();
        let mut design_units = measure(f32::from(face.units_per_em()));
        design_units.font_id = font_id.0;

        // At one layout unit per design unit, the outline sits on the font's
        // own coordinates: its pen starts at the origin.
        let outline = visible(&registry, "o", &design_units);
        let [o] = outline.glyphs() else {
            panic!("expected one glyph");
        };
        let mut recorded = RecordedOutline::default();
        face.outline_glyph(face.glyph_index('o').unwrap(), &mut recorded)
            .unwrap();
        assert!(!recorded.cubics.is_empty(), "a CFF 'o' is drawn in cubics");

        for cubic in recorded.cubics {
            for step in 1..CUBIC_SAMPLES {
                let on_curve = cubic_at(cubic, f32::from(step) / f32::from(CUBIC_SAMPLES));
                let probes = [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y]
                    .map(|direction| o.contains(on_curve + direction * CFF_PROBE_UNITS));
                assert!(
                    probes.contains(&true) && probes.contains(&false),
                    "the converted outline should pass within {CFF_PROBE_UNITS} units of \
                     {on_curve:?}; probes {probes:?}"
                );
            }
        }
    }
}
