use minui::{TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{EditorMode, EditorState};
use crate::ui::icons::{PopupKind, popup_title};
use crate::ui::widgets::popup::{
    PopupChrome, PopupLayout, anchored_popup_origin, clip_text_to_cells, draw_popup_frame_at,
    popup_inner_size, popup_window_view,
};
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle};

const COMMAND_PROMPT: &str = "❯";
const COMMAND_TITLE: &str = "Command";
const SEARCH_PROMPT: &str = "/";
const SEARCH_TITLE: &str = "Search";
const COMMAND_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);

pub fn draw_command_line_popup(
    state: &EditorState,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let _ = draw_command_line_popup_after(state, style, window, None)?;
    Ok(())
}

pub fn draw_command_line_popup_below(
    state: &EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    popup: PopupLayout,
    stacked_padding: u16,
) -> minui::Result<bool> {
    draw_command_line_popup_after(state, style, window, Some((popup, stacked_padding)))
}

fn draw_command_line_popup_after(
    state: &EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    popup: Option<(PopupLayout, u16)>,
) -> minui::Result<bool> {
    let (title, prompt) = match state.mode {
        EditorMode::Command => (
            popup_title(PopupKind::Command, COMMAND_TITLE, style.icons_enabled),
            COMMAND_PROMPT,
        ),
        EditorMode::Search => (
            popup_title(PopupKind::Search, SEARCH_TITLE, style.icons_enabled),
            SEARCH_PROMPT,
        ),
        _ => return Ok(false),
    };

    let (term_w, term_h) = window.get_size();
    let inner_h = style.command_line.inner_height_rows.max(1);
    let (inner_w, _) = popup_inner_size(
        term_w,
        term_h,
        style.command_line.width_percent,
        100,
        style.command_line.min_width,
        inner_h.saturating_add(2),
    );
    let (x, anchored_y) = anchored_popup_origin(term_w, term_h, inner_w, inner_h);
    let y = if let Some((popup, stacked_padding)) = popup {
        let y = popup
            .y
            .saturating_add(popup.outer_h())
            .saturating_add(stacked_padding);
        let available_h = term_h.saturating_sub(STATUS_BAR_HEIGHT_CELLS);
        if y.saturating_add(inner_h.saturating_add(2)) > available_h {
            return Ok(false);
        }
        y
    } else {
        anchored_y
    };

    let layout = draw_popup_frame_at(
        window,
        x,
        y,
        inner_w,
        inner_h,
        &title,
        PopupChrome::command_line(style),
    )?;
    let mut view = popup_window_view(window, layout);
    let row = inner_h / 2;
    let prompt_col = 1u16.min(inner_w.saturating_sub(1));
    view.write_str_colored(row, prompt_col, prompt, style.command_line.prompt)?;

    let input_col = prompt_col.saturating_add(command_text_width(prompt) as u16 + 1);
    let input_width = inner_w.saturating_sub(input_col);
    let (clipped, cursor_offset) = command_line_view(
        &state.command_line,
        state.command_line_cursor,
        input_width as usize,
    );
    view.write_str_colored(row, input_col, &clipped, style.command_line.text)?;

    window.request_cursor(minui::window::CursorSpec {
        x: layout
            .x
            .saturating_add(1)
            .saturating_add(input_col)
            .saturating_add(cursor_offset as u16),
        y: layout.y.saturating_add(1).saturating_add(row),
        visible: true,
    });

    Ok(true)
}

fn command_text_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|g| (cell_width(g, COMMAND_TAB_POLICY) as usize).max(1))
        .sum()
}

fn command_line_view(text: &str, cursor: usize, input_width: usize) -> (String, usize) {
    if input_width == 0 || text.is_empty() {
        return (String::new(), 0);
    }

    let cursor = clamp_cursor(text, cursor);
    let text_width = command_text_width(text);
    if text_width <= input_width {
        let clipped = clip_text_to_cells(text, input_width);
        let cursor_offset = command_text_width(&text[..cursor]);
        return (clipped, cursor_offset);
    }

    let (left, right) = text.split_at(cursor);
    let left_width = command_text_width(left);
    if left_width <= input_width {
        let clipped = clip_text_to_cells(text, input_width);
        return (clipped, left_width);
    }

    let visible_left_width = input_width.saturating_sub(1).max(1);
    let visible_left = clip_text_right_to_cells(left, visible_left_width);
    let cursor_offset = command_text_width(&visible_left);
    let remaining_width = input_width.saturating_sub(cursor_offset);
    let visible_right = clip_text_to_cells(right, remaining_width);
    let clipped = format!("{visible_left}{visible_right}");
    (clipped, cursor_offset)
}

fn clamp_cursor(text: &str, mut cursor: usize) -> usize {
    cursor = cursor.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

fn clip_text_right_to_cells(text: &str, max_cells: usize) -> String {
    if max_cells == 0 || text.is_empty() {
        return String::new();
    }

    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    let mut used = 0usize;
    let mut start = graphemes.len();
    while start > 0 {
        let grapheme = graphemes[start - 1];
        let grapheme_width = (cell_width(grapheme, COMMAND_TAB_POLICY) as usize).max(1);
        if used + grapheme_width > max_cells {
            break;
        }
        used += grapheme_width;
        start -= 1;
    }

    graphemes[start..].concat()
}

#[cfg(test)]
mod tests {
    use super::{clip_text_right_to_cells, command_line_view};

    #[test]
    fn command_line_view_keeps_short_input_unclipped() {
        let (visible, cursor_offset) = command_line_view("wq", 2, 8);
        assert_eq!(visible, "wq");
        assert_eq!(cursor_offset, 2);
    }

    #[test]
    fn command_line_view_scrolls_long_input_left_and_keeps_right_padding() {
        let text = "abcdefghijklmnopqrstuvwxyz";
        let (visible, cursor_offset) = command_line_view(text, text.len(), 8);
        assert_eq!(visible, "tuvwxyz");
        assert_eq!(cursor_offset, 7);
    }

    #[test]
    fn command_line_view_keeps_exact_fit_input_unclipped() {
        let (visible, cursor_offset) = command_line_view("exactfit", 8, 8);
        assert_eq!(visible, "exactfit");
        assert_eq!(cursor_offset, 8);
    }

    #[test]
    fn clip_text_right_to_cells_keeps_trailing_graphemes() {
        assert_eq!(clip_text_right_to_cells("abcdef", 3), "def");
    }
}
