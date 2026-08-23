//! UI helpers for rendering the editor viewport.
//!
//! Rendering keeps allocations bounded to visible rows, clips by terminal cell
//! width, and leaves long lines unwrapped.

use minui::{TabPolicy, cell_width};
use redox_core::TextBuffer;
use unicode_segmentation::UnicodeSegmentation;

/// For very long lines, avoid full-line allocation and grapheme hashing/caching.
///
/// This keeps startup and redraw latency reasonable for pathological cases
/// (single-line minified JSON, base64 blobs, logs with giant records, etc.).
const LONG_LINE_FAST_PATH_THRESHOLD_CHARS: usize = 8 * 1024;

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
#[derive(Debug, Clone)]
pub struct RenderSnapshot {
    #[allow(dead_code)]
    pub first_line: usize,
    pub lines: Vec<String>,
}

impl RenderSnapshot {
    pub fn new(first_line: usize, lines: Vec<String>) -> Self {
        Self { first_line, lines }
    }
}

/// Cache for grapheme boundary segmentation.
///
/// This is a simple LRU-ish cache keyed by `(line_idx, line_hash)`.
/// It’s designed for the current "read-only rendering" stage where the buffer
/// doesn’t change during runtime (so the cache stays hot).
///
/// When editing is added, the caller can invalidate the cache when a line changes.
#[derive(Debug, Default)]
pub struct GraphemeCache {
    max_entries: usize,
    entries: Vec<CacheEntry>,
    tick: u64,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    line_idx: usize,
    hash: u64,
    graphemes: Vec<Box<str>>,
    last_used_tick: u64,
}

impl GraphemeCache {
    /// Create a cache with a max number of cached lines.
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            entries: Vec::new(),
            tick: 0,
        }
    }

    /// Clear all cached lines.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.entries.clear();
        self.tick = 0;
    }

    /// Get grapheme slices for a buffer line.
    ///
    /// Returned as a slice of `Box<str>` stored in the cache.
    pub fn graphemes_for_line<'a>(
        &'a mut self,
        buffer: &TextBuffer,
        line_idx: usize,
    ) -> &'a [Box<str>] {
        self.tick = self.tick.wrapping_add(1);
        let h = hash64_line(buffer, line_idx);

        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.line_idx == line_idx && e.hash == h)
        {
            self.entries[pos].last_used_tick = self.tick;
            return &self.entries[pos].graphemes;
        }

        let line = buffer.line_slice(line_idx);
        let graphemes: Vec<Box<str>> = if let Some(line_text) = line.as_str() {
            line_text
                .graphemes(true)
                .map(|g| g.to_owned().into_boxed_str())
                .collect()
        } else {
            let line_text = line.to_string();
            line_text
                .graphemes(true)
                .map(|g| g.to_owned().into_boxed_str())
                .collect()
        };

        if self.entries.len() >= self.max_entries {
            if let Some((evict_idx, _)) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_used_tick)
            {
                self.entries.swap_remove(evict_idx);
            }
        }

        self.entries.push(CacheEntry {
            line_idx,
            hash: h,
            graphemes,
            last_used_tick: self.tick,
        });

        let last = self.entries.len() - 1;
        &self.entries[last].graphemes
    }
}

/// Build a *non-wrapping* snapshot of visible **document lines**.
///
/// - Long lines are not wrapped; they continue off-screen.
/// - Horizontal scrolling is applied first (in terminal cell units), then the
///   remaining content is clipped to `viewport.width` cells.
/// - `viewport.scroll_y` is interpreted as a **document line offset**.
/// - Vertical scrolling is clamped by the caller (typically to `len_lines - height`).
pub fn snapshot_lines_wrapped_cached(
    buffer: &TextBuffer,
    viewport: &TextViewport,
    cache: &mut GraphemeCache,
) -> RenderSnapshot {
    let max_cells = viewport.width as usize;
    let max_rows = viewport.height as usize;

    if max_cells == 0 || max_rows == 0 {
        return RenderSnapshot::new(0, Vec::new());
    }

    let first_line = viewport.scroll_y.min(buffer.len_lines().saturating_sub(1));
    let last_line = (first_line + max_rows).min(buffer.len_lines());

    let mut out_rows: Vec<String> = Vec::with_capacity(max_rows);

    for line_idx in first_line..last_line {
        let line_len = buffer.line_len_chars(line_idx);
        if line_len > LONG_LINE_FAST_PATH_THRESHOLD_CHARS {
            out_rows.push(render_line_window_fast(
                buffer,
                line_idx,
                viewport.scroll_x,
                max_cells,
            ));
        } else {
            let graphemes = cache.graphemes_for_line(buffer, line_idx);

            let start_g = skip_graphemes_by_cells(graphemes, viewport.scroll_x);
            let visible = clip_graphemes_to_cells(&graphemes[start_g..], max_cells);
            out_rows.push(visible);
        }
    }

    RenderSnapshot::new(first_line, out_rows)
}

/// Skip graphemes from the start of a line until at least `skip_cells` terminal cells have been skipped.
///
/// Returns the grapheme index to start rendering from.
///
/// Notes:
/// - Uses MinUI `cell_width` to count cells (tab-aware via `TabPolicy::Fixed(4)`).
/// - Never splits graphemes.
/// - If `skip_cells` lands in the middle of a wide grapheme, the whole grapheme is skipped.
fn skip_graphemes_by_cells(graphemes: &[Box<str>], skip_cells: usize) -> usize {
    if skip_cells == 0 || graphemes.is_empty() {
        return 0;
    }

    let mut skipped = 0usize;
    for (i, g) in graphemes.iter().enumerate() {
        if skipped >= skip_cells {
            return i;
        }
        let w = cell_width(g, TabPolicy::Fixed(4)) as usize;
        skipped = skipped.saturating_add(w);
    }

    graphemes.len()
}

/// Clip cached graphemes (`Box<str>`) to a maximum number of terminal cells.
///
/// - Does **not** split graphemes.
/// - Uses MinUI `cell_width` to count cells.
/// - Treats graphemes with width 0 as width 0.
/// - If a grapheme is wider than remaining space, it is not included.
#[allow(dead_code)]
fn clip_graphemes_to_cells(graphemes: &[Box<str>], max_cells: usize) -> String {
    if max_cells == 0 || graphemes.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;

    for g in graphemes {
        if used >= max_cells {
            break;
        }

        let w = cell_width(g, TabPolicy::Fixed(4)) as usize;

        if w > 0 && used + w > max_cells {
            break;
        }

        if g.as_ref() == "\t" {
            out.extend(std::iter::repeat_n(' ', w));
        } else {
            out.push_str(g);
        }
        used = used.saturating_add(w);
    }

    out
}

fn hash64_line(buffer: &TextBuffer, line_idx: usize) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut h = FNV_OFFSET;
    for chunk in buffer.line_slice(line_idx).chunks() {
        for &b in chunk.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
    }
    h
}

/// Render a clipped horizontal window for a single line directly from rope chars.
///
/// This intentionally avoids allocating the full line string, which is expensive
/// for extremely long lines.
fn render_line_window_fast(
    buffer: &TextBuffer,
    line_idx: usize,
    scroll_x_cells: usize,
    max_cells: usize,
) -> String {
    if max_cells == 0 {
        return String::new();
    }

    let range = buffer.line_char_range(line_idx);
    if range.start >= range.end {
        return String::new();
    }

    let mut skipped_cells = 0usize;
    let mut used_cells = 0usize;
    let mut out = String::new();

    for ch in buffer.chars(range) {
        let w = cell_width_for_char(ch);

        if skipped_cells < scroll_x_cells {
            skipped_cells = skipped_cells.saturating_add(w);
            continue;
        }

        if used_cells >= max_cells {
            break;
        }

        if w > 0 && used_cells + w > max_cells {
            break;
        }

        if ch == '\t' {
            out.extend(std::iter::repeat_n(' ', w));
        } else {
            out.push(ch);
        }
        used_cells = used_cells.saturating_add(w);
    }

    out
}

#[inline]
fn cell_width_for_char(ch: char) -> usize {
    let mut buf = [0_u8; 4];
    let s = ch.encode_utf8(&mut buf);
    cell_width(s, TabPolicy::Fixed(4)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_core::TextBuffer;

    #[test]
    fn fast_path_clips_and_scrolls_ascii_lines() {
        let b = TextBuffer::from_text("abcdefghijklmnopqrstuvwxyz\n");
        let out = render_line_window_fast(&b, 0, 5, 4);
        assert_eq!(out, "fghi");
    }

    #[test]
    fn fast_path_handles_empty_and_short_ranges() {
        let b = TextBuffer::from_text("\n");
        assert_eq!(render_line_window_fast(&b, 0, 0, 10), "");
        assert_eq!(render_line_window_fast(&b, 0, 5, 10), "");
    }

    #[test]
    fn tab_expands_to_spaces_for_rendering() {
        let b = TextBuffer::from_text("\tab\n");
        let mut cache = GraphemeCache::new(4);
        let snap = snapshot_lines_wrapped_cached(
            &b,
            &TextViewport {
                scroll_x: 0,
                scroll_y: 0,
                width: 8,
                height: 1,
            },
            &mut cache,
        );
        assert_eq!(snap.lines[0], "    ab");
    }
}
