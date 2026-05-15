use minui::{TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{EditorMode, EditorState};
use crate::ui::UiStyle;
use crate::ui::widgets::popup::{
    PopupChrome, anchored_popup_origin, clip_text_to_cells, draw_popup_frame_at, popup_window_view,
};

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
    let (title, prompt) = match state.mode {
        EditorMode::Command => (COMMAND_TITLE, COMMAND_PROMPT),
        EditorMode::Search => (SEARCH_TITLE, SEARCH_PROMPT),
        _ => return Ok(()),
    };

    let (term_w, term_h) = window.get_size();
    let inner_w = command_line_inner_width(term_w, style);
    let inner_h = style.command_line.inner_height_rows.max(1);
    let (x, y) = anchored_popup_origin(term_w, term_h, inner_w, inner_h);

    let layout = draw_popup_frame_at(
        window,
        x,
        y,
        inner_w,
        inner_h,
        title,
        PopupChrome {
            border: style.command_line.border,
            title: style.command_line.title,
            fill: style.command_line.text,
        },
    )?;
    let mut view = popup_window_view(window, layout);
    let row = inner_h / 2;
    let prompt_col = 1u16.min(inner_w.saturating_sub(1));
    view.write_str_colored(row, prompt_col, prompt, style.command_line.prompt)?;

    let input_col = prompt_col.saturating_add(command_text_width(prompt) as u16 + 1);
    let input_width = inner_w.saturating_sub(input_col);
    let (clipped, cursor_offset) = command_line_view(&state.command_line, input_width as usize);
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

    Ok(())
}

fn command_line_inner_width(term_w: u16, style: UiStyle) -> u16 {
    if term_w == 0 {
        return 0;
    }

    let popup_w = ((u32::from(term_w) * u32::from(style.command_line.width_percent)) / 100) as u16;
    let popup_w = popup_w
        .max(style.command_line.min_width.min(term_w))
        .min(if term_w > 2 { term_w - 2 } else { term_w });
    popup_w.saturating_sub(2)
}

fn command_text_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|g| (cell_width(g, COMMAND_TAB_POLICY) as usize).max(1))
        .sum()
}

fn command_line_view(text: &str, input_width: usize) -> (String, usize) {
    if input_width == 0 || text.is_empty() {
        return (String::new(), 0);
    }

    let text_width = command_text_width(text);
    if text_width <= input_width {
        let clipped = clip_text_to_cells(text, input_width);
        let cursor_offset = command_text_width(&clipped);
        return (clipped, cursor_offset);
    }

    let visible_width = input_width.saturating_sub(1).max(1);
    let clipped = clip_text_right_to_cells(text, visible_width);
    let cursor_offset = command_text_width(&clipped);
    (clipped, cursor_offset)
}

fn clip_text_right_to_cells(text: &str, max_cells: usize) -> String {
    if max_cells == 0 || text.is_empty() {
        return String::new();
    }

    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    let mut used = 0usize;
    let mut start = graphemes.len();
    while start > 0 {
        let g = graphemes[start - 1];
        let gw = (cell_width(g, COMMAND_TAB_POLICY) as usize).max(1);
        if used + gw > max_cells {
            break;
        }
        used += gw;
        start -= 1;
    }

    graphemes[start..].concat()
}

#[cfg(test)]
mod tests {
    use super::{clip_text_right_to_cells, command_line_view};

    #[test]
    fn command_line_view_keeps_short_input_unclipped() {
        let (visible, cursor_offset) = command_line_view("wq", 8);
        assert_eq!(visible, "wq");
        assert_eq!(cursor_offset, 2);
    }

    #[test]
    fn command_line_view_scrolls_long_input_left_and_keeps_right_padding() {
        let (visible, cursor_offset) = command_line_view("abcdefghijklmnopqrstuvwxyz", 8);
        assert_eq!(visible, "tuvwxyz");
        assert_eq!(cursor_offset, 7);
    }

    #[test]
    fn command_line_view_keeps_exact_fit_input_unclipped() {
        let (visible, cursor_offset) = command_line_view("exactfit", 8);
        assert_eq!(visible, "exactfit");
        assert_eq!(cursor_offset, 8);
    }

    #[test]
    fn clip_text_right_to_cells_keeps_trailing_graphemes() {
        assert_eq!(clip_text_right_to_cells("abcdef", 3), "def");
    }
}
