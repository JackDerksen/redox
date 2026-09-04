//! TUI cursor and viewport projection over core motion semantics.

use minui::{TabPolicy, cell_width, window::CursorSpec};
use redox_core::motion::{Motion, apply_motion_n};
use redox_core::{Pos, TextBuffer};
use std::cmp::min;
use unicode_segmentation::UnicodeSegmentation;

const LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS: usize = 8 * 1024;
const LONG_LINE_CURSOR_FAST_PATH_MAX_DELTA_CHARS: usize = 4 * 1024;
pub const DEFAULT_SCROLLOFF_ROWS: usize = 5;

/// Cursor-follow margins, collapsed at document boundaries.
#[derive(Debug, Clone, Copy)]
pub struct FollowConfig {
    pub top_margin_rows: usize,
    pub bottom_margin_rows: usize,
    pub horizontal_follow: bool,
}

impl Default for FollowConfig {
    fn default() -> Self {
        Self {
            top_margin_rows: DEFAULT_SCROLLOFF_ROWS,
            bottom_margin_rows: DEFAULT_SCROLLOFF_ROWS,
            horizontal_follow: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CursorController {
    /// Document position; the column is measured in characters.
    pub cursor: Pos,
    /// Horizontal scroll in terminal cells.
    pub scroll_x_cells: usize,
    /// Vertical scroll in document lines.
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

    pub fn set_scrolloff_rows(&mut self, rows: usize) {
        self.follow.top_margin_rows = rows;
        self.follow.bottom_margin_rows = rows;
    }

    /// Clamp an edited cursor position and keep it visible.
    pub fn reconcile_after_edit(
        &mut self,
        buffer: &TextBuffer,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        self.cursor = buffer.clamp_pos(self.cursor);
        self.preferred_col = None;
        self.invalidate_visual_cache();
        self.reconcile_scroll(buffer, viewport_width_cells, viewport_height_rows);
    }

    pub fn clamp_for_normal_mode(&mut self, buffer: &TextBuffer) {
        self.cursor = buffer.clamp_pos(self.cursor);
        let line_len = buffer.line_len_chars(self.cursor.line);
        if line_len > 0 && self.cursor.col >= line_len {
            self.cursor.col = line_len - 1;
        }
        self.invalidate_visual_cache();
    }

    /// Apply a core motion and keep the resulting cursor visible.
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

        self.reconcile_scroll(buffer, viewport_width_cells, viewport_height_rows);
    }

    fn reconcile_scroll(
        &mut self,
        buffer: &TextBuffer,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        self.clamp_scroll_to_content(buffer, viewport_width_cells, viewport_height_rows);
        let info = self.cursor_visual_info(buffer, viewport_width_cells);
        self.follow_cursor(buffer, info, viewport_width_cells, viewport_height_rows);
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

    /// Project the document cursor into the current terminal viewport.
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

        let vx = info.cursor_x_cells.saturating_sub(self.scroll_x_cells);
        let vy = info.cursor_y_lines.saturating_sub(self.scroll_y_lines);

        let visible = vx < viewport_width_cells && vy < viewport_height_rows;

        CursorSpec {
            x: (vx as u16).min(u16::MAX),
            y: (vy as u16).min(u16::MAX),
            visible,
        }
    }

    #[inline]
    pub fn viewport_scroll(&self) -> (usize, usize) {
        (self.scroll_x_cells, self.scroll_y_lines)
    }

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

        let top = self.scroll_y_lines;
        let bottom_exclusive = top + viewport_height_rows;

        let cursor_line = info.cursor_y_lines;
        let total_lines = buffer.len_lines().max(1);

        let max_margin = viewport_height_rows.saturating_sub(1);
        let desired_top_margin = self.follow.top_margin_rows.min(max_margin);
        let desired_bottom_margin = self.follow.bottom_margin_rows.min(max_margin);

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

        if cursor_line < top {
            self.scroll_y_lines = cursor_line;
        } else if cursor_line >= bottom_exclusive {
            self.scroll_y_lines =
                cursor_line.saturating_sub(viewport_height_rows.saturating_sub(1));
        } else {
            let top_threshold = top + effective_top_margin;
            if cursor_line < top_threshold {
                self.scroll_y_lines = cursor_line.saturating_sub(effective_top_margin);
            }

            let top = self.scroll_y_lines;
            let bottom_exclusive = top + viewport_height_rows;

            let bottom_threshold = bottom_exclusive.saturating_sub(1 + effective_bottom_margin);
            if cursor_line >= bottom_threshold {
                self.scroll_y_lines = cursor_line.saturating_sub(
                    (viewport_height_rows - 1).saturating_sub(effective_bottom_margin),
                );
            }
        }

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

    /// Clamp scroll offsets without preventing the last real line from sitting
    /// at the top of the viewport.
    fn clamp_scroll_to_content(
        &mut self,
        buffer: &TextBuffer,
        _viewport_width_cells: usize,
        _viewport_height_rows: usize,
    ) {
        let line = buffer.clamp_line(self.cursor.line);
        let line_len = buffer.line_len_chars(line);
        if line_len <= LONG_LINE_CURSOR_FAST_PATH_THRESHOLD_CHARS {
            let max_x = self.line_cell_width(buffer, line);
            self.scroll_x_cells = self.scroll_x_cells.min(max_x);
        }

        let total_lines = buffer.len_lines().max(1);
        let max_top = total_lines.saturating_sub(1);

        self.scroll_y_lines = self.scroll_y_lines.min(max_top);
    }

    /// Compute the non-wrapping document line and terminal-cell column.
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

        let x = cell_width_until_char_col(&line_text, pos.col, self.tab_policy);
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
                    for ch in buffer.chars(line_start + cache.col..line_start + target_col) {
                        x = x.saturating_add(cell_width_for_char(ch, self.tab_policy));
                    }
                } else {
                    for ch in buffer.chars(line_start + target_col..line_start + cache.col) {
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
        for ch in buffer.chars(line_start..line_start + target_col) {
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

fn cell_width_until_char_col(line: &str, cursor_col_chars: usize, tab_policy: TabPolicy) -> usize {
    let mut chars_seen = 0usize;
    let mut width = 0usize;
    for grapheme in line.graphemes(true) {
        let next = chars_seen + grapheme.chars().count();
        if cursor_col_chars < next {
            return width;
        }
        width += cell_width(grapheme, tab_policy) as usize;
        chars_seen = next;
        if chars_seen == cursor_col_chars {
            return width;
        }
    }
    width
}

#[inline]
fn cell_width_for_char(ch: char, tab_policy: TabPolicy) -> usize {
    let mut utf8_buffer = [0_u8; 4];
    let encoded = ch.encode_utf8(&mut utf8_buffer);
    cell_width(encoded, tab_policy) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_motion_keeps_preferred_column() {
        let buffer = TextBuffer::from_text("aaaa\nb\ncccccc\n");
        let mut cursor = CursorController::new();
        cursor.cursor = Pos::new(0, 3);

        cursor.apply_motion(&buffer, Motion::Down, 1, 80, 24);
        assert_eq!(cursor.cursor, Pos::new(1, 1));

        cursor.apply_motion(&buffer, Motion::Down, 1, 80, 24);
        assert_eq!(cursor.cursor, Pos::new(2, 3));
    }

    #[test]
    fn horizontal_motion_clears_preferred_column() {
        let buffer = TextBuffer::from_text("aaaa\nb\ncccccc\n");
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
        let buffer = TextBuffer::from_text(&long);
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
