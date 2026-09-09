use minui::widgets::WindowView;
use minui::{ColorPair, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui::UiStyle;

const POPUP_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);
const POPUP_ANCHOR_WIDTH_PERCENT: u16 = 65;

#[derive(Debug, Clone, Copy)]
pub struct PopupChrome {
    pub border: ColorPair,
    pub title: ColorPair,
    pub fill: ColorPair,
}

impl PopupChrome {
    pub fn new(border: ColorPair, title: ColorPair, fill: ColorPair) -> Self {
        Self {
            border,
            title,
            fill,
        }
    }

    pub fn about(style: UiStyle) -> Self {
        Self::new(style.about.border, style.about.title, style.about.text)
    }

    pub fn command_line(style: UiStyle) -> Self {
        Self::new(
            style.command_line.border,
            style.command_line.title,
            style.command_line.text,
        )
    }

    pub fn explorer(style: UiStyle) -> Self {
        Self::new(
            style.explorer.border,
            style.explorer.title,
            style.explorer.file,
        )
    }

    pub fn finder(style: UiStyle) -> Self {
        Self::new(style.finder.border, style.finder.title, style.finder.text)
    }

    pub fn finder_preview(style: UiStyle) -> Self {
        Self::new(
            style.finder.border,
            style.finder.preview_title,
            style.finder.text,
        )
    }

    pub fn finder_query(style: UiStyle) -> Self {
        Self::new(
            style.command_line.border,
            style.finder.query_title,
            style.command_line.text,
        )
    }

    pub fn perf(style: UiStyle) -> Self {
        Self::new(style.perf.border, style.perf.title, style.perf.text)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PopupLayout {
    pub inner_w: u16,
    pub inner_h: u16,
    pub x: u16,
    pub y: u16,
}

impl PopupLayout {
    pub fn outer_w(self) -> u16 {
        self.inner_w.saturating_add(2)
    }

    pub fn outer_h(self) -> u16 {
        self.inner_h.saturating_add(2)
    }

    pub fn occludes(self, x: u16, y: u16) -> bool {
        x >= self.x
            && x < self.x.saturating_add(self.outer_w())
            && y >= self.y
            && y < self.y.saturating_add(self.outer_h())
    }
}

pub fn popup_occludes_cursor(layout: PopupLayout, x: u16, y: u16) -> bool {
    layout.occludes(x, y)
}

pub fn popup_inner_size(
    term_w: u16,
    term_h: u16,
    width_percent: u16,
    height_percent: u16,
    min_width: u16,
    min_height: u16,
) -> (u16, u16) {
    let popup_w = centered_popup_width(term_w, width_percent, min_width);
    let popup_h = compute_popup_dim(term_h, height_percent, min_height);
    (popup_w.saturating_sub(2), popup_h.saturating_sub(2))
}

pub fn draw_anchored_popup_frame(
    window: &mut dyn Window,
    term_w: u16,
    term_h: u16,
    inner_w: u16,
    inner_h: u16,
    title: &str,
    chrome: PopupChrome,
) -> minui::Result<PopupLayout> {
    let (x, y) = anchored_popup_origin(term_w, term_h, inner_w, inner_h);
    draw_popup_frame_at(window, x, y, inner_w, inner_h, title, chrome)
}

pub fn anchored_popup_origin(term_w: u16, term_h: u16, inner_w: u16, inner_h: u16) -> (u16, u16) {
    let popup_w = inner_w.saturating_add(2);
    let x = term_w.saturating_sub(popup_w) / 2;
    let y = popup_anchor_top_padding(term_h);
    clamp_popup_origin(term_w, term_h, inner_w, inner_h, x, y)
}

fn clamp_popup_origin(
    term_w: u16,
    term_h: u16,
    inner_w: u16,
    inner_h: u16,
    x: u16,
    y: u16,
) -> (u16, u16) {
    let popup_w = inner_w.saturating_add(2);
    let popup_h = inner_h.saturating_add(2);
    (
        x.min(term_w.saturating_sub(popup_w)),
        y.min(term_h.saturating_sub(popup_h)),
    )
}

pub fn draw_popup_frame_at(
    window: &mut dyn Window,
    x: u16,
    y: u16,
    inner_w: u16,
    inner_h: u16,
    title: &str,
    chrome: PopupChrome,
) -> minui::Result<PopupLayout> {
    let popup_w = inner_w.saturating_add(2);
    let popup_h = inner_h.saturating_add(2);
    let horizontal = "─".repeat(popup_w.saturating_sub(2) as usize);
    window.write_str_colored(y, x, &format!("╭{}╮", horizontal), chrome.border)?;
    if popup_h > 1 {
        for row in (y + 1)..(y + popup_h.saturating_sub(1)) {
            window.write_str_colored(row, x, "│", chrome.border)?;
            window.write_str_colored(row, x + popup_w.saturating_sub(1), "│", chrome.border)?;
        }
    }
    if popup_h > 1 {
        window.write_str_colored(
            y + popup_h.saturating_sub(1),
            x,
            &format!("╰{}╯", horizontal),
            chrome.border,
        )?;
    }

    if popup_w > 3 {
        let title_max = popup_w.saturating_sub(4) as usize;
        let title_text = clip_with_ellipsis(title, title_max);
        window.write_str_colored(y, x + 2, &title_text, chrome.title)?;
    }

    if inner_w > 0 && inner_h > 0 {
        let blank_row = " ".repeat(inner_w as usize);
        for row in 0..inner_h {
            window.write_str_colored(y + 1 + row, x + 1, &blank_row, chrome.fill)?;
        }
    }

    Ok(PopupLayout {
        inner_w,
        inner_h,
        x,
        y,
    })
}

pub fn popup_window_view<'a>(window: &'a mut dyn Window, layout: PopupLayout) -> WindowView<'a> {
    WindowView {
        window,
        x_offset: layout.x + 1,
        y_offset: layout.y + 1,
        scroll_x: 0,
        scroll_y: 0,
        width: layout.inner_w,
        height: layout.inner_h,
    }
}

pub fn draw_popup_view_divider(
    view: &mut WindowView<'_>,
    inner_row: u16,
    colors: ColorPair,
) -> minui::Result<()> {
    if inner_row >= view.height {
        return Ok(());
    }
    view.window.write_str_colored(
        view.y_offset.saturating_add(inner_row),
        view.x_offset.saturating_sub(1),
        &popup_divider_text(view.width),
        colors,
    )
}

fn popup_divider_text(inner_w: u16) -> String {
    format!("├{}┤", "─".repeat(inner_w as usize))
}

pub fn wrap_text_to_cells(text: &str, max_cells: usize) -> Vec<String> {
    minui::wrap_to_cells(
        text,
        max_cells.min(u16::MAX as usize) as u16,
        minui::TextWrapMode::WrapWords,
        POPUP_TAB_POLICY,
    )
}

pub fn clip_text_to_cells(text: &str, max_cells: usize) -> String {
    if max_cells == 0 || text.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;
    for g in text.graphemes(true) {
        let gw = (cell_width(g, POPUP_TAB_POLICY) as usize).max(1);
        if used + gw > max_cells {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out
}

fn compute_popup_dim(total: u16, percent: u16, min: u16) -> u16 {
    if total == 0 {
        return 0;
    }

    let desired = ((u32::from(total) * u32::from(percent)) / 100) as u16;
    let floor = min.min(total);
    let ceiling = if total > 2 { total - 2 } else { total };
    desired.max(floor).min(ceiling.max(floor))
}

fn centered_popup_width(total: u16, percent: u16, min: u16) -> u16 {
    let width = compute_popup_dim(total, percent, min);
    if total.saturating_sub(width).is_multiple_of(2) {
        return width;
    }

    let floor = min.min(total);
    let ceiling = if total > 2 { total - 2 } else { total }.max(floor);
    if width < ceiling {
        width + 1
    } else if width > floor {
        width - 1
    } else {
        width
    }
}

fn popup_anchor_top_padding(term_h: u16) -> u16 {
    let available_percent = 100u16.saturating_sub(POPUP_ANCHOR_WIDTH_PERCENT.min(100));
    ((u32::from(term_h) * u32::from(available_percent)) / 200) as u16
}

fn clip_with_ellipsis(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }

    let mut clipped: String = text.chars().take(max_chars - 3).collect();
    clipped.push_str("...");
    clipped
}
