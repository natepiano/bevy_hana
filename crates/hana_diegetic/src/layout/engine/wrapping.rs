use std::borrow::Cow;

use bevy::math::Vec2;
use hana_kana::ToF32;

use super::layout_engine::ComputedLayout;
use super::layout_engine::MeasureTextFn;
use super::sizing;
use crate::layout::TextMeasure;
use crate::layout::TextSizing;
use crate::layout::TextStyle;
use crate::layout::TextWrap;
use crate::layout::element::ElementContent;
use crate::layout::element::LayoutTree;

/// Allow for the rounding lost when a Fit box adds and removes its chrome.
const TEXT_FIT_TOLERANCE: f32 = 1e-4;

const fn is_breaking_whitespace(character: char) -> bool {
    character.is_whitespace() && !matches!(character, '\u{a0}' | '\u{202f}')
}

fn has_nonbreaking_space(text: &str) -> bool {
    text.contains('\u{a0}') || text.contains('\u{202f}')
}

fn displayed_word(word: &str) -> Cow<'_, str> {
    if has_nonbreaking_space(word) {
        Cow::Owned(
            word.chars()
                .map(|character| match character {
                    '\u{a0}' | '\u{202f}' => ' ',
                    other => other,
                })
                .collect(),
        )
    } else {
        Cow::Borrowed(word)
    }
}

fn fits_width(width: f32, max_width: f32) -> bool {
    width <= max_width.max(0.0).mul_add(TEXT_FIT_TOLERANCE, max_width)
}

/// A single line of wrapped text with its measured width.
#[derive(Clone, Debug)]
pub(super) struct WrappedLine {
    pub(super) text:  String,
    pub(super) width: f32,
}

/// Pre-computed word-wrap results for a text element.
#[derive(Clone, Debug)]
pub(super) struct WrappedText {
    pub(super) lines:       Vec<WrappedLine>,
    pub(super) line_height: f32,
}

/// Measures the widest single word in `text` — the minimum width word-wrapped
/// text can compress to without breaking inside a word.
///
/// Words are split at breaking whitespace, matching `wrap_text_words`. Returns zero for
/// text with no words.
pub(super) fn min_content_width(
    text: &str,
    text_measure: &TextMeasure,
    measure: &MeasureTextFn,
) -> f32 {
    text.split(is_breaking_whitespace)
        .filter(|word| !word.is_empty())
        .map(|word| measure(&displayed_word(word), text_measure).width)
        .fold(0.0_f32, f32::max)
}

/// Finds the last fitting punctuation break within the remaining word.
fn fitting_word_piece<'a>(
    word: &'a str,
    character_ends: &[usize],
    next_character: &mut usize,
    max_width: f32,
    text_measure: &TextMeasure,
    measure: &MeasureTextFn,
) -> Option<&'a str> {
    let first_character = *next_character;
    let start = if first_character == 0 {
        0
    } else {
        character_ends[first_character - 1]
    };
    let remaining_characters = character_ends.len() - first_character;
    let mut fitting_count = 0;
    let mut too_long_count = remaining_characters + 1;

    while fitting_count + 1 < too_long_count {
        let candidate_count = fitting_count + (too_long_count - fitting_count) / 2;
        let end = character_ends[first_character + candidate_count - 1];
        if fits_width(measure(&word[start..end], text_measure).width, max_width) {
            fitting_count = candidate_count;
        } else {
            too_long_count = candidate_count;
        }
    }

    let end_character = if fitting_count == remaining_characters {
        character_ends.len() - 1
    } else {
        (first_character..first_character + fitting_count)
            .rev()
            .find(|&index| {
                let piece = &word[start..character_ends[index]];
                piece.ends_with("::")
                    || matches!(piece.chars().next_back(), Some('_' | '.' | '/' | '-'))
            })?
    };
    *next_character = end_character + 1;
    Some(&word[start..character_ends[end_character]])
}

fn push_wrapped_line(
    lines: &mut Vec<WrappedLine>,
    current_text: &mut String,
    text_measure: &TextMeasure,
    measure: &MeasureTextFn,
) {
    let width = measure(current_text, text_measure).width;
    lines.push(WrappedLine {
        text: std::mem::take(current_text),
        width,
    });
}

fn append_oversized_word(
    lines: &mut Vec<WrappedLine>,
    current_text: &mut String,
    word: &str,
    config: &TextStyle,
    max_width: f32,
    measure: &MeasureTextFn,
    font_scale: f32,
) -> f32 {
    let text_measure = config.as_measure().scaled(font_scale);
    let has_punctuation_break = word.contains("::")
        || word
            .bytes()
            .any(|byte| matches!(byte, b'_' | b'.' | b'/' | b'-'));
    if !has_punctuation_break {
        if !current_text.is_empty() {
            push_wrapped_line(lines, current_text, &text_measure, measure);
        }
        lines.extend(truncate_text_tail(word, config, max_width, measure, font_scale).lines);
        return 0.0;
    }
    let character_ends: Vec<usize> = word
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .collect();
    let mut next_character = 0;
    let mut current_width = 0.0;
    while next_character < character_ends.len() {
        let piece = fitting_word_piece(
            word,
            &character_ends,
            &mut next_character,
            max_width,
            &text_measure,
            measure,
        );
        let Some(piece) = piece else {
            if !current_text.is_empty() {
                push_wrapped_line(lines, current_text, &text_measure, measure);
            }
            let start = if next_character == 0 {
                0
            } else {
                character_ends[next_character - 1]
            };
            lines.extend(
                truncate_text_tail(&word[start..], config, max_width, measure, font_scale).lines,
            );
            return 0.0;
        };
        if current_text.is_empty() {
            current_text.push_str(piece);
            current_width = measure(piece, &text_measure).width;
        } else {
            let joined = format!("{current_text} {piece}");
            let joined_width = measure(&joined, &text_measure).width;
            if fits_width(joined_width, max_width) {
                *current_text = joined;
                current_width = joined_width;
            } else {
                push_wrapped_line(lines, current_text, &text_measure, measure);
                current_text.push_str(piece);
                current_width = measure(piece, &text_measure).width;
            }
        }

        if next_character < character_ends.len() {
            push_wrapped_line(lines, current_text, &text_measure, measure);
            current_width = 0.0;
        }
    }
    current_width
}

/// Word-wraps text within `max_width`, splitting at breaking whitespace boundaries.
///
/// Explicit `\n` characters are respected as paragraph breaks. Each paragraph
/// is then word-wrapped independently. A word wider than `max_width` splits at
/// preferred punctuation when possible. A segment with no fitting punctuation
/// stays on one line and ends in an ellipsis if that glyph fits.
fn wrap_text_words(
    text: &str,
    config: &TextStyle,
    max_width: f32,
    measure: &MeasureTextFn,
    font_scale: f32,
) -> WrappedText {
    let text_measure = config.as_measure().scaled(font_scale);
    let space_dims = measure(" ", &text_measure);
    let line_height = space_dims.line_height;
    let space_width = space_dims.width;
    let mut all_lines = Vec::new();

    for paragraph in text.split('\n') {
        let mut words = paragraph
            .split(is_breaking_whitespace)
            .filter(|word| !word.is_empty())
            .peekable();

        if words.peek().is_none() {
            all_lines.push(WrappedLine {
                text:  String::new(),
                width: 0.0,
            });
            continue;
        }

        let mut current_text = String::new();
        let mut current_width: f32 = 0.0;

        for word in words {
            let displayed = displayed_word(word);
            let word = displayed.as_ref();
            let word_width = measure(word, &text_measure).width;

            if !fits_width(word_width, max_width) {
                current_width = append_oversized_word(
                    &mut all_lines,
                    &mut current_text,
                    word,
                    config,
                    max_width,
                    measure,
                    font_scale,
                );
                continue;
            }

            if current_text.is_empty() {
                // First word on this line.
                current_text.push_str(word);
                current_width = word_width;
            } else {
                let projected = current_width + space_width + word_width;
                if fits_width(projected, max_width) {
                    current_text.push(' ');
                    current_text.push_str(word);
                    current_width = projected;
                } else {
                    // Break: emit current line, start new line with this word.
                    // Re-measure the complete line text so the width accounts for
                    // kerning and glyph bearings that word-level accumulation misses.
                    push_wrapped_line(&mut all_lines, &mut current_text, &text_measure, measure);
                    current_text = word.to_string();
                    current_width = word_width;
                }
            }
        }

        // Emit the last line of this paragraph — re-measure the full line.
        let line_width = if current_text.is_empty() {
            0.0
        } else {
            measure(&current_text, &text_measure).width
        };
        if !current_text.is_empty() || all_lines.is_empty() {
            all_lines.push(WrappedLine {
                text:  current_text,
                width: line_width,
            });
        }
    }

    if all_lines.is_empty() {
        all_lines.push(WrappedLine {
            text:  String::new(),
            width: 0.0,
        });
    }

    WrappedText {
        lines: all_lines,
        line_height,
    }
}

/// Splits text at explicit `\n` characters and measures each line as a single run.
fn wrap_text_newlines(
    text: &str,
    config: &TextStyle,
    measure: &MeasureTextFn,
    font_scale: f32,
) -> WrappedText {
    let text_measure = config.as_measure().scaled(font_scale);
    let mut lines = Vec::new();
    let mut line_height = 0.0_f32;

    for line in text.split('\n') {
        let dims = measure(line, &text_measure);
        line_height = dims.line_height;
        lines.push(WrappedLine {
            text:  line.to_string(),
            width: dims.width,
        });
    }

    if lines.is_empty() {
        lines.push(WrappedLine {
            text:  String::new(),
            width: 0.0,
        });
    }

    WrappedText { lines, line_height }
}

/// Retains the longest character prefix that fits with a trailing ellipsis.
/// An empty line preserves its height when even the ellipsis cannot fit.
fn truncate_text_tail(
    text: &str,
    config: &TextStyle,
    max_width: f32,
    measure: &MeasureTextFn,
    font_scale: f32,
) -> WrappedText {
    let text_measure = config.as_measure().scaled(font_scale);
    let character_ends: Vec<usize> = text
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .collect();
    let mut fitting_count = 0;
    let mut too_long_count = character_ends.len() + 1;
    while fitting_count + 1 < too_long_count {
        let candidate_count = fitting_count + (too_long_count - fitting_count) / 2;
        let prefix = text[..character_ends[candidate_count - 1]].trim_end();
        let candidate = format!("{prefix}…");
        if fits_width(measure(&candidate, &text_measure).width, max_width) {
            fitting_count = candidate_count;
        } else {
            too_long_count = candidate_count;
        }
    }
    let prefix = if fitting_count == 0 {
        ""
    } else {
        text[..character_ends[fitting_count - 1]].trim_end()
    };
    let truncated = format!("{prefix}…");
    let dimensions = measure(&truncated, &text_measure);
    let line = if fits_width(dimensions.width, max_width) {
        WrappedLine {
            text:  truncated,
            width: dimensions.width,
        }
    } else {
        WrappedLine {
            text:  String::new(),
            width: 0.0,
        }
    };
    WrappedText {
        lines:       vec![line],
        line_height: dimensions.line_height,
    }
}

/// Re-wraps text elements within their parent's content area and updates
/// computed widths and heights.
///
/// Returns per-element wrapped text data (indexed by element index) and a flag
/// indicating whether any computed sizes actually changed (used to skip
/// redundant re-propagation).
///
/// Two optimizations avoid work in the common case (short text that fits):
///
/// 1. **Cached natural width** — uses the `natural_text_width` stored during
///    `initialize_leaf_sizes` instead of re-calling the measure function. If the cached width fits
///    within the element's post-sizing width, the text does not reflow and wrapping is skipped.
///
/// 2. **Parent-aware traversal** — the pass walks from the root and carries the current parent's
///    content width, avoiding a separate parent lookup table.
pub(super) fn rewrap_text_elements(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    measure: &MeasureTextFn,
    font_scale: f32,
) -> (Vec<Option<WrappedText>>, bool) {
    let mut wrapped = Vec::new();
    let mut any_changed = false;

    if let Some(root) = tree.root {
        let root_width = computed[root].width;
        rewrap_subtree(
            tree,
            computed,
            measure,
            font_scale,
            &mut wrapped,
            &mut any_changed,
            root,
            root_width,
        );
    }

    (wrapped, any_changed)
}

fn rewrap_subtree(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    measure: &MeasureTextFn,
    font_scale: f32,
    wrapped: &mut Vec<Option<WrappedText>>,
    any_changed: &mut bool,
    index: usize,
    parent_content_width: f32,
) {
    let element = &tree.elements[index];
    if let ElementContent::Text {
        ref text,
        ref config,
        ref sizing,
        ..
    } = element.content
        && let Some(result) = {
            let content_width = sizing::content_box(
                element,
                Vec2::new(computed[index].width, computed[index].height),
            )
            .size
            .x;
            let chrome =
                element.padding.horizontal() + sizing::border_inset(element, sizing::Axis::X);
            let available_width = if chrome > 0.0 {
                content_width.min((parent_content_width - chrome).max(0.0))
            } else {
                parent_content_width
            };
            wrapped_text_result(
                text,
                config,
                sizing,
                computed[index].natural_text_width,
                available_width,
                measure,
                font_scale,
            )
        }
    {
        apply_wrapped_text(tree, computed, wrapped, any_changed, index, result);
    }

    let children = tree.children_of(index);
    if children.is_empty() {
        return;
    }

    let child_content_width = sizing::content_box(
        element,
        Vec2::new(computed[index].width, computed[index].height),
    )
    .size
    .x;
    for &child in children {
        rewrap_subtree(
            tree,
            computed,
            measure,
            font_scale,
            wrapped,
            any_changed,
            child,
            child_content_width,
        );
    }
}

fn wrapped_text_result(
    text: &str,
    config: &TextStyle,
    sizing: &TextSizing,
    natural_width: f32,
    parent_content_width: f32,
    measure: &MeasureTextFn,
    font_scale: f32,
) -> Option<WrappedText> {
    match sizing {
        TextSizing::Natural { wrap } => match wrap {
            TextWrap::None => None,
            TextWrap::Words => {
                // Fast path: compare the cached natural text width (measured
                // once in `initialize_leaf_sizes`) against the parent's
                // content area. If the text fits and has no explicit
                // newlines, wrapping would produce one identical line — skip.
                // Uses the cached width to avoid re-calling the measure fn.
                if !text.contains('\n')
                    && !has_nonbreaking_space(text)
                    && fits_width(natural_width, parent_content_width)
                {
                    None
                } else {
                    Some(wrap_text_words(
                        text,
                        config,
                        parent_content_width,
                        measure,
                        font_scale,
                    ))
                }
            },
            TextWrap::Newlines => {
                // Fast path: no explicit newlines means a single line.
                if text.contains('\n') {
                    Some(wrap_text_newlines(text, config, measure, font_scale))
                } else {
                    None
                }
            },
            TextWrap::TruncateTail => {
                if !text.contains('\n') && fits_width(natural_width, parent_content_width) {
                    None
                } else {
                    Some(truncate_text_tail(
                        text.split('\n').next().unwrap_or(text),
                        config,
                        parent_content_width,
                        measure,
                        font_scale,
                    ))
                }
            },
        },
        TextSizing::MeasureAs { .. } => None,
        TextSizing::WrapAtMeasure { .. } => {
            let wrap_width = natural_width.min(parent_content_width);
            Some(wrap_text_words(
                text, config, wrap_width, measure, font_scale,
            ))
        },
    }
}

fn apply_wrapped_text(
    tree: &LayoutTree,
    computed: &mut [ComputedLayout],
    wrapped: &mut Vec<Option<WrappedText>>,
    any_changed: &mut bool,
    index: usize,
    result: WrappedText,
) {
    let element = &tree.elements[index];
    let old_width = computed[index].width;
    let old_height = computed[index].height;

    let max_line_width = result.lines.iter().map(|l| l.width).fold(0.0_f32, f32::max);
    let horizontal_chrome =
        element.padding.horizontal() + sizing::border_inset(element, sizing::Axis::X);
    let vertical_chrome =
        element.padding.vertical() + sizing::border_inset(element, sizing::Axis::Y);
    if element.width.is_fit()
        && !matches!(
            element.content,
            ElementContent::Text {
                sizing: TextSizing::WrapAtMeasure { .. },
                ..
            }
        )
    {
        computed[index].width = (max_line_width + horizontal_chrome)
            .clamp(element.width.min_size(), element.width.max_size());
    }

    let new_height = result
        .line_height
        .mul_add(result.lines.len().to_f32(), vertical_chrome);
    computed[index].height = new_height.clamp(element.height.min_size(), element.height.max_size());

    if (computed[index].width - old_width).abs() > f32::EPSILON
        || (computed[index].height - old_height).abs() > f32::EPSILON
    {
        *any_changed = true;
    }

    if wrapped.is_empty() {
        wrapped.resize_with(tree.len(), || None);
    }
    wrapped[index] = Some(result);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::layout::TextDimensions;

    fn character_measure() -> MeasureTextFn {
        Arc::new(|text, _| TextDimensions {
            width:       text.chars().count().to_f32(),
            height:      1.0,
            line_height: 1.0,
        })
    }

    fn wrap(text: &str, max_width: f32) -> WrappedText {
        wrap_text_words(
            text,
            &TextStyle::new(1.0),
            max_width,
            &character_measure(),
            1.0,
        )
    }

    fn line_texts(wrapped: &WrappedText) -> Vec<&str> {
        wrapped
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect()
    }

    #[test]
    fn oversized_unbroken_word_ends_in_ellipsis_on_one_line() {
        let word = "abcdefghij";
        let wrapped = wrap(word, 5.0);

        assert_eq!(line_texts(&wrapped), ["abcd…"]);
        assert!(wrapped.lines.iter().all(|line| line.width <= 5.0));
    }

    #[test]
    fn oversized_word_prefers_double_colon_break() {
        let wrapped = wrap("hana::log", 7.0);

        assert_eq!(line_texts(&wrapped), ["hana::", "log"]);
    }

    #[test]
    fn one_glyph_limit_shows_only_ellipsis() {
        let wrapped = wrap("abcd", 1.0);

        assert_eq!(line_texts(&wrapped), ["…"]);
        assert_eq!(wrapped.lines[0].width, 1.0);
    }

    #[test]
    fn word_wrap_too_narrow_for_ellipsis_keeps_empty_line_height() {
        let wrapped = wrap("abcd", 0.5);

        assert_eq!(line_texts(&wrapped), [""]);
        assert_eq!(wrapped.lines[0].width, 0.0);
        assert_eq!(wrapped.line_height, 1.0);
    }

    #[test]
    fn long_unbroken_word_truncates_with_few_measurements() {
        let word = "a".repeat(1_000);
        let calls = Arc::new(AtomicUsize::new(0));
        let measured_calls = Arc::clone(&calls);
        let measure: MeasureTextFn = Arc::new(move |text, _| {
            measured_calls.fetch_add(1, Ordering::Relaxed);
            TextDimensions {
                width:       text.chars().count().to_f32(),
                height:      1.0,
                line_height: 1.0,
            }
        });

        let wrapped = wrap_text_words(&word, &TextStyle::new(1.0), 1.0, &measure, 1.0);

        assert_eq!(line_texts(&wrapped), ["…"]);
        assert!(
            calls.load(Ordering::Relaxed) < 18,
            "long-word truncation made {} measurements",
            calls.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn multibyte_word_truncation_keeps_character_boundaries() {
        let wrapped = wrap("é日本é", 3.0);

        assert_eq!(line_texts(&wrapped), ["é日…"]);
    }

    #[test]
    fn following_word_starts_after_truncated_word() {
        let wrapped = wrap("abcdefgh x", 5.0);

        assert_eq!(line_texts(&wrapped), ["abcd…", "x"]);
    }

    #[test]
    fn slash_breaks_word_only_at_punctuation() {
        let wrapped = wrap("alpha/beta", 6.0);

        assert_eq!(line_texts(&wrapped), ["alpha/", "beta"]);
    }

    #[test]
    fn first_piece_joins_previous_word_when_it_fits() {
        let wrapped = wrap("a b::cdefg", 5.0);

        assert_eq!(line_texts(&wrapped), ["a b::", "cdefg"]);
    }

    #[test]
    fn fitting_words_keep_whitespace_wrap() {
        let wrapped = wrap("one two three", 7.0);

        assert_eq!(line_texts(&wrapped), ["one two", "three"]);
        assert!((wrapped.lines[0].width - 7.0).abs() < 0.001);
        assert!((wrapped.lines[1].width - 5.0).abs() < 0.001);
    }

    #[test]
    fn nonbreaking_spaces_keep_joined_words_on_one_line_and_draw_as_spaces() {
        assert_eq!(line_texts(&wrap("x Save Scene", 10.0)), ["x Save", "Scene"]);
        for joined in ["x Save\u{a0}Scene", "x Save\u{202f}Scene"] {
            let wrapped = wrap(joined, 10.0);
            assert_eq!(line_texts(&wrapped), ["x", "Save Scene"]);
            assert!(wrapped.lines.iter().all(|line| line.width <= 10.0));
        }
    }

    #[test]
    fn minimum_content_width_keeps_nonbreaking_pairs_together() {
        let measure = character_measure();
        for joined in ["x Save\u{a0}Scene", "x Save\u{202f}Scene"] {
            let width = min_content_width(joined, &TextStyle::new(1.0).as_measure(), &measure);
            assert!((width - 10.0).abs() < 0.001);
        }
    }

    #[test]
    fn minimum_content_width_still_uses_whole_words() {
        let measure = character_measure();

        let width = min_content_width(
            "short abcdefghi",
            &TextStyle::new(1.0).as_measure(),
            &measure,
        );
        assert!((width - 9.0).abs() < 0.001);
    }

    fn truncate(text: &str, max_width: f32) -> Option<WrappedText> {
        wrapped_text_result(
            text,
            &TextStyle::new(1.0),
            &TextSizing::wrap(TextWrap::TruncateTail),
            text.chars().count().to_f32(),
            max_width,
            &character_measure(),
            1.0,
        )
    }

    #[test]
    fn truncating_text_that_fits_keeps_the_natural_line() {
        assert!(truncate("Opaque", 6.0).is_none());
    }

    #[test]
    fn truncating_text_keeps_the_longest_fitting_prefix() {
        let wrapped = truncate("Opaque", 5.0);
        assert!(wrapped.is_some());
        let Some(wrapped) = wrapped else { return };
        assert_eq!(line_texts(&wrapped), ["Opaq…"]);
        assert!((wrapped.lines[0].width - 5.0).abs() < 1e-4);
    }

    #[test]
    fn truncating_multibyte_text_uses_character_boundaries() {
        let wrapped = truncate("é日本é", 3.0);
        assert!(wrapped.is_some());
        let Some(wrapped) = wrapped else { return };
        assert_eq!(line_texts(&wrapped), ["é日…"]);
    }

    #[test]
    fn truncating_box_too_narrow_for_ellipsis_keeps_empty_line_height() {
        let wrapped = truncate("Opaque", 0.5);
        assert!(wrapped.is_some());
        let Some(wrapped) = wrapped else { return };
        assert_eq!(line_texts(&wrapped), [""]);
        assert_eq!(wrapped.lines[0].width, 0.0);
        assert_eq!(wrapped.line_height, 1.0);
    }

    #[test]
    fn truncating_box_just_wide_enough_contains_ellipsis() {
        let wrapped = truncate("Opaque", 1.0);
        assert!(wrapped.is_some());
        let Some(wrapped) = wrapped else { return };
        assert_eq!(line_texts(&wrapped), ["…"]);
        assert_eq!(wrapped.lines[0].width, 1.0);
        assert_eq!(wrapped.line_height, 1.0);
    }

    #[test]
    fn truncating_newline_uses_only_first_line_with_ellipsis() {
        let wrapped = truncate("A\nB", 3.0);
        assert!(wrapped.is_some());
        let Some(wrapped) = wrapped else { return };
        assert_eq!(line_texts(&wrapped), ["A…"]);
    }
}
