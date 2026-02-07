//! Core editor primitives.
//!
//! This crate provides a rope-based text buffer (via `ropey`) and small, focused
//! navigation utilities suitable for implementing a Vim-like editor.
//!
//! Notes on indexing
//! - `ropey::Rope` is UTF-8 text stored as a rope.
//! - Most editing operations are most naturally expressed in **char indices**
//!   (`usize` counts of Unicode scalar values), because `ropey` exposes many APIs
//!   in terms of `char` offsets.
//! - The UI may need **byte indices** for interoperability with external data,
//!   but those are not used as the primary index type in this crate.

pub mod buffer;
pub mod io;
pub mod logic;
pub mod text;

// Prefer using the rope-backed buffer implementation from `buffer`.
// Re-export the common types here for ergonomic access by downstream crates.
pub use buffer::{Edit, Pos, Selection, TextBuffer};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_has_one_line() {
        let b = TextBuffer::new();
        assert_eq!(b.len_lines(), 1);
        assert_eq!(b.len_chars(), 0);
    }

    #[test]
    fn insert_and_delete_selection_smoke() {
        let mut b = TextBuffer::from_str("ab");

        let sel = Selection::empty(Pos::new(0, 2));
        let new_cursor = b.insert(sel.cursor, "c");
        assert_eq!(b.to_string(), "abc");
        assert_eq!(new_cursor, Pos::new(0, 3));

        let sel2 = Selection::new(Pos::new(0, 1), Pos::new(0, 2));
        let (cur, did) = b.delete_selection(sel2);
        assert!(did);
        assert_eq!(cur, Pos::new(0, 1));
        assert_eq!(b.to_string(), "ac");
    }
}
