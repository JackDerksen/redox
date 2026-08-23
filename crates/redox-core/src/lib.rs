//! UI-independent text storage and editor behaviour for Redox.
//!
//! [`TextBuffer`] is the boundary around Ropey. Its primary coordinates are
//! Unicode scalar-value indices and zero-based [`Pos`] values. Byte conversion is
//! available for protocols and parsers; terminal cells and graphemes stay in the
//! frontend.
//!
//! # Example
//!
//! ```
//! use redox_core::prelude::*;
//!
//! let mut buffer = TextBuffer::from("hello");
//! let cursor = buffer.insert(Pos::new(0, 5), " world");
//!
//! assert_eq!(cursor, Pos::new(0, 11));
//! assert_eq!(String::from(&buffer), "hello world");
//! ```

pub mod buffer;
pub mod fuzzy;
pub mod motion;
pub mod session;

/// Common text-buffer types for editor frontends and integrations.
///
/// More specialized APIs, such as sessions, fuzzy matching, and text objects,
/// remain available through explicit imports from the crate root.
pub mod prelude {
    pub use crate::motion::{Motion, apply_motion, apply_motion_for_operator, apply_motion_n};
    pub use crate::{Edit, EditBatchSummary, Pos, Selection, TextBuffer, TextSlice, UndoHistory};
}

pub use buffer::{
    DelimiterKind, Edit, EditBatchSummary, Pos, Selection, TextBuffer, TextDiff,
    TextObjectEditPlan, TextObjectKind, TextObjectScope, TextObjectSpec, TextSlice, UndoCheckpoint,
    UndoHistory, UndoNodeId, UndoRecord, UndoTreeEntry, VisualModeKind, VisualSelectionEditPlan,
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
mod tests;
