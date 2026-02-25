//! Core `TextBuffer` definition and constructors.
//!
//! Note to self: this file intentionally only does a couple of things:
//! - It defines the `TextBuffer` type and its invariants.
//! - It provides basic constructors and low-level rope access.
//!
//! Everything else (line indexing, movement, slicing, editing) should live in
//! sibling modules as additional `impl TextBuffer` blocks.

use anyhow::{Context as _, Result};
use ropey::Rope;

/// A Ropey-backed text buffer.
///
/// Invariants and conventions:
/// - The backing store is a `ropey::Rope`.
/// - Public APIs should generally speak in char indices (Unicode scalar
///   value offsets) and logical positions (line/col in chars) because Ropey’s
///   safe indexing APIs are char-based.
/// - Byte indexing can be supported where needed, but should not be the primary
///   index type for the editor core.
///
/// Higher-level editor state (modes, undo, viewports, etc.) should be built on
/// top of this type rather than embedded inside it.
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
    /// Create an empty buffer
    #[inline]
    pub fn new() -> Self {
        Self { rope: Rope::new() }
    }

    /// Create a buffer from UTF-8 text
    #[inline]
    pub fn from_str(s: &str) -> Self {
        Self {
            rope: Rope::from_str(s),
        }
    }

    /// Load a file as UTF-8 and create a buffer.
    ///
    /// This is intentionally simple for now. It just:
    /// - reads the whole file into memory
    /// - requires valid UTF-8
    ///
    /// NOTE: If/when I add encoding detection or incremental IO, those should likely
    /// live in a separate IO-focused module.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();

        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read file: {}", path.to_string_lossy()))?;

        let s = String::from_utf8(bytes)
            .with_context(|| format!("file is not valid UTF-8: {}", path.to_string_lossy()))?;

        Ok(Self::from_str(&s))
    }

    /// Access the underlying rope.
    ///
    /// Prefer higher-level APIs in other modules for most editor operations.
    #[inline]
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Mutable access to the underlying rope (use this sparingly!)
    ///
    /// Prefer dedicated editing APIs so invariants and future bookkeeping (eg.
    /// undo/redo, marks, spans) remain easy to maintain.
    #[inline]
    pub fn rope_mut(&mut self) -> &mut Rope {
        &mut self.rope
    }

    /// Total number of chars in the buffer.
    ///
    /// Kept here because it is a fundamental primitive used by most other modules.
    #[inline]
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Whether or not the buffer contains zero characters (is empty).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }
}
