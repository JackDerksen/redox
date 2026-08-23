use anyhow::{Context as _, Result};
use ropey::{Rope, RopeSlice};
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::ops::Range;

/// Redox's UTF-8 text store.
///
/// Positions and ranges use Unicode scalar-value indices. Terminal cell widths,
/// grapheme boundaries, cursors, and viewport state belong to the frontend.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBuffer {
    pub(super) rope: Rope,
}

/// A borrowed, non-allocating view into a [`TextBuffer`].
///
/// This deliberately keeps Ropey out of the public API by exposing only the
/// operations that Redox needs.
#[derive(Debug, Clone, Copy)]
pub struct TextSlice<'a> {
    inner: RopeSlice<'a>,
}

impl<'a> TextSlice<'a> {
    pub(super) fn new(inner: RopeSlice<'a>) -> Self {
        Self { inner }
    }

    #[inline]
    pub fn len_chars(self) -> usize {
        self.inner.len_chars()
    }

    #[inline]
    pub fn len_bytes(self) -> usize {
        self.inner.len_bytes()
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.inner.len_chars() == 0
    }

    #[inline]
    pub fn as_str(self) -> Option<&'a str> {
        self.inner.as_str()
    }

    #[inline]
    pub fn chars(self) -> impl Iterator<Item = char> + 'a {
        self.inner.chars()
    }

    #[inline]
    pub fn chunks(self) -> impl Iterator<Item = &'a str> + 'a {
        self.inner.chunks()
    }
}

impl fmt::Display for TextSlice<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.inner.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBuffer {
    #[inline]
    pub fn new() -> Self {
        Self { rope: Rope::new() }
    }

    #[inline]
    pub fn from_text(s: &str) -> Self {
        Self {
            rope: Rope::from_str(s),
        }
    }

    /// Read a UTF-8 buffer from a byte stream. Invalid UTF-8 is rejected.
    pub fn from_reader(reader: impl Read) -> std::io::Result<Self> {
        Rope::from_reader(reader).map(|rope| Self { rope })
    }

    /// Load a UTF-8 file. Invalid UTF-8 is rejected for now.
    // TODO: add support for more file formats!
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .with_context(|| format!("failed to read file: {}", path.to_string_lossy()))?;
        let reader = BufReader::new(file);
        Self::from_reader(reader)
            .with_context(|| format!("file is not valid UTF-8: {}", path.to_string_lossy()))
    }

    #[inline]
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    #[inline]
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len_chars() == 0
    }

    /// Return the character at an absolute character index.
    ///
    /// This is the checked alternative to Ropey's panicking `char` operation.
    #[inline]
    pub fn char(&self, char_idx: usize) -> Option<char> {
        (char_idx < self.len_chars()).then(|| self.rope.char(char_idx))
    }

    /// Convert a character boundary to its UTF-8 byte offset, clamping at EOF.
    #[inline]
    pub fn char_to_byte(&self, char_idx: usize) -> usize {
        self.rope.char_to_byte(char_idx.min(self.len_chars()))
    }

    /// Convert a UTF-8 byte offset to a character index.
    #[inline]
    pub fn byte_to_char(&self, byte_idx: usize) -> Option<usize> {
        (byte_idx <= self.len_bytes()).then(|| self.rope.byte_to_char(byte_idx))
    }

    /// Iterate over a clamped, order-independent character range.
    pub fn chars(&self, range: Range<usize>) -> impl Iterator<Item = char> + '_ {
        let (start, end) = self.normalized_char_range(range.start, range.end);
        self.rope.slice(start..end).chars()
    }

    pub(crate) fn chars_reversed(&self, range: Range<usize>) -> impl Iterator<Item = char> + '_ {
        let (start, end) = self.normalized_char_range(range.start, range.end);
        self.rope.chars_at(end).reversed().take(end - start)
    }

    /// Iterate over Ropey's contiguous storage chunks without exposing Ropey.
    #[inline]
    pub fn chunks(&self) -> impl Iterator<Item = &str> {
        self.rope.chunks()
    }

    /// Write the UTF-8 contents to a byte stream without first allocating a
    /// contiguous `String`.
    pub fn write_to(&self, mut writer: impl Write) -> std::io::Result<()> {
        for chunk in self.chunks() {
            writer.write_all(chunk.as_bytes())?;
        }
        Ok(())
    }

    pub(crate) fn append(&mut self, text: &str) {
        if !text.is_empty() {
            self.rope.insert(self.len_chars(), text);
        }
    }

    #[inline]
    pub(super) fn char_at_index(&self, char_idx: usize) -> char {
        debug_assert!(char_idx < self.len_chars());
        self.rope.char(char_idx)
    }

    pub(super) fn normalized_char_range(&self, start: usize, end: usize) -> (usize, usize) {
        let start = start.min(self.len_chars());
        let end = end.min(self.len_chars());
        if start <= end {
            (start, end)
        } else {
            (end, start)
        }
    }
}

impl fmt::Display for TextBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl From<&str> for TextBuffer {
    fn from(text: &str) -> Self {
        Self::from_text(text)
    }
}

impl From<String> for TextBuffer {
    fn from(text: String) -> Self {
        Self {
            rope: Rope::from(text),
        }
    }
}

impl From<&TextBuffer> for String {
    fn from(buffer: &TextBuffer) -> Self {
        let mut text = String::with_capacity(buffer.len_bytes());
        text.extend(buffer.chunks());
        text
    }
}

impl From<TextBuffer> for String {
    fn from(buffer: TextBuffer) -> Self {
        buffer.rope.into()
    }
}

impl From<TextSlice<'_>> for String {
    fn from(slice: TextSlice<'_>) -> Self {
        slice.inner.into()
    }
}
