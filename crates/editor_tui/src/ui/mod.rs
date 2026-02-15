//! UI helpers for rendering the editor viewport.
//!
//! Goals (current):
//! - Provide a small viewport abstraction for rendering a `TextBuffer` into a MinUI `Window`.
//! - Be reasonably efficient for large files / very long lines (still some work to do here).
//! - Use grapheme clusters for horizontal scrolling (so combined characters stay intact).
//! - Clip by terminal *cell width* (so wide glyphs don’t overflow the viewport).
//! - Support *soft wrapping* (visual-only wrapping; does not modify the buffer).
//!
//! Notes:
//! - This module is UI-only and should not leak into `editor_core`.
//! - Allocations are intentionally kept bounded to the visible rows.
//! - The grapheme cache is an optimization (it avoids re-segmenting the same line).
//!   every frame when you are not editing the buffer.
//!
//! Future work:
//! - Cursor rendering, selection, and incremental updates.

use editor_core::TextBuffer;
use minui::{Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

/// Viewport parameters for rendering a slice of the buffer.
///
/// `scroll_x` is measured in **grapheme clusters**.
///
/// NOTE: once soft-wrapping is enabled, `scroll_y` will be a bit more tricky. For wrapped
/// rendering this interprets `scroll_y` as a **visual row offset** (wrapped rows),
/// not as a rope line index.
#[derive(Debug, Clone, Copy)]
pub struct TextViewport {
    pub scroll_x: usize,
    pub scroll_y: usize,
    pub width: u16,
    pub height: u16,
}

impl TextViewport {
    /// Build a viewport using the current window size.
    pub fn from_window(window: &dyn Window, scroll_x: usize, scroll_y: usize) -> Self {
        let (width, height) = window.get_size();
        Self {
            scroll_x,
            scroll_y,
            width,
            height,
        }
    }
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
/// It’s designed for the current “read-only rendering” stage where the buffer
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

    /// Get grapheme slices for `line_text`.
    ///
    /// Returned as a slice of `Box<str>` stored in the cache.
    pub fn graphemes_for_line<'a>(
        &'a mut self,
        line_idx: usize,
        line_text: &str,
    ) -> &'a [Box<str>] {
        self.tick = self.tick.wrapping_add(1);
        let h = hash64(line_text);

        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.line_idx == line_idx && e.hash == h)
        {
            // Bump usage
            self.entries[pos].last_used_tick = self.tick;
            return &self.entries[pos].graphemes;
        }

        // Miss: segment and insert.
        let graphemes: Vec<Box<str>> = line_text
            .graphemes(true)
            .map(|g| g.to_owned().into_boxed_str())
            .collect();

        if self.entries.len() >= self.max_entries {
            // Evict least recently used
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

        // Safe: we just pushed one entry, so it exists.
        let last = self.entries.len() - 1;
        &self.entries[last].graphemes
    }
}

/// Draw a snapshot into the window.
pub fn draw_snapshot(snapshot: &RenderSnapshot, window: &mut dyn Window) -> minui::Result<()> {
    for (row, line) in snapshot.lines.iter().enumerate() {
        window.write_str(row as u16, 0, line)?;
    }
    Ok(())
}

/// Build a *soft-wrapped* snapshot of visible rows.
///
/// - Soft wrap is visual-only: it does not modify the underlying buffer.
/// - Horizontal scrolling is applied first (in grapheme units), then wrap the
///   remaining content into rows of at most `viewport.width` cells.
/// - `viewport.scroll_y` is interpreted as a visual row offset into the wrapped
///   row stream.
///
/// TODO:
/// - For now this still allocates `String` per *source line* via `line_string`.
///   For very large single-line files, that's still expensive; later, this should
///   avoid allocating the full line when we only need a window into it.
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

    // Generate wrapped rows for the whole document, skipping until scroll_y.
    let mut skipped_rows = 0usize;
    let mut out_rows: Vec<String> = Vec::with_capacity(max_rows);

    // Start from a line that could contribute to visible rows after scrolling.
    // This optimization avoids iterating through all lines when scroll_y is large.
    let start_line_estimate = if viewport.scroll_x == 0 {
        // When not horizontally scrolled, estimate starting line by scroll_y
        viewport.scroll_y.min(buffer.len_lines())
    } else {
        // With horizontal scroll, lines might wrap differently, start from beginning
        0
    };

    for line_idx in start_line_estimate..buffer.len_lines() {
        }

        while !remaining.is_empty() {
            if out_rows.len() >= max_rows {
                break;
            }

            // Consume up to `max_cells` worth of graphemes, preferring to wrap on spaces.
            // Policy:
            // - Take as many graphemes as fit by cell width.
            // - If the taken chunk contains spaces, wrap at the last space (dropping that space).
            // - If the next row would start with spaces, skip them (so wraps don't indent).
            // - If no spaces fit (single long "word"), fall back to a hard wrap at cell boundary.
            let (row, consumed) = take_graphemes_by_cells_word_wrap(remaining, max_cells);

            // Ensure forward progress even if a single grapheme is wider than the viewport.
            let consumed = if consumed == 0 {
                1.min(remaining.len())
            } else {
                consumed
            };

            if skipped_rows < viewport.scroll_y {
                skipped_rows += 1;
            } else {
                out_rows.push(row);
            }

            remaining = &remaining[consumed..];

            // Skip leading spaces on the next visual row.
            while let Some(g) = remaining.first() {
                if g.as_ref() == " " {
                    remaining = &remaining[1..];
                } else {
                    break;
                }
            }
        }
    }

    // first_line is not super meaningful for wrapped mode yet so keep as 0 for now.
    RenderSnapshot::new(0, out_rows)
}

/// Build a grapheme-aware + cell-width-clipped snapshot of visible lines.
///
/// This variant uses an internal cache for grapheme boundaries. If I later don't
/// want caching, use [`snapshot_lines_uncached`].
/// Currently unused; the wrapped variant is preferred.
#[allow(dead_code)]
pub fn snapshot_lines_cached(
    buffer: &TextBuffer,
    viewport: &TextViewport,
    cache: &mut GraphemeCache,
) -> RenderSnapshot {
    let mut lines = Vec::with_capacity(viewport.height as usize);
    let first_line = viewport.scroll_y;
    let last_line = first_line.saturating_add(viewport.height as usize);

    let max_cells = viewport.width as usize;

    for line_idx in first_line..last_line {
        if line_idx >= buffer.len_lines() {
            break;
        }

        // Rope -> String allocation for the line (no trailing '\n').
        let line_text = buffer.line_string(line_idx);

        let graphemes = cache.graphemes_for_line(line_idx, &line_text);

        // Horizontal scroll is in grapheme units.
        let start_g = viewport.scroll_x.min(graphemes.len());

        let visible = clip_graphemes_to_cells(&graphemes[start_g..], max_cells);
        lines.push(visible);
    }

    RenderSnapshot::new(first_line, lines)
}

/// Build a grapheme-aware + cell-width-clipped snapshot of visible lines (no cache).
#[allow(dead_code)]
pub fn snapshot_lines_uncached(buffer: &TextBuffer, viewport: &TextViewport) -> RenderSnapshot {
    let mut lines = Vec::with_capacity(viewport.height as usize);
    let first_line = viewport.scroll_y;
    let last_line = first_line.saturating_add(viewport.height as usize);

    let max_cells = viewport.width as usize;

    for line_idx in first_line..last_line {
        if line_idx >= buffer.len_lines() {
            break;
        }

        let line_text = buffer.line_string(line_idx);
        let graphemes: Vec<&str> = line_text.graphemes(true).collect();

        let start_g = viewport.scroll_x.min(graphemes.len());
        let visible = clip_graphemes_to_cells_ref(&graphemes[start_g..], max_cells);

        lines.push(visible);
    }

    RenderSnapshot::new(first_line, lines)
}

/// Backwards-compatible entry point used by `main.rs`.
///
/// Uses uncached rendering by default. If I later want caching, switch call sites to
/// [`snapshot_lines_cached`] and store a `GraphemeCache` in your app state.
/// Currently unused (the wrapped variant is preferred).
#[allow(dead_code)]
pub fn snapshot_lines(buffer: &TextBuffer, viewport: &TextViewport) -> RenderSnapshot {
    snapshot_lines_uncached(buffer, viewport)
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

    // Build output with bounded width.
    let mut out = String::new();
    let mut used = 0usize;

    for g in graphemes {
        if used >= max_cells {
            break;
        }

        let w = cell_width(g, minui::prelude::TabPolicy::Fixed(4)) as usize;

        // If it doesn't fit, stop (don’t overrun).
        if w > 0 && used + w > max_cells {
            break;
        }

        out.push_str(g);
        used = used.saturating_add(w);
    }

    out
}

/// Take as many graphemes as fit within `max_cells`, returning:
/// - the concatenated row string
/// - the number of graphemes consumed
///
/// This does not split graphemes and stops before the first non-fitting grapheme.
fn take_graphemes_by_cells(graphemes: &[Box<str>], max_cells: usize) -> (String, usize) {
    if max_cells == 0 || graphemes.is_empty() {
        return (String::new(), 0);
    }

    let mut out = String::new();
    let mut used_cells = 0usize;
    let mut consumed = 0usize;

    for g in graphemes {
        let w = cell_width(g, minui::prelude::TabPolicy::Fixed(4)) as usize;

        if w > 0 && used_cells + w > max_cells {
            break;
        }

        out.push_str(g);
        used_cells = used_cells.saturating_add(w);
        consumed += 1;

        if used_cells >= max_cells {
            break;
        }
    }

    (out, consumed)
}

/// Like `take_graphemes_by_cells`, but prefers wrapping on spaces within the chunk.
///
/// Returns:
/// - row text (with any trailing space removed if we wrapped at a space)
/// - number of graphemes consumed from the input (including the space we wrapped at)
fn take_graphemes_by_cells_word_wrap(graphemes: &[Box<str>], max_cells: usize) -> (String, usize) {
    let (chunk, consumed) = take_graphemes_by_cells(graphemes, max_cells);
    if consumed == 0 {
        return (chunk, consumed);
    }

    // Find last space within the consumed graphemes.
    let mut last_space: Option<usize> = None;
    for i in 0..consumed {
        if graphemes[i].as_ref() == " " {
            last_space = Some(i);
        }
    }

    // Cut at the last space if possible, otherwise hard wrap at cell boundary.
    if let Some(space_idx) = last_space {
        // Build string from graphemes[0..space_idx]
        let mut out = String::new();
        for g in &graphemes[..space_idx] {
            out.push_str(g);
        }
        // Consume through the space so the next row starts after it.
        return (out, space_idx + 1);
    }

    // No spaces: hard wrap at cell boundary.
    (chunk, consumed)
}

/// Clip uncached graphemes (`&str`) to a maximum number of terminal cells.
///
/// Same behavior as [`clip_graphemes_to_cells`].
#[allow(dead_code)]
fn clip_graphemes_to_cells_ref(graphemes: &[&str], max_cells: usize) -> String {
    if max_cells == 0 || graphemes.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;

    for g in graphemes {
        if used >= max_cells {
            break;
        }

        let w = cell_width(g, minui::prelude::TabPolicy::Fixed(4)) as usize;
        if w > 0 && used + w > max_cells {
            break;
        }

        out.push_str(g);
        used = used.saturating_add(w);
    }

    out
}

/// Simple 64-bit FNV-1a hash for strings.
///
/// Not cryptographic but good enough.
fn hash64(s: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut h = FNV_OFFSET;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}
