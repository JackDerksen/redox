//! UI helpers for rendering the editor viewport.
//!
//! Goals (current):
//! - Provide a small viewport abstraction for rendering a `TextBuffer` into a MinUI `Window`.
//! - Be reasonably efficient for large files / very long lines (still some work to do here).
//! - Use grapheme clusters for horizontal slicing (so combined characters stay intact).
//! - Clip by terminal *cell width* (so wide glyphs don’t overflow the viewport).
//! - **Do not soft wrap**: long lines continue off-screen (like many modal editors).
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

impl TextViewport {
    // Intentionally no `from_window(...)` constructor for now.
    //
    // The current code constructs `TextViewport` directly at call sites. If/when we
    // need a helper again, we can reintroduce it.
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
        let line_text = buffer.line_string(line_idx);
        let graphemes = cache.graphemes_for_line(line_idx, &line_text);

        // Horizontal scroll is in terminal cells.
        let start_g = skip_graphemes_by_cells(graphemes, viewport.scroll_x);

        // Clip to viewport width (no wrapping).
        let visible = clip_graphemes_to_cells(&graphemes[start_g..], max_cells);
        out_rows.push(visible);
    }

    RenderSnapshot::new(first_line, out_rows)
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
        let w = cell_width(g, minui::prelude::TabPolicy::Fixed(4)) as usize;
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
