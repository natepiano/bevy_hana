use bevy::math::Vec2;
use bevy_kana::ToF32;

use super::layout_engine::ComputedLayout;
use super::layout_engine::MeasureTextFn;
use super::sizing;
use crate::layout::TextMeasure;
use crate::layout::TextSizing;
use crate::layout::TextStyle;
use crate::layout::TextWrap;
use crate::layout::element::ElementContent;
use crate::layout::element::LayoutTree;

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
/// Words are split at whitespace, matching `wrap_text_words`. Returns zero for
/// text with no words.
pub(super) fn min_content_width(
    text: &str,
    text_measure: &TextMeasure,
    measure: &MeasureTextFn,
) -> f32 {
    text.split_whitespace()
        .map(|word| measure(word, text_measure).width)
        .fold(0.0_f32, f32::max)
}

/// Word-wraps text within `max_width`, splitting at whitespace boundaries.
///
/// Explicit `\n` characters are respected as paragraph breaks. Each paragraph
/// is then word-wrapped independently. A word that exceeds `max_width` on its
/// own is placed on a single line without breaking.
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
        let words: Vec<&str> = paragraph.split_whitespace().collect();

        if words.is_empty() {
            all_lines.push(WrappedLine {
                text:  String::new(),
                width: 0.0,
            });
            continue;
        }

        let mut current_text = String::new();
        let mut current_width: f32 = 0.0;

        for word in words {
            let word_width = measure(word, &text_measure).width;

            if current_text.is_empty() {
                // First word on this line — always take it, even if it overflows.
                current_text.push_str(word);
                current_width = word_width;
            } else {
                let projected = current_width + space_width + word_width;
                if projected > max_width {
                    // Break: emit current line, start new line with this word.
                    // Re-measure the complete line text so the width accounts for
                    // kerning and glyph bearings that word-level accumulation misses.
                    let line_width = measure(&current_text, &text_measure).width;
                    all_lines.push(WrappedLine {
                        text:  current_text,
                        width: line_width,
                    });
                    current_text = word.to_string();
                    current_width = word_width;
                } else {
                    current_text.push(' ');
                    current_text.push_str(word);
                    current_width = projected;
                }
            }
        }

        // Emit the last line of this paragraph — re-measure the full line.
        let line_width = if current_text.is_empty() {
            0.0
        } else {
            measure(&current_text, &text_measure).width
        };
        all_lines.push(WrappedLine {
            text:  current_text,
            width: line_width,
        });
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

/// Re-wraps text elements within their parent's content area and updates
/// computed widths and heights.
///
/// Returns per-element wrapped text data (indexed by element index) and a flag
/// indicating whether any computed sizes actually changed (used to skip
/// redundant re-propagation).
///
/// Two key optimizations avoid work in the common case (short text that fits):
///
/// 1. **Cached natural width** — uses the `natural_text_width` stored during
///    `initialize_leaf_sizes` instead of re-calling the measure function. If the cached width fits
///    within the element's post-sizing width, the text won't reflow, so we skip wrapping entirely.
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
        && let Some(result) = wrapped_text_result(
            text,
            config,
            sizing,
            computed[index].natural_text_width,
            parent_content_width,
            measure,
            font_scale,
        )
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
                if !text.contains('\n') && natural_width <= parent_content_width {
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
    if element.width.is_fit()
        && !matches!(
            element.content,
            ElementContent::Text {
                sizing: TextSizing::WrapAtMeasure { .. },
                ..
            }
        )
    {
        computed[index].width =
            max_line_width.clamp(element.width.min_size(), element.width.max_size());
    }

    let new_height = result.line_height * result.lines.len().to_f32();
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
