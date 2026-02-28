//! Core IO helpers for loading/saving text buffers.
//!
//! This module is intentionally small and UI-agnostic. It just provides helpers
//! that read and write UTF-8 text to/from the rope-backed `TextBuffer`.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::buffer::TextBuffer;

/// Read a UTF-8 file into a `TextBuffer`.
///
/// The file is parsed as UTF-8.
pub fn load_buffer(path: impl AsRef<Path>) -> Result<TextBuffer> {
    TextBuffer::from_file(path)
}

/// Write a `TextBuffer` to a UTF-8 file.
///
/// This writes the entire buffer to disk in one go.
pub fn save_buffer(path: impl AsRef<Path>, buffer: &TextBuffer) -> Result<()> {
    let path = path.as_ref();
    std::fs::write(path, buffer.to_string())
        .with_context(|| format!("failed to write file: {}", path.to_string_lossy()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("redox_io_test_{tag}_{nanos}.txt"))
    }

    #[test]
    fn save_then_load_roundtrip() {
        let path = temp_path("roundtrip");
        let b = TextBuffer::from_str("hello\nworld\n");

        save_buffer(&path, &b).expect("save failed");
        let loaded = load_buffer(&path).expect("load failed");

        assert_eq!(loaded.to_string(), "hello\nworld\n");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_buffer_rejects_invalid_utf8() {
        let path = temp_path("invalid_utf8");
        std::fs::write(&path, [0xffu8, 0xfeu8]).expect("failed to write invalid utf8");

        let err = load_buffer(&path).expect_err("expected invalid utf8 error");
        let msg = err.to_string();
        assert!(msg.contains("not valid UTF-8") || msg.contains("file is not valid UTF-8"));

        let _ = std::fs::remove_file(path);
    }
}
