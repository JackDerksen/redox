//! Core editor primitives.
//!
//! `redox-core` provides the UI-agnostic pieces of the editor:
//! - [`TextBuffer`], a Ropey-backed text buffer with char-indexed editing APIs.
//! - [`EditorSession`], a multi-buffer session model with dirty tracking and
//!   bounded incremental loading.
//! - [`motion`], deterministic Vim-like cursor motion helpers.
//!
//! Notes on indexing
//! - `ropey::Rope` is UTF-8 text stored as a rope.
//! - Most editing operations are most naturally expressed in **char indices**
//!   (`usize` counts of Unicode scalar values), because `ropey` exposes many APIs
//!   in terms of `char` offsets.
//! - The UI may need **byte indices** for interoperability with external data,
//!   but those are not used as the primary index type in this crate.

pub const SOFT_TAB_WIDTH: usize = 4;
pub const SOFT_TAB: &str = "    ";

pub mod buffer;
pub mod fuzzy;
pub mod io;
pub mod logic;
pub mod motion;
pub mod session;
pub mod text;

pub use buffer::{
    DelimiterKind, Edit, EditBatchSummary, Pos, Selection, TextBuffer, TextDiff,
    TextObjectEditPlan, TextObjectKind, TextObjectScope, TextObjectSpec, UndoCheckpoint,
    UndoHistory, UndoRecord, VisualModeKind, VisualSelectionEditPlan,
};
pub use fuzzy::{
    FuzzyMatch, FuzzyQuery, PathMatchScore, compare_path_match_scores, fuzzy_match_ranges,
    path_match_score,
};
pub use session::{
    BufferId, BufferKind, BufferLoadPhase, BufferLoadStatus, BufferMeta, BufferSummary,
    EditorSession, ExternalFileChange, ExternalFileChangeKind,
};

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
