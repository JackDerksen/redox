//! Ropey-backed text buffer module.
//!
//! Public buffer APIs are organized by concern:
//! - `pos.rs`: logical positions and selections
//! - `edit.rs`: edit representation (char-indexed)
//! - `text_buffer/`: the [`TextBuffer`] implementation
//! - `util.rs`: internal helper functions
//! - `prelude.rs`: convenience re-exports for downstream crates

mod edit;
mod pos;
pub mod text_buffer;
mod undo;
mod util;

pub mod prelude;

pub use edit::{Edit, EditBatchSummary};
pub use pos::{Pos, Selection};
pub use text_buffer::{
    DelimiterKind, TextBuffer, TextObjectEditPlan, TextObjectKind, TextObjectScope, TextObjectSpec,
    VisualModeKind, VisualSelectionEditPlan,
};
pub use undo::{TextDiff, UndoCheckpoint, UndoHistory, UndoRecord};

#[cfg(test)]
mod tests;
