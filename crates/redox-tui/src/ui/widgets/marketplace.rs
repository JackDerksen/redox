use minui::widgets::WindowView;
use minui::{ColorPair, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{LspEntryStatusKind, LspMarketplacePopup};
use crate::ui::UiStyle;
use crate::ui::widgets::popup::{
    PopupChrome, clip_text_to_cells, draw_anchored_popup_frame, popup_inner_size, popup_window_view,
};

const LSP_TITLE: &str = "Language Tools"; // Maybe someday this will become a paid DLC
const MARKETPLACE_LANGUAGE_TOOL_GAP: u16 = 3;
const MARKETPLACE_TOOL_STATUS_GAP: u16 = 1;

pub fn draw_lsp_marketplace_popup(
    popup: &LspMarketplacePopup,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let (term_w, term_h) = window.get_size();
    let (inner_w, inner_h) = popup_inner_size(
        term_w,
        term_h,
        style.lsp_marketplace.width_percent,
        style.lsp_marketplace.height_percent,
        style.lsp_marketplace.min_width,
        style.lsp_marketplace.min_height,
    );
    let layout = draw_anchored_popup_frame(
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
    let shared_prefix_w = marketplace_shared_prefix_width(popup);
    let language_w = marketplace_language_width(popup);
    row = draw_marketplace_column_header(&mut view, style, row, shared_prefix_w, language_w)?;
    let _ = draw_marketplace_entries(&mut view, popup, style, row, shared_prefix_w, language_w)?;
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
    shared_prefix_w: u16,
    language_w: u16,
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
        let base_colors = selection_aware_color(style.finder.text, style.finder.selected, selected);
        let dim_colors = selection_aware_color(style.finder.dim, style.finder.selected, selected);
        let status_colors = {
            let base = match entry.status_kind {
                LspEntryStatusKind::Ready => style.finder.match_highlight,
                LspEntryStatusKind::Pending => style.finder.preview_title,
                LspEntryStatusKind::Missing => style.finder.prompt,
                LspEntryStatusKind::Informational => style.finder.dim,
            };
            selection_aware_color(base, style.finder.selected, selected)
        };

        let prefix = marketplace_entry_prefix(selected, &entry.action_label);
        if shared_prefix_w >= window.width.saturating_sub(1) {
            break;
        }
        let (language_w, tool_w, status_w) =
            marketplace_column_widths(window.width, shared_prefix_w, language_w);
        let language_x = 1u16.saturating_add(shared_prefix_w);
        let tool_x = language_x
            .saturating_add(language_w)
            .saturating_add(MARKETPLACE_LANGUAGE_TOOL_GAP);
        let status_x = tool_x
            .saturating_add(tool_w)
            .saturating_add(MARKETPLACE_TOOL_STATUS_GAP);

        if selected {
            let fill = " ".repeat(window.width.saturating_sub(2) as usize);
            window.write_str_colored(row, 1, &fill, style.finder.selected)?;
        }
        window.write_str_colored(
            row,
            1,
            &pad_or_clip(&prefix, shared_prefix_w as usize),
            base_colors,
        )?;
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

fn selection_aware_color(base: ColorPair, selected: ColorPair, is_selected: bool) -> ColorPair {
    if is_selected {
        ColorPair::new(base.fg, selected.bg)
    } else {
        base
    }
}

fn draw_marketplace_column_header(
    window: &mut WindowView<'_>,
    style: UiStyle,
    row: u16,
    shared_prefix_w: u16,
    language_w: u16,
) -> minui::Result<u16> {
    if row >= window.height {
        return Ok(row);
    }

    let prefix = "      ";
    if shared_prefix_w >= window.width.saturating_sub(1) {
        return Ok(row.saturating_add(1));
    }
    let (language_w, tool_w, status_w) =
        marketplace_column_widths(window.width, shared_prefix_w, language_w);
    let language_x = 1u16.saturating_add(shared_prefix_w);
    let tool_x = language_x
        .saturating_add(language_w)
        .saturating_add(MARKETPLACE_LANGUAGE_TOOL_GAP);
    let status_x = tool_x
        .saturating_add(tool_w)
        .saturating_add(MARKETPLACE_TOOL_STATUS_GAP);
    let header_colors = style.finder.dim;

    window.write_str_colored(
        row,
        1,
        &pad_or_clip(prefix, shared_prefix_w as usize),
        header_colors,
    )?;
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

fn marketplace_column_widths(
    total_width: u16,
    prefix_width: u16,
    preferred_language_width: u16,
) -> (u16, u16, u16) {
    let inner = total_width.saturating_sub(2);
    let gutter_count = MARKETPLACE_LANGUAGE_TOOL_GAP.saturating_add(MARKETPLACE_TOOL_STATUS_GAP);
    let available = inner
        .saturating_sub(prefix_width)
        .saturating_sub(gutter_count);
    if available <= 12 {
        let language_w = available.saturating_mul(2) / 5;
        let status_w = available.saturating_sub(language_w);
        return (language_w, 0, status_w);
    }

    let status_w = (available / 3).clamp(10, 24);
    let language_w = preferred_language_width
        .max(text_width("Language") as u16)
        .min(available.saturating_sub(status_w).saturating_sub(1))
        .max(1);
    let tool_w = available
        .saturating_sub(language_w)
        .saturating_sub(status_w);
    (language_w, tool_w, status_w)
}

fn marketplace_language_width(popup: &LspMarketplacePopup) -> u16 {
    popup
        .entries
        .iter()
        .map(|entry| text_width(&entry.language_label))
        .max()
        .unwrap_or_else(|| text_width("Language"))
        .max(text_width("Language"))
        .min(u16::MAX as usize) as u16
}

fn marketplace_shared_prefix_width(popup: &LspMarketplacePopup) -> u16 {
    marketplace_shared_prefix_width_from_labels(
        popup
            .entries
            .iter()
            .map(|entry| entry.action_label.as_str()),
    )
}

fn marketplace_shared_prefix_width_from_labels<'a>(labels: impl Iterator<Item = &'a str>) -> u16 {
    let header_w = text_width("      ");
    let width = labels
        .map(|label| text_width(&marketplace_entry_prefix(false, label)))
        .max()
        .unwrap_or(header_w)
        .max(header_w);
    width.min(u16::MAX as usize) as u16
}

fn marketplace_entry_prefix(selected: bool, action_label: &str) -> String {
    format!("{} [{}] ", if selected { "›" } else { " " }, action_label)
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
