//! Editing operations for `TextBuffer`.
//!
//! This file is meant to be included as part of the `buffer::text_buffer` module
//! and adds editing-focused methods via an `impl TextBuffer` block.
//!
//! Design goals:
//! - keep public methods small and composable
//! - use char indices (ropey’s primary indexing model) internally
//! - return updated `Pos`/`Selection` to make call sites explicit
//! - keep it easy to extend later (undo/redo, transactions, multiple cursors, etc.)

use ropey::Rope;

use crate::buffer::{Edit, Pos, Selection, TextBuffer};

impl TextBuffer {
    /// Insert `text` at the given logical position.
    ///
    /// Returns the new cursor position (at the end of inserted text).
    ///
    /// NOTE: This is a primitive operation I can build higher-level commands on top of
    /// (e.g. replace-selection-then-insert, paste, auto-indent, etc).
    pub fn insert(&mut self, pos: Pos, text: &str) -> Pos {
        let at = self.pos_to_char(pos);
        self.rope.insert(at, text);

        // Compute end position by converting at + inserted_chars.
        // We avoid `text.chars().count()` to keep indexing consistent with ropey.
        let inserted_chars = Rope::from_str(text).len_chars();
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
    /// NOTE: This is intended as a low-level building block for future undo/redo
    /// so I can store `Edit`s, invert them, and replay them.
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
            let inserted_chars = Rope::from_str(&edit.insert).len_chars();
            self.char_to_pos(start + inserted_chars)
        } else {
            self.char_to_pos(start)
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
}
