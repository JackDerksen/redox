use minui::prelude::TabPolicy;
use minui::{Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{EditorMode, EditorState};
use crate::ui::UiStyle;
use crate::ui::widgets::popup::{
    PopupChrome, clip_text_to_cells, draw_popup_frame_at, popup_window_view,
};

const COMMAND_PROMPT: &str = "❯";
const COMMAND_TITLE: &str = "Command";
const COMMAND_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);

pub fn draw_command_line_popup(
    state: &EditorState,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    if state.mode != EditorMode::Command {
        return Ok(());
    }

    let (term_w, term_h) = window.get_size();
    let inner_w = command_line_inner_width(term_w, style);
    let inner_h = style.command_line.inner_height_rows.max(1);
    let popup_w = inner_w.saturating_add(2);
    let popup_h = inner_h.saturating_add(2);
    let x = term_w.saturating_sub(popup_w) / 2;
    let max_y = term_h.saturating_sub(popup_h);
    let y = style.command_line.top_margin_rows.min(max_y);

    let layout = draw_popup_frame_at(
        window,
        x,
        y,
        inner_w,
        inner_h,
        COMMAND_TITLE,
        PopupChrome {
            border: style.command_line.border,
            title: style.command_line.title,
            fill: style.command_line.text,
        },
    )?;
    let mut view = popup_window_view(window, layout);
    let row = inner_h / 2;
    let prompt_col = 1u16.min(inner_w.saturating_sub(1));
    view.write_str_colored(row, prompt_col, COMMAND_PROMPT, style.command_line.prompt)?;

    let input_col = prompt_col.saturating_add(command_text_width(COMMAND_PROMPT) as u16 + 1);
    let input_width = inner_w.saturating_sub(input_col);
    let clipped = clip_text_to_cells(&state.command_line, input_width as usize);
    view.write_str_colored(row, input_col, &clipped, style.command_line.text)?;

    window.request_cursor(minui::window::CursorSpec {
        x: layout
            .x
            .saturating_add(1)
            .saturating_add(input_col)
            .saturating_add(command_text_width(&clipped) as u16),
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
