//! Editing operations for `TextBuffer`.
//!
//! This file is meant to be included as part of the `buffer::text_buffer` module
//! and adds editing-focused methods via an `impl TextBuffer` block.
//!
//! Design goals:
//! - keep public methods small and composable
//! - use char indices (ropey’s primary indexing model) internally
//! - return updated `Pos`/`Selection` to make call sites explicit
//! - support both single edits and batched edit application

use crate::buffer::{Edit, EditBatchSummary, Pos, Selection, TextBuffer};

impl TextBuffer {
    /// Insert `text` at the given logical position.
    ///
    /// Returns the new cursor position (at the end of inserted text).
    ///
    /// This is a primitive operation for higher-level editing commands.
    pub fn insert(&mut self, pos: Pos, text: &str) -> Pos {
        let at = self.pos_to_char(pos);
        self.rope.insert(at, text);

        let inserted_chars = text.chars().count();
        self.char_to_pos(at + inserted_chars)
    }

    /// Delete a range between two positions (order-independent).
    ///
    /// Returns the resulting cursor position (at the start of deletion).
    pub fn delete_range(&mut self, a: Pos, b: Pos) -> Pos {
        let start = self.pos_to_char(crate::buffer::util::min_pos(self, a, b));
        let end = self.pos_to_char(crate::buffer::util::max_pos(self, a, b));

        if start < end {
            self.rope.remove(start..end);
        }

        self.char_to_pos(start)
    }

    /// Delete the selection (if any). Returns `(new_cursor, did_delete)`.
    pub fn delete_selection(&mut self, sel: Selection) -> (Pos, bool) {
        if sel.is_empty() {
            return (self.clamp_pos(sel.cursor), false);
        }

        let (start, end) = sel.ordered();
        let new_cursor = self.delete_range(start, end);
        (new_cursor, true)
    }

    /// Backspace behavior:
    /// - if the selection is non-empty, delete it
    /// - otherwise delete the char before the cursor (if any)
    ///
    /// Returns an empty selection at the updated cursor.
    pub fn backspace(&mut self, sel: Selection) -> Selection {
        if !sel.is_empty() {
            let (cursor, _) = self.delete_selection(sel);
            return Selection::empty(cursor);
        }

        let cursor = self.clamp_pos(sel.cursor);
        let at = self.pos_to_char(cursor);
        if at == 0 {
            return Selection::empty(cursor);
        }

        let start = at - 1;
        self.rope.remove(start..at);
        let new_cursor = self.char_to_pos(start);
        Selection::empty(new_cursor)
    }

    /// Delete (forward) behavior:
    /// - if the selection is non-empty, delete it
    /// - otherwise delete the char at the cursor (if any)
    ///
    /// Returns an empty selection at the updated cursor.
    pub fn delete(&mut self, sel: Selection) -> Selection {
        if !sel.is_empty() {
            let (cursor, _) = self.delete_selection(sel);
            return Selection::empty(cursor);
        }

        let cursor = self.clamp_pos(sel.cursor);
        let at = self.pos_to_char(cursor);
        let maxc = self.len_chars();

        if at >= maxc {
            return Selection::empty(cursor);
        }

        self.rope.remove(at..at + 1);
        let new_cursor = self.char_to_pos(at);
        Selection::empty(new_cursor)
    }

    /// Insert a newline at the cursor (or replace the selection).
    ///
    /// Returns an empty selection at the updated cursor.
    pub fn insert_newline(&mut self, sel: Selection) -> Selection {
        if !sel.is_empty() {
            let (start, end) = sel.ordered();
            let cursor = self.delete_range(start, end);
            let new_cursor = self.insert(cursor, "\n");
            return Selection::empty(new_cursor);
        }

        let cursor = self.clamp_pos(sel.cursor);
        let new_cursor = self.insert(cursor, "\n");
        Selection::empty(new_cursor)
    }

    /// Apply an `Edit` expressed in char indices.
    ///
    /// Returns the resulting cursor position (end of inserted text, or start of deletion).
    pub fn apply_edit(&mut self, edit: Edit) -> Pos {
        let maxc = self.len_chars();
        let start = edit.range.start.min(maxc);
        let end = edit.range.end.min(maxc);
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };

        if start < end {
            self.rope.remove(start..end);
        }

        if !edit.insert.is_empty() {
            self.rope.insert(start, &edit.insert);
            let inserted_chars = edit.insert.chars().count();
            self.char_to_pos(start + inserted_chars)
        } else {
            self.char_to_pos(start)
        }
    }

    /// Apply multiple edits sequentially and return a transaction-style summary.
    ///
    /// Edits are applied in input order against the current buffer state.
    pub fn apply_edits(&mut self, edits: &[Edit]) -> EditBatchSummary {
        let mut changed_start = usize::MAX;
        let mut changed_end = 0usize;
        let mut cursor = self.char_to_pos(self.len_chars());

        for edit in edits {
            let maxc = self.len_chars();
            let start = edit.range.start.min(maxc);
            let end = edit.range.end.min(maxc);
            let (start, _) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };

            cursor = self.apply_edit(edit.clone());
            let cursor_char = self.pos_to_char(cursor);

            changed_start = changed_start.min(start);
            changed_end = changed_end.max(cursor_char.max(start));
        }

        if edits.is_empty() {
            let cursor = self.char_to_pos(self.len_chars());
            let at = self.pos_to_char(cursor);
            return EditBatchSummary {
                changed_range: at..at,
                cursor,
                edits_applied: 0,
            };
        }

        EditBatchSummary {
            changed_range: changed_start..changed_end,
            cursor,
            edits_applied: edits.len(),
        }
    }

    /// Replace the current selection with `text` (if selection is empty, behaves like insert).
    /// This is a convenience method that a bunch of editor actions can use.
    ///
    /// Returns an empty selection at the updated cursor.
    pub fn replace_selection(&mut self, sel: Selection, text: &str) -> Selection {
        if !sel.is_empty() {
            let (start, end) = sel.ordered();
            let cursor = self.delete_range(start, end);
            let cursor = self.insert(cursor, text);
            Selection::empty(cursor)
        } else {
            let cursor = self.insert(sel.cursor, text);
            Selection::empty(cursor)
        }
    }

    /// Move a contiguous line range up by one line.
    ///
    /// Returns the moved range after the operation, or `None` when movement is
    /// not possible (for example when the range already starts at line 0).
    pub fn move_line_range_up_once(
        &mut self,
        start_line: usize,
        end_line_inclusive: usize,
    ) -> Option<(usize, usize)> {
        let (start, end) = self.normalized_line_range(start_line, end_line_inclusive);
        if start == 0 {
            return None;
        }

        let first = start - 1;
        let last = end;
        let mut entries = self.collect_line_entries(first, last);
        entries.rotate_left(1);
        let mut replacement = entries.join("\n");
        if last + 1 < self.len_lines() {
            replacement.push('\n');
        }

        let replace_start = self.line_to_char(first);
        let replace_end = self.line_full_end_char(last);
        self.rope.remove(replace_start..replace_end);
        self.rope.insert(replace_start, &replacement);

        Some((start - 1, end - 1))
    }

    /// Move a contiguous line range down by one line.
    ///
    /// Returns the moved range after the operation, or `None` when movement is
    /// not possible (for example when the range already ends at the final line).
    pub fn move_line_range_down_once(
        &mut self,
        start_line: usize,
        end_line_inclusive: usize,
    ) -> Option<(usize, usize)> {
        let (start, end) = self.normalized_line_range(start_line, end_line_inclusive);
        if end + 1 >= self.len_lines() {
            return None;
        }

        let first = start;
        let last = end + 1;
        let mut entries = self.collect_line_entries(first, last);
        entries.rotate_right(1);
        let mut replacement = entries.join("\n");
        if last + 1 < self.len_lines() {
            replacement.push('\n');
        }

        let replace_start = self.line_to_char(first);
        let replace_end = self.line_full_end_char(last);
        self.rope.remove(replace_start..replace_end);
        self.rope.insert(replace_start, &replacement);

        Some((start + 1, end + 1))
    }

    fn normalized_line_range(
        &self,
        start_line: usize,
        end_line_inclusive: usize,
    ) -> (usize, usize) {
        let (start, end) = if start_line <= end_line_inclusive {
            (start_line, end_line_inclusive)
        } else {
            (end_line_inclusive, start_line)
        };
        let start = self.clamp_line(start);
        let end = self.clamp_line(end.max(start));
        (start, end)
    }

    fn collect_line_entries(&self, start_line: usize, end_line_inclusive: usize) -> Vec<String> {
        let mut entries = Vec::with_capacity(end_line_inclusive.saturating_sub(start_line) + 1);
        for line in start_line..=end_line_inclusive {
            entries.push(self.line_string(line));
        }
        entries
    }
}
