use minui::widgets::WindowView;
use minui::{ColorPair, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::state::{
    CodeActionPopup, DiagnosticSeverity, DiagnosticsCodeActionsPane, DiagnosticsPopup,
    DiagnosticsPopupFocus, SymbolInfoBlock, SymbolInfoDisplayKind, SymbolInfoDisplayLine,
    SymbolInfoKind, SymbolInfoPopup,
};
use crate::ui::UiStyle;
use crate::ui::syntax::{
    LineSyntaxSpan, SyntaxLanguage, language_for_name, lexical_fallback_line_spans,
    line_spans_for_source, syntax_color_for_range,
};
use crate::ui::widgets::popup::{
    PopupChrome, clip_text_to_cells, draw_popup_frame, draw_popup_frame_at, popup_inner_size,
    popup_window_view, wrap_text_to_cells,
};

const DIAGNOSTICS_TITLE: &str = "Diagnostics";
const CODE_ACTIONS_TITLE: &str = "Code Actions";
const DIAGNOSTIC_VISIBLE_ROWS: usize = 12;
const DIAGNOSTIC_DETAIL_MIN_HEIGHT: u16 = 8;
const DIAGNOSTIC_DETAIL_MAX_ROWS: usize = 6;
const SYMBOL_INFO_MAX_WIDTH: u16 = 72;
const SYMBOL_INFO_MAX_HEIGHT: u16 = 12;
pub fn symbol_info_content_width_limit(term_w: u16) -> usize {
    SYMBOL_INFO_MAX_WIDTH.min(term_w.saturating_sub(4)) as usize
}

pub fn draw_symbol_info_popup(
    popup: &SymbolInfoPopup<'_>,
    style: UiStyle,
    window: &mut dyn Window,
    cursor_x: u16,
    cursor_y: u16,
) -> minui::Result<()> {
    if popup.display_lines.is_empty() {
        return Ok(());
    }
    let (term_w, term_h) = window.get_size();
    if term_w < 8 || term_h < 4 {
        return Ok(());
    }
    let display_lines = &popup.display_lines;
    let available_right = term_w.saturating_sub(cursor_x.saturating_add(2));
    let content_width = display_lines
        .iter()
        .map(|line| text_width(&line.text) as u16)
        .max()
        .unwrap_or(24)
        .clamp(24, SYMBOL_INFO_MAX_WIDTH)
        .min(term_w.saturating_sub(4));
    let x = if available_right >= content_width.saturating_add(2) {
        cursor_x.saturating_add(1)
    } else {
        cursor_x.saturating_sub(content_width.saturating_add(3))
    };
    let inner_h = (display_lines.len() as u16).clamp(1, SYMBOL_INFO_MAX_HEIGHT);
    let max_scroll = display_lines.len().saturating_sub(inner_h as usize);
    let scroll = popup.scroll.min(max_scroll);
    let below_y = cursor_y.saturating_add(1);
    let y = if below_y.saturating_add(inner_h).saturating_add(2) <= term_h {
        below_y
    } else {
        cursor_y.saturating_sub(inner_h.saturating_add(2))
    };
    let layout = draw_popup_frame_at(
        window,
        x,
        y,
        content_width,
        inner_h,
        &popup.title,
        PopupChrome {
            border: style.finder.border,
            title: style.finder.title,
            fill: style.finder.text,
        },
    )?;
    let mut view = popup_window_view(window, layout);
    for (idx, line) in display_lines
        .iter()
        .skip(scroll)
        .take(inner_h as usize)
        .enumerate()
    {
        draw_symbol_info_line(&mut view, idx as u16, &line, content_width as usize, style)?;
    }
    Ok(())
}

pub fn build_symbol_info_display_lines(
    blocks: &[SymbolInfoBlock],
    width: usize,
) -> Vec<SymbolInfoDisplayLine> {
    let mut display = Vec::new();
    for block in blocks {
        let mut block_lines = symbol_info_block_lines(block, width);
        if block_lines.is_empty() {
            continue;
        }
        if !display.is_empty()
            && display
                .last()
                .is_some_and(|line: &SymbolInfoDisplayLine| !line.text.is_empty())
        {
            display.push(SymbolInfoDisplayLine {
                text: String::new(),
                kind: SymbolInfoDisplayKind::PlainText,
                spans: Vec::new(),
            });
        }
        display.append(&mut block_lines);
    }
    while display.last().is_some_and(|line| line.text.is_empty()) {
        display.pop();
    }
    display
}

fn draw_symbol_info_line(
    window: &mut WindowView<'_>,
    row: u16,
    line: &SymbolInfoDisplayLine,
    max_width: usize,
    style: UiStyle,
) -> minui::Result<()> {
    if line.text.is_empty() {
        return Ok(());
    }
    let clipped = clip_text_to_cells(&line.text, max_width);
    let base_color = symbol_info_base_color(style, &line.kind);
    draw_symbol_info_spans(
        window,
        row,
        0,
        &clipped,
        max_width,
        base_color,
        style,
        &line.spans,
    )
}

fn symbol_info_block_lines(block: &SymbolInfoBlock, width: usize) -> Vec<SymbolInfoDisplayLine> {
    match &block.kind {
        SymbolInfoKind::Code { language } => wrapped_render_lines(
            &block.text,
            SymbolInfoDisplayKind::Code {
                language: language.as_deref().and_then(language_for_name),
            },
            width,
        ),
        SymbolInfoKind::Markdown => markdown_render_lines(&block.text, width),
        SymbolInfoKind::PlainText => plain_text_render_lines(&block.text, width),
    }
}

fn wrapped_render_lines(
    text: &str,
    kind: SymbolInfoDisplayKind,
    width: usize,
) -> Vec<SymbolInfoDisplayLine> {
    let mut lines = Vec::new();
    for source_line in text.lines() {
        match kind {
            SymbolInfoDisplayKind::Code { .. } => {
                let wrapped = wrap_code_line_segments(source_line, width);
                let source_spans = symbol_info_line_spans(source_line, &kind);
                if wrapped.is_empty() {
                    lines.push(SymbolInfoDisplayLine {
                        text: String::new(),
                        kind: kind.clone(),
                        spans: Vec::new(),
                    });
                    continue;
                }
                lines.extend(wrapped.into_iter().map(|(text, start_byte, end_byte)| {
                    SymbolInfoDisplayLine {
                        spans: clip_line_spans(&source_spans, start_byte, end_byte),
                        text,
                        kind: kind.clone(),
                    }
                }));
            }
            SymbolInfoDisplayKind::PlainText | SymbolInfoDisplayKind::Markdown => {
                let wrapped = wrap_text_to_cells(source_line, width);
                if wrapped.is_empty() {
                    lines.push(SymbolInfoDisplayLine {
                        text: String::new(),
                        kind: kind.clone(),
                        spans: Vec::new(),
                    });
                    continue;
                }
                lines.extend(wrapped.into_iter().map(|text| SymbolInfoDisplayLine {
                    spans: symbol_info_line_spans(&text, &kind),
                    text,
                    kind: kind.clone(),
                }));
            }
        }
    }
    lines
}

fn clip_line_spans(
    spans: &[LineSyntaxSpan],
    start_byte: usize,
    end_byte: usize,
) -> Vec<LineSyntaxSpan> {
    spans
        .iter()
        .filter_map(|span| {
            if span.end_byte <= start_byte || span.start_byte >= end_byte {
                return None;
            }
            let mut clipped = *span;
            clipped.start_byte = clipped.start_byte.max(start_byte) - start_byte;
            clipped.end_byte = clipped.end_byte.min(end_byte) - start_byte;
            Some(clipped)
        })
        .collect()
}

fn markdown_render_lines(text: &str, width: usize) -> Vec<SymbolInfoDisplayLine> {
    let mut display = Vec::new();
    let mut paragraph = Vec::new();
    let mut in_code_block = false;
    let mut in_indented_code_block = false;
    let mut code_language = None;
    let mut code_lines = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(language) = fenced_code_language(trimmed) {
            if !paragraph.is_empty() {
                display.extend(wrapped_render_lines(
                    &paragraph.join("\n"),
                    SymbolInfoDisplayKind::Markdown,
                    width,
                ));
                paragraph.clear();
            }
            if in_code_block {
                display.extend(wrapped_render_lines(
                    &code_lines.join("\n"),
                    SymbolInfoDisplayKind::Code {
                        language: code_language,
                    },
                    width,
                ));
                code_lines.clear();
                code_language = None;
                in_code_block = false;
            } else {
                in_code_block = true;
                code_language = language.and_then(language_for_name);
            }
            continue;
        }

        if !in_code_block && markdown_indented_code_line(line) {
            if !paragraph.is_empty() {
                display.extend(wrapped_render_lines(
                    &paragraph.join("\n"),
                    SymbolInfoDisplayKind::Markdown,
                    width,
                ));
                paragraph.clear();
            }
            in_indented_code_block = true;
            code_lines.push(strip_markdown_code_indent(line).to_string());
            continue;
        }

        if in_indented_code_block {
            if trimmed.is_empty() {
                code_lines.push(String::new());
                continue;
            }
            if markdown_indented_code_line(line) {
                code_lines.push(strip_markdown_code_indent(line).to_string());
                continue;
            }
            display.extend(wrapped_render_lines(
                &code_lines.join("\n"),
                SymbolInfoDisplayKind::Code { language: None },
                width,
            ));
            code_lines.clear();
            in_indented_code_block = false;
        }

        if in_code_block {
            code_lines.push(line.to_string());
        } else {
            paragraph.push(line.to_string());
        }
    }

    if in_code_block {
        display.extend(wrapped_render_lines(
            &code_lines.join("\n"),
            SymbolInfoDisplayKind::Code {
                language: code_language,
            },
            width,
        ));
    } else if in_indented_code_block {
        display.extend(wrapped_render_lines(
            &code_lines.join("\n"),
            SymbolInfoDisplayKind::Code { language: None },
            width,
        ));
    } else if !paragraph.is_empty() {
        display.extend(wrapped_render_lines(
            &paragraph.join("\n"),
            SymbolInfoDisplayKind::Markdown,
            width,
        ));
    }

    display
}

fn plain_text_render_lines(text: &str, width: usize) -> Vec<SymbolInfoDisplayLine> {
    let mut display = Vec::new();
    let mut paragraph = Vec::new();
    let mut in_indented_code_block = false;
    let mut code_lines = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if !in_indented_code_block && plain_text_indented_code_line(line) {
            if !paragraph.is_empty() {
                display.extend(wrapped_render_lines(
                    &paragraph.join("\n"),
                    SymbolInfoDisplayKind::PlainText,
                    width,
                ));
                paragraph.clear();
            }
            in_indented_code_block = true;
            code_lines.push(strip_plain_text_code_indent(line).to_string());
            continue;
        }

        if in_indented_code_block {
            if trimmed.is_empty() {
                code_lines.push(String::new());
                continue;
            }
            if plain_text_indented_code_line(line) {
                code_lines.push(strip_plain_text_code_indent(line).to_string());
                continue;
            }
            display.extend(wrapped_render_lines(
                &code_lines.join("\n"),
                SymbolInfoDisplayKind::Code { language: None },
                width,
            ));
            code_lines.clear();
            in_indented_code_block = false;
        }

        paragraph.push(line.to_string());
    }

    if in_indented_code_block {
        display.extend(wrapped_render_lines(
            &code_lines.join("\n"),
            SymbolInfoDisplayKind::Code { language: None },
            width,
        ));
    } else if !paragraph.is_empty() {
        display.extend(wrapped_render_lines(
            &paragraph.join("\n"),
            SymbolInfoDisplayKind::PlainText,
            width,
        ));
    }

    display
}

fn fenced_code_language(line: &str) -> Option<Option<&str>> {
    if !line.starts_with("```") {
        return None;
    }
    let language = line[3..].trim();
    if language.is_empty() {
        Some(None)
    } else {
        Some(Some(language))
    }
}

fn markdown_indented_code_line(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

fn strip_markdown_code_indent(line: &str) -> &str {
    line.strip_prefix("    ")
        .or_else(|| line.strip_prefix('\t'))
        .unwrap_or(line)
}

fn plain_text_indented_code_line(line: &str) -> bool {
    line.starts_with("    ")
}

fn strip_plain_text_code_indent(line: &str) -> &str {
    line.strip_prefix("    ").unwrap_or(line)
}

#[allow(dead_code)]
fn wrap_code_line_to_cells(line: &str, max_cells: usize) -> Vec<String> {
    wrap_code_line_segments(line, max_cells)
        .into_iter()
        .map(|(text, _, _)| text)
        .collect()
}

fn wrap_code_line_segments(line: &str, max_cells: usize) -> Vec<(String, usize, usize)> {
    if max_cells == 0 {
        return Vec::new();
    }
    if line.is_empty() {
        return vec![(String::new(), 0, 0)];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    let mut segment_start = 0usize;
    let mut byte_idx = 0usize;

    for grapheme in line.graphemes(true) {
        let width = (cell_width(grapheme, TabPolicy::Fixed(4)) as usize).max(1);
        let next_byte_idx = byte_idx.saturating_add(grapheme.len());
        if used + width > max_cells && !current.is_empty() {
            out.push((std::mem::take(&mut current), segment_start, byte_idx));
            used = 0;
            segment_start = byte_idx;
        }
        current.push_str(grapheme);
        used += width;
        byte_idx = next_byte_idx;
    }

    if !current.is_empty() {
        out.push((current, segment_start, byte_idx));
    }

    out
}

fn symbol_info_base_color(style: UiStyle, kind: &SymbolInfoDisplayKind) -> ColorPair {
    match kind {
        SymbolInfoDisplayKind::PlainText => style.finder.dim,
        SymbolInfoDisplayKind::Markdown | SymbolInfoDisplayKind::Code { .. } => style.finder.text,
    }
}

fn symbol_info_line_spans(line: &str, kind: &SymbolInfoDisplayKind) -> Vec<LineSyntaxSpan> {
    match kind {
        SymbolInfoDisplayKind::PlainText => Vec::new(),
        SymbolInfoDisplayKind::Markdown => {
            line_spans_for_source(line, Some(SyntaxLanguage::Markdown))
                .and_then(|mut spans| spans.pop())
                .unwrap_or_default()
        }
        SymbolInfoDisplayKind::Code { language: None } => Vec::new(),
        SymbolInfoDisplayKind::Code {
            language: Some(language),
        } => line_spans_for_source(line, Some(*language))
            .and_then(|mut spans| spans.pop())
            .filter(|spans| !spans.is_empty())
            .unwrap_or_else(|| lexical_fallback_line_spans(line)),
    }
}

fn draw_symbol_info_spans(
    window: &mut WindowView<'_>,
    row: u16,
    base_col: u16,
    source_line: &str,
    width_cells: usize,
    base_color: ColorPair,
    style: UiStyle,
    spans: &[LineSyntaxSpan],
) -> minui::Result<()> {
    let mut line_cells = 0usize;
    let mut byte_idx = 0usize;
    let mut syntax_idx = 0usize;
    let mut pending_start: Option<usize> = None;
    let mut pending_end = 0usize;
    let mut pending_col = 0u16;
    let mut pending_colors = base_color;

    for grapheme in source_line.graphemes(true) {
        let grapheme_width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
        let start_byte = byte_idx;
        let end_byte = byte_idx.saturating_add(grapheme.len());
        byte_idx = end_byte;

        if line_cells >= width_cells {
            break;
        }

        while syntax_idx < spans.len() && spans[syntax_idx].end_byte <= start_byte {
            syntax_idx += 1;
        }

        let colors = syntax_color_for_range(
            base_color,
            style,
            &spans[syntax_idx..],
            start_byte,
            end_byte,
        );

        if grapheme == "\t" {
            flush_symbol_info_span(
                window,
                row,
                base_col,
                source_line,
                pending_start.take(),
                pending_end,
                pending_col,
                pending_colors,
            )?;
            let visible_width = grapheme_width.min(width_cells.saturating_sub(line_cells));
            let spaces = " ".repeat(visible_width);
            window.write_str_colored(
                row,
                base_col.saturating_add(line_cells as u16),
                &spaces,
                colors,
            )?;
            line_cells = line_cells.saturating_add(visible_width);
            continue;
        }

        if pending_start.is_some() && pending_colors == colors && pending_end == start_byte {
            pending_end = end_byte;
            line_cells = line_cells.saturating_add(grapheme_width);
            continue;
        }

        flush_symbol_info_span(
            window,
            row,
            base_col,
            source_line,
            pending_start.take(),
            pending_end,
            pending_col,
            pending_colors,
        )?;
        pending_start = Some(start_byte);
        pending_end = end_byte;
        pending_col = line_cells as u16;
        pending_colors = colors;
        line_cells = line_cells.saturating_add(grapheme_width);
    }

    flush_symbol_info_span(
        window,
        row,
        base_col,
        source_line,
        pending_start,
        pending_end,
        pending_col,
        pending_colors,
    )
}

fn flush_symbol_info_span(
    window: &mut WindowView<'_>,
    row: u16,
    base_col: u16,
    source_line: &str,
    start: Option<usize>,
    end: usize,
    col: u16,
    colors: ColorPair,
) -> minui::Result<()> {
    let Some(start) = start else {
        return Ok(());
    };
    if start >= end {
        return Ok(());
    }
    window.write_str_colored(
        row,
        base_col.saturating_add(col),
        &source_line[start..end],
        colors,
    )
}

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
    let diagnostics_active = popup.focus == DiagnosticsPopupFocus::Diagnostics;

    let detail_rows = if popup.code_actions.is_some() {
        0
    } else if view.height >= DIAGNOSTIC_DETAIL_MIN_HEIGHT {
        ((view.height as usize) / 3).clamp(3, DIAGNOSTIC_DETAIL_MAX_ROWS)
    } else {
        0
    };
    let split_reserved_rows = if popup.code_actions.is_some() { 6 } else { 0 };
    let reserved_rows = if split_reserved_rows > 0 {
        split_reserved_rows
    } else if detail_rows > 0 {
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
            let highlight = if diagnostics_active {
                style.finder.selected
            } else {
                style.finder.dim
            };
            view.write_str_colored(row, 1, &fill, highlight)?;
        }

        let highlight = if diagnostics_active {
            style.finder.selected
        } else {
            style.finder.dim
        };
        let row_colors = selection_aware_color(style.finder.text, highlight, selected);
        let dim_colors = selection_aware_color(style.finder.dim, highlight, selected);
        let severity_colors =
            selection_aware_color(severity_color(style, entry.severity), highlight, selected);

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
        let action_hint = if selected && popup.code_actions.is_none() {
            Some("[a]")
        } else {
            None
        };
        let action_hint_width = action_hint.map_or(0, |hint| text_width(hint) as u16);
        let message_w = message_w.saturating_sub(action_hint_width as usize + 1);
        let message = clip_text_to_cells(&entry.summary, message_w);
        view.write_str_colored(row, prefix_w as u16 + 1, &message, row_colors)?;
        if let Some(hint) = action_hint
            && action_hint_width.saturating_add(2) < view.width
        {
            let hint_col = view
                .width
                .saturating_sub(action_hint_width.saturating_add(1));
            view.write_str_colored(row, hint_col, hint, dim_colors)?;
        }
    }

    let separator_row = 1u16.saturating_add((end.saturating_sub(start)) as u16);
    if let Some(code_actions) = popup.code_actions.as_ref() {
        if separator_row < view.height {
            draw_diagnostics_code_actions_split(
                &mut view,
                separator_row,
                code_actions,
                popup.focus == DiagnosticsPopupFocus::CodeActions,
                style,
            )?;
        }
    } else if detail_rows > 0 {
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

pub fn draw_code_actions_popup(
    popup: &CodeActionPopup,
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
        CODE_ACTIONS_TITLE,
        PopupChrome {
            border: style.finder.border,
            title: style.finder.title,
            fill: style.finder.text,
        },
    )?;
    let mut view = popup_window_view(window, layout);
    let summary = format!("{} actions", popup.entries.len());
    let _ = draw_section_header(&mut view, 0, &summary, style.finder.query_title)?;

    if view.height > 2 {
        view.write_str_colored(
            1,
            1,
            &clip_text_to_cells(&popup.title, view.width.saturating_sub(2) as usize),
            style.finder.dim,
        )?;
    }

    let list_start_row = if view.height > 2 { 2 } else { 1 };
    draw_code_action_entries(
        &mut view,
        list_start_row,
        popup.entries.as_slice(),
        popup.selected,
        popup.scroll,
        true,
        style,
    )?;

    Ok(())
}

fn draw_diagnostics_code_actions_split(
    view: &mut WindowView<'_>,
    separator_row: u16,
    pane: &DiagnosticsCodeActionsPane,
    active: bool,
    style: UiStyle,
) -> minui::Result<()> {
    if separator_row >= view.height {
        return Ok(());
    }
    let divider = "─".repeat(view.width as usize);
    view.write_str_colored(separator_row, 0, &divider, style.finder.dim)?;
    let title_row = separator_row.saturating_add(1);
    if title_row >= view.height {
        return Ok(());
    }
    let title_colors = if active {
        style.finder.query_title
    } else {
        style.finder.dim
    };
    view.write_str_colored(
        title_row,
        1,
        &clip_text_to_cells(&pane.title, view.width.saturating_sub(2) as usize),
        title_colors,
    )?;
    if pane.loading {
        let message_row = title_row.saturating_add(1);
        if message_row < view.height {
            view.write_str_colored(
                message_row,
                1,
                &clip_text_to_cells(
                    "Loading quick fixes...",
                    view.width.saturating_sub(2) as usize,
                ),
                style.finder.dim,
            )?;
        }
        return Ok(());
    }
    let list_start_row = title_row.saturating_add(1);
    draw_code_action_entries(
        view,
        list_start_row,
        pane.entries.as_slice(),
        pane.selected,
        pane.scroll,
        active,
        style,
    )
}

fn draw_code_action_entries(
    view: &mut WindowView<'_>,
    list_start_row: u16,
    entries: &[crate::app::state::CodeActionPopupEntry],
    selected_index: usize,
    scroll: usize,
    active: bool,
    style: UiStyle,
) -> minui::Result<()> {
    let list_capacity = view.height.saturating_sub(list_start_row).max(1).min(12) as usize;
    let mut start = scroll.min(entries.len());
    if selected_index < start {
        start = selected_index;
    }
    if selected_index >= start.saturating_add(list_capacity) {
        start = selected_index
            .saturating_add(1)
            .saturating_sub(list_capacity);
    }
    let end = (start + list_capacity).min(entries.len());
    let highlight = if active {
        style.finder.selected
    } else {
        style.finder.dim
    };

    for (visible_idx, entry) in entries[start..end].iter().enumerate() {
        let row = list_start_row.saturating_add(visible_idx as u16);
        if row >= view.height {
            break;
        }
        let idx = start + visible_idx;
        let selected = idx == selected_index;
        if selected {
            let fill = " ".repeat(view.width.saturating_sub(2) as usize);
            view.write_str_colored(row, 1, &fill, highlight)?;
        }

        let row_colors = selection_aware_color(style.finder.text, highlight, selected);
        let dim_colors = selection_aware_color(style.finder.dim, highlight, selected);
        let marker = if selected { "› " } else { "  " };
        let badge = if entry.preferred { "★ " } else { "" };
        let title = format!("{badge}{}", entry.title);
        let kind = entry.kind.as_deref().unwrap_or("action");
        let kind_text = format!("[{kind}]");
        let kind_width = text_width(&kind_text) as u16;
        let kind_col = view.width.saturating_sub(kind_width.saturating_add(1));

        view.write_str_colored(row, 1, marker, dim_colors)?;
        let title_width = kind_col.saturating_sub(4) as usize;
        view.write_str_colored(row, 3, &clip_text_to_cells(&title, title_width), row_colors)?;
        if kind_col > 3 {
            view.write_str_colored(row, kind_col, &kind_text, dim_colors)?;
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
        DiagnosticSeverity::Error => ColorPair::new(style.diagnostic_inline.error.fg, popup_bg),
        DiagnosticSeverity::Warning => ColorPair::new(style.diagnostic_inline.warning.fg, popup_bg),
        DiagnosticSeverity::Information => {
            ColorPair::new(style.diagnostic_inline.information.fg, popup_bg)
        }
        DiagnosticSeverity::Hint => ColorPair::new(style.diagnostic_inline.hint.fg, popup_bg),
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

#[cfg(test)]
mod tests {
    use super::{
        build_symbol_info_display_lines, markdown_render_lines, plain_text_render_lines,
        symbol_info_line_spans, wrap_code_line_to_cells,
    };
    use crate::app::state::{SymbolInfoBlock, SymbolInfoDisplayKind, SymbolInfoKind};
    use crate::ui::syntax::SyntaxLanguage;

    #[test]
    fn markdown_render_lines_keeps_untyped_fenced_code_blocks_separate() {
        let lines = markdown_render_lines("Before\n```\nlet value = 1;\n```\nAfter", 80);

        assert_eq!(lines.len(), 3);
        assert!(matches!(lines[0].kind, SymbolInfoDisplayKind::Markdown));
        assert_eq!(lines[0].text, "Before");
        assert!(matches!(
            lines[1].kind,
            SymbolInfoDisplayKind::Code { language: None }
        ));
        assert_eq!(lines[1].text, "let value = 1;");
        assert!(matches!(lines[2].kind, SymbolInfoDisplayKind::Markdown));
        assert_eq!(lines[2].text, "After");
    }

    #[test]
    fn markdown_render_lines_treats_indented_blocks_as_code() {
        let lines =
            markdown_render_lines("Use mod.\n\n    mod foo {\n        mod bar {\n    }\n", 80);

        assert!(matches!(lines[0].kind, SymbolInfoDisplayKind::Markdown));
        assert_eq!(lines[0].text, "Use mod.");
        let code_lines = lines
            .iter()
            .filter(|line| matches!(line.kind, SymbolInfoDisplayKind::Code { language: None }))
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(code_lines, vec!["mod foo {", "    mod bar {", "}"]);
    }

    #[test]
    fn wrapped_markdown_segments_keep_spans_within_segment_bounds() {
        let lines = build_symbol_info_display_lines(
            &[SymbolInfoBlock {
                kind: SymbolInfoKind::Markdown,
                text: "Organize code into [modules](https://doc.rust-lang.org/stable/reference/items/modules.html).".to_string(),
            }],
            32,
        );

        let wrapped_markdown = lines
            .into_iter()
            .filter(|line| matches!(line.kind, SymbolInfoDisplayKind::Markdown))
            .collect::<Vec<_>>();
        assert!(wrapped_markdown.len() > 1);
        for line in wrapped_markdown {
            for span in line.spans {
                assert!(span.end_byte <= line.text.len());
            }
        }
    }

    #[test]
    fn wrapped_code_comment_tails_preserve_comment_colouring() {
        let lines = build_symbol_info_display_lines(
            &[SymbolInfoBlock {
                kind: SymbolInfoKind::Code {
                    language: Some("go".to_string()),
                },
                text: "position int // current position in input (points to current char)"
                    .to_string(),
            }],
            36,
        );

        let wrapped_code = lines
            .into_iter()
            .filter(|line| matches!(line.kind, SymbolInfoDisplayKind::Code { .. }))
            .collect::<Vec<_>>();
        assert!(wrapped_code.len() > 1);
        assert!(!wrapped_code[1].spans.is_empty());
        let first_role = wrapped_code[1].spans[0].role;
        assert!(
            wrapped_code[1]
                .spans
                .iter()
                .all(|span| span.role == first_role)
        );
        assert!(
            wrapped_code[1]
                .spans
                .iter()
                .any(|span| span.start_byte == 0 && span.end_byte == wrapped_code[1].text.len())
        );
    }

    #[test]
    fn wrap_code_line_to_cells_preserves_leading_indentation() {
        let wrapped = wrap_code_line_to_cells("    mod bar {", 80);
        assert_eq!(wrapped, vec!["    mod bar {".to_string()]);
    }

    #[test]
    fn plain_text_render_lines_treats_indented_blocks_as_code() {
        let lines =
            plain_text_render_lines("Use mod.\n\n    mod foo {\n        mod bar {\n    }\n", 80);
        let code_lines = lines
            .iter()
            .filter(|line| matches!(line.kind, SymbolInfoDisplayKind::Code { language: None }))
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(code_lines, vec!["mod foo {", "    mod bar {", "}"]);
    }

    #[test]
    fn untyped_code_blocks_render_monochromatically() {
        let spans = symbol_info_line_spans(
            "fn baz() {}",
            &SymbolInfoDisplayKind::Code { language: None },
        );
        assert!(spans.is_empty());

        let typed_spans = symbol_info_line_spans(
            "fn baz() {}",
            &SymbolInfoDisplayKind::Code {
                language: Some(SyntaxLanguage::Rust),
            },
        );
        assert!(!typed_spans.is_empty());
    }
}
