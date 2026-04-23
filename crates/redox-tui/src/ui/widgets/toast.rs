use minui::{ColorPair, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{EditorMode, EditorState};
use crate::ui::UiStyle;
use crate::ui::widgets::perf::{PerfPopupLayout, perf_popup_layout};
use crate::ui::widgets::popup::{
    PopupChrome, PopupLayout, clip_text_to_cells, draw_popup_frame_at, popup_window_view,
    wrap_text_to_cells,
};

const TOAST_MARGIN_COLS: u16 = 1;
const TOAST_MARGIN_ROWS: u16 = 0;
const TOAST_HORIZONTAL_PADDING: u16 = 1;
const TOAST_VERTICAL_PADDING: u16 = 0;
const TOAST_MAX_TEXT_WIDTH: usize = 40;
const TOAST_GAP_ROWS: u16 = 0;
const TOAST_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);

pub fn draw_status_toast(
    state: &EditorState,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<Option<PopupLayout>> {
    let Some(message) = &state.status_msg else {
        return Ok(None);
    };

    let (term_w, term_h) = window.get_size();
    let perf_popup = status_toast_perf_popup_layout(state, term_w, term_h, style);
    let Some(toast) = toast_layout(message, term_w, term_h, perf_popup) else {
        return Ok(None);
    };

    let chrome = PopupChrome {
        border: style.command_line.border,
        title: style.command_line.title,
        fill: style.command_line.text,
    };
    let layout = draw_popup_frame_at(
        window,
        toast.x,
        toast.y,
        toast.inner_w,
        toast.inner_h,
        "",
        chrome,
    )?;
    let mut view = popup_window_view(window, layout);
    for (row, line) in toast.lines.iter().enumerate() {
        if row as u16 >= toast.inner_h {
            break;
        }
        write_toast_line(
            &mut view,
            row as u16 + TOAST_VERTICAL_PADDING,
            TOAST_HORIZONTAL_PADDING,
            line,
            style,
        )?;
    }
    Ok(Some(layout))
}

pub fn status_toast_occludes_cursor(layout: PopupLayout, x: u16, y: u16) -> bool {
    let popup_w = layout.inner_w.saturating_add(2);
    let popup_h = layout.inner_h.saturating_add(2);
    x >= layout.x
        && x < layout.x.saturating_add(popup_w)
        && y >= layout.y
        && y < layout.y.saturating_add(popup_h)
}

fn status_toast_perf_popup_layout(
    state: &EditorState,
    term_w: u16,
    term_h: u16,
    style: UiStyle,
) -> Option<PerfPopupLayout> {
    let perf_popup_visible = state.perf_popup().is_some()
        && !matches!(state.mode, EditorMode::Command | EditorMode::Search);
    perf_popup_visible.then(|| perf_popup_layout(term_w, term_h, style))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToastLayout {
    x: u16,
    y: u16,
    inner_w: u16,
    inner_h: u16,
    lines: Vec<String>,
}

fn toast_layout(
    message: &str,
    term_w: u16,
    term_h: u16,
    perf_popup: Option<PerfPopupLayout>,
) -> Option<ToastLayout> {
    if term_w <= 2 || term_h <= 2 {
        return None;
    }

    if let Some(perf) = perf_popup {
        if let Some(layout) = toast_layout_below_perf(message, term_w, term_h, perf) {
            return Some(layout);
        }
        if let Some(layout) = toast_layout_left_of_perf(message, term_h, perf) {
            return Some(layout);
        }
    }

    let y = TOAST_MARGIN_ROWS.min(term_h.saturating_sub(3));
    let popup_max_w = term_w.saturating_sub(TOAST_MARGIN_COLS);
    let popup_max_h = term_h.saturating_sub(y).saturating_sub(TOAST_MARGIN_ROWS);
    let (lines, inner_w, inner_h) = build_toast_body(message, popup_max_w, popup_max_h)?;
    let popup_w = inner_w.saturating_add(2);
    let x = term_w.saturating_sub(popup_w.saturating_add(TOAST_MARGIN_COLS));

    Some(ToastLayout {
        x,
        y,
        inner_w,
        inner_h,
        lines,
    })
}

fn toast_layout_below_perf(
    message: &str,
    term_w: u16,
    term_h: u16,
    perf_popup: PerfPopupLayout,
) -> Option<ToastLayout> {
    let y = perf_popup
        .y
        .saturating_add(perf_popup.inner_h)
        .saturating_add(2)
        .saturating_add(TOAST_GAP_ROWS);
    let popup_max_w = term_w.saturating_sub(TOAST_MARGIN_COLS);
    let popup_max_h = term_h.saturating_sub(y).saturating_sub(TOAST_MARGIN_ROWS);
    let (lines, inner_w, inner_h) = build_toast_body(message, popup_max_w, popup_max_h)?;
    let popup_w = inner_w.saturating_add(2);
    let x = term_w.saturating_sub(popup_w.saturating_add(TOAST_MARGIN_COLS));

    Some(ToastLayout {
        x,
        y,
        inner_w,
        inner_h,
        lines,
    })
}

fn toast_layout_left_of_perf(
    message: &str,
    term_h: u16,
    perf_popup: PerfPopupLayout,
) -> Option<ToastLayout> {
    let y = perf_popup.y;
    let popup_max_w = perf_popup.x.saturating_sub(TOAST_MARGIN_COLS);
    let popup_max_h = term_h.saturating_sub(y).saturating_sub(TOAST_MARGIN_ROWS);
    let (lines, inner_w, inner_h) = build_toast_body(message, popup_max_w, popup_max_h)?;
    let popup_w = inner_w.saturating_add(2);
    let x = perf_popup.x.saturating_sub(popup_w);

    Some(ToastLayout {
        x,
        y,
        inner_w,
        inner_h,
        lines,
    })
}

fn build_toast_body(
    message: &str,
    popup_max_w: u16,
    popup_max_h: u16,
) -> Option<(Vec<String>, u16, u16)> {
    if popup_max_w <= 2 {
        return None;
    }
    if popup_max_h <= 2 {
        return None;
    }
    let inner_max_w = popup_max_w.saturating_sub(2);
    let text_max_w = (inner_max_w
        .saturating_sub(TOAST_HORIZONTAL_PADDING.saturating_mul(2))
        .max(1) as usize)
        .min(TOAST_MAX_TEXT_WIDTH);

    let mut lines = wrap_text_to_cells(message, text_max_w);
    let inner_max_h = popup_max_h.saturating_sub(2);
    let text_max_h = inner_max_h
        .saturating_sub(TOAST_VERTICAL_PADDING.saturating_mul(2))
        .max(1) as usize;
    if lines.len() > text_max_h {
        lines.truncate(text_max_h);
        if let Some(last_line) = lines.last_mut() {
            *last_line = clip_text_to_cells(last_line, text_max_w.saturating_sub(1));
            last_line.push('…');
        }
    }

    let widest_line = lines
        .iter()
        .map(|line| text_cell_width(line))
        .max()
        .unwrap_or(0);
    let inner_w = widest_line as u16 + TOAST_HORIZONTAL_PADDING.saturating_mul(2);
    let inner_h = lines.len() as u16 + TOAST_VERTICAL_PADDING.saturating_mul(2);
    Some((lines, inner_w, inner_h))
}

fn write_toast_line(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    line: &str,
    style: UiStyle,
) -> minui::Result<()> {
    if line.trim() == "press y" {
        return window.write_str_colored(
            row,
            col,
            line,
            ColorPair::new(style.theme.dark_gray, style.command_line.text.bg),
        );
    }

    let mut cursor = 0;
    let mut cell_col = col;
    while cursor < line.len() {
        let ch = line[cursor..]
            .chars()
            .next()
            .expect("cursor should be on a character boundary");
        if is_toast_token_delimiter(ch) {
            let segment = ch.to_string();
            window.write_str_colored(row, cell_col, &segment, style.command_line.text)?;
            cell_col = cell_col.saturating_add(text_cell_width(&segment) as u16);
            cursor += ch.len_utf8();
            continue;
        }

        let token_end = line[cursor..]
            .char_indices()
            .find_map(|(idx, ch)| is_toast_token_delimiter(ch).then_some(cursor + idx))
            .unwrap_or(line.len());
        let token = &line[cursor..token_end];
        cell_col = write_toast_token(window, row, cell_col, token, style)?;
        cursor = token_end;
    }

    Ok(())
}

fn write_toast_token(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    token: &str,
    style: UiStyle,
) -> minui::Result<u16> {
    let mut cell_col = col;
    for (segment, is_dir) in toast_token_segments(token) {
        let color = if is_dir {
            style.explorer.directory
        } else {
            style.command_line.text
        };
        window.write_str_colored(row, cell_col, segment, color)?;
        cell_col = cell_col.saturating_add(text_cell_width(segment) as u16);
    }

    Ok(cell_col)
}

fn toast_token_segments(token: &str) -> Vec<(&str, bool)> {
    let mut segments = Vec::new();
    let mut cursor = 0;

    for (idx, ch) in token.char_indices() {
        if ch != '/' {
            continue;
        }

        let end = idx + ch.len_utf8();
        segments.push((&token[cursor..end], true));
        cursor = end;
    }

    if cursor < token.len() {
        segments.push((&token[cursor..], false));
    }

    segments
}

fn is_toast_token_delimiter(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, ',' | ';' | ':')
}

fn text_cell_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|g| (cell_width(g, TOAST_TAB_POLICY) as usize).max(1))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{toast_layout, toast_token_segments};
    use crate::ui::widgets::perf::PerfPopupLayout;

    #[test]
    fn toast_layout_anchors_to_top_right() {
        let layout = toast_layout("written", 40, 20, None).expect("toast layout should fit");
        assert_eq!(layout.y, 0);
        assert_eq!(layout.x + layout.inner_w + 2 + 1, 40);
    }

    #[test]
    fn toast_layout_prefers_height_for_long_messages() {
        let layout = toast_layout("abcdefghijklmnopqrstuvwxyz", 30, 20, None)
            .expect("toast layout should fit");
        assert_eq!(layout.inner_w, 27);
        assert!(layout.inner_h > 1);
        assert_eq!(layout.lines, vec!["abcdefghijklmnopqrstuvwxy", "z"]);
    }

    #[test]
    fn toast_layout_respects_perf_popup_offset() {
        let layout = toast_layout(
            "written",
            40,
            20,
            Some(PerfPopupLayout {
                x: 20,
                y: 0,
                inner_w: 18,
                inner_h: 6,
            }),
        )
        .expect("toast layout should fit");
        assert_eq!(layout.y, 8);
    }

    #[test]
    fn toast_layout_falls_back_to_left_of_perf_when_below_does_not_fit() {
        let layout = toast_layout(
            "written",
            40,
            9,
            Some(PerfPopupLayout {
                x: 20,
                y: 0,
                inner_w: 18,
                inner_h: 6,
            }),
        )
        .expect("toast layout should fit");
        assert_eq!(layout.y, 0);
        assert_eq!(layout.x + layout.inner_w + 2, 20);
    }

    #[test]
    fn toast_token_segments_mark_directory_prefixes() {
        assert_eq!(
            toast_token_segments("nested/file.txt"),
            vec![("nested/", true), ("file.txt", false)]
        );
        assert_eq!(
            toast_token_segments("one/two/three.txt"),
            vec![("one/", true), ("two/", true), ("three.txt", false)]
        );
        assert_eq!(toast_token_segments("nested/"), vec![("nested/", true)]);
    }
}
