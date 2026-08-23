//! Character-indexed text edits.

/// A text edit expressed in character indices within the buffer.
///
/// `range` is half-open. An empty range inserts; empty `insert` text deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: core::ops::Range<usize>,
    pub insert: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditBatchSummary {
    /// Conservative range in the resulting buffer.
    pub changed_range: core::ops::Range<usize>,
    pub cursor: crate::buffer::Pos,
    pub edits_applied: usize,
}

impl Edit {
    pub fn insert(at_char: usize, text: impl Into<String>) -> Self {
        Self {
            range: at_char..at_char,
            insert: text.into(),
        }
    }

    pub fn delete(range: core::ops::Range<usize>) -> Self {
        Self {
            range,
            insert: String::new(),
        }
    }

    pub fn replace(range: core::ops::Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            insert: text.into(),
        }
    }
}
