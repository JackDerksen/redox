//! Clamped line access. Line content excludes its trailing newline.

use std::cmp::min;

use crate::buffer::{TextBuffer, TextSlice};

impl TextBuffer {
    /// Return the logical line count. Even an empty buffer has one line.
    #[inline]
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    #[inline]
    pub fn clamp_line(&self, line: usize) -> usize {
        let last = self.len_lines().saturating_sub(1);
        min(line, last)
    }

    /// Return the absolute character index at the start of a clamped line.
    #[inline]
    pub fn line_to_char(&self, line: usize) -> usize {
        let line = self.clamp_line(line);
        self.rope.line_to_char(line)
    }

    /// Return the line containing a clamped character index.
    #[inline]
    pub fn char_to_line(&self, char_idx: usize) -> usize {
        let c = min(char_idx, self.len_chars());
        self.rope.char_to_line(c)
    }

    /// Return a line's editable character length, excluding its newline.
    pub fn line_len_chars(&self, line: usize) -> usize {
        let line = self.clamp_line(line);
        let slice = self.rope.line(line);

        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
        }

        len
    }

    /// Return the line end when the line contains only whitespace.
    pub fn line_first_non_whitespace_col(&self, line: usize) -> usize {
        let line = self.clamp_line(line);
        let slice = self.line_slice(line);

        for (idx, ch) in slice.chars().enumerate() {
            if !ch.is_whitespace() {
                return idx;
            }
        }

        self.line_len_chars(line)
    }

    pub fn line_string(&self, line: usize) -> String {
        self.line_slice(line).to_string()
    }

    pub fn line_slice(&self, line: usize) -> TextSlice<'_> {
        let line = self.clamp_line(line);
        let range = self.line_char_range(line);
        TextSlice::new(self.rope.slice(range))
    }

    /// Return the half-open character range of a line without its newline.
    pub fn line_char_range(&self, line: usize) -> std::ops::Range<usize> {
        let line = self.clamp_line(line);
        let start = self.rope.line_to_char(line);

        let end_including_newline = start + self.rope.line(line).len_chars();
        let end =
            if end_including_newline > start && self.rope.char(end_including_newline - 1) == '\n' {
                end_including_newline - 1
            } else {
                end_including_newline
            };

        start..end
    }

    /// Return the end boundary including a newline when one exists.
    pub fn line_full_end_char(&self, line: usize) -> usize {
        let line = self.clamp_line(line);
        if line + 1 < self.len_lines() {
            self.rope.line_to_char(line + 1)
        } else {
            self.rope.line_to_char(line) + self.line_len_chars(line)
        }
    }
}
