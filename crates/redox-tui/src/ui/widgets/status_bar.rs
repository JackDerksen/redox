//! Segment-based status bar widget for `redox-tui`.
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
use redox_core::BufferLoadPhase;

use crate::app::{EditorMode, EditorState};
use crate::ui::style::{StatusModuleColors, StatusModuleKind};
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle};

const SCROLL_MINIMAP_GLYPHS: [&str; 8] = ["▇", "▆", "▅", "▄", "▄", "▃", "▂", "▁"];
const STATUS_MODULE_EDGE_LEFT: &str = "▌";
const STATUS_MODULE_EDGE_RIGHT: &str = "▐";
const STATUS_MODULE_EDGE_WIDTH: u16 = 1;

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

fn balanced_status_side_width(
    left_content_width: u16,
    left_min_width: u16,
    right_content_width: u16,
    right_min_width: u16,
) -> u16 {
    left_content_width
        .max(left_min_width)
        .max(right_content_width.max(right_min_width))
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

    pub fn spacer(width: u16) -> Self {
        Self::new("").with_min_width(width)
    }
}

#[derive(Debug, Clone)]
struct StatusModule {
    colors: StatusModuleColors,
    content: String,
    content_align: Align,
}

impl StatusModule {
    fn new(content: impl Into<String>, colors: StatusModuleColors) -> Self {
        Self {
            colors,
            content: content.into(),
            content_align: Align::Left,
        }
    }

    fn with_content_colors(mut self, content_colors: ColorPair) -> Self {
        self.colors.content = content_colors;
        self
    }

    fn with_content_align(mut self, align: Align) -> Self {
        self.content_align = align;
        self
    }

    fn wrapped_text(&self) -> String {
        format!(
            "{STATUS_MODULE_EDGE_LEFT}{}{STATUS_MODULE_EDGE_RIGHT}",
            self.content
        )
    }

    fn content_width(&self) -> u16 {
        self.content.chars().count() as u16
    }

    fn width(&self) -> u16 {
        self.content_width() + (STATUS_MODULE_EDGE_WIDTH * 2)
    }

    fn into_segments(self) -> [Segment; 3] {
        let content_width = self.content_width();
        [
            Segment::new(STATUS_MODULE_EDGE_LEFT)
                .with_color(self.colors.wrapper)
                .with_min_width(STATUS_MODULE_EDGE_WIDTH),
            Segment::new(self.content)
                .with_color(self.colors.content)
                .with_align(self.content_align)
                .with_min_width(content_width),
            Segment::new(STATUS_MODULE_EDGE_RIGHT)
                .with_color(self.colors.wrapper)
                .with_min_width(STATUS_MODULE_EDGE_WIDTH),
        ]
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

    fn add_module(mut self, module: StatusModule) -> Self {
        self.segments.extend(module.into_segments());
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
    let module_theme = style.palette.status_modules;
    let (mode_label, mode_colors) = if state.rain_is_active() {
        ("RAIN", style.palette.mode_command)
    } else {
        match state.mode {
            EditorMode::Normal => ("NORMAL", style.palette.mode_normal),
            EditorMode::Insert => ("INSERT", style.palette.mode_insert),
            EditorMode::Command => ("COMMAND", style.palette.mode_command),
            EditorMode::Visual => ("VISUAL", style.palette.mode_visual),
            EditorMode::VisualLine => ("V-LINE", style.palette.mode_visual),
        }
    };

    let mode_module = StatusModule::new(mode_label, StatusModuleColors::solid(mode_colors));
    let mut left_text = mode_module.wrapped_text();
    if state.active_dirty() {
        left_text.push_str("[+]");
    }
    let left_text_width = left_text.chars().count() as u16;

    let center_text = if let Some(msg) = &state.status_msg {
        format!(" {} ", msg)
    } else if state.explorer_popup().is_some() {
        " explorer ".to_string()
    } else {
        let mut name = state.active_display_name().to_string();
        let load = state.session.active_buffer_load_status();
        if load.phase == BufferLoadPhase::Loading {
            let progress = match load.total_bytes {
                Some(total) if total > 0 => {
                    let pct = (load.bytes_loaded.saturating_mul(100) / total).min(100);
                    format!("{pct}%")
                }
                _ => format!("{} bytes", load.bytes_loaded),
            };
            name.push_str(&format!(" [loading {progress}]"));
        }
        format!(" {name} ")
    };

    let cursor = state.active_cursor_pos();
    let total_lines = state.session.active_buffer().len_lines();
    let minimap_module_colors = module_theme.colors(StatusModuleKind::Minimap);
    let (scroll_glyph, scroll_colors) = scroll_minimap_cell(
        cursor.line,
        total_lines,
        style.palette.minimap,
        style.palette.minimap_alt,
        minimap_module_colors.wrapper.bg,
    );
    let coords_module = StatusModule::new(
        format!("{}:{}", cursor.line + 1, cursor.col + 1),
        module_theme.colors(StatusModuleKind::Coords),
    )
    .with_content_align(Align::Right);
    let minimap_module =
        StatusModule::new(scroll_glyph, minimap_module_colors).with_content_colors(scroll_colors);
    let right_module_width =
        coords_module.width() + style.layout.status_module_gap_width + minimap_module.width();
    let side_reserve_width = balanced_status_side_width(
        left_text_width,
        style.layout.status_left_min_width,
        right_module_width,
        style.layout.status_right_min_width,
    );
    let right_padding_width = style
        .layout
        .status_right_min_width
        .max(side_reserve_width)
        .saturating_sub(right_module_width);

    EditorStatusBar::new()
        .with_height(STATUS_BAR_HEIGHT_CELLS)
        .with_bg(style.palette.status_bar_bg)
        .add_segment(
            Segment::new(left_text)
                .with_color(mode_colors)
                .with_align(Align::Left)
                .with_min_width(side_reserve_width),
        )
        .add_segment(
            Segment::new(center_text)
                .with_color(style.palette.status_bar_bg)
                .with_align(Align::Center),
        )
        .add_segment(Segment::spacer(right_padding_width))
        .add_module(coords_module)
        .add_segment(Segment::spacer(style.layout.status_module_gap_width))
        .add_module(minimap_module)
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
        let style = UiStyle::default();
        let palette = style.palette;
        let minimap_bg = style
            .palette
            .status_modules
            .colors(StatusModuleKind::Minimap)
            .wrapper
            .bg;
        let (top_glyph, top_colors) =
            scroll_minimap_cell(0, 100, palette.minimap, palette.minimap_alt, minimap_bg);
        let (mid_glyph, _) =
            scroll_minimap_cell(50, 100, palette.minimap, palette.minimap_alt, minimap_bg);
        let (bottom_glyph, bottom_colors) =
            scroll_minimap_cell(99, 100, palette.minimap, palette.minimap_alt, minimap_bg);
        assert_eq!(top_glyph, "▇");
        assert_eq!(bottom_glyph, "▁");
        assert_ne!(mid_glyph, top_glyph);
        assert_eq!(top_colors.bg, style.theme.white);
        assert_eq!(bottom_colors.fg, style.theme.white);
        assert_eq!(top_colors.fg, minimap_bg);
        assert_eq!(bottom_colors.bg, minimap_bg);
    }

    #[test]
    fn right_coords_module_uses_light_gray_highlight() {
        let style = UiStyle::default();
        let colors = style
            .palette
            .status_modules
            .colors(StatusModuleKind::Coords);

        assert_eq!(colors.wrapper.fg, style.palette.status_bar_bg.bg);
        assert_eq!(colors.wrapper.bg, style.theme.light_gray);
        assert_eq!(colors.content, colors.wrapper);
    }

    #[test]
    fn right_minimap_module_uses_distinct_highlight() {
        let style = UiStyle::default();
        let colors = style
            .palette
            .status_modules
            .colors(StatusModuleKind::Minimap);

        assert_eq!(colors.wrapper.fg, style.palette.status_bar_bg.bg);
        assert_eq!(colors.wrapper.bg, style.theme.dark_gray);
        assert_eq!(colors.content, colors.wrapper);
    }

    #[test]
    fn status_module_emits_wrapped_segments() {
        let style = UiStyle::default();
        let wrapper_colors = style
            .palette
            .status_modules
            .colors(StatusModuleKind::Coords);
        let content_colors = style
            .palette
            .status_modules
            .colors(StatusModuleKind::Minimap);
        let module = StatusModule::new("42:7", wrapper_colors)
            .with_content_colors(content_colors.wrapper)
            .with_content_align(Align::Right);
        let width = module.width();
        let segments = module.into_segments();

        assert_eq!(width, 6);
        assert_eq!(segments[0].text, STATUS_MODULE_EDGE_LEFT);
        assert_eq!(segments[0].colors, Some(wrapper_colors.wrapper));
        assert_eq!(segments[1].text, "42:7");
        assert_eq!(segments[1].colors, Some(content_colors.wrapper));
        assert_eq!(segments[1].align, Align::Right);
        assert_eq!(segments[2].text, STATUS_MODULE_EDGE_RIGHT);
        assert_eq!(segments[2].colors, Some(wrapper_colors.wrapper));
    }

    #[test]
    fn right_modules_respect_configured_gap_width() {
        let style = UiStyle::default();
        let coords = StatusModule::new(
            "3:1",
            style
                .palette
                .status_modules
                .colors(StatusModuleKind::Coords),
        );
        let minimap = StatusModule::new(
            "▇",
            style
                .palette
                .status_modules
                .colors(StatusModuleKind::Minimap),
        );

        assert_eq!(
            coords.width() + style.layout.status_module_gap_width + minimap.width(),
            8
        );
    }

    #[test]
    fn status_bar_balances_side_reservations_for_centering() {
        assert_eq!(balanced_status_side_width(8, 12, 18, 18), 18);
        assert_eq!(balanced_status_side_width(20, 12, 18, 18), 20);
    }
}
