use minui::widgets::WindowView;
use minui::{ColorPair, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{DiagnosticSeverity, DiagnosticsPopup};
use crate::ui::UiStyle;
use crate::ui::widgets::popup::{
    PopupChrome, clip_text_to_cells, draw_popup_frame, popup_inner_size, popup_window_view,
    wrap_text_to_cells,
};

const DIAGNOSTICS_TITLE: &str = "Diagnostics";
const DIAGNOSTIC_VISIBLE_ROWS: usize = 12;
const DIAGNOSTIC_DETAIL_MIN_HEIGHT: u16 = 8;
const DIAGNOSTIC_DETAIL_MAX_ROWS: usize = 6;

pub fn draw_diagnostics_popup(
    popup: &DiagnosticsPopup,
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
        DIAGNOSTICS_TITLE,
        PopupChrome {
            border: style.finder.border,
            title: style.finder.title,
            fill: style.finder.text,
        },
    )?;
    let mut view = popup_window_view(window, layout);
    let summary = format!("{} diagnostics", popup.entries.len());
    let _ = draw_section_header(&mut view, 0, &summary, style.finder.query_title)?;

    let detail_rows = if view.height >= DIAGNOSTIC_DETAIL_MIN_HEIGHT {
        ((view.height as usize) / 3).clamp(3, DIAGNOSTIC_DETAIL_MAX_ROWS)
    } else {
        0
    };
    let reserved_rows = if detail_rows > 0 {
        detail_rows.saturating_add(1)
    } else {
        0
    };
    let list_capacity = (view.height as usize)
        .saturating_sub(1)
        .saturating_sub(reserved_rows)
        .max(1)
        .min(DIAGNOSTIC_VISIBLE_ROWS);
    let mut start = popup.scroll.min(popup.entries.len());
    if popup.selected < start {
        start = popup.selected;
    }
    if popup.selected >= start.saturating_add(list_capacity) {
        start = popup
            .selected
            .saturating_add(1)
            .saturating_sub(list_capacity);
    }
    let end = (start + list_capacity).min(popup.entries.len());
    let location_width = popup.entries[start..end]
        .iter()
        .map(|entry| format!("{}:{}", entry.line + 1, entry.col + 1).len())
        .max()
        .unwrap_or(0);
    for (visible_idx, entry) in popup.entries[start..end].iter().enumerate() {
        let row = visible_idx as u16 + 1;
        if row >= view.height {
            break;
        }
        let idx = start + visible_idx;
        let selected = idx == popup.selected;
        if selected {
            let fill = " ".repeat(view.width.saturating_sub(2) as usize);
            view.write_str_colored(row, 1, &fill, style.finder.selected)?;
        }

        let row_colors = selection_aware_color(style.finder.text, style.finder.selected, selected);
        let dim_colors = selection_aware_color(style.finder.dim, style.finder.selected, selected);
        let severity_colors = selection_aware_color(
            severity_color(style, entry.severity),
            style.finder.selected,
            selected,
        );

        let marker = if selected { "› " } else { "  " };
        let location = format!("{}:{}", entry.line + 1, entry.col + 1);
        let location = format!("{location:>location_width$}");
        let glyph = severity_glyph(entry.severity);
        let prefix_w = text_width(marker)
            .saturating_add(location_width)
            .saturating_add(1)
            .saturating_add(text_width(glyph))
            .saturating_add(1);

        view.write_str_colored(row, 1, marker, dim_colors)?;
        view.write_str_colored(row, 3, &location, dim_colors)?;
        view.write_str_colored(row, 3 + location_width as u16, " ", row_colors)?;
        view.write_str_colored(row, 4 + location_width as u16, glyph, severity_colors)?;
        view.write_str_colored(
            row,
            4 + location_width as u16 + text_width(glyph) as u16,
            " ",
            row_colors,
        )?;

        let message_w = view.width.saturating_sub(prefix_w as u16).saturating_sub(3) as usize;
        let message = clip_text_to_cells(&entry.summary, message_w);
        view.write_str_colored(row, prefix_w as u16 + 1, &message, row_colors)?;
    }

    if detail_rows > 0 {
        let separator_row = 1u16.saturating_add((end.saturating_sub(start)) as u16);
        if separator_row < view.height {
            let divider = "─".repeat(view.width as usize);
            view.write_str_colored(separator_row, 0, &divider, style.finder.dim)?;
        }

        if let Some(selected) = popup.entries.get(popup.selected) {
            let title_row = separator_row.saturating_add(1);
            if title_row < view.height {
                let title = format!(
                    "{} {}:{}",
                    severity_glyph(selected.severity),
                    selected.line + 1,
                    selected.col + 1
                );
                view.write_str_colored(
                    title_row,
                    1,
                    &clip_text_to_cells(&title, view.width.saturating_sub(2) as usize),
                    severity_color(style, selected.severity),
                )?;
            }

            let detail_width = view.width.saturating_sub(2) as usize;
            let wrapped = wrap_text_to_cells(&selected.message, detail_width);
            for (idx, line) in wrapped.into_iter().take(detail_rows).enumerate() {
                let row = title_row.saturating_add(1 + idx as u16);
                if row >= view.height {
                    break;
                }
                view.write_str_colored(
                    row,
                    1,
                    &clip_text_to_cells(&line, detail_width),
                    style.finder.text,
                )?;
            }
        }
    }

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

fn severity_glyph(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "×",
        DiagnosticSeverity::Warning => "△",
        DiagnosticSeverity::Information => "•",
        DiagnosticSeverity::Hint => "⚬",
    }
}

fn severity_color(style: UiStyle, severity: DiagnosticSeverity) -> ColorPair {
    let popup_bg = style.finder.text.bg;
    match severity {
        DiagnosticSeverity::Error => ColorPair::new(
            minui::Color::Rgb {
                r: 255,
                g: 157,
                b: 177,
            },
            popup_bg,
        ),
        DiagnosticSeverity::Warning => ColorPair::new(
            minui::Color::Rgb {
                r: 255,
                g: 199,
                b: 158,
            },
            popup_bg,
        ),
        DiagnosticSeverity::Information => ColorPair::new(
            minui::Color::Rgb {
                r: 95,
                g: 92,
                b: 102,
            },
            popup_bg,
        ),
        DiagnosticSeverity::Hint => ColorPair::new(
            minui::Color::Rgb {
                r: 187,
                g: 232,
                b: 238,
            },
            popup_bg,
        ),
    }
}

fn selection_aware_color(base: ColorPair, selected: ColorPair, is_selected: bool) -> ColorPair {
    if is_selected {
        ColorPair::new(base.fg, selected.bg)
    } else {
        base
    }
}

fn text_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|grapheme| (cell_width(grapheme, TabPolicy::Fixed(4)) as usize).max(1))
        .sum()
}
