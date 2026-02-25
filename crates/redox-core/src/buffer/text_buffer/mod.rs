//! Split implementation of the Ropey-backed `TextBuffer`.
//!
//! The goal of this submodule is to keep the `TextBuffer` implementation easy to
//! navigate and extend by separating it into focused files:
//! - `core.rs`: struct definition + basic constructors/accessors
//! - `lines.rs`: line indexing helpers
//! - `pos.rs`: (line, col) conversions and cursor-ish movement
//! - `slice.rs`: extracting text
//! - `edit.rs`: mutation operations (insert/delete/apply edits)
//! - `word.rs`: word-ish motions (intentionally minimal, easy to swap later)
//!
//! `TextBuffer` remains a single public type re-exported by `buffer::mod.rs`.
//! All methods are inherent impls spread across these modules.

mod core;
mod editing;
mod lines;
mod positions;
mod slicing;
mod words;

pub use core::TextBuffer;
