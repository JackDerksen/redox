//! Ropey-backed text buffer module.
//!
//! This module is split across multiple files to keep each concern small:
//! - `pos.rs`: logical positions and selections
//! - `edit.rs`: edit representation (char-indexed)
//! - `text_buffer.rs`: the `TextBuffer` implementation (backed by `ropey::Rope`)
//! - `util.rs`: internal helper functions
//! - `tests.rs`: unit tests
//! - `prelude.rs`: convenience re-exports for downstream crates

mod edit;
mod pos;
pub mod text_buffer;
mod util;

pub mod prelude;

pub use edit::Edit;
pub use pos::{Pos, Selection};
pub use text_buffer::TextBuffer;

#[cfg(test)]
mod tests;
