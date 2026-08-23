//! Text storage, editing, selections, text objects, and undo history.

mod edit;
mod pos;
pub mod text_buffer;
mod undo;
mod util;

pub use edit::{Edit, EditBatchSummary};
pub use pos::{Pos, Selection};
pub use text_buffer::{
    DelimiterKind, TextBuffer, TextObjectEditPlan, TextObjectKind, TextObjectScope, TextObjectSpec,
    TextSlice, VisualModeKind, VisualSelectionEditPlan,
};
pub use undo::{TextDiff, UndoCheckpoint, UndoHistory, UndoNodeId, UndoRecord, UndoTreeEntry};
