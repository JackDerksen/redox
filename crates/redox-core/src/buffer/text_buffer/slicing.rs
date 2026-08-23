use super::{TextBuffer, TextSlice};
use crate::buffer::{Pos, Selection};

impl TextBuffer {
    /// Borrow a clamped, order-independent character range.
    pub fn slice_chars_ref(&self, start: usize, end: usize) -> TextSlice<'_> {
        let (start, end) = self.normalized_char_range(start, end);
        TextSlice::new(self.rope.slice(start..end))
    }

    pub fn slice_selection_ref(&self, sel: Selection) -> TextSlice<'_> {
        let (a, b) = sel.ordered();
        let start = self.pos_to_char(a);
        let end = self.pos_to_char(b);
        self.slice_chars_ref(start, end)
    }

    pub fn slice_pos_range_ref(&self, a: Pos, b: Pos) -> TextSlice<'_> {
        let start = self.pos_to_char(a);
        let end = self.pos_to_char(b);
        self.slice_chars_ref(start, end)
    }

    pub fn slice_chars(&self, start: usize, end: usize) -> String {
        self.slice_chars_ref(start, end).to_string()
    }

    pub fn slice_selection(&self, sel: Selection) -> String {
        self.slice_selection_ref(sel).to_string()
    }

    pub fn slice_pos_range(&self, a: Pos, b: Pos) -> String {
        self.slice_pos_range_ref(a, b).to_string()
    }
}
