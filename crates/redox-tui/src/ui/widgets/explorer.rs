use std::fs;
use std::path::{Path, PathBuf};

use minui::widgets::{Widget, WindowView};
use minui::{cell_width, ColorPair, TabPolicy, Window};
use redox_core::TextBuffer;
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{EditorState, ExplorerPopup};
use crate::ui::{build_editor_status_bar, snapshot_lines_wrapped_cached, TextViewport, UiStyle};

const GUTTER_CONTENT_PADDING: u16 = 1;

pub fn draw_explorer_popup_view(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    popup: ExplorerPopup,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let (inner_w, inner_h) = explorer_popup_inner_size(vw, vh, style);
    let popup_w = inner_w.saturating_add(2);
    let popup_h = inner_h.saturating_add(2);
    let x = (vw.saturating_sub(popup_w)) / 2;
    let y = (vh.saturating_sub(popup_h)) / 2;
    let border_color = style.explorer.border;
    let title_color = style.explorer.title;

    let horizontal = "─".repeat(popup_w.saturating_sub(2) as usize);
    window.write_str_colored(y, x, &format!("╭{}╮", horizontal), border_color)?;
    if popup_h > 1 {
        for row in (y + 1)..(y + popup_h.saturating_sub(1)) {
            window.write_str_colored(row, x, "│", border_color)?;
            window.write_str_colored(row, x + popup_w.saturating_sub(1), "│", border_color)?;
        }
    }
    if popup_h > 1 {
        window.write_str_colored(
            y + popup_h.saturating_sub(1),
            x,
            &format!("╰{}╯", horizontal),
            border_color,
        )?;
    }

    let title_text = if popup.title.chars().count() > popup_w.saturating_sub(4) as usize {
        let mut clipped: String = popup
            .title
            .chars()
            .take(popup_w.saturating_sub(7) as usize)
            .collect();
        clipped.push_str("...");
        clipped
    } else {
        popup.title
    };
    window.write_str_colored(y, x + 2, &title_text, title_color)?;

    let mut view = WindowView {
        window,
        x_offset: x + 1,
        y_offset: y + 1,
        scroll_x: 0,
        scroll_y: 0,
        width: inner_w,
        height: inner_h,
    };
    if inner_w > 0 && inner_h > 0 {
        let blank_row = " ".repeat(inner_w as usize);
        for row in 0..inner_h {
            view.write_str_colored(row, 0, &blank_row, style.explorer.file)?;
        }
    }

    let visual_selection = state.active_visual_selection();
    let (snapshot, spec, line_styles, cursor_line, total_lines, scroll_x) = state
        .with_active_buffer_view_mut(|buffer, explorer_view| {
            reconcile_explorer_cursor_for_popup(
                &mut explorer_view.cursor,
                buffer,
                inner_w,
                inner_h,
            );
            let total_lines = buffer.len_lines().max(1);
            let gutter_w = line_number_gutter_width(total_lines);
            let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
            let text_w = inner_w.saturating_sub(content_x);
            let (scroll_x, scroll_y) = explorer_view.cursor.viewport_scroll();
            let viewport = TextViewport {
                scroll_x,
                scroll_y,
                width: text_w,
                height: inner_h,
            };
            let snapshot =
                snapshot_lines_wrapped_cached(buffer, &viewport, &mut explorer_view.grapheme_cache);
            let spec = explorer_view
                .cursor
                .cursor_spec(buffer, text_w as usize, inner_h as usize);
            let line_styles = (0..snapshot.lines.len())
                .map(|row| {
                    let line_idx = snapshot.first_line + row;
                    let source = buffer.line_string(line_idx);
                    explorer_entry_color(style, &popup.dir_path, &source)
                })
                .collect::<Vec<_>>();
            (
                snapshot,
                spec,
                line_styles,
                explorer_view.cursor.cursor.line,
                total_lines,
                scroll_x,
            )
        });

    let gutter_w = line_number_gutter_width(total_lines);
    let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    draw_relative_line_numbers(
        &mut view,
        style,
        gutter_w,
        inner_h,
        snapshot.first_line,
        cursor_line,
        total_lines,
    )?;

    for (row, line) in snapshot.lines.iter().enumerate() {
        let color = line_styles.get(row).copied().unwrap_or(style.explorer.file);
        let line_idx = snapshot.first_line + row;
        if let Some((selection, mode)) = visual_selection {
            if let Some(sel_range) = state
                .session
                .active_buffer()
                .visual_selection_char_range_on_line(selection, mode, line_idx)
            {
                let source_line = state.session.active_buffer().line_string(line_idx);
                draw_line_with_selection(
                    &mut view,
                    row as u16,
                    content_x,
                    &source_line,
                    scroll_x,
                    inner_w.saturating_sub(content_x) as usize,
                    sel_range.start,
                    sel_range.end,
                    color,
                    ColorPair::new(style.theme.selection_fg, style.theme.selection_bg),
                )?;
                continue;
            }
        }
        view.write_str_colored(row as u16, content_x, line, color)?;
    }
    if spec.visible {
        view.request_cursor(minui::window::CursorSpec {
            x: spec.x.saturating_add(content_x),
            y: spec.y,
            visible: true,
        });
    }

    let status = build_editor_status_bar(state, style);
    status.draw(window)?;

    Ok(())
}

fn reconcile_explorer_cursor_for_popup(
    cursor: &mut crate::input::cursor::CursorController,
    buffer: &TextBuffer,
    inner_w: u16,
    inner_h: u16,
) {
    let gutter_w = line_number_gutter_width(buffer.len_lines().max(1));
    let content_x = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    let text_w = inner_w.saturating_sub(content_x) as usize;

    cursor.follow.top_margin_rows = 0;
    cursor.follow.bottom_margin_rows = 0;
    cursor.reconcile_after_edit(buffer, text_w, inner_h as usize);
}

pub fn explorer_popup_inner_size(term_w: u16, term_h: u16, style: UiStyle) -> (u16, u16) {
    let popup_w = compute_popup_dim(
        term_w,
        style.explorer.width_percent,
        style.explorer.min_width,
    );
    let popup_h = compute_popup_dim(
        term_h,
        style.explorer.height_percent,
        style.explorer.min_height,
    );
    (popup_w.saturating_sub(2), popup_h.saturating_sub(2))
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

fn explorer_entry_color(style: UiStyle, dir_path: &Path, source_line: &str) -> ColorPair {
    let line = source_line.trim();
    if line.is_empty() {
        return style.explorer.file;
    }

    let is_dir = line == ".." || line.ends_with('/');
    let name = line.strip_suffix('/').unwrap_or(line);
    let is_hidden = name.starts_with('.');
    if is_hidden {
        return style.explorer.hidden;
    }

    if is_dir {
        return style.explorer.directory;
    }

    if is_executable(dir_path.join(name)) {
        return style.explorer.executable;
    }

    style.explorer.file
}

fn is_executable(path: PathBuf) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

#[cfg(test)]
mod tests {
    use minui::window::CursorSpec;
    use redox_core::Pos;

    use super::*;
    use crate::input::cursor::CursorController;

    #[test]
    fn popup_reconcile_keeps_selected_entry_visible() {
        let buffer =
            TextBuffer::from_str("../\na/\nb/\nc/\nd/\ne/\nf/\ng/\nh/\ni/\nj/\nopen.txt\nz.txt");
        let mut cursor = CursorController::default();
        cursor.cursor = Pos::new(11, 0);

        reconcile_explorer_cursor_for_popup(&mut cursor, &buffer, 20, 5);

        assert_eq!(cursor.scroll_y_lines, 7);
        let spec: CursorSpec = cursor.cursor_spec(&buffer, 17, 5);
        assert!(spec.visible);
        assert_eq!(spec.y, 4);
    }
}

fn line_number_gutter_width(total_lines: usize) -> u16 {
    let digits = total_lines.max(1).ilog10() as u16 + 1;
    digits.saturating_add(1)
}

fn draw_relative_line_numbers(
    view: &mut WindowView<'_>,
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
            view.write_str_colored(row, 0, &text, color)?;
        }

        view.write_str_colored(row, sep_x, "▕", color)?;
    }

    Ok(())
}

fn draw_line_with_selection(
    view: &mut WindowView<'_>,
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
        let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
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

        view.write_str_colored(row, col + used_cells as u16, g, color)?;
        used_cells = used_cells.saturating_add(g_width);
    }

    Ok(())
}
