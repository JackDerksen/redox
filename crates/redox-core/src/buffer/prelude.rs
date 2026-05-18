//! Convenience re-exports for the `buffer` module.
//!
//! Downstream crates can `use redox_core::buffer::prelude::*` when they need the
//! common buffer types without importing each symbol individually.

pub use super::Edit;
pub use super::Pos;
pub use super::Selection;
pub use super::TextBuffer;
pub use super::TextObjectEditPlan;
pub use super::TextObjectKind;
pub use super::TextObjectScope;
pub use super::TextObjectSpec;
pub use super::VisualModeKind;
pub use super::VisualSelectionEditPlan;
