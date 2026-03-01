use std::env;
use std::path::PathBuf;

use redox_core::{BufferId, EditorSession, Selection};

use minui::{ColorPair, Window, prelude::*};
use unicode_segmentation::UnicodeSegmentation;

mod app;
mod input;
mod ui;

use app::EditorState;
use input::{InputAction, map_event_with_state};

use ui::{
    STATUS_BAR_HEIGHT_CELLS, TextViewport, UiStyle, about_popup_inner_size,
    build_editor_status_bar, draw_about_popup_view, draw_explorer_popup_view,
    explorer_popup_inner_size, snapshot_lines_wrapped_cached,
};

const GUTTER_CONTENT_PADDING: u16 = 1;

enum LaunchTarget {
    File(PathBuf),
    Explorer(PathBuf),
}

fn draw_buffer_view(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let editor_text = ColorPair::new(style.theme.white, style.theme.bg);
    fill_background(window, vw, vh, editor_text)?;
    let status_h: u16 = STATUS_BAR_HEIGHT_CELLS;
    let text_h = vh.saturating_sub(status_h);
    state.pump_active_loading(text_h as usize);

    if let Some(popup) = state.explorer_popup() {
        if let Some(background_id) = state.explorer_background_buffer_id() {
            draw_buffer_snapshot_for_id(
                state,
                style,
                background_id,
                vw,
                text_h,
                editor_text,
                window,
            )?;
        }
        let (inner_w, inner_h) = explorer_popup_inner_size(vw, vh, style);
        state.set_viewport_size(
            inner_w as usize,
            inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
        );
        draw_explorer_popup_view(state, style, window, popup)?;
        return Ok(());
    }

    if let Some(popup) = state.about_popup() {
        if let Some(background_id) = state.about_background_buffer_id() {
            draw_buffer_snapshot_for_id(
                state,
                style,
                background_id,
                vw,
                text_h,
                editor_text,
                window,
            )?;
        }
        let (inner_w, inner_h) = about_popup_inner_size(vw, vh, style);
        state.set_viewport_size(
            inner_w as usize,
            inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
        );
        draw_about_popup_view(state, style, window, popup)?;
        return Ok(());
    }

    let active_cursor_line = state.active_cursor_pos().line;
    let total_lines = state.session.active_buffer().len_lines().max(1);
    let gutter_w = line_number_gutter_width(total_lines);
    let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    let text_w = vw.saturating_sub(content_x);
    state.set_viewport_size(
        text_w as usize,
        text_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
    );

    let visual_selection = state.active_visual_selection();
    let (snapshot, spec, scroll_x, source_lines) =
        state.with_active_buffer_view_mut(|buffer, view| {
            let (scroll_x, scroll_y) = view.cursor.viewport_scroll();
            let viewport = TextViewport {
                scroll_x,
                scroll_y,
                width: text_w,
                height: text_h,
            };
            let snapshot =
                snapshot_lines_wrapped_cached(buffer, &viewport, &mut view.grapheme_cache);
            let spec = view
                .cursor
                .cursor_spec(buffer, text_w as usize, text_h as usize);
            let source_lines = (0..snapshot.lines.len())
                .map(|row| {
                    let line_idx = snapshot.first_line + row;
                    buffer.line_string(line_idx)
                })
                .collect::<Vec<_>>();
            (snapshot, spec, scroll_x, source_lines)
        });

    draw_relative_line_numbers(
        window,
        style,
        gutter_w,
        text_h,
        snapshot.first_line,
        active_cursor_line,
        total_lines,
    )?;
    draw_gutter_padding(window, style, gutter_w, text_h, GUTTER_CONTENT_PADDING)?;

    for (row, line) in snapshot.lines.iter().enumerate() {
        let line_idx = snapshot.first_line + row;
        let source_line = source_lines.get(row).map(String::as_str).unwrap_or("");
        if let Some((selection, line_mode)) = visual_selection {
            if let Some((sel_start, sel_end)) =
                visual_range_for_line(selection, line_mode, line_idx, source_line)
            {
                draw_line_with_selection(
                    window,
                    row as u16,
                    content_x,
                    source_line,
                    scroll_x,
                    text_w as usize,
                    sel_start,
                    sel_end,
                    editor_text,
                    ColorPair::new(style.theme.selection_fg, style.theme.selection_bg),
                )?;
                continue;
            }
        }

        window.write_str_colored(row as u16, content_x, line, editor_text)?;
    }

    // --- Status bar (bottom row) ---
    let status = build_editor_status_bar(state, style);

    status.draw(window)?;

    if spec.visible {
        window.request_cursor(minui::window::CursorSpec {
            x: spec.x.saturating_add(content_x),
            y: spec.y,
            visible: true,
        });
    }

    Ok(())
}

fn draw_gutter_padding(
    window: &mut dyn Window,
    style: UiStyle,
    gutter_w: u16,
    text_h: u16,
    padding_w: u16,
) -> minui::Result<()> {
    if padding_w == 0 || text_h == 0 {
        return Ok(());
    }

    let pad = " ".repeat(padding_w as usize);
    let color = ColorPair::new(style.theme.bg, style.theme.bg);
    for row in 0..text_h {
        window.write_str_colored(row, gutter_w, &pad, color)?;
    }
    Ok(())
}

fn line_number_gutter_width(total_lines: usize) -> u16 {
    let digits = total_lines.max(1).ilog10() as u16 + 1;
    // digits + separator column
    digits.saturating_add(1)
}

fn draw_relative_line_numbers(
    window: &mut dyn Window,
    style: UiStyle,
    gutter_w: u16,
    text_h: u16,
    first_line: usize,
    cursor_line: usize,
    total_lines: usize,
) -> minui::Result<()> {
    if gutter_w == 0 || text_h == 0 {
        return Ok(());
    }

    let sep_x = gutter_w.saturating_sub(1);
    let number_w = gutter_w.saturating_sub(1) as usize;
    let relative_color = ColorPair::new(style.theme.dark_gray, style.theme.bg);
    let current_color = ColorPair::new(style.theme.white, style.theme.bg);

    for row in 0..text_h {
        let line_idx = first_line.saturating_add(row as usize);
        if line_idx >= total_lines {
            continue;
        }

        let num = if line_idx == cursor_line {
            (line_idx + 1).to_string()
        } else {
            line_idx.abs_diff(cursor_line).to_string()
        };

        let clipped_num = if num.chars().count() > number_w {
            num.chars()
                .rev()
                .take(number_w)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        } else {
            num
        };

        let text = format!("{clipped_num:>number_w$}");

        let color = if line_idx == cursor_line {
            current_color
        } else {
            relative_color
        };

        if number_w > 0 {
            window.write_str_colored(row, 0, &text, color)?;
        }

        window.write_str_colored(row, sep_x, "▕", color)?;
    }

    Ok(())
}

fn visual_range_for_line(
    selection: Selection,
    line_mode: bool,
    line_idx: usize,
    source_line: &str,
) -> Option<(usize, usize)> {
    let line_len = source_line.chars().count();
    if line_mode {
        let start_line = selection.anchor.line.min(selection.cursor.line);
        let end_line = selection.anchor.line.max(selection.cursor.line);
        if line_idx < start_line || line_idx > end_line {
            return None;
        }
        return Some((0, line_len));
    }

    let (start, end) = selection.ordered();
    if line_idx < start.line || line_idx > end.line {
        return None;
    }
    if line_len == 0 {
        return None;
    }

    let max_char = line_len.saturating_sub(1);
    let sel_start = if line_idx == start.line {
        start.col.min(max_char)
    } else {
        0
    };
    let sel_end_inclusive = if line_idx == end.line {
        end.col.min(max_char)
    } else {
        max_char
    };
    if sel_start > sel_end_inclusive {
        return None;
    }

    Some((sel_start, sel_end_inclusive.saturating_add(1)))
}

fn draw_line_with_selection(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    sel_start_char: usize,
    sel_end_char_exclusive: usize,
    normal_color: ColorPair,
    selected_color: ColorPair,
) -> minui::Result<()> {
    if width_cells == 0 {
        return Ok(());
    }

    let mut used_cells = 0usize;
    let mut line_cells = 0usize;
    let mut char_idx = 0usize;

    for g in source_line.graphemes(true) {
        let g_width = minui::cell_width(g, minui::prelude::TabPolicy::Fixed(4)) as usize;
        let g_chars = g.chars().count();
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width);
        let start_char = char_idx;
        let end_char = char_idx.saturating_add(g_chars);

        line_cells = end_cell;
        char_idx = end_char;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell < scroll_x {
            continue;
        }

        if used_cells.saturating_add(g_width) > width_cells {
            break;
        }

        let is_selected = start_char < sel_end_char_exclusive && end_char > sel_start_char;
        let color = if is_selected {
            selected_color
        } else {
            normal_color
        };

        if g == "\t" {
            let spaces = " ".repeat(g_width.max(1));
            window.write_str_colored(row, col.saturating_add(used_cells as u16), &spaces, color)?;
        } else {
            window.write_str_colored(row, col.saturating_add(used_cells as u16), g, color)?;
        }
        used_cells = used_cells.saturating_add(g_width);
    }

    Ok(())
}

fn fill_background(
    window: &mut dyn Window,
    width: u16,
    height: u16,
    colors: ColorPair,
) -> minui::Result<()> {
    if width == 0 || height == 0 {
        return Ok(());
    }

    let row = " ".repeat(width as usize);
    for y in 0..height {
        window.write_str_colored(y, 0, &row, colors)?;
    }
    Ok(())
}

fn draw_buffer_snapshot_for_id(
    state: &mut EditorState,
    style: UiStyle,
    buffer_id: BufferId,
    width: u16,
    height: u16,
    colors: ColorPair,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let Some((snapshot, cursor_line, total_lines)) =
        state.with_buffer_view_mut(buffer_id, |buffer, view| {
            let total_lines = buffer.len_lines().max(1);
            let gutter_w = line_number_gutter_width(total_lines);
            let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
            let text_w = width.saturating_sub(content_x);
            let (scroll_x, scroll_y) = view.cursor.viewport_scroll();
            let viewport = TextViewport {
                scroll_x,
                scroll_y,
                width: text_w,
                height,
            };
            let snapshot =
                snapshot_lines_wrapped_cached(buffer, &viewport, &mut view.grapheme_cache);
            (snapshot, view.cursor.cursor.line, total_lines)
        })
    else {
        return Ok(());
    };

    let gutter_w = line_number_gutter_width(total_lines);
    let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    draw_relative_line_numbers(
        window,
        style,
        gutter_w,
        height,
        snapshot.first_line,
        cursor_line,
        total_lines,
    )?;
    draw_gutter_padding(window, style, gutter_w, height, GUTTER_CONTENT_PADDING)?;

    for (row, line) in snapshot.lines.iter().enumerate() {
        window.write_str_colored(row as u16, content_x, line, colors)?;
    }

    Ok(())
}

fn parse_path_arg() -> anyhow::Result<LaunchTarget> {
    let mut args = env::args().skip(1);
    let raw = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("expected a file path argument"))?;
    let path = PathBuf::from(&raw);
    if path.is_dir() {
        return Ok(LaunchTarget::Explorer(path));
    }
    Ok(LaunchTarget::File(path))
}

pub fn run() -> minui::Result<()> {
    let launch =
        parse_path_arg().expect("file path required (e.g. redox ./file.txt, redox ., redox src)");
    let launch_explorer_dir = match &launch {
        LaunchTarget::Explorer(dir) => Some(dir.clone()),
        LaunchTarget::File(_) => None,
    };
    let session = match launch {
        LaunchTarget::File(path) => {
            EditorSession::open_initial_file(path).expect("failed to open initial file")
        }
        LaunchTarget::Explorer(_) => {
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session")
        }
    };

    let mut state = EditorState::new(session);
    if let Some(dir_path) = launch_explorer_dir {
        state
            .open_explorer_at_path(dir_path)
            .expect("failed to open explorer directory");
    }

    let mut app = App::new(state)?;
    let mut clipboard = minui::input::Clipboard::new().ok();
    let style = UiStyle::default();

    app.run(
        |state, event| {
            let action = match &event {
                Event::Paste(text) => InputAction::Paste(text.clone()),
                _ => map_event_with_state(&mut state.input, state.mode.as_input_mode(), &event),
            };

            let (w, h) = state.viewport_size();
            state.apply_input(action, w, h);
            if let Some(text) = state.take_pending_system_clipboard() {
                match clipboard.as_mut() {
                    Some(system_clipboard) => {
                        if let Err(e) = system_clipboard.copy(&text) {
                            state.set_status(format!("clipboard copy failed: {e}"));
                        } else {
                            state.set_status("yanked to system clipboard");
                        }
                    }
                    None => {
                        state.set_status("system clipboard unavailable");
                    }
                }
            }
            !state.should_quit
        },
        |state, window| {
            window.clear_cursor_request();
            let (w, h) = window.get_size();
            state.set_viewport_size(w as usize, h as usize);

            draw_buffer_view(state, style, window)?;

            window.end_frame()?;
            Ok(())
        },
    )?;

    Ok(())
}
