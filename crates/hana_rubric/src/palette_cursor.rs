//! Which command-palette result row is highlighted, and how far the list has scrolled.

use std::ops::Range;

use bevy::prelude::Component;

/// Which result row a command palette highlights, and how far its list has scrolled.
///
/// A palette keeps one on its panel while it is open. A new cursor follows the
/// query: it highlights whichever row the query picks by itself and shows the
/// list from the top. [`Self::step`] moves the highlight onto an explicit row,
/// wrapping past either end, and scrolls the list only as far as keeps that row
/// in view. A palette whose query text changes starts a new cursor, because the
/// rows it indexed are gone.
#[derive(Component, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PaletteRowCursor {
    highlight:         PaletteRowHighlight,
    first_visible_row: usize,
}

/// The row a [`PaletteRowCursor`] highlights.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PaletteRowHighlight {
    /// Whatever row the query picks by itself, if it picks one.
    #[default]
    FollowsQuery,
    /// This row, which a step moved the highlight onto.
    Row(usize),
}

/// Which way [`PaletteRowCursor::step`] moves the highlight.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaletteRowStep {
    /// The row above, or the last row from the first.
    Previous,
    /// The row below, or the first row from the last.
    Next,
}

/// The result list one palette query renders, as a [`PaletteRowCursor`] indexes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaletteRowList {
    /// How many rows the list holds, including any the palette scrolls out of view.
    pub len:         usize,
    /// The row the query picks by itself, if it picks one.
    pub query_row:   Option<usize>,
    /// How many rows the palette shows at once.
    pub visible_len: usize,
}

impl PaletteRowCursor {
    /// The row the palette highlights and Enter runs.
    ///
    /// `None` when the list is empty, or when the query picks no row and no step
    /// has moved the highlight. A stepped row past the end of a list that has
    /// since shrunk reads as the last row.
    #[must_use]
    pub fn highlighted_row(self, rows: PaletteRowList) -> Option<usize> {
        match self.highlight {
            PaletteRowHighlight::FollowsQuery => rows.query_row.filter(|row| *row < rows.len),
            PaletteRowHighlight::Row(row) => rows.len.checked_sub(1).map(|last| row.min(last)),
        }
    }

    /// The rows the palette shows, top to bottom.
    #[must_use]
    pub fn visible_rows(self, rows: PaletteRowList) -> Range<usize> {
        let first = self
            .first_visible_row
            .min(rows.len.saturating_sub(rows.visible_len));
        first..rows.len.min(first + rows.visible_len)
    }

    /// Moves the highlight one row, wrapping past either end.
    ///
    /// From no highlighted row, [`PaletteRowStep::Next`] lands on the first row
    /// and [`PaletteRowStep::Previous`] on the last. The list scrolls only as far
    /// as keeps the new row in view.
    pub fn step(&mut self, step: PaletteRowStep, rows: PaletteRowList) {
        let Some(last) = rows.len.checked_sub(1) else {
            return;
        };
        let row = match (self.highlighted_row(rows), step) {
            (None, PaletteRowStep::Next) => 0,
            (None, PaletteRowStep::Previous) => last,
            (Some(row), PaletteRowStep::Next) => {
                if row == last {
                    0
                } else {
                    row + 1
                }
            },
            (Some(row), PaletteRowStep::Previous) => row.checked_sub(1).unwrap_or(last),
        };
        let visible = self.visible_rows(rows);
        self.first_visible_row = if row < visible.start {
            row
        } else if row >= visible.start + rows.visible_len {
            row + 1 - rows.visible_len
        } else {
            visible.start
        };
        self.highlight = PaletteRowHighlight::Row(row);
    }
}

#[cfg(test)]
mod tests {
    use super::PaletteRowCursor;
    use super::PaletteRowList;
    use super::PaletteRowStep;

    const fn rows(len: usize, query_row: Option<usize>) -> PaletteRowList {
        PaletteRowList {
            len,
            query_row,
            visible_len: 3,
        }
    }

    fn stepped(steps: &[PaletteRowStep], rows: PaletteRowList) -> PaletteRowCursor {
        let mut cursor = PaletteRowCursor::default();
        for step in steps {
            cursor.step(*step, rows);
        }
        cursor
    }

    #[test]
    fn a_new_cursor_highlights_the_query_row_and_shows_the_top() {
        let cursor = PaletteRowCursor::default();

        assert_eq!(cursor.highlighted_row(rows(5, Some(0))), Some(0));
        assert_eq!(cursor.highlighted_row(rows(5, None)), None);
        assert_eq!(cursor.highlighted_row(rows(0, Some(0))), None);
        assert_eq!(cursor.visible_rows(rows(5, None)), 0..3);
        assert_eq!(cursor.visible_rows(rows(2, None)), 0..2);
    }

    #[test]
    fn stepping_from_no_highlight_lands_on_either_end() {
        let list = rows(5, None);

        assert_eq!(
            stepped(&[PaletteRowStep::Next], list).highlighted_row(list),
            Some(0)
        );
        let from_the_bottom = stepped(&[PaletteRowStep::Previous], list);
        assert_eq!(from_the_bottom.highlighted_row(list), Some(4));
        assert_eq!(from_the_bottom.visible_rows(list), 2..5);
    }

    #[test]
    fn stepping_moves_from_the_query_row_and_wraps_at_both_ends() {
        let list = rows(3, Some(0));

        assert_eq!(
            stepped(&[PaletteRowStep::Next], list).highlighted_row(list),
            Some(1)
        );
        assert_eq!(
            stepped(&[PaletteRowStep::Previous], list).highlighted_row(list),
            Some(2)
        );
        assert_eq!(
            stepped(&[PaletteRowStep::Next; 3], list).highlighted_row(list),
            Some(0)
        );
    }

    #[test]
    fn the_list_scrolls_only_when_the_highlight_leaves_it() {
        let list = rows(6, Some(0));
        let mut cursor = stepped(&[PaletteRowStep::Next; 3], list);
        assert_eq!(cursor.highlighted_row(list), Some(3));
        assert_eq!(cursor.visible_rows(list), 1..4);

        cursor.step(PaletteRowStep::Previous, list);
        assert_eq!(cursor.highlighted_row(list), Some(2));
        assert_eq!(cursor.visible_rows(list), 1..4);

        cursor.step(PaletteRowStep::Previous, list);
        cursor.step(PaletteRowStep::Previous, list);
        assert_eq!(cursor.highlighted_row(list), Some(0));
        assert_eq!(cursor.visible_rows(list), 0..3);
    }

    #[test]
    fn a_shrunken_list_keeps_the_highlight_on_its_last_row_in_view() {
        let cursor = stepped(&[PaletteRowStep::Previous], rows(6, None));

        let shrunken = rows(4, None);
        assert_eq!(cursor.highlighted_row(shrunken), Some(3));
        assert_eq!(cursor.visible_rows(shrunken), 1..4);
        assert_eq!(cursor.highlighted_row(rows(0, None)), None);
    }

    #[test]
    fn stepping_an_empty_list_changes_nothing() {
        let list = rows(0, None);

        assert_eq!(
            stepped(&[PaletteRowStep::Next], list),
            PaletteRowCursor::default()
        );
    }
}
