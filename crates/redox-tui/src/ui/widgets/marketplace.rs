use minui::widgets::WindowView;
use minui::{ColorPair, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{LspEntryStatusKind, LspMarketplacePopup};
use crate::ui::UiStyle;
use crate::ui::widgets::popup::{
    PopupChrome, clip_text_to_cells, draw_popup_frame, popup_inner_size, popup_window_view,
};

const LSP_TITLE: &str = "Language Tools"; // Maybe someday this will become a paid DLC

pub fn draw_lsp_marketplace_popup(
    popup: &LspMarketplacePopup,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let (term_w, term_h) = window.get_size();
    let (inner_w, inner_h) = popup_inner_size(
        term_w,
        term_h,
        style.finder.width_percent,
        style.finder.height_percent,
        style.finder.min_width,
        style.finder.min_height,
    );
    let layout = draw_popup_frame(
        window,
        term_w,
        term_h,
        inner_w,
        inner_h,
        LSP_TITLE,
        PopupChrome {
            border: style.finder.border,
            title: style.finder.title,
            fill: style.finder.text,
        },
    )?;
    let mut view = popup_window_view(window, layout);

    let installed_count = popup.entries.iter().filter(|entry| entry.installed).count();
    let mut row = draw_section_header(
        &mut view,
        0,
        &format!("{} tools, {} enabled", popup.entries.len(), installed_count),
        style.finder.query_title,
    )?;
    row = draw_marketplace_column_header(&mut view, style, row)?;
    let _ = draw_marketplace_entries(&mut view, popup, style, row)?;
    Ok(())
}

fn draw_section_header(
    window: &mut WindowView<'_>,
    row: u16,
    text: &str,
    colors: ColorPair,
) -> minui::Result<u16> {
    if row >= window.height {
        return Ok(row);
    }
    window.write_str_colored(row, 1, text, colors)?;
    Ok(row.saturating_add(1))
}

fn draw_marketplace_entries(
    window: &mut WindowView<'_>,
    popup: &LspMarketplacePopup,
    style: UiStyle,
    mut row: u16,
) -> minui::Result<u16> {
    let visible_rows = window.height.saturating_sub(row) as usize;
    let installed_count = popup.entries.iter().filter(|entry| entry.installed).count();
    let has_separator = installed_count > 0 && installed_count < popup.entries.len();
    let selected_virtual = popup
        .selected
        .saturating_add((has_separator && popup.selected >= installed_count) as usize);
    let total_virtual_rows = popup.entries.len().saturating_add(has_separator as usize);
    let mut start = popup.scroll.min(total_virtual_rows);
    if selected_virtual < start {
        start = selected_virtual;
    }
    if visible_rows > 0 && selected_virtual >= start.saturating_add(visible_rows) {
        start = selected_virtual
            .saturating_add(1)
            .saturating_sub(visible_rows);
    }
    let end = start.saturating_add(visible_rows).min(total_virtual_rows);

    for virtual_idx in start..end {
        if row >= window.height {
            break;
        }

        if has_separator && virtual_idx == installed_count {
            let divider = "─".repeat(window.width.saturating_sub(0) as usize);
            window.write_str_colored(row, 0, &divider, style.finder.dim)?;
            row = row.saturating_add(1);
            continue;
        }

        let idx = if has_separator && virtual_idx > installed_count {
            virtual_idx.saturating_sub(1)
        } else {
            virtual_idx
        };
        let Some(entry) = popup.entries.get(idx) else {
            continue;
        };

        let selected = idx == popup.selected;
        let base_colors = if selected {
            style.finder.selected
        } else {
            style.finder.text
        };
        let dim_colors = if selected {
            style.finder.selected
        } else {
            style.finder.dim
        };
        let status_colors = if selected {
            style.finder.selected
        } else {
            match entry.status_kind {
                LspEntryStatusKind::Ready => style.finder.match_highlight,
                LspEntryStatusKind::Pending => style.finder.preview_title,
                LspEntryStatusKind::Missing => style.finder.prompt,
                LspEntryStatusKind::Informational => style.finder.dim,
            }
        };

        let prefix = format!(
            "{} [{}] ",
            if selected { "›" } else { " " },
            entry.action_label
        );
        let prefix_w = text_width(&prefix) as u16;
        if prefix_w >= window.width.saturating_sub(1) {
            break;
        }
        let (language_w, tool_w, status_w) = marketplace_column_widths(window.width);
        let language_x = 1u16.saturating_add(prefix_w);
        let tool_x = language_x.saturating_add(language_w).saturating_add(1);
        let status_x = tool_x.saturating_add(tool_w).saturating_add(1);

        if selected {
            let fill = " ".repeat(window.width.saturating_sub(2) as usize);
            window.write_str_colored(row, 1, &fill, style.finder.selected)?;
        }
        window.write_str_colored(row, 1, &prefix, base_colors)?;
        window.write_str_colored(
            row,
            language_x,
            &pad_or_clip(&entry.language_label, language_w as usize),
            dim_colors,
        )?;
        window.write_str_colored(
            row,
            tool_x,
            &pad_or_clip(&entry.tool_label, tool_w as usize),
            base_colors,
        )?;
        window.write_str_colored(
            row,
            status_x,
            &clip_text_to_cells(&entry.status_label, status_w as usize),
            status_colors,
        )?;

        row = row.saturating_add(1);
    }

    Ok(row)
}

fn draw_marketplace_column_header(
    window: &mut WindowView<'_>,
    style: UiStyle,
    row: u16,
) -> minui::Result<u16> {
    if row >= window.height {
        return Ok(row);
    }

    let prefix = "      ";
    let prefix_w = text_width(prefix) as u16;
    let (language_w, tool_w, status_w) = marketplace_column_widths(window.width);
    let language_x = 1u16.saturating_add(prefix_w);
    let tool_x = language_x.saturating_add(language_w).saturating_add(1);
    let status_x = tool_x.saturating_add(tool_w).saturating_add(1);
    let header_colors = style.finder.dim;

    window.write_str_colored(row, 1, prefix, header_colors)?;
    window.write_str_colored(
        row,
        language_x,
        &pad_or_clip("Language", language_w as usize),
        header_colors,
    )?;
    window.write_str_colored(
        row,
        tool_x,
        &pad_or_clip("Tool", tool_w as usize),
        header_colors,
    )?;
    window.write_str_colored(
        row,
        status_x,
        &clip_text_to_cells("Status", status_w as usize),
        header_colors,
    )?;
    Ok(row.saturating_add(1))
}

fn marketplace_column_widths(total_width: u16) -> (u16, u16, u16) {
    let inner = total_width.saturating_sub(2);
    let prefix = 6u16;
    let gutter_count = 2u16;
    let available = inner.saturating_sub(prefix).saturating_sub(gutter_count);
    if available <= 12 {
        let language_w = available.saturating_mul(2) / 5;
        let status_w = available.saturating_sub(language_w);
        return (language_w, 0, status_w);
    }

    let status_w = (available / 3).clamp(10, 24);
    let language_w = (available / 4).clamp(8, 14);
    let tool_w = available
        .saturating_sub(language_w)
        .saturating_sub(status_w);
    (language_w, tool_w, status_w)
}

fn pad_or_clip(text: &str, width: usize) -> String {
    let clipped = clip_text_to_cells(text, width);
    let pad = width.saturating_sub(text_width(&clipped));
    format!("{clipped}{}", " ".repeat(pad))
}

fn text_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|grapheme| (cell_width(grapheme, TabPolicy::Fixed(4)) as usize).max(1))
        .sum()
}
