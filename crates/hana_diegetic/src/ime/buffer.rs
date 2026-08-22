//! UTF-8-safe single-line IME edit buffer.

use std::ops::Range;

/// Validated UTF-8 byte boundary in the committed edit buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImeBufferBoundary(usize);

impl ImeBufferBoundary {
    pub(super) const fn new(index: usize) -> Self { Self(index) }

    /// Returns the byte index represented by this boundary.
    #[must_use]
    pub const fn as_usize(self) -> usize { self.0 }
}

/// Validated UTF-8 byte boundary in the active preedit string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImePreeditBoundary(usize);

impl ImePreeditBoundary {
    pub(super) const fn new(index: usize) -> Self { Self(index) }

    /// Returns the byte index represented by this boundary.
    #[must_use]
    pub const fn as_usize(self) -> usize { self.0 }
}

/// Selected range in the committed edit buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImeBufferRange {
    /// Start of the range.
    pub start: ImeBufferBoundary,
    /// End of the range.
    pub end:   ImeBufferBoundary,
}

/// Snapshot of the committed-buffer selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImeSelectionSnapshot {
    /// Fixed selection end.
    pub anchor: ImeBufferBoundary,
    /// Moving selection end.
    pub focus:  ImeBufferBoundary,
}

/// Active preedit text shown at the committed-buffer selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImePreedit {
    /// Composing text supplied by the platform IME.
    pub text:        String,
    /// Committed-buffer range the preedit would replace on commit.
    pub replacement: ImeBufferRange,
    /// Optional cursor inside `text`.
    pub cursor:      Option<ImePreeditBoundary>,
}

/// Cursor or selection state in the committed edit buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImeCursorState {
    /// Insertion point at a UTF-8 boundary.
    Insertion(ImeBufferBoundary),
    /// Non-empty selection.
    Selection(ImeSelectionSnapshot),
}

/// Snapshot of the full single-line IME buffer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImeBufferSnapshot {
    /// Committed buffer text only. Preedit text is separate.
    pub committed_text: String,
    /// Cursor or selection in `committed_text`.
    pub cursor:         ImeCursorState,
    /// Active composing text, if any.
    pub preedit:        Option<ImePreedit>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImeMovementDirection {
    Backward,
    Forward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImeMovementUnit {
    Character,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImeSelectionMode {
    Move,
    Extend,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ImeEditCommand {
    InsertText(String),
    DeleteBackward(ImeMovementUnit),
    DeleteForward(ImeMovementUnit),
    PlaceCursor(usize),
    SelectWordAt(usize),
    Move {
        direction: ImeMovementDirection,
        unit:      ImeMovementUnit,
        selection: ImeSelectionMode,
    },
    SelectAll,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ImeEditBuffer {
    text:      String,
    selection: ImeSelection,
}

impl ImeEditBuffer {
    pub(super) fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            selection: ImeSelection::insertion(end),
        }
    }

    pub(super) fn new_selected(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        let selection = if end == 0 {
            ImeSelection::insertion(0)
        } else {
            ImeSelection {
                anchor: 0,
                focus:  end,
            }
        };
        Self { text, selection }
    }

    pub(super) fn committed_text(&self) -> &str { &self.text }

    pub(super) fn replacement_range(&self) -> ImeBufferRange {
        let range = self.selection.range();
        ImeBufferRange {
            start: ImeBufferBoundary::new(range.start),
            end:   ImeBufferBoundary::new(range.end),
        }
    }

    pub(super) fn snapshot(&self, preedit: Option<ImePreedit>) -> ImeBufferSnapshot {
        ImeBufferSnapshot {
            committed_text: self.text.clone(),
            cursor: self.selection.snapshot(),
            preedit,
        }
    }

    pub(super) fn apply(&mut self, command: ImeEditCommand) -> ImeBufferEdit {
        match command {
            ImeEditCommand::InsertText(text) => self.insert_text(&text),
            ImeEditCommand::DeleteBackward(unit) => self.delete_backward(unit),
            ImeEditCommand::DeleteForward(unit) => self.delete_forward(unit),
            ImeEditCommand::PlaceCursor(index) => self.place_cursor(index),
            ImeEditCommand::SelectWordAt(index) => self.select_word_at(index),
            ImeEditCommand::Move {
                direction,
                unit,
                selection,
            } => self.move_cursor(direction, unit, selection),
            ImeEditCommand::SelectAll => self.select_all(),
        }
    }

    fn insert_text(&mut self, text: &str) -> ImeBufferEdit {
        let single_line = text
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        if single_line.is_empty() {
            return ImeBufferEdit::Unchanged;
        }

        let range = self.selection.range();
        self.text.replace_range(range.clone(), &single_line);
        self.selection = ImeSelection::insertion(range.start + single_line.len());
        ImeBufferEdit::Changed
    }

    fn delete_backward(&mut self, unit: ImeMovementUnit) -> ImeBufferEdit {
        if self.delete_selection() == ImeBufferEdit::Changed {
            return ImeBufferEdit::Changed;
        }

        let focus = self.selection.focus;
        let start = match unit {
            ImeMovementUnit::Character => self.previous_boundary(focus),
            ImeMovementUnit::Word => self.previous_word_boundary(focus),
            ImeMovementUnit::Line => 0,
        };
        self.delete_range(start..focus)
    }

    fn delete_forward(&mut self, unit: ImeMovementUnit) -> ImeBufferEdit {
        if self.delete_selection() == ImeBufferEdit::Changed {
            return ImeBufferEdit::Changed;
        }

        let focus = self.selection.focus;
        let end = match unit {
            ImeMovementUnit::Character => self.next_boundary(focus),
            ImeMovementUnit::Word => self.next_word_boundary(focus),
            ImeMovementUnit::Line => self.text.len(),
        };
        self.delete_range(focus..end)
    }

    fn move_cursor(
        &mut self,
        direction: ImeMovementDirection,
        unit: ImeMovementUnit,
        selection: ImeSelectionMode,
    ) -> ImeBufferEdit {
        let from = self.selection.focus;
        let to = match (direction, unit) {
            (ImeMovementDirection::Backward, ImeMovementUnit::Character) => {
                self.previous_boundary(from)
            },
            (ImeMovementDirection::Forward, ImeMovementUnit::Character) => self.next_boundary(from),
            (ImeMovementDirection::Backward, ImeMovementUnit::Word) => {
                self.previous_word_boundary(from)
            },
            (ImeMovementDirection::Forward, ImeMovementUnit::Word) => self.next_word_boundary(from),
            (ImeMovementDirection::Backward, ImeMovementUnit::Line) => 0,
            (ImeMovementDirection::Forward, ImeMovementUnit::Line) => self.text.len(),
        };

        let next = match selection {
            ImeSelectionMode::Move => ImeSelection::insertion(to),
            ImeSelectionMode::Extend => self.selection.with_focus(to),
        };
        if self.selection == next {
            ImeBufferEdit::Unchanged
        } else {
            self.selection = next;
            ImeBufferEdit::Changed
        }
    }

    fn select_all(&mut self) -> ImeBufferEdit {
        let next = ImeSelection {
            anchor: 0,
            focus:  self.text.len(),
        };
        if self.selection == next {
            ImeBufferEdit::Unchanged
        } else {
            self.selection = next;
            ImeBufferEdit::Changed
        }
    }

    fn place_cursor(&mut self, index: usize) -> ImeBufferEdit {
        if !self.text.is_char_boundary(index) {
            return ImeBufferEdit::Unchanged;
        }
        let next = ImeSelection::insertion(index);
        if self.selection == next {
            ImeBufferEdit::Unchanged
        } else {
            self.selection = next;
            ImeBufferEdit::Changed
        }
    }

    fn select_word_at(&mut self, index: usize) -> ImeBufferEdit {
        if !self.text.is_char_boundary(index) || index == self.text.len() {
            return self.place_cursor(index);
        }
        let end = self.next_boundary(index);
        let Some(character) = self.char_at(index) else {
            return self.place_cursor(index);
        };
        let next = if is_word_character(character) {
            ImeSelection {
                anchor: self.previous_word_boundary(end),
                focus:  self.next_word_boundary(index),
            }
        } else {
            ImeSelection {
                anchor: index,
                focus:  end,
            }
        };
        if self.selection == next {
            ImeBufferEdit::Unchanged
        } else {
            self.selection = next;
            ImeBufferEdit::Changed
        }
    }

    fn delete_selection(&mut self) -> ImeBufferEdit {
        let range = self.selection.range();
        self.delete_range(range)
    }

    fn delete_range(&mut self, range: Range<usize>) -> ImeBufferEdit {
        if range.start == range.end {
            return ImeBufferEdit::Unchanged;
        }

        self.text.replace_range(range.clone(), "");
        self.selection = ImeSelection::insertion(range.start);
        ImeBufferEdit::Changed
    }

    fn previous_boundary(&self, from: usize) -> usize {
        self.text[..from]
            .char_indices()
            .last()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self, from: usize) -> usize {
        self.text[from..]
            .chars()
            .next()
            .map_or(self.text.len(), |character| from + character.len_utf8())
    }

    fn previous_word_boundary(&self, from: usize) -> usize {
        let mut position = from;
        while position > 0 && !self.char_before(position).is_some_and(is_word_character) {
            position = self.previous_boundary(position);
        }
        while position > 0 && self.char_before(position).is_some_and(is_word_character) {
            position = self.previous_boundary(position);
        }
        position
    }

    fn next_word_boundary(&self, from: usize) -> usize {
        let mut position = from;
        while position < self.text.len() && !self.char_at(position).is_some_and(is_word_character) {
            position = self.next_boundary(position);
        }
        while position < self.text.len() && self.char_at(position).is_some_and(is_word_character) {
            position = self.next_boundary(position);
        }
        position
    }

    fn char_before(&self, position: usize) -> Option<char> {
        self.text[..position].chars().next_back()
    }

    fn char_at(&self, position: usize) -> Option<char> { self.text[position..].chars().next() }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImeBufferEdit {
    Changed,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ImeSelection {
    anchor: usize,
    focus:  usize,
}

impl ImeSelection {
    const fn insertion(index: usize) -> Self {
        Self {
            anchor: index,
            focus:  index,
        }
    }

    const fn with_focus(&self, focus: usize) -> Self {
        Self {
            anchor: self.anchor,
            focus,
        }
    }

    fn range(&self) -> Range<usize> { self.anchor.min(self.focus)..self.anchor.max(self.focus) }

    const fn snapshot(&self) -> ImeCursorState {
        if self.anchor == self.focus {
            ImeCursorState::Insertion(ImeBufferBoundary::new(self.focus))
        } else {
            ImeCursorState::Selection(ImeSelectionSnapshot {
                anchor: ImeBufferBoundary::new(self.anchor),
                focus:  ImeBufferBoundary::new(self.focus),
            })
        }
    }
}

pub(super) fn preedit_cursor_boundary(
    text: &str,
    cursor: Option<(usize, usize)>,
) -> Option<ImePreeditBoundary> {
    let (_, focus) = cursor?;
    text.is_char_boundary(focus)
        .then_some(ImePreeditBoundary::new(focus))
}

fn is_word_character(character: char) -> bool { character.is_alphanumeric() || character == '_' }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_keeps_utf8_boundaries() {
        let mut buffer = ImeEditBuffer::new("aé");

        buffer.apply(ImeEditCommand::Move {
            direction: ImeMovementDirection::Backward,
            unit:      ImeMovementUnit::Character,
            selection: ImeSelectionMode::Move,
        });
        buffer.apply(ImeEditCommand::InsertText("🙂".to_owned()));

        assert_eq!(buffer.committed_text(), "a🙂é");
    }

    #[test]
    fn selection_replaces_text() {
        let mut buffer = ImeEditBuffer::new("alpha beta");

        buffer.apply(ImeEditCommand::Move {
            direction: ImeMovementDirection::Backward,
            unit:      ImeMovementUnit::Word,
            selection: ImeSelectionMode::Extend,
        });
        buffer.apply(ImeEditCommand::InsertText("gamma".to_owned()));

        assert_eq!(buffer.committed_text(), "alpha gamma");
    }

    #[test]
    fn selected_initial_text_is_ready_for_replacement() {
        let mut buffer = ImeEditBuffer::new_selected("Editable text");

        assert!(matches!(
            buffer.snapshot(None).cursor,
            ImeCursorState::Selection(ImeSelectionSnapshot { anchor, focus })
                if anchor.as_usize() == 0 && focus.as_usize() == "Editable text".len()
        ));

        buffer.apply(ImeEditCommand::InsertText("Replacement".to_owned()));
        assert_eq!(buffer.committed_text(), "Replacement");
    }

    #[test]
    fn pointer_cursor_replaces_the_initial_selection() {
        let mut buffer = ImeEditBuffer::new_selected("alpha beta");

        buffer.apply(ImeEditCommand::PlaceCursor("alpha ".len()));

        assert_eq!(
            buffer.snapshot(None).cursor,
            ImeCursorState::Insertion(ImeBufferBoundary::new("alpha ".len())),
        );
    }

    #[test]
    fn pointer_word_selection_uses_the_character_under_the_click() {
        let mut buffer = ImeEditBuffer::new("alpha béta");

        buffer.apply(ImeEditCommand::SelectWordAt("alpha b".len()));

        assert_eq!(
            buffer.snapshot(None).cursor,
            ImeCursorState::Selection(ImeSelectionSnapshot {
                anchor: ImeBufferBoundary::new("alpha ".len()),
                focus:  ImeBufferBoundary::new("alpha béta".len()),
            }),
        );
    }

    #[test]
    fn word_delete_uses_safe_boundaries() {
        let mut buffer = ImeEditBuffer::new("alpha béta");

        buffer.apply(ImeEditCommand::DeleteBackward(ImeMovementUnit::Word));

        assert_eq!(buffer.committed_text(), "alpha ");
    }

    #[test]
    fn insertion_ignores_control_characters() {
        let mut buffer = ImeEditBuffer::new("a");

        buffer.apply(ImeEditCommand::InsertText("\u{8}b\u{7f}".to_owned()));

        assert_eq!(buffer.committed_text(), "ab");
    }

    #[test]
    fn snapshot_separates_preedit_from_committed_text() {
        let buffer = ImeEditBuffer::new("base");
        let preedit = ImePreedit {
            text:        "候補".to_owned(),
            replacement: buffer.replacement_range(),
            cursor:      Some(ImePreeditBoundary::new("候".len())),
        };

        let snapshot = buffer.snapshot(Some(preedit));

        assert_eq!(snapshot.committed_text, "base");
        assert!(snapshot.preedit.is_some());
    }
}
