//! Position conversion and cursor movement helpers for `TextBuffer`.
//!
//! This file is intended to be included by `buffer::text_buffer`'s module wiring,
//! so that `TextBuffer`'s implementation can be split into focused, maintainable
//! chunks.
//!
//! Design notes on extensibility:
//! - Positions are logical `(line, col)` in **char units** (Unicode scalar values),
//!   matching Ropey's indexing model.
//! - Methods clamp inputs defensively, so higher-level code can stay simpler.
//! - Visual column/grapheme cluster concerns are deliberately out of scope here;
//!   those can be layered on later (eg. a view layer that maps `Pos` <-> screen).

use std::cmp::min;

use super::TextBuffer;
use crate::buffer::Pos;

impl TextBuffer {
    /// Clamp a position to a valid location in the buffer.
    ///
    /// - Line is clamped to `[0, len_lines - 1]`
    /// - Column is clamped to `[0, line_len_chars(line)]`
    #[inline]
    pub fn clamp_pos(&self, pos: Pos) -> Pos {
        let line = self.clamp_line(pos.line);
        let max_col = self.line_len_chars(line);
        let col = min(pos.col, max_col);
        Pos { line, col }
    }

    /// Convert `Pos` (line+col) to absolute char index in the rope.
    ///
    /// The position is clamped first.
    #[inline]
    pub fn pos_to_char(&self, pos: Pos) -> usize {
        let pos = self.clamp_pos(pos);
        self.rope.line_to_char(pos.line) + pos.col
    }

    /// Convert absolute char index to `Pos` (line+col).
    ///
    /// `char_idx` is clamped to `[0, len_chars]`.
    #[inline]
    pub fn char_to_pos(&self, char_idx: usize) -> Pos {
        let c = min(char_idx, self.len_chars());
        let line = self.rope.char_to_line(c);
        let line_start = self.rope.line_to_char(line);
        let col = c - line_start;

        // If `c` points at a newline, clamp col to `line_len` (ie. end of the line).
        let max_col = self.line_len_chars(line);
        Pos {
            line,
            col: min(col, max_col),
        }
    }

    /// Move position left by one char, staying within buffer.
    #[inline]
    pub fn move_left(&self, pos: Pos) -> Pos {
        let c = self.pos_to_char(pos);
        if c == 0 {
            return Pos::zero();
        }
        self.char_to_pos(c - 1)
    }

    /// Move position right by one char, staying within buffer.
    #[inline]
    pub fn move_right(&self, pos: Pos) -> Pos {
        let c = self.pos_to_char(pos);
        let maxc = self.len_chars();
        if c >= maxc {
            return self.char_to_pos(maxc);
        }
        self.char_to_pos(c + 1)
    }

    /// Move up one line, preserving column as much as possible.
    ///
    /// NOTE: This is a simple version with no goal/preferred column tracking.
    /// If I decide I want Vim-like vertical motion that remembers a preferred column,
    /// I should store that in higher-level state and clamp it using `line_len_chars(...)`.
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

    /// Move down one line, preserving column as much as possible.
    ///
    /// This is a simple version (no goal/preferred column tracking).
    /// NOTE: Same as above :)
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

    /// Get the char at a position, if it's within the line's content (not including newline).
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

    /// Get the char before a position, if one exists.
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
