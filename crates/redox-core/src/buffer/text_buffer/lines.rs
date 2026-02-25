//! Line-oriented helpers for `TextBuffer`.
//!
//! This file is intended to be included by the parent `text_buffer` module, and
//! adds line/indexing utilities as an inherent `impl` on `TextBuffer`.
//!
//! Design notes
//! - These APIs use **char indices** (Unicode scalar values), matching `ropey`.
//! - Treats the trailing `'\n'` as *not part of the editable line*, so
//!   `line_len_chars()` excludes it when present.
//! - All functions are defensive, meaning they clamp out-of-range inputs.

use std::cmp::min;

use crate::buffer::TextBuffer;

impl TextBuffer {
    /// Number of lines in the buffer.
    ///
    /// Ropey counts lines by `'\n'` boundaries and always reports at least 1 line,
    /// even for empty text.
    #[inline]
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// Clamp a line index to the valid range `[0, len_lines - 1]`.
    ///
    /// If the buffer is empty, Ropey still reports `len_lines() == 1`, so this
    /// always returns a valid line index.
    #[inline]
    pub fn clamp_line(&self, line: usize) -> usize {
        let last = self.len_lines().saturating_sub(1);
        min(line, last)
    }

    /// Returns the absolute char index at the start of `line`.
    ///
    /// `line` is clamped into a valid range.
    #[inline]
    pub fn line_to_char(&self, line: usize) -> usize {
        let line = self.clamp_line(line);
        self.rope.line_to_char(line)
    }

    /// Returns the line index containing `char_idx`.
    ///
    /// `char_idx` is clamped to `[0, len_chars]`.
    #[inline]
    pub fn char_to_line(&self, char_idx: usize) -> usize {
        let c = min(char_idx, self.len_chars());
        self.rope.char_to_line(c)
    }

    /// Returns the length of `line` in chars, excluding a trailing `'\n'` if present.
    ///
    /// This corresponds to the number of valid "columns" for a `(line, col)` cursor
    /// model where the newline is not considered part of the line.
    pub fn line_len_chars(&self, line: usize) -> usize {
        let line = self.clamp_line(line);
        let slice = self.rope.line(line);

        // Ropey line slices typically include the newline if present.
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
        }

        len
    }

    /// Returns the line content as a `String`, excluding a trailing `'\n'` if present.
    pub fn line_string(&self, line: usize) -> String {
        let line = self.clamp_line(line);
        let slice = self.rope.line(line);
        let s = slice.to_string();
        s.strip_suffix('\n').unwrap_or(&s).to_string()
    }

    /// Returns the char range `[start, end)` for the line content, excluding a trailing `'\n'`.
    ///
    /// This will be useful for operations like "delete to end of line" or yanking the line
    /// content without the newline.
    pub fn line_char_range(&self, line: usize) -> std::ops::Range<usize> {
        let line = self.clamp_line(line);
        let start = self.rope.line_to_char(line);

        // `line(line).len_chars()` includes the newline if present.
        let end_including_newline = start + self.rope.line(line).len_chars();

        // Drop exactly one trailing '\n' if present.
        let end =
            if end_including_newline > start && self.rope.char(end_including_newline - 1) == '\n' {
                end_including_newline - 1
            } else {
                end_including_newline
            };

        start..end
    }
}
