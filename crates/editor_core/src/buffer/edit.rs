//! Edit types for applying changes to a `TextBuffer`.
//!
//! This module is intentionally small and self-contained so it can be reused by
//! higher-level systems (undo/redo, macros, repeat, etc).
//!
//! All indices are **character indices** (Unicode scalar values), matching
//! `ropey`'s primary indexing model.

/// A text edit expressed in character indices within the buffer.
///
/// The `range` is half-open: `[start, end)`.
/// - To represent an insertion, use an empty range: `start == end`.
/// - To represent a deletion, use an empty `insert` string.
/// - To represent a replacement, set both a non-empty range and insert text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: core::ops::Range<usize>,
    pub insert: String,
}

impl Edit {
    /// Create an insertion at the given char index.
    pub fn insert(at_char: usize, text: impl Into<String>) -> Self {
        Self {
            range: at_char..at_char,
            insert: text.into(),
        }
    }

    /// Create a deletion for the given char range.
    pub fn delete(range: core::ops::Range<usize>) -> Self {
        Self {
            range,
            insert: String::new(),
        }
    }

    /// Create a replacement for the given char range.
    pub fn replace(range: core::ops::Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            insert: text.into(),
        }
    }
}
