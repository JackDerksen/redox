use minui::widgets::Widget;
use minui::{Color, ColorPair, Result, TabPolicy, Window, cell_width};
use redox_core::BufferLoadPhase;
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{EditorMode, EditorState};
use crate::ui::helpers::clip_path_with_filename;
use crate::ui::style::StatusModuleColors;
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle};

const SCROLL_MINIMAP_GLYPHS: [&str; 8] = ["▇", "▆", "▅", "▄", "▄", "▃", "▂", "▁"];
pub(crate) const STATUS_MODULE_EDGE_LEFT: &str = "▌";
pub(crate) const STATUS_MODULE_EDGE_RIGHT: &str = "▐";
const STATUS_MODULE_EDGE_WIDTH: u16 = 1;
pub(crate) const STATUS_MODULE_SEPARATOR: &str = "┃";
const STATUS_MODULE_SEPARATOR_WIDTH: u16 = 1;
const DIRTY_GAP_WIDTH: u16 = 0;

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

pub(crate) fn scroll_minimap_cell(
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
    clip: ClipMode,
}

impl Segment {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            colors: None,
            align: Align::Left,
            min_width: None,
            clip: ClipMode::End,
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

    pub fn with_path_clip(mut self) -> Self {
        self.clip = ClipMode::Path;
        self
    }

    pub fn spacer(width: u16) -> Self {
        Self::new("").with_min_width(width)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipMode {
    End,
    Path,
}

#[derive(Debug, Clone)]
struct StatusModule {
    colors: StatusModuleColors,
    status_bg: Color,
    content: String,
    content_align: Align,
}

impl StatusModule {
    fn new(content: impl Into<String>, colors: StatusModuleColors, status_bg: Color) -> Self {
        Self {
            colors,
            status_bg,
            content: content.into(),
            content_align: Align::Left,
        }
    }

    fn content_width(&self) -> u16 {
        self.content.chars().count() as u16
    }

    fn width(&self) -> u16 {
        status_module_width(self.content_width())
    }

    fn into_segments(self) -> Vec<Segment> {
        let mut first = true;
        let mut parts = Vec::new();
        for part in self.content.split(STATUS_MODULE_SEPARATOR) {
            if !first {
                parts.push(
                    Segment::new(STATUS_MODULE_SEPARATOR)
                        .with_color(self.colors.wrapper)
                        .with_min_width(STATUS_MODULE_SEPARATOR_WIDTH),
                );
            }
            if !part.is_empty() {
                parts.push(
                    Segment::new(part)
                        .with_color(self.colors.content)
                        .with_align(self.content_align)
                        .with_min_width(part.chars().count() as u16),
                );
            }
            first = false;
        }

        status_module_segments(self.status_bg, self.colors, parts)
    }
}

fn status_module_width(content_width: u16) -> u16 {
    content_width + (STATUS_MODULE_EDGE_WIDTH * 2)
}

fn status_module_segments(
    status_bg: Color,
    colors: StatusModuleColors,
    parts: Vec<Segment>,
) -> Vec<Segment> {
    // `▌` paints its left half with the foreground and its right half with the
    // background; `▐` does the reverse. Deriving this pair prevents themes from
    // accidentally turning module edges into solid vertical blocks.
    let edge_colors = ColorPair::new(status_bg, colors.content.bg);
    let mut segments = vec![
        Segment::new(STATUS_MODULE_EDGE_LEFT)
            .with_color(edge_colors)
            .with_min_width(STATUS_MODULE_EDGE_WIDTH),
    ];
    segments.extend(parts);
    segments.push(
        Segment::new(STATUS_MODULE_EDGE_RIGHT)
            .with_color(edge_colors)
            .with_min_width(STATUS_MODULE_EDGE_WIDTH),
    );
    segments
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

    fn add_segments(mut self, segments: impl IntoIterator<Item = Segment>) -> Self {
        self.segments.extend(segments);
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

    fn clip_with_ellipsis(text: &str, max_chars: u16, mode: ClipMode) -> String {
        if max_chars == 0 {
            return String::new();
        }

        let len = text.chars().count();
        if len <= usize::from(max_chars) {
            return text.to_owned();
        }

        if max_chars == 1 {
            return "…".to_owned();
        }

        match mode {
            ClipMode::End => {
                let keep = max_chars.saturating_sub(1) as usize;
                let mut out: String = text.chars().take(keep).collect();
                out.push('…');
                out
            }
            ClipMode::Path => clip_path_with_filename(text, usize::from(max_chars)),
        }
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
        let clipped = Self::clip_with_ellipsis(&seg.text, region_w, seg.clip);
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
    let buffer_id = state.statusline_buffer_id();
    let Some(buffer) = state.session.buffer(buffer_id) else {
        return EditorStatusBar::new()
            .with_height(STATUS_BAR_HEIGHT_CELLS)
            .with_bg(style.status_line.bar);
    };
    let Some(meta) = state.session.meta(buffer_id) else {
        return EditorStatusBar::new()
            .with_height(STATUS_BAR_HEIGHT_CELLS)
            .with_bg(style.status_line.bar);
    };

    let (mode_label, mode_colors) =
        status_bar_mode_presentation(state.statusline_mode(), state.rain_is_active(), style);

    let mode_module = StatusModule::new(
        mode_label,
        StatusModuleColors::solid(mode_colors),
        style.status_line.bar.bg,
    );
    let mode_width = mode_module.width();
    let metadata_module = metadata_text(state, buffer_id)
        .map(|text| StatusModule::new(text, style.status_line.metadata, style.status_line.bar.bg));
    let metadata_module_width = metadata_module
        .as_ref()
        .map(StatusModule::width)
        .unwrap_or(0);
    let left_text_width = mode_width.saturating_add(metadata_module_width);

    let center_text = if let Some(label) = state.statusline_popup_label() {
        format!(" {label} ")
    } else {
        let mut name = meta.display_name.to_string();
        if let Some(load) = state.session.buffer_load_status(buffer_id)
            && load.phase == BufferLoadPhase::Loading
        {
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

    let cursor = state.cursor_pos_for_buffer(buffer_id);
    let total_lines = buffer.len_lines();
    let (scroll_glyph, scroll_colors) = scroll_minimap_cell(
        cursor.line,
        total_lines,
        style.status_line.minimap,
        style.status_line.minimap_alt,
        style.status_line.coords.content.bg,
    );
    let visual_col = visual_column(buffer.line_string(cursor.line).as_str(), cursor.col);
    let coords_text = format!("{}:{}", cursor.line + 1, visual_col + 1);
    let right_module_colors = style.status_line.coords;
    let coords_width = coords_text.chars().count() as u16;
    let scroll_width = scroll_glyph.chars().count() as u16;
    let coords_minimap_width =
        status_module_width(coords_width + STATUS_MODULE_SEPARATOR_WIDTH + scroll_width);
    let change_marker_width = u16::from(meta.dirty || meta.external_changed);
    let right_module_width = change_marker_width + DIRTY_GAP_WIDTH + coords_minimap_width;
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

    let left_padding_width = side_reserve_width.saturating_sub(left_text_width);

    let status_bar = EditorStatusBar::new()
        .with_height(STATUS_BAR_HEIGHT_CELLS)
        .with_bg(style.status_line.bar)
        .add_module(mode_module);
    let status_bar = if let Some(module) = metadata_module {
        status_bar.add_module(module)
    } else {
        status_bar
    };
    status_bar
        .add_segment(Segment::spacer(left_padding_width))
        .add_segment(
            Segment::new(center_text)
                .with_color(style.status_line.path)
                .with_align(Align::Center)
                .with_path_clip(),
        )
        .add_segment(Segment::spacer(right_padding_width))
        .add_segment(if meta.external_changed {
            Segment::new("!")
                .with_color(style.status_line.dirty)
                .with_min_width(change_marker_width)
        } else if meta.dirty {
            Segment::new("+")
                .with_color(style.status_line.dirty)
                .with_min_width(change_marker_width)
        } else {
            Segment::spacer(0)
        })
        .add_segment(Segment::spacer(DIRTY_GAP_WIDTH))
        .add_segments(status_module_segments(
            style.status_line.bar.bg,
            right_module_colors,
            vec![
                Segment::new(coords_text)
                    .with_color(right_module_colors.content)
                    .with_align(Align::Right)
                    .with_min_width(coords_width),
                Segment::new(STATUS_MODULE_SEPARATOR)
                    .with_color(ColorPair::new(
                        right_module_colors.wrapper.fg,
                        right_module_colors.content.bg,
                    ))
                    .with_min_width(STATUS_MODULE_SEPARATOR_WIDTH),
                Segment::new(scroll_glyph)
                    .with_color(scroll_colors)
                    .with_min_width(scroll_width),
            ],
        ))
}

fn visual_column(line: &str, char_col: usize) -> usize {
    if char_col == 0 {
        return 0;
    }

    let mut width = 0usize;
    let mut chars_seen = 0usize;
    for grapheme in line.graphemes(true) {
        let grapheme_chars = grapheme.chars().count();
        if chars_seen + grapheme_chars > char_col {
            return width;
        }
        width += cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
        chars_seen += grapheme_chars;
        if chars_seen == char_col {
            return width;
        }
    }

    width
}

fn status_bar_mode_presentation(
    mode: EditorMode,
    rain_active: bool,
    style: UiStyle,
) -> (&'static str, ColorPair) {
    if rain_active {
        return ("RAIN", style.status_line.mode_command);
    }

    match mode {
        EditorMode::Normal => ("NORMAL", style.status_line.mode_normal),
        EditorMode::Insert => ("INSERT", style.status_line.mode_insert),
        EditorMode::Command => ("COMMAND", style.status_line.mode_command),
        EditorMode::Search => ("SEARCH", style.status_line.mode_command),
        EditorMode::Finder => ("FINDER", style.status_line.mode_command),
        EditorMode::PinSelect => ("PINBOARD", style.status_line.mode_command),
        EditorMode::LspMarketplace | EditorMode::DiagnosticsList => {
            ("NORMAL", style.status_line.mode_normal)
        }
        EditorMode::CodeActions => ("ACTIONS", style.status_line.mode_normal),
        EditorMode::SymbolInfo => ("INFO", style.status_line.mode_normal),
        EditorMode::Visual => ("VISUAL", style.status_line.mode_visual),
        EditorMode::VisualLine => ("V-LINE", style.status_line.mode_visual),
        EditorMode::VisualBlock => ("V-BLOCK", style.status_line.mode_visual),
    }
}

fn metadata_text(state: &EditorState, buffer_id: redox_core::BufferId) -> Option<String> {
    match (
        git_diff_summary(state, buffer_id),
        diagnostic_summary_text(state, buffer_id),
    ) {
        (Some(git), Some(diagnostics)) => {
            Some(format!("{git}{STATUS_MODULE_SEPARATOR}{diagnostics}"))
        }
        (Some(git), None) => Some(git),
        (None, Some(diagnostics)) => Some(diagnostics),
        (None, None) => None,
    }
}

fn git_diff_summary(state: &EditorState, buffer_id: redox_core::BufferId) -> Option<String> {
    let Some(diff) = state.git_diff_for_buffer(buffer_id) else {
        return None;
    };
    if diff.stats.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    if diff.stats.added > 0 {
        parts.push(format!("+{}", diff.stats.added));
    }
    if diff.stats.modified > 0 {
        parts.push(format!("~{}", diff.stats.modified));
    }
    if diff.stats.removed > 0 {
        parts.push(format!("-{}", diff.stats.removed));
    }

    Some(parts.join(""))
}

fn diagnostic_summary_text(state: &EditorState, buffer_id: redox_core::BufferId) -> Option<String> {
    let summary = state.diagnostic_summary_for_buffer(buffer_id);
    if summary.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    for (count, glyph) in [
        (summary.errors, "×"),
        (summary.warnings, "△"),
        (summary.information, "•"),
        (summary.hints, "⚬"),
    ] {
        if count == 0 {
            continue;
        }
        parts.push(format!("{glyph}{count}"));
    }

    Some(parts.join(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_edges_are_derived_and_separator_uses_the_wrapper() {
        let status_bg = Color::Rgb { r: 1, g: 2, b: 3 };
        let body_bg = Color::Rgb { r: 4, g: 5, b: 6 };
        let accent = Color::Rgb { r: 7, g: 8, b: 9 };
        let colors = StatusModuleColors {
            // The wrapper is deliberately unrelated to both surrounding
            // backgrounds so the separator assertion covers the complete pair.
            wrapper: ColorPair::new(
                accent,
                Color::Rgb {
                    r: 90,
                    g: 91,
                    b: 92,
                },
            ),
            content: ColorPair::new(Color::White, body_bg),
        };
        let segments = StatusModule::new(
            format!("left{STATUS_MODULE_SEPARATOR}right"),
            colors,
            status_bg,
        )
        .into_segments();

        let edge = ColorPair::new(status_bg, body_bg);
        assert_eq!(
            segments.first().and_then(|segment| segment.colors),
            Some(edge)
        );
        assert_eq!(
            segments.last().and_then(|segment| segment.colors),
            Some(edge)
        );
        let separator = segments
            .iter()
            .find(|segment| segment.text == STATUS_MODULE_SEPARATOR)
            .expect("separator segment");
        assert_eq!(separator.colors, Some(colors.wrapper));
    }
}
