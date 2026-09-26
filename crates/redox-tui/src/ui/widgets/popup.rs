pub(crate) use minui::widgets::WidgetArea as MouseRect;
use minui::widgets::WindowView;
use minui::{ColorPair, Event, RouteTarget, TabPolicy, UiScene, Window, cell_width};
use redox_core::Pos;
use std::path::PathBuf;
use unicode_segmentation::UnicodeSegmentation;

use crate::ui::UiStyle;
use crate::ui::helpers::proportional_size;

const POPUP_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);
const POPUP_ANCHOR_WIDTH_PERCENT: u16 = 65;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MousePopup {
    Finder,
    Pinboard,
    Explorer,
    About,
    LanguageTools,
    Diagnostics,
    CodeActions,
    Completion,
    SymbolInfo,
    Command,
    Search,
    Perf,
    WhichKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MouseTarget {
    Entry { index: usize, identity: String },
    FinderEntry(PathBuf),
    BufferPosition(Pos),
    InputCursor(usize),
    DiagnosticAction { index: usize, identity: String },
    Key(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MouseScroll {
    List,
    Preview,
    Details,
    Actions,
    Text,
}

impl From<PopupLayout> for MouseRect {
    fn from(layout: PopupLayout) -> Self {
        Self {
            x: layout.x,
            y: layout.y,
            width: layout.outer_w(),
            height: layout.outer_h(),
        }
    }
}

pub(crate) struct PopupMouseLayout {
    pub kind: MousePopup,
    pub frames: Vec<MouseRect>,
    pub clicks: Vec<(MouseRect, MouseTarget)>,
    pub scrolls: Vec<(MouseRect, MouseScroll)>,
    scene: UiScene,
}

impl std::fmt::Debug for PopupMouseLayout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PopupMouseLayout")
            .field("kind", &self.kind)
            .field("frames", &self.frames)
            .field("clicks", &self.clicks)
            .field("scrolls", &self.scrolls)
            .finish_non_exhaustive()
    }
}

impl PopupMouseLayout {
    pub(crate) fn new(kind: MousePopup) -> Self {
        Self {
            kind,
            frames: Vec::new(),
            clicks: Vec::new(),
            scrolls: Vec::new(),
            scene: UiScene::new(),
        }
    }

    pub(crate) fn cache_interactions(&mut self) {
        self.scene.begin_frame();
        for frame in &self.frames {
            self.scene.register(0, *frame);
        }
        let scroll_start = self.clicks.len() + 1;
        for (index, (area, _)) in self.scrolls.iter().enumerate() {
            self.scene.register_scrollable(scroll_start + index, *area);
        }
        for (index, (area, _)) in self.clicks.iter().enumerate() {
            let id = index + 1;
            self.scene.register(id, *area);
            if let Some(owner) = self
                .scrolls
                .iter()
                .rposition(|(scroll, _)| scroll.contains_point(area.x, area.y))
            {
                self.scene.set_owner(id, scroll_start + owner);
            }
        }
    }

    pub(crate) fn hit_click(&mut self, x: u16, y: u16) -> Option<Option<MouseTarget>> {
        let index = self.scene.hit_test(x, y)?.id;
        Some(
            index
                .checked_sub(1)
                .and_then(|index| self.clicks.get(index))
                .map(|(_, target)| target.clone()),
        )
    }

    pub(crate) fn hit_scroll(&mut self, event: &Event) -> Option<(MouseRect, MouseScroll)> {
        let RouteTarget::Id(id) = self.scene.route_wheel_event(event)? else {
            return None;
        };
        let index = id.checked_sub(self.clicks.len() + 1)?;
        self.scrolls.get(index).copied()
    }

    pub(crate) fn add_frame(&mut self, layout: PopupLayout) {
        self.frames.push(layout.into());
    }

    pub(crate) fn add_row(&mut self, layout: PopupLayout, row: u16, target: MouseTarget) {
        if row < layout.inner_h && layout.inner_w > 0 {
            self.clicks.push((
                MouseRect {
                    x: layout.x.saturating_add(1),
                    y: layout.y.saturating_add(1).saturating_add(row),
                    width: layout.inner_w,
                    height: 1,
                },
                target,
            ));
        }
    }

    pub(crate) fn add_input(&mut self, rect: MouseRect, text: &str, start_byte: usize) {
        let mut column = 0u16;
        let mut end_byte = start_byte;
        for (byte, grapheme) in text.grapheme_indices(true) {
            let width = cell_width(grapheme, POPUP_TAB_POLICY).max(1);
            if column.saturating_add(width) > rect.width {
                break;
            }
            self.clicks.push((
                MouseRect {
                    x: rect.x.saturating_add(column),
                    width,
                    ..rect
                },
                MouseTarget::InputCursor(start_byte + byte),
            ));
            column = column.saturating_add(width);
            end_byte = start_byte + byte + grapheme.len();
        }
        if column < rect.width {
            self.clicks.push((
                MouseRect {
                    x: rect.x.saturating_add(column),
                    width: rect.width - column,
                    ..rect
                },
                MouseTarget::InputCursor(end_byte),
            ));
        }
    }

    pub(crate) fn add_buffer_line(
        &mut self,
        rect: MouseRect,
        line: usize,
        text: &str,
        scroll_x: usize,
    ) {
        let mut cell = 0usize;
        let mut column = 0usize;
        let mut painted = 0u16;
        for grapheme in text.trim_end_matches(['\r', '\n']).graphemes(true) {
            if cell >= scroll_x.saturating_add(rect.width as usize) {
                break;
            }
            let end = cell.saturating_add(cell_width(grapheme, POPUP_TAB_POLICY).max(1) as usize);
            if end > scroll_x && cell < scroll_x.saturating_add(rect.width as usize) {
                let start = cell.saturating_sub(scroll_x).min(rect.width as usize) as u16;
                let width = end.saturating_sub(scroll_x).min(rect.width as usize) as u16 - start;
                self.clicks.push((
                    MouseRect {
                        x: rect.x.saturating_add(start),
                        width,
                        ..rect
                    },
                    MouseTarget::BufferPosition(Pos::new(line, column)),
                ));
                painted = start.saturating_add(width);
            }
            cell = end;
            column += grapheme.chars().count();
        }
        if painted < rect.width {
            self.clicks.push((
                MouseRect {
                    x: rect.x.saturating_add(painted),
                    width: rect.width - painted,
                    ..rect
                },
                MouseTarget::BufferPosition(Pos::new(line, column)),
            ));
        }
    }
}

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
    let desired = proportional_size(total, percent, min);
    let floor = min.min(total);
    let ceiling = if total > 2 { total - 2 } else { total };
    desired.min(ceiling.max(floor))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_text_targets_preserve_graphemes_and_blank_space_after_scrolling() {
        let rect = MouseRect {
            x: 10,
            y: 4,
            width: 6,
            height: 1,
        };
        let target_at = |layout: &mut PopupMouseLayout, x| layout.hit_click(x, 4).flatten();
        let mut input = PopupMouseLayout::new(MousePopup::Command);
        input.frames.push(MouseRect {
            x: 9,
            y: 3,
            width: 8,
            height: 3,
        });
        input.scrolls.push((rect, MouseScroll::Text));
        input.add_input(rect, "界e\u{301}", 3);
        input.cache_interactions();
        assert_eq!(input.hit_click(0, 0), None);
        assert_eq!(input.hit_click(9, 3), Some(None));
        assert_eq!(
            input.hit_scroll(&Event::MouseScroll {
                x: 10,
                y: 4,
                delta: 1
            }),
            Some((rect, MouseScroll::Text))
        );
        assert_eq!(
            input.hit_scroll(&Event::MouseScroll {
                x: 9,
                y: 3,
                delta: 1
            }),
            None
        );
        assert_eq!(target_at(&mut input, 10), Some(MouseTarget::InputCursor(3)));
        assert_eq!(target_at(&mut input, 11), Some(MouseTarget::InputCursor(3)));
        assert_eq!(target_at(&mut input, 12), Some(MouseTarget::InputCursor(6)));
        assert_eq!(target_at(&mut input, 15), Some(MouseTarget::InputCursor(9)));

        let mut buffer = PopupMouseLayout::new(MousePopup::Explorer);
        buffer.add_buffer_line(rect, 7, "a界e\u{301}\n", 2);
        buffer.cache_interactions();
        assert_eq!(
            target_at(&mut buffer, 10),
            Some(MouseTarget::BufferPosition(Pos::new(7, 1)))
        );
        assert_eq!(
            target_at(&mut buffer, 11),
            Some(MouseTarget::BufferPosition(Pos::new(7, 2)))
        );
        assert_eq!(
            target_at(&mut buffer, 15),
            Some(MouseTarget::BufferPosition(Pos::new(7, 4)))
        );
    }
}
