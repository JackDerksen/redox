//! Split implementation of the Ropey-backed `TextBuffer`.
//!
//! `TextBuffer` is one public type, implemented across focused modules:
//! - `core.rs`: struct definition + basic constructors/accessors
//! - `lines.rs`: line indexing helpers
//! - `positions.rs`: (line, col) conversions and cursor movement
//! - `slicing.rs`: extracting text
//! - `editing.rs`: mutation operations (insert/delete/apply edits)
//! - `words.rs`: word motions

mod core;
mod editing;
mod lines;
mod positions;
mod search;
mod selection;
mod slicing;
mod text_objects;
mod words;

pub use core::{TextBuffer, TextSlice};
pub use selection::{VisualModeKind, VisualSelectionEditPlan};
pub use text_objects::{
    DelimiterKind, TextObjectEditPlan, TextObjectKind, TextObjectScope, TextObjectSpec,
};
