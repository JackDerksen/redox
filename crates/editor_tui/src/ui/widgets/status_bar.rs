//! Segment-based status bar widget for `editor_tui`.
//!
//! This is a small custom status bar widget, different from MinUI's built-in `StatusBar`
//! (see `minui::widgets::statusbar`). This one is generalized to support an arbitrary
//! number of segments with per-segment colours and alignment, and just be generally
//! more configurable.
//!
//! Design goals:
//! - single-row by default (height=1), anchored at the bottom of the window, just like vim.
//! - optional background fill (useful for a bar background colour)
//! - multiple segments, each aligned Left/Center/Right within its own allocated region
//!
//! Notes:
//! - Width computations are based on `chars().count()` (like MinUI's own StatusBar).
//!   This is not terminal-cell accurate for wide glyphs, but it's good enough for now.
//!   It also intelligently adjusts along with the window size.
//! - This widget draws at x=0 and computes y based on window height.

use minui::widgets::Widget;
use minui::{Color, ColorPair, Result, Window};

use crate::app::{EditorMode, EditorState};
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle};

const SCROLL_MINIMAP_GLYPHS: [&str; 8] = ["▇", "▆", "▅", "▄", "▄", "▃", "▂", "▁"];
const SCROLL_MINIMAP_WIDTH: u16 = 1;

fn scroll_progress_idx(cursor_line: usize, total_lines: usize) -> usize {
    if total_lines <= 1 {
        return 0;
    }

    let max_line = total_lines.saturating_sub(1);
    let clamped_line = cursor_line.min(max_line);
    let ratio = clamped_line as f32 / max_line as f32;
    let idx = ((SCROLL_MINIMAP_GLYPHS.len() - 1) as f32 * ratio).round() as usize;
    idx.min(SCROLL_MINIMAP_GLYPHS.len() - 1)
}

fn resolve_transparent_to(color: Color, fallback: Color) -> Color {
    if matches!(color, Color::Transparent) {
        fallback
    } else {
        color
    }
}

fn resolve_minimap_pair(base: ColorPair, status_bg: Color) -> ColorPair {
    ColorPair::new(
        resolve_transparent_to(base.fg, status_bg),
        resolve_transparent_to(base.bg, status_bg),
    )
}

fn scroll_minimap_cell(
    cursor_line: usize,
    total_lines: usize,
    minimap: ColorPair,
    minimap_alt: ColorPair,
    status_bg: Color,
) -> (&'static str, ColorPair) {
    let idx = scroll_progress_idx(cursor_line, total_lines);
    let glyph = SCROLL_MINIMAP_GLYPHS[idx];
    let colors = if idx < 4 {
        resolve_minimap_pair(minimap_alt, status_bg)
    } else {
        resolve_minimap_pair(minimap, status_bg)
    };
    (glyph, colors)
}

/// Horizontal alignment of a segment within its allotted region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// One status bar segment.
///
/// `min_width` allocates a fixed region width for the segment. If `None`,
/// the segment shares remaining space equally with other flexible segments.
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub colors: Option<ColorPair>,
    pub align: Align,
    pub min_width: Option<u16>,
}

impl Segment {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            colors: None,
            align: Align::Left,
            min_width: None,
        }
    }

    pub fn with_color(mut self, colors: ColorPair) -> Self {
        self.colors = Some(colors);
        self
    }

    pub fn with_align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn with_min_width(mut self, width: u16) -> Self {
        self.min_width = Some(width);
        self
    }
}

/// Segment-based status bar widget.
///
/// By default:
/// - `height = 1`
/// - `bg_colors = None` (no background fill)
/// - anchored at bottom (`y = window_height - height`)
#[derive(Debug, Clone)]
pub struct EditorStatusBar {
    segments: Vec<Segment>,
    bg_colors: Option<ColorPair>,
    height: u16,
}

impl Default for EditorStatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorStatusBar {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
            bg_colors: None,
            height: 1,
        }
    }

    pub fn with_bg(mut self, colors: ColorPair) -> Self {
        self.bg_colors = Some(colors);
        self
    }

    pub fn with_height(mut self, height: u16) -> Self {
        self.height = height.max(1);
        self
    }

    pub fn add_segment(mut self, segment: Segment) -> Self {
        self.segments.push(segment);
        self
    }

    fn calculate_y(&self, window_height: u16) -> u16 {
        if self.height >= window_height {
            return 0;
        }
        window_height - self.height
    }

    fn draw_background_row(&self, window: &mut dyn Window, y: u16, width: u16) -> Result<()> {
        if width == 0 {
            return Ok(());
        }

        if let Some(bg) = self.bg_colors {
            // Fill full line with spaces in the background colour.
            // Matches MinUI's approach.
            let full = " ".repeat(width as usize);
            window.write_str_colored(y, 0, &full, bg)?;
        }
        Ok(())
    }

    fn segment_region_widths(&self, width: u16) -> Vec<u16> {
        if self.segments.is_empty() {
            return Vec::new();
        }

        let fixed_sum: u16 = self.segments.iter().filter_map(|s| s.min_width).sum();

        let flexible_count: u16 = self
            .segments
            .iter()
            .filter(|s| s.min_width.is_none())
            .count() as u16;

        let remaining = width.saturating_sub(fixed_sum);
        let default_flex = if flexible_count > 0 {
            remaining / flexible_count
        } else {
            0
        };

        // Distribute any remainder to the first few flexible segments so total sums to `width`.
        let mut remainder = if flexible_count > 0 {
            remaining - default_flex * flexible_count
        } else {
            0
        };

        let mut out = Vec::with_capacity(self.segments.len());
        for seg in &self.segments {
            let mut w = seg.min_width.unwrap_or(default_flex);
            if seg.min_width.is_none() && remainder > 0 {
                w = w.saturating_add(1);
                remainder -= 1;
            }
            out.push(w);
        }

        out
    }

    fn clip_with_ellipsis(text: &str, max_chars: u16) -> String {
        if max_chars == 0 {
            return String::new();
        }

        let len = text.chars().count() as u16;
        if len <= max_chars {
            return text.to_owned();
        }

        // Reserve 1 char for ellipsis when possible.
        if max_chars == 1 {
            return "…".to_owned();
        }

        let take = (max_chars - 1) as usize;
        let mut out: String = text.chars().take(take).collect();
        out.push('…');
        out
    }

    fn draw_segment(
        &self,
        window: &mut dyn Window,
        y: u16,
        region_x: u16,
        region_w: u16,
        seg: &Segment,
    ) -> Result<()> {
        if region_w == 0 {
            return Ok(());
        }

        // Clip segment text to fit region.
        let clipped = Self::clip_with_ellipsis(&seg.text, region_w);
        let text_w = clipped.chars().count() as u16;

        let x = match seg.align {
            Align::Left => region_x,
            Align::Center => region_x + (region_w.saturating_sub(text_w) / 2),
            Align::Right => region_x + region_w.saturating_sub(text_w),
        };

        if let Some(colors) = seg.colors {
            window.write_str_colored(y, x, &clipped, colors)?;
        } else {
            window.write_str(y, x, &clipped)?;
        }

        Ok(())
    }

    fn render_row(&self, window: &mut dyn Window, y: u16, width: u16) -> Result<()> {
        if width == 0 {
            return Ok(());
        }

        self.draw_background_row(window, y, width)?;

        let region_widths = self.segment_region_widths(width);

        let mut x = 0u16;
        for (seg, region_w) in self.segments.iter().zip(region_widths.iter().copied()) {
            if x >= width {
                break;
            }

            // Clamp region to window width.
            let region_w = region_w.min(width - x);

            if region_w > 0 {
                self.draw_segment(window, y, x, region_w, seg)?;
            }

            x = x.saturating_add(region_w);
        }

        Ok(())
    }
}

impl Widget for EditorStatusBar {
    fn draw(&self, window: &mut dyn Window) -> Result<()> {
        let (width, height) = window.get_size();
        let y0 = self.calculate_y(height);

        // Multi-height: each row currently identical (background fill + same segments).
        for i in 0..self.height {
            let y = y0 + i;
            if y >= height {
                break;
            }
            self.render_row(window, y, width)?;
        }

        Ok(())
    }

    fn get_size(&self) -> (u16, u16) {
        (u16::MAX, self.height)
    }

    fn get_position(&self) -> (u16, u16) {
        (0, 0)
    }
}

/// Build the editor's standard bottom status bar from state + style.
pub fn build_editor_status_bar(state: &EditorState, style: UiStyle) -> EditorStatusBar {
    let (mode_label, mode_colors) = match state.mode {
        EditorMode::Normal => ("NORMAL", style.palette.mode_normal),
        EditorMode::Insert => ("INSERT", style.palette.mode_insert),
        EditorMode::Command => ("COMMAND", style.palette.mode_command),
    };

    let mut left_text = format!("▌{}▐", mode_label);
    if state.active_dirty() {
        left_text.push_str("[+]");
    }

    let center_text = if state.mode == EditorMode::Command {
        format!(" :{} ", state.command_line)
    } else if let Some(msg) = &state.status_msg {
        format!(" {} ", msg)
    } else {
        format!(" {} ", state.active_display_name())
    };

    let cursor = state.active_cursor_pos();
    let total_lines = state.session.active_buffer().len_lines();
    let (scroll_glyph, scroll_colors) = scroll_minimap_cell(
        cursor.line,
        total_lines,
        style.palette.minimap,
        style.palette.minimap_alt,
        style.palette.status_bar_bg.bg,
    );
    let right_text = format!(" {}:{} ", cursor.line + 1, cursor.col + 1);

    EditorStatusBar::new()
        .with_height(STATUS_BAR_HEIGHT_CELLS)
        .with_bg(style.palette.status_bar_bg)
        .add_segment(
            Segment::new(left_text)
                .with_color(mode_colors)
                .with_align(Align::Left)
                .with_min_width(style.layout.status_left_min_width),
        )
        .add_segment(
            Segment::new(center_text)
                .with_color(style.palette.status_bar_bg)
                .with_align(Align::Center),
        )
        .add_segment(
            Segment::new(right_text)
                .with_color(style.palette.status_bar_bg)
                .with_align(Align::Right)
                .with_min_width(style.layout.status_right_min_width),
        )
        .add_segment(
            Segment::new(scroll_glyph)
                .with_color(scroll_colors)
                .with_align(Align::Right)
                .with_min_width(SCROLL_MINIMAP_WIDTH),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_minimap_clamps_for_empty_or_single_line() {
        let palette = UiStyle::default().palette;
        let (glyph_a, _) = scroll_minimap_cell(
            0,
            0,
            palette.minimap,
            palette.minimap_alt,
            palette.status_bar_bg.bg,
        );
        let (glyph_b, _) = scroll_minimap_cell(
            0,
            1,
            palette.minimap,
            palette.minimap_alt,
            palette.status_bar_bg.bg,
        );
        let (glyph_c, _) = scroll_minimap_cell(
            10,
            1,
            palette.minimap,
            palette.minimap_alt,
            palette.status_bar_bg.bg,
        );
        assert_eq!(glyph_a, "▇");
        assert_eq!(glyph_b, "▇");
        assert_eq!(glyph_c, "▇");
    }

    #[test]
    fn scroll_minimap_moves_from_top_to_bottom() {
        let palette = UiStyle::default().palette;
        let (top_glyph, top_colors) = scroll_minimap_cell(
            0,
            100,
            palette.minimap,
            palette.minimap_alt,
            palette.status_bar_bg.bg,
        );
        let (mid_glyph, _) = scroll_minimap_cell(
            50,
            100,
            palette.minimap,
            palette.minimap_alt,
            palette.status_bar_bg.bg,
        );
        let (bottom_glyph, bottom_colors) = scroll_minimap_cell(
            99,
            100,
            palette.minimap,
            palette.minimap_alt,
            palette.status_bar_bg.bg,
        );
        assert_eq!(top_glyph, "▇");
        assert_eq!(bottom_glyph, "▁");
        assert_ne!(mid_glyph, top_glyph);
        assert_eq!(top_colors.bg, Color::White);
        assert_eq!(bottom_colors.fg, Color::White);
        assert_eq!(top_colors.fg, palette.status_bar_bg.bg);
        assert_eq!(bottom_colors.bg, palette.status_bar_bg.bg);
    }
}
