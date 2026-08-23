//! Mutations that return the resulting cursor or selection.

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

    /// Replace the leading whitespace on `line`.
    ///
    /// Returns `(removed_chars, added_chars)` when the line changed.
    pub fn replace_line_indent(&mut self, line: usize, indent: &str) -> Option<(usize, usize)> {
        let line = self.clamp_line(line);
        let text = self.line_string(line);
        let existing_chars = text
            .chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .count();
        let existing: String = text.chars().take(existing_chars).collect();
        if existing == indent {
            return None;
        }

        let _ = self.delete_range(Pos::new(line, 0), Pos::new(line, existing_chars));
        let _ = self.insert(Pos::new(line, 0), indent);
        Some((existing_chars, indent.chars().count()))
    }

    /// Remove trailing spaces and tabs from every line in the buffer.
    ///
    /// Returns `true` when at least one line was changed.
    pub fn trim_trailing_whitespace(&mut self) -> bool {
        let mut changed = false;

        for line in (0..self.len_lines()).rev() {
            let text = self.line_string(line);
            let line_len = text.chars().count();
            let trimmed_len = text.trim_end_matches([' ', '\t']).chars().count();
            if trimmed_len == line_len {
                continue;
            }

            let _ = self.delete_range(Pos::new(line, trimmed_len), Pos::new(line, line_len));
            changed = true;
        }

        changed
    }

    /// Apply an `Edit` expressed in char indices.
    ///
    /// Returns the resulting cursor position (end of inserted text, or start of deletion).
    pub fn apply_edit(&mut self, edit: Edit) -> Pos {
        self.apply_edit_ref(&edit)
    }

    fn apply_edit_ref(&mut self, edit: &Edit) -> Pos {
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

            cursor = self.apply_edit_ref(edit);
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

    /// Paste text before the given cursor.
    ///
    /// When `linewise` is true, insertion happens at the start of the current line
    /// and the returned cursor moves to the end of inserted text.
    /// When `linewise` is false, insertion happens at the cursor and the returned
    /// cursor is at the end of inserted text.
    pub fn paste_before(&mut self, cursor: Pos, text: &str, linewise: bool) -> Pos {
        let insert_pos = if linewise {
            let line = self.clamp_line(cursor.line);
            self.clamp_pos(Pos::new(line, 0))
        } else {
            self.clamp_pos(cursor)
        };

        let end_pos = self.insert(insert_pos, text);
        if linewise {
            linewise_paste_cursor(end_pos)
        } else {
            end_pos
        }
    }

    /// Paste text after the given cursor.
    ///
    /// When `linewise` is true, insertion happens at the beginning of the next
    /// logical line (clamped at the buffer boundary), and the returned cursor
    /// moves to the end of inserted text.
    /// When `linewise` is false, insertion happens after the cursor char on the
    /// current line (or at line end when already at EOL), and the returned cursor
    /// is at the end of inserted text.
    pub fn paste_after(&mut self, cursor: Pos, text: &str, linewise: bool) -> Pos {
        let insert_pos = if linewise {
            let line = self.clamp_line(cursor.line);
            let target_line = (line + 1).min(self.len_lines());
            self.clamp_pos(Pos::new(target_line, 0))
        } else {
            let line = self.clamp_line(cursor.line);
            let line_len = self.line_len_chars(line);
            let col = if cursor.col < line_len {
                cursor.col.saturating_add(1)
            } else {
                line_len
            };
            Pos::new(line, col)
        };

        let end_pos = self.insert(insert_pos, text);
        if linewise {
            linewise_paste_cursor(end_pos)
        } else {
            end_pos
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

    /// Move a contiguous line range up by up to `count` lines.
    ///
    /// Returns the moved range after all possible steps, or `None` when the range
    /// cannot be moved up at all.
    pub fn move_line_range_up(
        &mut self,
        start_line: usize,
        end_line_inclusive: usize,
        count: usize,
    ) -> Option<(usize, usize)> {
        if count == 0 {
            return None;
        }

        let mut current = self.normalized_line_range(start_line, end_line_inclusive);
        let mut moved = false;
        for _ in 0..count {
            let Some(next) = self.move_line_range_up_once(current.0, current.1) else {
                break;
            };
            current = next;
            moved = true;
        }

        if moved { Some(current) } else { None }
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

    /// Move a contiguous line range down by up to `count` lines.
    ///
    /// Returns the moved range after all possible steps, or `None` when the range
    /// cannot be moved down at all.
    pub fn move_line_range_down(
        &mut self,
        start_line: usize,
        end_line_inclusive: usize,
        count: usize,
    ) -> Option<(usize, usize)> {
        if count == 0 {
            return None;
        }

        let mut current = self.normalized_line_range(start_line, end_line_inclusive);
        let mut moved = false;
        for _ in 0..count {
            let Some(next) = self.move_line_range_down_once(current.0, current.1) else {
                break;
            };
            current = next;
            moved = true;
        }

        if moved { Some(current) } else { None }
    }

    /// Indent each line and return the character count added per line.
    pub fn indent_line_span(
        &mut self,
        start_line: usize,
        end_line_inclusive: usize,
        count: usize,
        indent_unit: &str,
    ) -> Vec<(usize, usize)> {
        if count == 0 || indent_unit.is_empty() {
            return Vec::new();
        }

        let (start, end) = self.normalized_line_range(start_line, end_line_inclusive);
        let indent = indent_unit.repeat(count);
        let mut added_by_line = Vec::with_capacity(end.saturating_sub(start) + 1);
        for line in start..=end {
            let _ = self.insert(Pos::new(line, 0), &indent);
            added_by_line.push((line, indent.chars().count()));
        }
        added_by_line
    }

    /// Outdent each line and return the character count removed per line.
    pub fn outdent_line_span(
        &mut self,
        start_line: usize,
        end_line_inclusive: usize,
        count: usize,
        tab_width: usize,
    ) -> Vec<(usize, usize)> {
        if count == 0 || tab_width == 0 {
            return Vec::new();
        }

        let (start, end) = self.normalized_line_range(start_line, end_line_inclusive);
        let mut removed_by_line = Vec::with_capacity(end.saturating_sub(start) + 1);

        for line in start..=end {
            let text = self.line_string(line);
            let chars: Vec<char> = text.chars().collect();
            let mut idx = 0usize;
            let mut levels_left = count;
            while levels_left > 0 && idx < chars.len() {
                if chars[idx] == '\t' {
                    idx += 1;
                    levels_left -= 1;
                    continue;
                }

                let mut spaces = 0usize;
                while idx + spaces < chars.len() && chars[idx + spaces] == ' ' && spaces < tab_width
                {
                    spaces += 1;
                }
                if spaces == 0 {
                    break;
                }

                idx += spaces;
                levels_left -= 1;
            }

            if idx > 0 {
                let _ = self.delete_range(Pos::new(line, 0), Pos::new(line, idx));
            }
            removed_by_line.push((line, idx));
        }

        removed_by_line
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

fn linewise_paste_cursor(end_pos: Pos) -> Pos {
    Pos::new(end_pos.line.saturating_sub(1), 0)
}
