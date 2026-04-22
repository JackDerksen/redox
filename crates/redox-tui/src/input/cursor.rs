//! Document cursor controller + viewport scrolling logic for `redox-tui`.
//!
//! This module is intentionally **TUI/UI specific** and should stay lightweight.
//! - It owns viewport state (`scroll_x_cells`, `scroll_y_lines`) and projects a document cursor
//!   into terminal cell coordinates for rendering.
//! - It delegates *document navigation semantics* (Vim motions like `w`, `gg`, `G`, etc.) to
//!   `redox_core::motion` so those behaviors are shared across frontends.
//!
//! Design goals:
//! - Cursor should not move beyond end-of-file (line clamped to last line, col clamped to line len).
//! - Viewport can scroll down until the **last line** is the only one at the top of the viewport.
//!   This may reveal blank space below EOF, but that space is not part of the buffer and must
//!   never be reachable by the cursor.
//! - Horizontal scrolling is cell-accurate (tabs + wide glyphs) and never affects vertical scroll.
//!
//! This file intentionally does not depend on the rest of `redox-tui::ui` to avoid circular deps.

use minui::{TabPolicy, cell_width, window::CursorSpec};
use redox_core::motion::{Motion, apply_motion_n};
use redox_core::{Pos, TextBuffer};
use std::cmp::min;
use unicode_segmentation::UnicodeSegmentation;

const LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS: usize = 8 * 1024;
const LONG_LINE_CURSOR_FAST_PATH_MAX_DELTA_CHARS: usize = 4 * 1024;

/// How the viewport should follow the cursor.
#[derive(Debug, Clone, Copy)]
pub struct FollowConfig {
    /// Vim-style "scrolloff" behaviour which keeps some padding around the cursor when scrolling.
    /// For the bottom margin rows, it is disabled near the EOF so that we don't
    /// manufacture blank space at the bottom of the viewport.
    pub top_margin_rows: usize,
    pub bottom_margin_rows: usize,

    /// If true, horizontal scrolling tries to keep the cursor within the viewport
    /// but does not add a left/right margin.
    pub horizontal_follow: bool,
}

impl Default for FollowConfig {
    fn default() -> Self {
        Self {
            top_margin_rows: 8,
            bottom_margin_rows: 8,
            horizontal_follow: true,
        }
    }
}

/// A document cursor controller + view state.
#[derive(Debug, Clone)]
pub struct CursorController {
    /// Document cursor in logical units: (line, col) where col is in **char units** (Ropey model).
    pub cursor: Pos,

    /// Horizontal scroll in terminal **cells**.
    pub scroll_x_cells: usize,

    /// Vertical scroll in **document lines**.
    pub scroll_y_lines: usize,

    pub follow: FollowConfig,

    tab_policy: TabPolicy,
    preferred_col: Option<usize>,
    visual_cache: Option<VisualCache>,
}

impl Default for CursorController {
    fn default() -> Self {
        Self {
            cursor: Pos::zero(),
            scroll_x_cells: 0,
            scroll_y_lines: 0,
            follow: FollowConfig::default(),
            tab_policy: TabPolicy::Fixed(4),
            preferred_col: None,
            visual_cache: None,
        }
    }
}

impl CursorController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconcile cursor + viewport after an edit.
    ///
    /// This clamps the cursor to real buffer content and then updates scroll offsets
    /// to keep the cursor visible, **without** applying any motion semantics.
    ///
    /// Useful after mutations like insert/backspace/newline where the cursor
    /// is already updated by the caller.
    pub fn reconcile_after_edit(
        &mut self,
        buffer: &TextBuffer,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        // Clamp the cursor first (edits can invalidate col/line).
        self.cursor = buffer.clamp_pos(self.cursor);
        self.preferred_col = None;
        self.invalidate_visual_cache();

        // Clamp scroll to content first (keeps state sane).
        self.clamp_scroll_to_content(buffer, viewport_width_cells, viewport_height_rows);

        // Compute cursor visual position under current scroll.
        let info = self.cursor_visual_info(buffer, viewport_width_cells);

        // Follow cursor with margins.
        self.follow_cursor(buffer, info, viewport_width_cells, viewport_height_rows);

        // Final clamp (ensures no negative scroll).
        self.clamp_scroll_to_content(buffer, viewport_width_cells, viewport_height_rows);
    }

    pub fn clamp_for_normal_mode(&mut self, buffer: &TextBuffer) {
        self.cursor = buffer.clamp_pos(self.cursor);
        let line_len = buffer.line_len_chars(self.cursor.line);
        if line_len > 0 && self.cursor.col >= line_len {
            self.cursor.col = line_len - 1;
        }
        self.invalidate_visual_cache();
    }

    /// Apply a core (UI-agnostic) motion with a Vim-style count, then adjust scrolling
    /// to keep the cursor visible.
    ///
    /// This is the preferred entry point for frontends that translate input into
    /// `redox_core::motion::Motion`.
    pub fn apply_motion(
        &mut self,
        buffer: &TextBuffer,
        motion: Motion,
        count: usize,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        let count = count.max(1);

        self.cursor = buffer.clamp_pos(self.cursor);
        match motion {
            Motion::Up | Motion::Down => self.apply_vertical_motion(buffer, motion, count),
            _ => {
                let next = apply_motion_n(buffer, self.cursor, motion, count);
                self.cursor = buffer.clamp_pos(next);
                self.preferred_col = None;
            }
        }

        // Clamp scroll to content first (keeps state sane).
        self.clamp_scroll_to_content(buffer, viewport_width_cells, viewport_height_rows);

        // Compute cursor visual position under current scroll.
        let info = self.cursor_visual_info(buffer, viewport_width_cells);

        // Follow cursor with margins.
        self.follow_cursor(buffer, info, viewport_width_cells, viewport_height_rows);

        // Final clamp (ensures no negative scroll).
        self.clamp_scroll_to_content(buffer, viewport_width_cells, viewport_height_rows);
    }

    fn apply_vertical_motion(&mut self, buffer: &TextBuffer, motion: Motion, count: usize) {
        let preferred_col = self.preferred_col.unwrap_or(self.cursor.col);

        for _ in 0..count {
            let next_line = match motion {
                Motion::Up if self.cursor.line > 0 => self.cursor.line - 1,
                Motion::Down => {
                    let last = buffer.len_lines().saturating_sub(1);
                    if self.cursor.line >= last {
                        break;
                    }
                    self.cursor.line + 1
                }
                _ => break,
            };

            let next_col = min(preferred_col, buffer.line_len_chars(next_line));
            let next = Pos::new(next_line, next_col);
            if next == self.cursor {
                break;
            }
            self.cursor = next;
        }

        self.preferred_col = Some(preferred_col);
    }

    /// Produce a MinUI `CursorSpec` for the current cursor under the current scroll offsets.
    ///
    /// If the cursor is not within the current viewport, returns a spec with `visible: false`.
    pub fn cursor_spec(
        &mut self,
        buffer: &TextBuffer,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) -> CursorSpec {
        if viewport_width_cells == 0 || viewport_height_rows == 0 {
            return CursorSpec {
                x: 0,
                y: 0,
                visible: false,
            };
        }

        let info = self.cursor_visual_info(buffer, viewport_width_cells);

        // Cursor position in viewport coordinates.
        let vx = info.cursor_x_cells.saturating_sub(self.scroll_x_cells);
        let vy = info.cursor_y_lines.saturating_sub(self.scroll_y_lines);

        let visible = vx < viewport_width_cells && vy < viewport_height_rows;

        CursorSpec {
            x: (vx as u16).min(u16::MAX),
            y: (vy as u16).min(u16::MAX),
            visible,
        }
    }

    /// Returns the scroll values that should be used to render a viewport.
    #[inline]
    pub fn viewport_scroll(&self) -> (usize, usize) {
        (self.scroll_x_cells, self.scroll_y_lines)
    }

    // --- Follow logic ---

    fn follow_cursor(
        &mut self,
        buffer: &TextBuffer,
        info: CursorVisualInfo,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        if viewport_width_cells == 0 || viewport_height_rows == 0 {
            return;
        }

        // Keep the cursor on-screen first, then apply scrolloff-style margins.
        let top = self.scroll_y_lines;
        let bottom_exclusive = top + viewport_height_rows;

        let cursor_line = info.cursor_y_lines;
        let total_lines = buffer.len_lines().max(1);

        // "scrolloff" margins, but disabled near BOF/EOF so we don't create blank space at
        // the viewport edges.
        let max_margin = viewport_height_rows.saturating_sub(1);
        let desired_top_margin = self.follow.top_margin_rows.min(max_margin);
        let desired_bottom_margin = self.follow.bottom_margin_rows.min(max_margin);

        // If we're close enough to BOF/EOF, disable the margin on that side.
        // (When there are fewer than `margin` lines available above/below, act like margin=0.)
        let effective_top_margin = if cursor_line < desired_top_margin {
            0
        } else {
            desired_top_margin
        };

        let lines_below_cursor = total_lines.saturating_sub(1).saturating_sub(cursor_line);
        let effective_bottom_margin = if lines_below_cursor < desired_bottom_margin {
            0
        } else {
            desired_bottom_margin
        };

        // 1) Hard guarantee: if cursor is above the viewport, scroll up to it.
        if cursor_line < top {
            self.scroll_y_lines = cursor_line;
        } else if cursor_line >= bottom_exclusive {
            // 2) Hard guarantee: if cursor is below the viewport, scroll down just enough.
            self.scroll_y_lines =
                cursor_line.saturating_sub(viewport_height_rows.saturating_sub(1));
        } else {
            // 3) Cursor is currently on-screen: apply top/bottom margin policies.
            let top_threshold = top + effective_top_margin;
            if cursor_line < top_threshold {
                self.scroll_y_lines = cursor_line.saturating_sub(effective_top_margin);
            }

            // Recompute after any top adjustment.
            let top = self.scroll_y_lines;
            let bottom_exclusive = top + viewport_height_rows;

            let bottom_threshold = bottom_exclusive.saturating_sub(1 + effective_bottom_margin);
            if cursor_line >= bottom_threshold {
                self.scroll_y_lines = cursor_line.saturating_sub(
                    (viewport_height_rows - 1).saturating_sub(effective_bottom_margin),
                );
            }
        }

        // Horizontal follow (no margins for now; keep cursor within viewport).
        if self.follow.horizontal_follow {
            let left = self.scroll_x_cells;
            let right_exclusive = left + viewport_width_cells;

            if info.cursor_x_cells < left {
                self.scroll_x_cells = info.cursor_x_cells;
            } else if info.cursor_x_cells >= right_exclusive {
                self.scroll_x_cells = info.cursor_x_cells.saturating_sub(viewport_width_cells - 1);
            }
        }
    }

    // --- Clamping ---

    /// Clamp scroll offsets so they are valid and don't move past EOF.
    ///
    /// For vertical clamping, we compute the total number of visual rows in the wrapped stream,
    /// then allow `scroll_y_rows` to be at most `total_rows - 1` (so the last line can be top-most).
    fn clamp_scroll_to_content(
        &mut self,
        buffer: &TextBuffer,
        _viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        // Horizontal clamp: keep within current line width for sanity.
        let line = buffer.clamp_line(self.cursor.line);
        let line_len = buffer.line_len_chars(line);
        if line_len <= LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS {
            let max_x = self.line_cell_width(buffer, line);
            self.scroll_x_cells = self.scroll_x_cells.min(max_x);
        }

        // Vertical clamp (line-based, no wrapping):
        //
        // We allow scrolling until the last real file line is at the top of the viewport.
        // This will expose "vacuum" space (blank area) below EOF when the viewport is taller.
        // That space is not part of the buffer and must not be reachable by the cursor.
        let total_lines = buffer.len_lines().max(1);
        let max_top = total_lines.saturating_sub(1);

        self.scroll_y_lines = self.scroll_y_lines.min(max_top);

        // Note: we intentionally do NOT clamp to `total_lines - viewport_height_rows` here.
        // Doing so would prevent the last line from being able to sit at the top.
        let _ = viewport_height_rows;
    }

    // --- Non-wrapping projection ---

    /// Compute cursor visual (x,y) under non-wrapping rendering.
    ///
    /// - `cursor_y_lines` is the document line index (0-based).
    /// - `cursor_x_cells` is the cursor column in terminal cells on that line.
    fn cursor_visual_info(
        &mut self,
        buffer: &TextBuffer,
        _viewport_width_cells: usize,
    ) -> CursorVisualInfo {
        let pos = buffer.clamp_pos(self.cursor);
        let line_idx = buffer.clamp_line(pos.line);
        let line_len = buffer.line_len_chars(line_idx);

        if line_len > LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS {
            let x = self.cursor_x_cells_long_line(buffer, line_idx, pos.col);
            return CursorVisualInfo {
                cursor_y_lines: line_idx,
                cursor_x_cells: x,
            };
        }

        // Compute x in terminal cells by measuring grapheme widths up to the cursor char column.
        let line_text = buffer.line_string(line_idx);
        if line_text.is_empty() {
            self.visual_cache = Some(VisualCache {
                line: line_idx,
                col: 0,
                x_cells: 0,
            });
            return CursorVisualInfo {
                cursor_y_lines: line_idx,
                cursor_x_cells: 0,
            };
        }

        let graphemes: Vec<&str> = line_text.graphemes(true).collect();
        let cursor_g = char_col_to_grapheme_index(&graphemes, pos.col);

        let x = graphemes_cell_width(&graphemes[..cursor_g.min(graphemes.len())], self.tab_policy);
        self.visual_cache = Some(VisualCache {
            line: line_idx,
            col: pos.col,
            x_cells: x,
        });

        CursorVisualInfo {
            cursor_y_lines: line_idx,
            cursor_x_cells: x,
        }
    }

    fn line_cell_width(&self, buffer: &TextBuffer, line_idx: usize) -> usize {
        let text = buffer.line_string(line_idx);
        if text.is_empty() {
            return 0;
        }
        let mut width = 0usize;
        for g in text.graphemes(true) {
            width += cell_width(g, self.tab_policy) as usize;
        }
        width
    }

    fn cursor_x_cells_long_line(
        &mut self,
        buffer: &TextBuffer,
        line_idx: usize,
        target_col: usize,
    ) -> usize {
        let line_start = buffer.line_to_char(line_idx);

        if target_col == 0 {
            self.visual_cache = Some(VisualCache {
                line: line_idx,
                col: 0,
                x_cells: 0,
            });
            return 0;
        }

        if let Some(cache) = self.visual_cache
            && cache.line == line_idx
        {
            if cache.col == target_col {
                return cache.x_cells;
            }

            let delta = cache.col.abs_diff(target_col);
            if delta <= LONG_LINE_CURSOR_FAST_PATH_MAX_DELTA_CHARS {
                let mut x = cache.x_cells;
                if target_col > cache.col {
                    for col in cache.col..target_col {
                        let ch = buffer.rope().char(line_start + col);
                        x = x.saturating_add(cell_width_for_char(ch, self.tab_policy));
                    }
                } else {
                    for col in target_col..cache.col {
                        let ch = buffer.rope().char(line_start + col);
                        x = x.saturating_sub(cell_width_for_char(ch, self.tab_policy));
                    }
                }

                self.visual_cache = Some(VisualCache {
                    line: line_idx,
                    col: target_col,
                    x_cells: x,
                });
                return x;
            }
        }

        let mut x = 0usize;
        for col in 0..target_col {
            let ch = buffer.rope().char(line_start + col);
            x = x.saturating_add(cell_width_for_char(ch, self.tab_policy));
        }

        self.visual_cache = Some(VisualCache {
            line: line_idx,
            col: target_col,
            x_cells: x,
        });
        x
    }

    #[inline]
    fn invalidate_visual_cache(&mut self) {
        self.visual_cache = None;
    }
}

#[derive(Debug, Clone, Copy)]
struct CursorVisualInfo {
    cursor_x_cells: usize,
    cursor_y_lines: usize,
}

#[derive(Debug, Clone, Copy)]
struct VisualCache {
    line: usize,
    col: usize,
    x_cells: usize,
}

/// Convert a cursor column in char units to a grapheme index into `graphemes`.
///
/// If the cursor is at/after end-of-line, returns `graphemes.len()`.
fn char_col_to_grapheme_index(graphemes: &[&str], cursor_col_chars: usize) -> usize {
    if cursor_col_chars == 0 {
        return 0;
    }

    let mut chars_seen = 0usize;
    for (i, g) in graphemes.iter().enumerate() {
        let gc = g.chars().count();
        if chars_seen + gc > cursor_col_chars {
            return i;
        }
        chars_seen += gc;
        if chars_seen == cursor_col_chars {
            return i + 1; // cursor is between graphemes
        }
    }

    graphemes.len()
}

/// Cell width of a slice of graphemes (`&str`) using MinUI's `cell_width`.
fn graphemes_cell_width(graphemes: &[&str], tab_policy: TabPolicy) -> usize {
    let mut w = 0usize;
    for g in graphemes {
        w += cell_width(*g, tab_policy) as usize;
    }
    w
}

#[inline]
fn cell_width_for_char(ch: char, tab_policy: TabPolicy) -> usize {
    let mut buf = [0_u8; 4];
    let s = ch.encode_utf8(&mut buf);
    cell_width(s, tab_policy) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_motion_keeps_preferred_column() {
        let buffer = TextBuffer::from_str("aaaa\nb\ncccccc\n");
        let mut cursor = CursorController::new();
        cursor.cursor = Pos::new(0, 3);

        cursor.apply_motion(&buffer, Motion::Down, 1, 80, 24);
        assert_eq!(cursor.cursor, Pos::new(1, 1));

        cursor.apply_motion(&buffer, Motion::Down, 1, 80, 24);
        assert_eq!(cursor.cursor, Pos::new(2, 3));
    }

    #[test]
    fn horizontal_motion_clears_preferred_column() {
        let buffer = TextBuffer::from_str("aaaa\nb\ncccccc\n");
        let mut cursor = CursorController::new();
        cursor.cursor = Pos::new(0, 3);

        cursor.apply_motion(&buffer, Motion::Down, 1, 80, 24);
        assert_eq!(cursor.cursor, Pos::new(1, 1));

        cursor.apply_motion(&buffer, Motion::Right, 1, 80, 24);
        assert_eq!(cursor.cursor, Pos::new(2, 0));

        cursor.apply_motion(&buffer, Motion::Up, 1, 80, 24);
        assert_eq!(cursor.cursor, Pos::new(1, 0));
    }

    #[test]
    fn long_line_fast_path_updates_x_incrementally() {
        let long = format!(
            "{}\t字",
            "a".repeat(LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS + 64)
        );
        let buffer = TextBuffer::from_str(&long);
        let mut cursor = CursorController::new();

        cursor.cursor = Pos::new(0, LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS + 63);
        let x_before = cursor.cursor_spec(&buffer, 400, 24).x as usize;

        cursor.cursor = Pos::new(0, LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS + 64);
        let x_tab = cursor.cursor_spec(&buffer, 400, 24).x as usize;
        let tab_delta = x_tab.saturating_sub(x_before);
        assert!((1..=4).contains(&tab_delta));

        cursor.cursor = Pos::new(0, LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS + 65);
        let x_wide = cursor.cursor_spec(&buffer, 400, 24).x as usize;
        let wide_delta = x_wide.saturating_sub(x_tab);
        assert!(wide_delta >= 1);
    }
}
