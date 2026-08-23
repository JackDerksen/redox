//! Clamped conversion and movement for logical `(line, column)` positions.

use std::cmp::min;

use super::TextBuffer;
use crate::buffer::Pos;

impl TextBuffer {
    #[inline]
    pub fn clamp_pos(&self, pos: Pos) -> Pos {
        let line = self.clamp_line(pos.line);
        let max_col = self.line_len_chars(line);
        let col = min(pos.col, max_col);
        Pos { line, col }
    }

    /// Convert a position to an absolute character index.
    #[inline]
    pub fn pos_to_char(&self, pos: Pos) -> usize {
        let pos = self.clamp_pos(pos);
        self.rope.line_to_char(pos.line) + pos.col
    }

    /// Convert a clamped character index to a position.
    #[inline]
    pub fn char_to_pos(&self, char_idx: usize) -> Pos {
        let c = min(char_idx, self.len_chars());
        let line = self.rope.char_to_line(c);
        let line_start = self.rope.line_to_char(line);
        let col = c - line_start;

        let max_col = self.line_len_chars(line);
        Pos {
            line,
            col: min(col, max_col),
        }
    }

    #[inline]
    pub fn move_left(&self, pos: Pos) -> Pos {
        let c = self.pos_to_char(pos);
        if c == 0 {
            return Pos::zero();
        }
        self.char_to_pos(c - 1)
    }

    #[inline]
    pub fn move_right(&self, pos: Pos) -> Pos {
        let c = self.pos_to_char(pos);
        let maxc = self.len_chars();
        if c >= maxc {
            return self.char_to_pos(maxc);
        }
        self.char_to_pos(c + 1)
    }

    #[inline]
    pub fn move_up(&self, pos: Pos) -> Pos {
        let pos = self.clamp_pos(pos);
        if pos.line == 0 {
            return pos;
        }
        let new_line = pos.line - 1;
        let new_col = min(pos.col, self.line_len_chars(new_line));
        Pos::new(new_line, new_col)
    }

    #[inline]
    pub fn move_down(&self, pos: Pos) -> Pos {
        let pos = self.clamp_pos(pos);
        let last = self.len_lines().saturating_sub(1);
        if pos.line >= last {
            return pos;
        }
        let new_line = pos.line + 1;
        let new_col = min(pos.col, self.line_len_chars(new_line));
        Pos::new(new_line, new_col)
    }

    /// Return the character at a position, excluding line-ending newlines.
    #[inline]
    pub fn char_at(&self, pos: Pos) -> Option<char> {
        let pos = self.clamp_pos(pos);
        let line_len = self.line_len_chars(pos.line);
        if pos.col >= line_len {
            return None;
        }
        let idx = self.pos_to_char(pos);
        Some(self.rope.char(idx))
    }

    #[inline]
    pub fn char_before(&self, pos: Pos) -> Option<char> {
        let c = self.pos_to_char(pos);
        if c == 0 {
            None
        } else {
            Some(self.rope.char(c - 1))
        }
    }
}
