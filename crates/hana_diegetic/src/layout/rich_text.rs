//! Typed colour and word-spacing runs authored as one read-only paragraph.

use std::ops::Range;

use bevy::color::Color;

use super::Dimension;
use super::LayoutTreeChange;
use super::TextMeasure;
use super::TextStyle;
use super::TextWrap;
use super::builder::CommonEl;
use super::builder::TextSizing;
use crate::PanelElementId;
use crate::cascade::Cascade;

/// One read-only rich text leaf, sharing one box and one paragraph measure.
#[derive(Clone, Debug)]
#[must_use]
pub struct RichText {
    pub(super) layout: CommonEl,
    pub(super) base:   TextStyle,
    pub(super) sizing: TextSizing,
}

impl RichText {
    /// Authors a paragraph whose spans inherit this box style.
    pub fn new(base: TextStyle) -> Self {
        Self {
            layout: CommonEl::default(),
            base,
            sizing: TextSizing::wrap(TextWrap::None),
        }
    }

    /// Selects no wrapping. Other wrap modes require the typography phase.
    ///
    /// # Panics
    /// Panics when the requested mode is not [`TextWrap::None`].
    pub fn wrap(mut self, wrap: TextWrap) -> Self {
        assert!(
            matches!(wrap, TextWrap::None),
            "rich paragraphs currently support TextWrap::None only"
        );
        self.sizing = TextSizing::wrap(wrap);
        self
    }

    /// Trims the cap-line and baseline margins, like [`super::Text::trim_text_box`].
    pub fn trim_text_box(
        mut self,
        top: impl Into<Dimension>,
        bottom: impl Into<Dimension>,
    ) -> Self {
        let top = top.into();
        let bottom = bottom.into();
        self.layout.padding.top = Dimension {
            value: -top.value,
            unit:  top.unit,
        };
        self.layout.padding.bottom = Dimension {
            value: -bottom.value,
            unit:  bottom.unit,
        };
        self
    }

    /// Assigns the paragraph a stable panel-local identity.
    pub fn id(mut self, id: impl Into<PanelElementId>) -> Self {
        self.layout.id = Some(id.into());
        self
    }
}

/// Span overrides. Inherited fields use the paragraph's base value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[must_use]
pub struct SpanStyle {
    color:        Cascade<Color>,
    word_spacing: Cascade<f32>,
}

impl SpanStyle {
    /// Inherits all properties from the box.
    pub const fn new() -> Self {
        Self {
            color:        Cascade::Inherit,
            word_spacing: Cascade::Inherit,
        }
    }
    /// Overrides only this span's ink.
    pub const fn with_color(mut self, color: Color) -> Self {
        self.color = Cascade::Override(color);
        self
    }
    /// Overrides the added advance of spaces in this span, in font units.
    pub const fn with_word_spacing(mut self, spacing: f32) -> Self {
        self.word_spacing = Cascade::Override(spacing);
        self
    }
}

/// One borrowed text span and its explicitly authored overrides.
pub struct Span<'a> {
    text:  &'a str,
    style: SpanStyle,
}
impl<'a> From<&'a str> for Span<'a> {
    fn from(text: &'a str) -> Self {
        Self {
            text,
            style: SpanStyle::new(),
        }
    }
}
impl<'a> From<(&'a str, SpanStyle)> for Span<'a> {
    fn from((text, style): (&'a str, SpanStyle)) -> Self { Self { text, style } }
}

/// Collects source text and resolved runs without creating child leaves.
pub struct RichTextBuilder {
    paragraph: RichParagraph,
}
impl RichTextBuilder {
    pub(crate) const fn new(base: TextStyle, sizing: TextSizing) -> Self {
        Self {
            paragraph: RichParagraph {
                text: String::new(),
                runs: Vec::new(),
                base,
                sizing,
            },
        }
    }
    /// Appends a span, ignoring empty text and coalescing equal resolved neighbours.
    pub fn span<'a>(&mut self, span: impl Into<Span<'a>>) -> &mut Self {
        let Span { text, style } = span.into();
        if text.is_empty() {
            return self;
        }
        let start = self.paragraph.text.len();
        self.paragraph.text.push_str(text);
        let end = self.paragraph.text.len();
        let mut measure = self.paragraph.base.as_measure();
        if let Cascade::Override(spacing) = style.word_spacing {
            measure.word_spacing = spacing;
        }
        let color = match style.color {
            Cascade::Inherit => self.paragraph.base.color(),
            Cascade::Override(color) => color,
        };
        if let Some(previous) = self.paragraph.runs.last_mut()
            && previous.measure == measure
            && previous.color == color
        {
            previous.range.end = end;
        } else {
            self.paragraph.runs.push(RichRun {
                range: start..end,
                measure,
                color,
            });
        }
        self
    }
    pub(crate) fn finish(self) -> RichParagraph { self.paragraph }
}

/// A resolved byte range, with geometry and paint kept distinct.
#[derive(Clone, Debug, PartialEq)]
pub struct RichRun {
    /// UTF-8 character-boundary source range.
    pub range:   Range<usize>,
    /// Resolved measurement properties.
    pub measure: TextMeasure,
    /// Resolved ink colour, excluded from the shaping key.
    pub color:   Color,
}

/// Resolved source and runs for one layout leaf and one shaped paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct RichParagraph {
    /// Concatenated UTF-8 source.
    pub text:   String,
    /// Coalesced resolved source runs.
    pub runs:   Vec<RichRun>,
    /// Shared box style, including render-only properties.
    pub base:   TextStyle,
    /// Shared paragraph sizing policy.
    pub sizing: TextSizing,
}
impl RichParagraph {
    /// Paint changes alone use the plain leaf's unwrapped sizing rules.
    pub(super) fn uses_base_geometry(&self) -> bool {
        let base = self.base.as_measure();
        self.runs.iter().all(|run| run.measure == base)
    }

    pub(super) fn scaled(&self, scale: f32) -> Self {
        Self {
            text:   self.text.clone(),
            runs:   self
                .runs
                .iter()
                .map(|run| RichRun {
                    range:   run.range.clone(),
                    measure: run.measure.scaled(scale),
                    color:   run.color,
                })
                .collect(),
            base:   self.base.scaled(scale),
            sizing: self.sizing.clone(),
        }
    }
    /// Coalesces measurement ranges independently of paint boundaries.
    pub(super) fn geometry_runs(&self) -> Vec<(Range<usize>, TextMeasure)> {
        let mut runs: Vec<(Range<usize>, TextMeasure)> = Vec::new();
        for run in &self.runs {
            if let Some((range, measure)) = runs.last_mut()
                && *measure == run.measure
            {
                range.end = run.range.end;
            } else {
                runs.push((run.range.clone(), run.measure));
            }
        }
        runs
    }
    // Keep paragraph range allocation and cleanup out of the plain tree classifier.
    #[inline(never)]
    pub(super) fn classify_change(&self, next: &Self) -> LayoutTreeChange {
        if self.text != next.text
            || self.sizing != next.sizing
            || !self.base.layout_eq_excluding_visuals(&next.base)
            || self.geometry_runs() != next.geometry_runs()
        {
            LayoutTreeChange::LayoutAffecting
        } else if self != next {
            LayoutTreeChange::VisualOnly
        } else {
            LayoutTreeChange::Identical
        }
    }
}
