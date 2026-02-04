//! Convenience re-exports for the `buffer` module.
//!
//! The plan:
//! - `use editor_core::buffer::prelude::*;` in higher-level editor code.
//! - keep call sites clean without importing many individual symbols.

pub use super::Edit;
pub use super::Pos;
pub use super::Selection;
pub use super::TextBuffer;
