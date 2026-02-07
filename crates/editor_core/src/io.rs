//! Core IO helpers for loading/saving text buffers.
//!
//! This module is intentionally small and UI-agnostic. It just provides helpers
//! that read and write UTF-8 text to/from the rope-backed `TextBuffer`.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::buffer::TextBuffer;

/// Read a UTF-8 file into a `TextBuffer`.
///
/// This is a simple, whole-file read (simple for the early development stage):
/// - loads entire file into memory
/// - requires valid UTF-8
///
/// Might add higher-level functions for encoding detection and streaming IO later
pub fn load_buffer(path: impl AsRef<Path>) -> Result<TextBuffer> {
    let path = path.as_ref();

    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read file: {}", path.to_string_lossy()))?;

    let text = String::from_utf8(bytes)
        .with_context(|| format!("file is not valid UTF-8: {}", path.to_string_lossy()))?;

    Ok(TextBuffer::from_str(&text))
}

/// Write a `TextBuffer` to a UTF-8 file.
///
/// This writes the entire buffer to disk in one go.
/// Will add variants later for stuff like incremental or atomic writes.
pub fn save_buffer(path: impl AsRef<Path>, buffer: &TextBuffer) -> Result<()> {
    let path = path.as_ref();
    std::fs::write(path, buffer.to_string())
        .with_context(|| format!("failed to write file: {}", path.to_string_lossy()))?;
    Ok(())
}
