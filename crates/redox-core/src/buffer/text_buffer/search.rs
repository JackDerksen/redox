//! Plain-text search helpers for `TextBuffer`.
//!
//! These helpers intentionally implement only literal substring and same-line
//! character search semantics for now. Regex-aware search can layer on later
//! for stuff like `:s/` commands.

use super::TextBuffer;
use crate::buffer::Pos;

impl TextBuffer {
    /// Find the next occurrence of `needle` after `pos` on the same line.
    pub fn find_char_after_on_line(&self, pos: Pos, needle: char) -> Option<Pos> {
        let pos = self.clamp_pos(pos);
        let line = self.clamp_line(pos.line);
        let line_text = self.line_slice(line);

        for (col, ch) in line_text
            .chars()
            .enumerate()
            .skip(pos.col.saturating_add(1))
        {
            if ch == needle {
                return Some(Pos::new(line, col));
            }
        }

        None
    }

    /// Find all non-overlapping literal matches of `needle` in the buffer.
    ///
    /// Returned ranges are half-open `(start, end)` position pairs.
    pub fn find_matches(&self, needle: &str) -> Vec<(Pos, Pos)> {
        if needle.is_empty() {
            return Vec::new();
        }

        // TODO: this is inefficient as hell since it's converting the whole buffer to a string.
        // Fine for now, but fix it when I add regex-based searching.
        let source = self.to_string();
        source
            .match_indices(needle)
            .map(|(start_byte, matched)| {
                let start_char = self.rope().byte_to_char(start_byte);
                let end_char = self
                    .rope()
                    .byte_to_char(start_byte.saturating_add(matched.len()));
                (self.char_to_pos(start_char), self.char_to_pos(end_char))
            })
            .collect()
    }
}
