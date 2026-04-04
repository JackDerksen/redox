use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use redox_core::{BufferId, EditorSession};

use minui::input::Clipboard;
use minui::{ColorPair, Event, KeyKind, TerminalWindow, Window, prelude::*};
use unicode_segmentation::UnicodeSegmentation;

mod app;
mod input;
mod ui;

use app::EditorState;
use input::{InputAction, map_event_with_context};

use crate::ui::helpers::apply_color_column;
use ui::overlays::{
    active_delimiter_highlights, active_scope_indent_guides, draw_delimiter_highlights,
    draw_indent_guides,
};
use ui::syntax::{draw_line_with_syntax, syntax_color_for_range};
use ui::{
    STATUS_BAR_HEIGHT_CELLS, TextViewport, UiStyle, about_popup_inner_size,
    build_editor_status_bar, draw_about_popup_view, draw_command_line_popup,
    draw_explorer_popup_view, explorer_popup_inner_size, language_for_path,
    snapshot_lines_wrapped_cached,
};

const GUTTER_CONTENT_PADDING: u16 = 1;
const COLOR_COLUMN: usize = 79;

enum LaunchTarget {
    Empty,
    File(PathBuf),
    Explorer(PathBuf),
}

fn draw_buffer_view(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let popup_overlay_active = state.mode == app::EditorMode::Command
        || state.explorer_popup().is_some()
        || state.about_popup().is_some();
    let background_style = if popup_overlay_active {
        style.dimmed()
    } else {
        style
    };
    let editor_text = ColorPair::new(background_style.theme.white, background_style.theme.bg);
    fill_background(window, vw, vh, editor_text)?;
    let status_h: u16 = STATUS_BAR_HEIGHT_CELLS;
    let text_h = vh.saturating_sub(status_h);
    state.pump_active_loading(text_h as usize);

    if let Some(popup) = state.explorer_popup() {
        if let Some(background_id) = state.explorer_background_buffer_id()
            && !state.explorer_background_is_placeholder_blank()
        {
            draw_buffer_snapshot_for_id(
                state,
                background_style,
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
                background_style,
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
        hide_cursor(window);
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
    state.ensure_rain_animation(text_w, text_h, editor_text, background_style);

    if let Some(animation) = state.active_rain_animation() {
        draw_relative_line_numbers(
            window,
            background_style,
            gutter_w,
            text_h,
            animation.first_line(),
            active_cursor_line,
            total_lines,
        )?;
        draw_gutter_padding(
            window,
            background_style,
            gutter_w,
            text_h,
            GUTTER_CONTENT_PADDING,
        )?;
        animation.draw(window, 0, content_x, text_w as usize, text_h as usize)?;

        let status = build_editor_status_bar(state, style);
        status.draw(window)?;
        if state.mode == app::EditorMode::Command {
            draw_command_line_popup(state, style, window)?;
            return Ok(());
        }
        hide_cursor(window);
        return Ok(());
    }

    let visual_selection = state.active_visual_selection();
    let one_shot_highlight = state.one_shot_highlight();
    let syntax_language = language_for_path(state.session.active_meta().path.as_deref());
    let (snapshot, spec, scroll_x, syntax_spans, delimiter_highlights, active_scope_guides) = state
        .with_active_buffer_view_mut(|buffer, view| {
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
            let syntax_spans = view.syntax_highlighter.visible_line_spans(
                buffer,
                syntax_language,
                snapshot.first_line,
                snapshot.lines.len(),
            );
            let tree_sitter_scope = view.syntax_highlighter.active_scope_pair(
                buffer,
                syntax_language,
                view.cursor.cursor,
            );
            let delimiter_highlights = active_delimiter_highlights(
                buffer,
                view.cursor.cursor,
                snapshot.first_line,
                snapshot.lines.len(),
            );
            let active_scope_guides = active_scope_indent_guides(
                tree_sitter_scope,
                buffer,
                view.cursor.cursor,
                snapshot.first_line,
                snapshot.lines.len(),
                scroll_x,
                text_w as usize,
            );
            (
                snapshot,
                spec,
                scroll_x,
                syntax_spans,
                delimiter_highlights,
                active_scope_guides,
            )
        });

    draw_relative_line_numbers(
        window,
        background_style,
        gutter_w,
        text_h,
        snapshot.first_line,
        active_cursor_line,
        total_lines,
    )?;
    draw_gutter_padding(
        window,
        background_style,
        gutter_w,
        text_h,
        GUTTER_CONTENT_PADDING,
    )?;

    draw_snapshot_lines(
        window,
        state.session.active_buffer(),
        &snapshot,
        content_x,
        scroll_x,
        text_w as usize,
        editor_text,
        background_style,
        syntax_spans.as_deref(),
        &delimiter_highlights,
        &active_scope_guides,
        visual_selection,
        one_shot_highlight,
    )?;
    state.advance_one_shot_highlight();

    // --- Status bar (bottom row) ---
    let status = build_editor_status_bar(state, style);

    status.draw(window)?;

    if state.mode == app::EditorMode::Command {
        draw_command_line_popup(state, style, window)?;
    } else if spec.visible {
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
    selection_bg: Color,
    color_column: Option<(usize, Color)>,
    style: UiStyle,
    syntax_spans: Option<&[ui::syntax::LineSyntaxSpan]>,
    highlight_empty_line: bool,
) -> minui::Result<()> {
    if width_cells == 0 {
        return Ok(());
    }

    if source_line.is_empty() {
        if highlight_empty_line {
            window.write_str_colored(
                row,
                col,
                " ",
                ColorPair::new(normal_color.fg, selection_bg),
            )?;
            if let Some((visible_col, bg)) = color_column
                && visible_col < width_cells
                && visible_col != 0
            {
                window.write_str_colored(
                    row,
                    col.saturating_add(visible_col as u16),
                    " ",
                    ColorPair::new(normal_color.fg, bg),
                )?;
            }
        } else if let Some((visible_col, bg)) = color_column
            && visible_col < width_cells
        {
            window.write_str_colored(
                row,
                col.saturating_add(visible_col as u16),
                " ",
                ColorPair::new(normal_color.fg, bg),
            )?;
        }
        return Ok(());
    }

    let mut used_cells = 0usize;
    let mut line_cells = 0usize;
    let mut char_idx = 0usize;
    let mut byte_idx = 0usize;

    for g in source_line.graphemes(true) {
        let g_width = minui::cell_width(g, minui::prelude::TabPolicy::Fixed(4)) as usize;
        let g_chars = g.chars().count();
        let g_bytes = g.len();
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width);
        let start_char = char_idx;
        let end_char = char_idx.saturating_add(g_chars);
        let start_byte = byte_idx;
        let end_byte = byte_idx.saturating_add(g_bytes);

        line_cells = end_cell;
        char_idx = end_char;
        byte_idx = end_byte;

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
        let base_color = syntax_spans
            .map(|spans| syntax_color_for_range(normal_color, style, spans, start_byte, end_byte))
            .unwrap_or(normal_color);
        let color = if is_selected {
            ColorPair::new(base_color.fg, selection_bg)
        } else {
            apply_color_column(base_color, color_column, start_cell, end_cell)
        };

        if g == "\t" {
            let spaces = " ".repeat(g_width.max(1));
            window.write_str_colored(row, col.saturating_add(used_cells as u16), &spaces, color)?;
        } else {
            window.write_str_colored(row, col.saturating_add(used_cells as u16), g, color)?;
        }
        used_cells = used_cells.saturating_add(g_width);
    }

    if let Some((visible_col, bg)) = color_column
        && visible_col < width_cells
        && visible_col >= used_cells
    {
        window.write_str_colored(
            row,
            col.saturating_add(visible_col as u16),
            " ",
            ColorPair::new(normal_color.fg, bg),
        )?;
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

fn hide_cursor(window: &mut dyn Window) {
    window.request_cursor(minui::window::CursorSpec {
        x: 0,
        y: 0,
        visible: false,
    });
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
    let syntax_language = state
        .session
        .meta(buffer_id)
        .and_then(|meta| language_for_path(meta.path.as_deref()));
    let Some((
        snapshot,
        cursor_line,
        total_lines,
        scroll_x,
        syntax_spans,
        delimiter_highlights,
        active_scope_guides,
    )) = state.with_buffer_view_mut(buffer_id, |buffer, view| {
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
        let snapshot = snapshot_lines_wrapped_cached(buffer, &viewport, &mut view.grapheme_cache);
        let syntax_spans = view.syntax_highlighter.visible_line_spans(
            buffer,
            syntax_language,
            snapshot.first_line,
            snapshot.lines.len(),
        );
        let tree_sitter_scope =
            view.syntax_highlighter
                .active_scope_pair(buffer, syntax_language, view.cursor.cursor);
        let delimiter_highlights = active_delimiter_highlights(
            buffer,
            view.cursor.cursor,
            snapshot.first_line,
            snapshot.lines.len(),
        );
        let active_scope_guides = active_scope_indent_guides(
            tree_sitter_scope,
            buffer,
            view.cursor.cursor,
            snapshot.first_line,
            snapshot.lines.len(),
            scroll_x,
            width.saturating_sub(content_x) as usize,
        );
        (
            snapshot,
            view.cursor.cursor.line,
            total_lines,
            scroll_x,
            syntax_spans,
            delimiter_highlights,
            active_scope_guides,
        )
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

    let buffer = state
        .session
        .buffer(buffer_id)
        .expect("snapshot buffer must exist in session map");
    draw_snapshot_lines(
        window,
        buffer,
        &snapshot,
        content_x,
        scroll_x,
        width.saturating_sub(content_x) as usize,
        colors,
        style,
        syntax_spans.as_deref(),
        &delimiter_highlights,
        &active_scope_guides,
        None,
        None,
    )?;

    Ok(())
}

fn draw_snapshot_lines(
    window: &mut dyn Window,
    buffer: &redox_core::TextBuffer,
    snapshot: &ui::render::RenderSnapshot,
    content_x: u16,
    scroll_x: usize,
    text_w: usize,
    default_colors: ColorPair,
    style: UiStyle,
    syntax_spans: Option<&[Vec<ui::syntax::LineSyntaxSpan>]>,
    delimiter_highlights: &BTreeMap<usize, Vec<usize>>,
    active_scope_guides: &BTreeMap<usize, Vec<usize>>,
    visual_selection: Option<(redox_core::Selection, redox_core::VisualModeKind)>,
    one_shot_highlight: Option<(redox_core::Selection, redox_core::VisualModeKind)>,
) -> minui::Result<()> {
    let color_column = visible_color_column(scroll_x, text_w, style.theme.color_column);
    for row in 0..snapshot.lines.len() {
        let line_idx = snapshot.first_line + row;
        let source_line = buffer.line_string(line_idx);
        let highlighted_chars = delimiter_highlights
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let visible_indent_guides = active_scope_guides
            .get(&line_idx)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let transient_selection = visual_selection
            .map(|(selection, mode)| (selection, mode, style.theme.selection_bg))
            .or_else(|| {
                one_shot_highlight
                    .map(|(selection, mode)| (selection, mode, style.theme.light_purple))
            });
        if let Some((selection, mode, selection_bg)) = transient_selection {
            let highlight_empty_line = buffer.line_len_chars(line_idx) == 0
                && selected_empty_line(selection, mode, line_idx);
            if let Some(sel_range) =
                buffer.visual_selection_char_range_on_line(selection, mode, line_idx)
            {
                let selected_cells = selected_visible_cells(
                    &source_line,
                    scroll_x,
                    text_w as usize,
                    sel_range.start,
                    sel_range.end,
                );
                draw_line_with_selection(
                    window,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    text_w,
                    sel_range.start,
                    sel_range.end,
                    default_colors,
                    selection_bg,
                    color_column,
                    style,
                    syntax_spans.and_then(|rows| rows.get(row).map(Vec::as_slice)),
                    highlight_empty_line,
                )?;
                draw_indent_guides(
                    window,
                    row as u16,
                    content_x,
                    visible_indent_guides,
                    style,
                    Some((&selected_cells, selection_bg)),
                )?;
                draw_delimiter_highlights(
                    window,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    text_w,
                    highlighted_chars,
                    style,
                )?;
                continue;
            }
            if highlight_empty_line {
                draw_line_with_selection(
                    window,
                    row as u16,
                    content_x,
                    "",
                    scroll_x,
                    text_w,
                    0,
                    0,
                    default_colors,
                    selection_bg,
                    color_column,
                    style,
                    syntax_spans.and_then(|rows| rows.get(row).map(Vec::as_slice)),
                    true,
                )?;
                continue;
            }
        }

        if let Some(spans) = syntax_spans.and_then(|rows| rows.get(row))
            && !spans.is_empty()
        {
            draw_line_with_syntax(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                default_colors,
                color_column,
                style,
                spans,
            )?;
            draw_indent_guides(
                window,
                row as u16,
                content_x,
                visible_indent_guides,
                style,
                None,
            )?;
            draw_delimiter_highlights(
                window,
                row as u16,
                content_x,
                &source_line,
                scroll_x,
                text_w,
                highlighted_chars,
                style,
            )?;
            continue;
        }

        draw_plain_line(
            window,
            row as u16,
            content_x,
            &source_line,
            scroll_x,
            text_w,
            default_colors,
            color_column,
        )?;
        draw_indent_guides(
            window,
            row as u16,
            content_x,
            visible_indent_guides,
            style,
            None,
        )?;
        draw_delimiter_highlights(
            window,
            row as u16,
            content_x,
            &source_line,
            scroll_x,
            text_w,
            highlighted_chars,
            style,
        )?;
    }

    Ok(())
}

fn selected_visible_cells(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    sel_start_char: usize,
    sel_end_char_exclusive: usize,
) -> Vec<bool> {
    let mut selected = vec![false; width_cells];
    if width_cells == 0 || source_line.is_empty() || sel_start_char >= sel_end_char_exclusive {
        return selected;
    }

    let mut line_cells = 0usize;
    let mut char_idx = 0usize;
    let max_visible_cell = scroll_x.saturating_add(width_cells);

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
        if start_cell >= max_visible_cell {
            break;
        }
        if !(start_char < sel_end_char_exclusive && end_char > sel_start_char) {
            continue;
        }

        let visible_start = start_cell.max(scroll_x).saturating_sub(scroll_x);
        let visible_end = end_cell.min(max_visible_cell).saturating_sub(scroll_x);
        for cell in visible_start..visible_end {
            selected[cell] = true;
        }
    }

    selected
}

fn selected_empty_line(
    selection: redox_core::Selection,
    mode: redox_core::VisualModeKind,
    line_idx: usize,
) -> bool {
    matches!(mode, redox_core::VisualModeKind::Line) && {
        let (start, end) = selection.ordered();
        line_idx >= start.line && line_idx <= end.line
    }
}

fn draw_plain_line(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    default_colors: ColorPair,
    color_column: Option<(usize, Color)>,
) -> minui::Result<()> {
    if width_cells == 0 {
        return Ok(());
    }

    let mut used_cells = 0usize;
    let mut line_cells = 0usize;

    for g in source_line.graphemes(true) {
        let g_width = minui::cell_width(g, minui::prelude::TabPolicy::Fixed(4)) as usize;
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width);
        line_cells = end_cell;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell < scroll_x {
            continue;
        }
        if used_cells.saturating_add(g_width) > width_cells {
            break;
        }

        let colors = apply_color_column(default_colors, color_column, start_cell, end_cell);
        if g == "\t" {
            let spaces = " ".repeat(g_width.max(1));
            window.write_str_colored(
                row,
                col.saturating_add(used_cells as u16),
                &spaces,
                colors,
            )?;
        } else {
            window.write_str_colored(row, col.saturating_add(used_cells as u16), g, colors)?;
        }
        used_cells = used_cells.saturating_add(g_width);
    }

    if let Some((visible_col, bg)) = color_column
        && visible_col < width_cells
        && visible_col >= used_cells
    {
        window.write_str_colored(
            row,
            col.saturating_add(visible_col as u16),
            " ",
            ColorPair::new(default_colors.fg, bg),
        )?;
    }

    Ok(())
}

fn visible_color_column(scroll_x: usize, text_w: usize, bg: Color) -> Option<(usize, Color)> {
    if COLOR_COLUMN < scroll_x {
        return None;
    }
    let visible_col = COLOR_COLUMN - scroll_x;
    (visible_col < text_w).then_some((visible_col, bg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minui::{ColorPair, Window};

    struct TestWindow {
        width: u16,
        height: u16,
        cells: Vec<Vec<char>>,
    }

    impl TestWindow {
        fn new(width: u16, height: u16) -> Self {
            Self {
                width,
                height,
                cells: vec![vec![' '; width as usize]; height as usize],
            }
        }

        fn row_text(&self, row: u16) -> String {
            self.cells[row as usize].iter().collect()
        }

        fn write_text(&mut self, y: u16, x: u16, s: &str) {
            if y >= self.height {
                return;
            }

            let row = &mut self.cells[y as usize];
            for (idx, ch) in s.chars().enumerate() {
                let cell_x = x as usize + idx;
                if cell_x >= row.len() {
                    break;
                }
                row[cell_x] = ch;
            }
        }
    }

    impl Window for TestWindow {
        fn write_str(&mut self, y: u16, x: u16, s: &str) -> minui::Result<()> {
            self.write_text(y, x, s);
            Ok(())
        }

        fn write_str_colored(
            &mut self,
            y: u16,
            x: u16,
            s: &str,
            _colors: ColorPair,
        ) -> minui::Result<()> {
            self.write_text(y, x, s);
            Ok(())
        }

        fn flush(&mut self) -> minui::Result<()> {
            Ok(())
        }

        fn set_cursor_position(&mut self, _x: u16, _y: u16) -> minui::Result<()> {
            Ok(())
        }

        fn show_cursor(&mut self, _show: bool) -> minui::Result<()> {
            Ok(())
        }

        fn get_size(&self) -> (u16, u16) {
            (self.width, self.height)
        }

        fn clear_screen(&mut self) -> minui::Result<()> {
            for row in &mut self.cells {
                row.fill(' ');
            }
            Ok(())
        }

        fn clear_line(&mut self, y: u16) -> minui::Result<()> {
            if let Some(row) = self.cells.get_mut(y as usize) {
                row.fill(' ');
            }
            Ok(())
        }

        fn clear_area(&mut self, y1: u16, x1: u16, y2: u16, x2: u16) -> minui::Result<()> {
            let row_start = usize::from(y1.min(y2));
            let row_end = usize::from(y1.max(y2)).min(self.cells.len().saturating_sub(1));
            let col_start = usize::from(x1.min(x2));
            let col_end = usize::from(x1.max(x2)).min(self.width.saturating_sub(1) as usize);

            for row in row_start..=row_end {
                for col in col_start..=col_end {
                    self.cells[row][col] = ' ';
                }
            }

            Ok(())
        }
    }

    #[test]
    fn draw_snapshot_lines_renders_scrolled_plain_text_without_double_scroll() {
        let buffer = redox_core::TextBuffer::from_str("abcdefghijklmnopqrstuvwxyz\n");
        let snapshot = ui::render::RenderSnapshot::new(0, vec!["ijklmnop".to_string()]);
        let mut window = TestWindow::new(8, 1);
        let style = UiStyle::default();
        let default_colors = ColorPair::new(style.theme.white, style.theme.bg);

        draw_snapshot_lines(
            &mut window,
            &buffer,
            &snapshot,
            0,
            8,
            8,
            default_colors,
            style,
            None,
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            None,
        )
        .expect("plain snapshot draw should succeed");

        assert_eq!(window.row_text(0), "ijklmnop");
    }
}

fn parse_path_arg() -> anyhow::Result<LaunchTarget> {
    let mut args = env::args().skip(1);
    let Some(raw) = args.next() else {
        return Ok(LaunchTarget::Empty);
    };
    let path = PathBuf::from(&raw);
    if path.is_dir() {
        return Ok(LaunchTarget::Explorer(path));
    }
    Ok(LaunchTarget::File(path))
}

fn is_cancel_event(event: &Event) -> bool {
    matches!(event, Event::Escape)
        || matches!(
            event,
            Event::KeyWithModifiers(key)
                if matches!(key.key, KeyKind::Escape)
                    && !key.mods.ctrl
                    && !key.mods.alt
                    && !key.mods.super_key
        )
        || matches!(
            event,
            Event::KeyWithModifiers(key)
                if key.mods.ctrl
                    && !key.mods.alt
                    && !key.mods.super_key
                    && matches!(key.key, KeyKind::Char('c') | KeyKind::Char('C'))
        )
}

fn handle_editor_event(
    state: &mut EditorState,
    clipboard: &mut Option<Clipboard>,
    event: Event,
) -> bool {
    if state.rain_is_active() {
        if is_cancel_event(&event) {
            state.stop_rain_animation();
        }
        return !state.should_quit;
    }

    if is_cancel_event(&event) && state.handle_normal_mode_escape_on_surface() {
        return !state.should_quit;
    }

    let confirm_explorer_delete = state.has_pending_explorer_delete_confirmation();
    let action = match &event {
        Event::Paste(text) => InputAction::Paste(text.clone()),
        _ => map_event_with_context(
            &mut state.input,
            state.mode.as_input_mode(),
            confirm_explorer_delete,
            &event,
        ),
    };

    let (w, h) = state.viewport_size();
    match action {
        InputAction::PasteSystemClipboard => match clipboard.as_mut() {
            Some(system_clipboard) => match system_clipboard.paste() {
                Ok(text) => state.apply_input(InputAction::PasteSystemClipboardText(text), w, h),
                Err(e) => state.set_status(format!("clipboard paste failed: {e}")),
            },
            None => state.set_status("system clipboard unavailable"),
        },
        action => state.apply_input(action, w, h),
    }
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
}

pub fn run() -> minui::Result<()> {
    let launch = parse_path_arg().expect("failed to parse launch target");
    let launch_empty = matches!(&launch, LaunchTarget::Empty);
    let launch_explorer_dir = match &launch {
        LaunchTarget::Explorer(dir) => Some(dir.clone()),
        LaunchTarget::Empty | LaunchTarget::File(_) => None,
    };
    let session = match launch {
        LaunchTarget::Empty => {
            EditorSession::open_initial_unnamed().expect("failed to open unnamed session")
        }
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
    if launch_empty {
        state.command_open_about();
    }

    let mut window = TerminalWindow::new()?;
    window.set_auto_flush(false);
    let mut clipboard = Clipboard::new().ok();
    let style = UiStyle::default();

    const MAX_EVENTS_PER_FRAME: usize = 256;
    const ACTIVE_FRAME_BUDGET: Duration = Duration::from_millis(16);
    const IDLE_FRAME_BUDGET: Duration = Duration::from_millis(20);

    loop {
        let frame_start = Instant::now();

        for _ in 0..MAX_EVENTS_PER_FRAME {
            match window.poll_input()? {
                Some(event) => {
                    if !handle_editor_event(&mut state, &mut clipboard, event) {
                        return Ok(());
                    }
                }
                None => break,
            }
        }

        if state.rain_is_active() {
            state.advance_rain_animation();
        }

        window.clear_cursor_request();
        let (w, h) = window.get_size();
        state.set_viewport_size(w as usize, h as usize);
        window.clear_screen()?;
        draw_buffer_view(&mut state, style, &mut window)?;
        window.end_frame()?;

        if state.should_quit {
            return Ok(());
        }

        let frame_budget = if state.rain_is_active() {
            ACTIVE_FRAME_BUDGET
        } else {
            IDLE_FRAME_BUDGET
        };
        let remaining = frame_budget.saturating_sub(frame_start.elapsed());
        if !remaining.is_zero() {
            thread::sleep(remaining);
        }
    }
}
