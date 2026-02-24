use std::fs;
use std::path::{Path, PathBuf};

use minui::widgets::{Widget, WindowView};
use minui::{ColorPair, Window};

use crate::app::{EditorState, ExplorerPopup};
use crate::ui::{TextViewport, UiStyle, build_editor_status_bar, snapshot_lines_wrapped_cached};

pub fn draw_explorer_popup_view(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
    popup: ExplorerPopup,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let popup_w = compute_popup_dim(vw, style.explorer.width_percent, style.explorer.min_width);
    let popup_h = compute_popup_dim(vh, style.explorer.height_percent, style.explorer.min_height);
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

    let inner_w = popup_w.saturating_sub(2);
    let inner_h = popup_h.saturating_sub(2);
    let mut view = WindowView {
        window,
        x_offset: x + 1,
        y_offset: y + 1,
        scroll_x: 0,
        scroll_y: 0,
        width: inner_w,
        height: inner_h,
    };

    let (snapshot, spec, line_styles) =
        state.with_active_buffer_view_mut(|buffer, explorer_view| {
            let (scroll_x, scroll_y) = explorer_view.cursor.viewport_scroll();
            let viewport = TextViewport {
                scroll_x,
                scroll_y,
                width: inner_w,
                height: inner_h,
            };
            let snapshot =
                snapshot_lines_wrapped_cached(buffer, &viewport, &mut explorer_view.grapheme_cache);
            let spec = explorer_view
                .cursor
                .cursor_spec(buffer, inner_w as usize, inner_h as usize);
            let line_styles = (0..snapshot.lines.len())
                .map(|row| {
                    let line_idx = snapshot.first_line + row;
                    let source = buffer.line_string(line_idx);
                    explorer_entry_color(style, &popup.dir_path, &source)
                })
                .collect::<Vec<_>>();
            (snapshot, spec, line_styles)
        });
    for (row, line) in snapshot.lines.iter().enumerate() {
        let color = line_styles.get(row).copied().unwrap_or(style.explorer.file);
        view.write_str_colored(row as u16, 0, line, color)?;
    }
    view.request_cursor(spec);

    let status = build_editor_status_bar(state, style);
    status.draw(window)?;

    Ok(())
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
    let is_hidden = name.starts_with('.') && name != "..";
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
