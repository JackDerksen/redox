//! `TextBuffer` slicing helpers.
//!
//! Design notes:
//! - All indices are **character indices** (Unicode scalar values) to match `ropey`.
//! - Non-allocating `RopeSlice` APIs are provided for hot paths.
//! - Allocating `String` helpers remain for ergonomics.

use std::cmp::min;

use ropey::RopeSlice;

use super::TextBuffer;
use crate::buffer::{Pos, Selection};

impl TextBuffer {
    /// Get a non-allocating slice for a character range `[start, end)`.
    ///
    /// - Indices are clamped to the buffer bounds.
    /// - If `start > end`, the values are swapped.
    pub fn slice_chars_ref(&self, mut start: usize, mut end: usize) -> RopeSlice<'_> {
        let maxc = self.len_chars();
        start = min(start, maxc);
        end = min(end, maxc);
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        self.rope().slice(start..end)
    }

    /// Get a non-allocating slice for a selection (ordered).
    pub fn slice_selection_ref(&self, sel: Selection) -> RopeSlice<'_> {
        let (a, b) = sel.ordered();
        let start = self.pos_to_char(a);
        let end = self.pos_to_char(b);
        self.slice_chars_ref(start, end)
    }

    /// Convenience: non-allocating slice by two logical positions (order-independent).
    pub fn slice_pos_range_ref(&self, a: Pos, b: Pos) -> RopeSlice<'_> {
        let start = self.pos_to_char(a);
        let end = self.pos_to_char(b);
        self.slice_chars_ref(start, end)
    }

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
    pub fn slice_chars(&self, start: usize, end: usize) -> String {
        self.slice_chars_ref(start, end).to_string()
    }

    /// Get the selected text for a selection (ordered).
    ///
    /// This is a convenience API; it allocates.
    pub fn slice_selection(&self, sel: Selection) -> String {
        self.slice_selection_ref(sel).to_string()
    }

    /// Convenience: slice by two logical positions (order-independent).
    ///
    /// This is useful when there are two cursors/marks and the substring
    /// between them without explicitly building a `Selection`.
    pub fn slice_pos_range(&self, a: Pos, b: Pos) -> String {
        self.slice_pos_range_ref(a, b).to_string()
    }
}
