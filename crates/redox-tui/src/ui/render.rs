//! UI helpers for rendering the editor viewport.
//!
//! Rendering keeps allocations bounded to visible rows, clips by terminal cell
//! width, and leaves long lines unwrapped.

use std::ops::Range;
use std::sync::Arc;

use minui::{TabPolicy, cell_width};
use redox_core::TextBuffer;
use unicode_segmentation::UnicodeSegmentation;

/// For very long lines, avoid grapheme segmentation.
///
/// This keeps startup and redraw latency reasonable for pathological cases
/// (single-line minified JSON, base64 blobs, logs with giant records, etc.).
const LONG_LINE_FAST_PATH_THRESHOLD_CHARS: usize = 8 * 1024;
const RENDER_LINE_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Viewport parameters for rendering a slice of the buffer.
///
/// `scroll_x` is measured in **terminal cells** (columns), not graphemes/chars/bytes.
///
/// `scroll_y` is measured in **document lines** (not wrapped visual rows).
#[derive(Debug, Clone, Copy)]
pub struct TextViewport {
    /// Horizontal scroll offset in terminal cells.
    pub scroll_x: usize,
    /// Vertical scroll offset in document lines.
    pub scroll_y: usize,
    pub width: u16,
    pub height: u16,
}

/// Snapshot of visible text lines for the current frame.
///
/// `first_line` is the document line index corresponding to `lines[0]`.
#[derive(Debug)]
pub struct RenderSnapshot {
    first_line: usize,
    lines: Vec<RenderLine>,
}

impl RenderSnapshot {
    pub fn first_line(&self) -> usize {
        self.first_line
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, RenderLine> {
        self.lines.iter()
    }
}

/// A source line and its horizontally clipped representation.
#[derive(Debug)]
pub struct RenderLine {
    source: Arc<str>,
    grapheme_ranges: Option<Arc<[Range<usize>]>>,
    visible: String,
}

impl RenderLine {
    pub fn source(&self) -> &str {
        self.source.as_ref()
    }

    pub fn visible(&self) -> &str {
        &self.visible
    }

    pub fn grapheme_indices(&self) -> RenderLineGraphemes<'_> {
        match self.grapheme_ranges.as_deref() {
            Some(ranges) => RenderLineGraphemes::Cached {
                source: &self.source,
                ranges: ranges.iter(),
            },
            None => RenderLineGraphemes::Segmented(self.source.grapheme_indices(true)),
        }
    }
}

pub enum RenderLineGraphemes<'a> {
    Cached {
        source: &'a str,
        ranges: std::slice::Iter<'a, Range<usize>>,
    },
    Segmented(unicode_segmentation::GraphemeIndices<'a>),
}

impl<'a> Iterator for RenderLineGraphemes<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            RenderLineGraphemes::Cached { source, ranges } => {
                let range = ranges.next()?;
                Some((range.start, &source[range.clone()]))
            }
            RenderLineGraphemes::Segmented(graphemes) => graphemes.next(),
        }
    }
}

/// LRU-style cache for immutable source lines and their grapheme boundaries.
#[derive(Debug)]
pub struct RenderLineCache {
    max_entries: usize,
    max_bytes: usize,
    cached_bytes: usize,
    entries: Vec<CacheEntry>,
    tick: u64,
}

#[derive(Debug)]
struct CacheEntry {
    line_index: usize,
    hash: u64,
    source: Arc<str>,
    grapheme_ranges: Option<Arc<[Range<usize>]>>,
    last_used_tick: u64,
    size_bytes: usize,
}

struct RenderLineData {
    source: Arc<str>,
    grapheme_ranges: Option<Arc<[Range<usize>]>>,
}

impl RenderLineCache {
    /// Create a cache bounded by line count and retained payload size.
    pub fn new(max_entries: usize) -> Self {
        Self::with_limits(max_entries, RENDER_LINE_CACHE_MAX_BYTES)
    }

    fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
            cached_bytes: 0,
            entries: Vec::new(),
            tick: 0,
        }
    }

    /// Clear all cached lines.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cached_bytes = 0;
        self.tick = 0;
    }

    /// Build a non-wrapping snapshot of the visible document lines.
    pub fn snapshot(&mut self, buffer: &TextBuffer, viewport: &TextViewport) -> RenderSnapshot {
        let max_cells = viewport.width as usize;
        let max_rows = viewport.height as usize;

        if max_cells == 0 || max_rows == 0 {
            return RenderSnapshot {
                first_line: 0,
                lines: Vec::new(),
            };
        }

        let first_line = viewport.scroll_y.min(buffer.len_lines().saturating_sub(1));
        let last_line = (first_line + max_rows).min(buffer.len_lines());
        let mut lines = Vec::with_capacity(max_rows);

        for line_index in first_line..last_line {
            let is_long_line =
                buffer.line_len_chars(line_index) > LONG_LINE_FAST_PATH_THRESHOLD_CHARS;
            let (source, grapheme_ranges, visible) = if is_long_line {
                let visible =
                    render_line_window_fast(buffer, line_index, viewport.scroll_x, max_cells);
                let line = self.line_data(buffer, line_index, false);
                (line.source, None, visible)
            } else {
                let line = self.line_data(buffer, line_index, true);
                let grapheme_ranges = line
                    .grapheme_ranges
                    .as_deref()
                    .expect("ordinary render lines should have grapheme boundaries");
                let visible = clip_graphemes_to_cells(
                    &line.source,
                    grapheme_ranges,
                    viewport.scroll_x,
                    max_cells,
                );
                (line.source, line.grapheme_ranges, visible)
            };
            lines.push(RenderLine {
                source,
                grapheme_ranges,
                visible,
            });
        }

        RenderSnapshot { first_line, lines }
    }

    fn line_data(
        &mut self,
        buffer: &TextBuffer,
        line_index: usize,
        needs_graphemes: bool,
    ) -> RenderLineData {
        self.tick = self.tick.wrapping_add(1);
        let hash = hash64_line(buffer, line_index);

        if let Some(entry_index) = self
            .entries
            .iter()
            .position(|entry| entry.line_index == line_index && entry.hash == hash)
        {
            let (line, added_bytes) = {
                let entry = &mut self.entries[entry_index];
                entry.last_used_tick = self.tick;
                let added_bytes = if needs_graphemes && entry.grapheme_ranges.is_none() {
                    let ranges = grapheme_ranges(&entry.source);
                    let added_bytes = std::mem::size_of_val(ranges.as_ref());
                    entry.grapheme_ranges = Some(ranges);
                    entry.size_bytes = entry.size_bytes.saturating_add(added_bytes);
                    added_bytes
                } else {
                    0
                };
                (
                    RenderLineData {
                        source: Arc::clone(&entry.source),
                        grapheme_ranges: entry.grapheme_ranges.as_ref().map(Arc::clone),
                    },
                    added_bytes,
                )
            };
            self.cached_bytes = self.cached_bytes.saturating_add(added_bytes);
            self.evict_until_within_limits();
            return line;
        }

        let source = Arc::<str>::from(buffer.line_string(line_index));
        let grapheme_ranges = needs_graphemes.then(|| grapheme_ranges(&source));
        let size_bytes = cached_line_size(&source, grapheme_ranges.as_deref());
        let line = RenderLineData {
            source: Arc::clone(&source),
            grapheme_ranges: grapheme_ranges.as_ref().map(Arc::clone),
        };

        if let Some(stale_index) = self
            .entries
            .iter()
            .position(|entry| entry.line_index == line_index)
        {
            self.remove_entry(stale_index);
        }

        if size_bytes <= self.max_bytes {
            self.make_room_for(size_bytes);
            self.entries.push(CacheEntry {
                line_index,
                hash,
                source,
                grapheme_ranges,
                last_used_tick: self.tick,
                size_bytes,
            });
            self.cached_bytes = self.cached_bytes.saturating_add(size_bytes);
        }

        line
    }

    fn make_room_for(&mut self, size_bytes: usize) {
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries
                || self.cached_bytes.saturating_add(size_bytes) > self.max_bytes)
        {
            self.evict_least_recently_used();
        }
    }

    fn evict_until_within_limits(&mut self) {
        while !self.entries.is_empty()
            && (self.entries.len() > self.max_entries || self.cached_bytes > self.max_bytes)
        {
            self.evict_least_recently_used();
        }
    }

    fn evict_least_recently_used(&mut self) {
        let Some((entry_index, _)) = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.last_used_tick)
        else {
            return;
        };
        self.remove_entry(entry_index);
    }

    fn remove_entry(&mut self, entry_index: usize) {
        let entry = self.entries.swap_remove(entry_index);
        self.cached_bytes = self.cached_bytes.saturating_sub(entry.size_bytes);
    }
}

fn cached_line_size(source: &str, grapheme_ranges: Option<&[Range<usize>]>) -> usize {
    source.len().saturating_add(
        grapheme_ranges
            .map(std::mem::size_of_val)
            .unwrap_or_default(),
    )
}

fn grapheme_ranges(source: &str) -> Arc<[Range<usize>]> {
    source
        .grapheme_indices(true)
        .map(|(start_byte, grapheme)| start_byte..start_byte + grapheme.len())
        .collect::<Vec<_>>()
        .into()
}

/// Clip cached graphemes to a maximum number of terminal cells.
///
/// Horizontal scrolling never splits a grapheme. If it lands in the middle of
/// a wide grapheme, the whole grapheme is skipped.
fn clip_graphemes_to_cells(
    source: &str,
    grapheme_ranges: &[Range<usize>],
    scroll_x: usize,
    max_cells: usize,
) -> String {
    if max_cells == 0 || grapheme_ranges.is_empty() {
        return String::new();
    }

    let first_grapheme = skip_graphemes_by_cells(source, grapheme_ranges, scroll_x);
    let mut output = String::new();
    let mut used_cells = 0usize;

    for range in &grapheme_ranges[first_grapheme..] {
        if used_cells >= max_cells {
            break;
        }

        let grapheme = &source[range.clone()];
        let width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
        if width > 0 && used_cells + width > max_cells {
            break;
        }

        if grapheme == "\t" {
            output.extend(std::iter::repeat_n(' ', width));
        } else {
            output.push_str(grapheme);
        }
        used_cells = used_cells.saturating_add(width);
    }

    output
}

fn skip_graphemes_by_cells(
    source: &str,
    grapheme_ranges: &[Range<usize>],
    skip_cells: usize,
) -> usize {
    if skip_cells == 0 || grapheme_ranges.is_empty() {
        return 0;
    }

    let mut skipped_cells = 0usize;
    for (grapheme_index, range) in grapheme_ranges.iter().enumerate() {
        if skipped_cells >= skip_cells {
            return grapheme_index;
        }
        let grapheme = &source[range.clone()];
        let width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
        skipped_cells = skipped_cells.saturating_add(width);
    }

    grapheme_ranges.len()
}

fn hash64_line(buffer: &TextBuffer, line_index: usize) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for chunk in buffer.line_slice(line_index).chunks() {
        for &byte in chunk.as_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

/// Render a clipped horizontal window for a single line directly from rope chars.
///
/// This intentionally avoids grapheme segmentation, which is expensive for
/// extremely long lines.
fn render_line_window_fast(
    buffer: &TextBuffer,
    line_index: usize,
    scroll_x_cells: usize,
    max_cells: usize,
) -> String {
    if max_cells == 0 {
        return String::new();
    }

    let range = buffer.line_char_range(line_index);
    if range.start >= range.end {
        return String::new();
    }

    let mut skipped_cells = 0usize;
    let mut used_cells = 0usize;
    let mut output = String::new();

    for character in buffer.chars(range) {
        let width = cell_width_for_char(character);

        if skipped_cells < scroll_x_cells {
            skipped_cells = skipped_cells.saturating_add(width);
            continue;
        }

        if used_cells >= max_cells {
            break;
        }

        if width > 0 && used_cells + width > max_cells {
            break;
        }

        if character == '\t' {
            output.extend(std::iter::repeat_n(' ', width));
        } else {
            output.push(character);
        }
        used_cells = used_cells.saturating_add(width);
    }

    output
}

#[inline]
fn cell_width_for_char(character: char) -> usize {
    let mut utf8_buffer = [0_u8; 4];
    let encoded = character.encode_utf8(&mut utf8_buffer);
    cell_width(encoded, TabPolicy::Fixed(4)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_preserves_source_while_clipping_in_terminal_cells() {
        let buffer = TextBuffer::from_text("abcdefghijklmnopqrstuvwxyz\n\tab\n");
        let mut cache = RenderLineCache::new(4);
        let snapshot = cache.snapshot(
            &buffer,
            &TextViewport {
                scroll_x: 5,
                scroll_y: 0,
                width: 4,
                height: 2,
            },
        );
        let lines = snapshot.iter().collect::<Vec<_>>();

        assert_eq!(snapshot.first_line(), 0);
        assert_eq!(lines[0].source(), "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(lines[0].visible(), "fghi");
        assert_eq!(lines[1].source(), "\tab");
        assert_eq!(lines[1].visible(), "b");
    }

    #[test]
    fn long_line_fast_path_does_not_retain_oversized_source() {
        let mut text = "a".repeat(LONG_LINE_FAST_PATH_THRESHOLD_CHARS + 8);
        text.push_str("tail\n");
        let buffer = TextBuffer::from_text(&text);
        let mut cache = RenderLineCache::with_limits(4, 64);

        let snapshot = cache.snapshot(
            &buffer,
            &TextViewport {
                scroll_x: LONG_LINE_FAST_PATH_THRESHOLD_CHARS + 8,
                scroll_y: 0,
                width: 4,
                height: 1,
            },
        );
        let line = snapshot.iter().next().expect("snapshot line should exist");

        assert_eq!(line.visible(), "tail");
        assert!(line.source().ends_with("tail"));
        assert!(cache.entries.is_empty());
        assert_eq!(cache.cached_bytes, 0);
    }
}
