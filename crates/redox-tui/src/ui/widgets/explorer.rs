use std::path::Path;

use minui::widgets::{Widget, WindowView};
use minui::{ColorPair, TabPolicy, Window, cell_width, window::CursorSpec};
use redox_core::TextBuffer;
use unicode_segmentation::UnicodeSegmentation;

use crate::app::state::{ExplorerRenderRow, ExplorerRenderRowKind};
use crate::app::{EditorState, ExplorerPopup, GitFileStatusKind};
use crate::ui::icons::{PREFIX_WIDTH, PopupKind, file_icon, folder_icon, popup_title};
use crate::ui::widgets::popup::{
    PopupChrome, PopupLayout, draw_popup_frame_at, popup_inner_size, popup_window_view,
};
use crate::ui::{TextViewport, UiStyle, build_editor_status_bar};

const GUTTER_CONTENT_PADDING: u16 = 1;
const EXPLORER_STATUS_DOT: &str = "● ";
const EXPLORER_STATUS_DOT_WIDTH: u16 = 2;

#[derive(Debug, Clone, Copy)]
struct ExplorerRowStyle {
    text: ColorPair,
    git_status: Option<GitFileStatusKind>,
}

/// Draws the Explorer popup and returns its requested cursor position.
///
/// `reconcile_inner_h` retains the popup's original unshrunk inner height separately from
/// `layout.inner_h`, preventing abrupt scrolling when the command line stacks below the popup.
pub fn draw_explorer_popup_view(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    popup: ExplorerPopup,
    layout: PopupLayout,
    reconcile_inner_h: u16,
) -> minui::Result<Option<CursorSpec>> {
    let PopupLayout {
        inner_w,
        inner_h,
        x,
        y,
    } = layout;
    let title = popup_title(PopupKind::Explorer, &popup.title, style.icons_enabled);
    let layout = draw_popup_frame_at(
        window,
        x,
        y,
        inner_w,
        inner_h,
        &title,
        PopupChrome::explorer(style),
    )?;
    let mut view = popup_window_view(window, layout);

    state.refresh_git_repo_status_for_dir(&popup.dir_path);
    let show_git_status_column = state.refresh_explorer_render_model();

    let visual_selection = state.active_visual_selection();
    let (snapshot, spec, cursor_line, total_lines, scroll_x) =
        state.with_active_buffer_view_mut(|buffer, explorer_view| {
            reconcile_explorer_cursor_for_popup(
                &mut explorer_view.cursor,
                buffer,
                inner_w,
                reconcile_inner_h,
                show_git_status_column,
                style.icons_enabled,
            );
            let total_lines = buffer.len_lines().max(1);
            let gutter_w = line_number_gutter_width(total_lines, show_git_status_column);
            let content_x = gutter_w
                .saturating_add(GUTTER_CONTENT_PADDING)
                .saturating_add(if style.icons_enabled { PREFIX_WIDTH } else { 0 });
            let text_w = inner_w.saturating_sub(content_x);
            let (scroll_x, scroll_y) = explorer_view.cursor.viewport_scroll();
            let viewport = TextViewport {
                scroll_x,
                scroll_y,
                width: text_w,
                height: inner_h,
            };
            let snapshot = explorer_view.render_line_cache.snapshot(buffer, &viewport);
            let spec = explorer_view
                .cursor
                .cursor_spec(buffer, text_w as usize, inner_h as usize);
            (
                snapshot,
                spec,
                explorer_view.cursor.cursor.line,
                total_lines,
                scroll_x,
            )
        });
    let render_rows = state.explorer_render_rows(snapshot.first_line(), snapshot.line_count());

    let gutter_w = line_number_gutter_width(total_lines, show_git_status_column);
    let icon_col = gutter_w.saturating_add(GUTTER_CONTENT_PADDING);
    let content_x = icon_col.saturating_add(if style.icons_enabled { PREFIX_WIDTH } else { 0 });
    draw_relative_line_numbers(
        &mut view,
        style,
        gutter_w,
        inner_h,
        show_git_status_column,
        snapshot.first_line(),
        cursor_line,
        total_lines,
    )?;

    for (row, line) in snapshot.iter().enumerate() {
        let row_style = render_rows
            .get(row)
            .copied()
            .map(|row| explorer_row_style(style, row))
            .unwrap_or(ExplorerRowStyle {
                text: style.explorer.file,
                git_status: None,
            });
        let line_idx = snapshot.first_line() + row;
        let source_line = line.source();
        if style.icons_enabled
            && let Some(icon) = explorer_entry_icon(&popup.dir_path, source_line)
        {
            view.write_str_colored(row as u16, icon_col, icon, row_style.text)?;
        }
        if let Some((selection, mode)) = visual_selection
            && let Some(sel_range) = state
                .session
                .active_buffer()
                .visual_selection_char_range_on_line(selection, mode, line_idx)
        {
            draw_line_with_selection(
                &mut view,
                row as u16,
                content_x,
                source_line,
                scroll_x,
                inner_w.saturating_sub(content_x) as usize,
                sel_range.start,
                sel_range.end,
                row_style.text,
                ColorPair::new(row_style.text.fg, style.theme.selection_bg),
            )?;
            draw_explorer_status_dot(&mut view, style, 0, row as u16, row_style.git_status)?;
            continue;
        }
        view.write_str_colored(row as u16, content_x, line.visible(), row_style.text)?;
        draw_explorer_status_dot(&mut view, style, 0, row as u16, row_style.git_status)?;
    }
    let cursor = spec.visible.then_some(CursorSpec {
        x: x.saturating_add(1)
            .saturating_add(spec.x)
            .saturating_add(content_x),
        y: y.saturating_add(1).saturating_add(spec.y),
        visible: true,
    });

    let status = build_editor_status_bar(state, style);
    status.draw(window)?;

    Ok(cursor)
}

fn reconcile_explorer_cursor_for_popup(
    cursor: &mut crate::input::cursor::CursorController,
    buffer: &TextBuffer,
    inner_w: u16,
    inner_h: u16,
    show_git_status_column: bool,
    icons_enabled: bool,
) {
    let gutter_w = line_number_gutter_width(buffer.len_lines().max(1), show_git_status_column);
    let content_x = gutter_w
        .saturating_add(GUTTER_CONTENT_PADDING)
        .saturating_add(if icons_enabled { PREFIX_WIDTH } else { 0 });
    let text_w = inner_w.saturating_sub(content_x) as usize;

    cursor.follow.top_margin_rows = 0;
    cursor.follow.bottom_margin_rows = 0;
    cursor.reconcile_after_edit(buffer, text_w, inner_h as usize);
}

fn explorer_entry_icon(dir_path: &Path, source_line: &str) -> Option<&'static str> {
    let line = source_line.trim();
    if line.is_empty() {
        return None;
    }
    if line == ".." {
        return Some(folder_icon(true));
    }
    let is_dir = line.ends_with('/');
    let name = line.strip_suffix('/').unwrap_or(line);
    Some(if is_dir {
        folder_icon(false)
    } else {
        file_icon(&dir_path.join(name))
    })
}

pub fn explorer_popup_inner_size(term_w: u16, term_h: u16, style: UiStyle) -> (u16, u16) {
    popup_inner_size(
        term_w,
        term_h,
        style.explorer.width_percent,
        style.explorer.height_percent,
        style.explorer.min_width,
        style.explorer.min_height,
    )
}

fn explorer_row_style(style: UiStyle, row: ExplorerRenderRow) -> ExplorerRowStyle {
    ExplorerRowStyle {
        text: match row.kind {
            ExplorerRenderRowKind::File => style.explorer.file,
            ExplorerRenderRowKind::Directory => style.explorer.directory,
            ExplorerRenderRowKind::Hidden => style.explorer.hidden,
            ExplorerRenderRowKind::Executable => style.explorer.executable,
        },
        git_status: row.git_status,
    }
}

fn draw_explorer_status_dot(
    view: &mut WindowView<'_>,
    style: UiStyle,
    col: u16,
    row: u16,
    status: Option<GitFileStatusKind>,
) -> minui::Result<()> {
    let Some(status) = status else {
        return Ok(());
    };

    let color = style.git.file_status(status);
    view.write_str_colored(row, col, EXPLORER_STATUS_DOT, color)?;
    Ok(())
}

fn line_number_gutter_width(total_lines: usize, show_git_status_column: bool) -> u16 {
    let digits = total_lines.max(1).ilog10() as u16 + 1;
    digits
        .saturating_add(u16::from(show_git_status_column) * EXPLORER_STATUS_DOT_WIDTH)
        .saturating_add(1)
}

fn draw_relative_line_numbers(
    view: &mut WindowView<'_>,
    style: UiStyle,
    gutter_w: u16,
    text_h: u16,
    show_git_status_column: bool,
    first_line: usize,
    cursor_line: usize,
    total_lines: usize,
) -> minui::Result<()> {
    if gutter_w == 0 || text_h == 0 {
        return Ok(());
    }

    let sep_x = gutter_w.saturating_sub(1);
    let marker_offset = u16::from(show_git_status_column) * EXPLORER_STATUS_DOT_WIDTH;
    let number_w = gutter_w.saturating_sub(marker_offset).saturating_sub(1) as usize;
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
            view.write_str_colored(row, marker_offset, &text, color)?;
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

#[cfg(test)]
mod tests {
    use minui::window::CursorSpec;
    use redox_core::Pos;

    use super::*;
    use crate::input::cursor::CursorController;

    #[test]
    fn popup_reconcile_keeps_selected_entry_visible() {
        let buffer =
            TextBuffer::from_text("../\na/\nb/\nc/\nd/\ne/\nf/\ng/\nh/\ni/\nj/\nopen.txt\nz.txt");
        let mut cursor = CursorController::default();
        cursor.cursor = Pos::new(11, 0);

        reconcile_explorer_cursor_for_popup(&mut cursor, &buffer, 20, 5, false, false);

        assert_eq!(cursor.scroll_y_lines, 7);
        let spec: CursorSpec = cursor.cursor_spec(&buffer, 17, 5);
        assert!(spec.visible);
        assert_eq!(spec.y, 4);
    }
}
