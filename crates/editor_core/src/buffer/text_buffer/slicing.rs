//! `TextBuffer` slicing helpers.
//!
//! Design notes:
//! - All indices are **character indices** (Unicode scalar values) to match `ropey`.
//! - These helpers are intentionally allocating (`String`) for ergonomics.
//!   If I later need more performance, add `RopeSlice`-returning variants
//!   without changing call sites that just need owned strings.

use std::cmp::min;

use super::TextBuffer;
use crate::buffer::{Pos, Selection};

impl TextBuffer {
    /// Get the full buffer as a `String`.
    ///
    /// For large buffers, this allocates. Kept as an inherent method so call sites
    /// can use `b.to_string()` without depending on `Display`/`ToString`.
    #[inline]
    pub fn to_string(&self) -> String {
        self.rope().to_string()
    }

    /// Get a `String` for a character range `[start, end)`.
    ///
    /// - Indices are clamped to the buffer bounds.
    /// - If `start > end`, the values are swapped.
    ///
    /// This is a convenience API; it allocates.
    pub fn slice_chars(&self, mut start: usize, mut end: usize) -> String {
        let maxc = self.len_chars();
        start = min(start, maxc);
        end = min(end, maxc);
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        self.rope().slice(start..end).to_string()
    }

    /// Get the selected text for a selection (ordered).
    ///
    /// This is a convenience API; it allocates.
    pub fn slice_selection(&self, sel: Selection) -> String {
        let (a, b) = sel.ordered();
        let start = self.pos_to_char(a);
        let end = self.pos_to_char(b);
        self.slice_chars(start, end)
    }

    /// Convenience: slice by two logical positions (order-independent).
    ///
    /// This is useful if there are two cursors/marks and I want the substring
    /// between them without explicitly building a `Selection`.
    pub fn slice_pos_range(&self, a: Pos, b: Pos) -> String {
        let start = self.pos_to_char(a);
        let end = self.pos_to_char(b);
        self.slice_chars(start, end)
    }
}
