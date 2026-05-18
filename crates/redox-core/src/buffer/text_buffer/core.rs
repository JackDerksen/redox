//! Core `TextBuffer` definition and constructors.
//!
//! Line indexing, movement, slicing, and editing live in sibling modules as
//! additional `impl TextBuffer` blocks.

use anyhow::{Context as _, Result};
use ropey::Rope;
use std::fs::File;
use std::io::BufReader;

/// A Ropey-backed text buffer.
///
/// Conventions:
/// - The backing store is a `ropey::Rope`.
/// - Public APIs use character indices and logical positions (`line`, `col` in
///   chars), matching Ropey's safe indexing model.
/// - Visual columns, modes, undo history, and viewport state live outside this
///   type so multiple frontends can build on the same core.
#[derive(Debug, Clone)]
pub struct TextBuffer {
    pub(super) rope: Rope,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBuffer {
    /// Create an empty buffer.
    #[inline]
    pub fn new() -> Self {
        Self { rope: Rope::new() }
    }

    /// Create a buffer from UTF-8 text.
    #[inline]
    pub fn from_str(s: &str) -> Self {
        Self {
            rope: Rope::from_str(s),
        }
    }

    /// Load a file as UTF-8 and create a buffer.
    ///
    /// This uses `ropey`'s streaming reader path and requires valid UTF-8.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .with_context(|| format!("failed to read file: {}", path.to_string_lossy()))?;
        let reader = BufReader::new(file);
        let rope = Rope::from_reader(reader)
            .with_context(|| format!("file is not valid UTF-8: {}", path.to_string_lossy()))?;
        Ok(Self { rope })
    }

    /// Access the underlying rope.
    ///
    /// Prefer higher-level APIs in other modules for most editor operations.
    #[inline]
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Mutable access to the underlying rope.
    ///
    /// Prefer dedicated editing APIs so invariants and bookkeeping remain easy to maintain.
    #[inline]
    pub fn rope_mut(&mut self) -> &mut Rope {
        &mut self.rope
    }

    /// Number of Unicode scalar values in the buffer.
    #[inline]
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Returns true when the buffer contains no characters.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }
}
