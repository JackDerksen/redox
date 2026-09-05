use std::ops::Range;

use minui::{ColorPair, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{CompletionEntry, CompletionPopup};
use crate::ui::icons::completion_kind_icon;
use crate::ui::style::SyntaxRole;
use crate::ui::widgets::popup::{PopupChrome, clip_text_to_cells, draw_popup_frame_at};
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle};

const COMPLETION_VISIBLE_ROWS: usize = 8;
const COMPLETION_MIN_WIDTH: u16 = 28;
const COMPLETION_MAX_WIDTH: u16 = 72;
const COMPLETION_SELECTOR_GAP: usize = 1;
const COMPLETION_TRAILING_PADDING: usize = 1;
const COMPLETION_KIND_GAP: usize = 2;
const COMPLETION_MIN_KEYWORD_WIDTH: usize = 8;

pub fn draw_completion_popup(
    popup: &CompletionPopup,
    style: UiStyle,
    window: &mut dyn Window,
    anchor_x: u16,
    anchor_y: u16,
    text_bottom_y: u16,
) -> minui::Result<()> {
    if popup.entries.is_empty() {
        return Ok(());
    }

    let (term_w, term_h) = window.get_size();
    let text_bottom_y = text_bottom_y.min(term_h.saturating_sub(STATUS_BAR_HEIGHT_CELLS));
    if term_w < COMPLETION_MIN_WIDTH || text_bottom_y < 3 {
        return Ok(());
    }

    let visible_rows = popup_visible_len(popup);
    let layout = completion_layout(popup, visible_rows, term_w, style.icons_enabled);
    let width = layout.width;
    let below_rows = text_bottom_y.saturating_sub(anchor_y.saturating_add(1)) as usize;
    let above_rows = anchor_y as usize;
    let frame_extra_rows = 2;
    let below_capacity = below_rows
        .saturating_sub(frame_extra_rows)
        .min(visible_rows);
    let above_capacity = above_rows
        .saturating_sub(frame_extra_rows)
        .min(visible_rows);
    let draw_below = below_capacity >= above_capacity || above_capacity == 0;
    let capacity = if draw_below {
        below_capacity
    } else {
        above_capacity
    };
    if capacity == 0 {
        return Ok(());
    }

    let frame_h = capacity.saturating_add(frame_extra_rows) as u16;
    let y = if draw_below {
        anchor_y.saturating_add(1)
    } else {
        anchor_y.saturating_sub(frame_h)
    };
    let x = anchor_x.min(term_w.saturating_sub(width));

    draw_popup_frame_at(
        window,
        x,
        y,
        width.saturating_sub(2),
        frame_h.saturating_sub(2),
        "",
        PopupChrome::finder(style),
    )?;
    draw_entries(window, popup, style, x, y, layout, capacity)?;
    Ok(())
}

pub fn draw_completion_preview(
    window: &mut dyn Window,
    style: UiStyle,
    x: u16,
    y: u16,
    available_width: u16,
    text: &str,
    suffix: &str,
) -> minui::Result<()> {
    if text.is_empty() || available_width == 0 {
        return Ok(());
    }
    let preview = clip_text_to_cells(text, available_width as usize);
    if preview.is_empty() {
        return Ok(());
    }
    let color = ColorPair::new(style.syntax.comment.fg, style.theme.bg);
    window.write_str_colored(y, x, &preview, color)?;

    let used_width = text_width(&preview) as u16;
    let remaining_width = available_width.saturating_sub(used_width);
    if remaining_width == 0 || suffix.is_empty() {
        return Ok(());
    }
    let suffix = clip_text_to_cells(suffix, remaining_width as usize);
    if suffix.is_empty() {
        return Ok(());
    }
    window.write_str_colored(
        y,
        x.saturating_add(used_width),
        &suffix,
        ColorPair::new(style.theme.white, style.theme.bg),
    )
}

#[derive(Debug, Clone, Copy)]
struct CompletionLayout {
    width: u16,
    keyword_width: usize,
    kind_width: usize,
}

fn completion_layout(
    popup: &CompletionPopup,
    visible_rows: usize,
    term_w: u16,
    icons_enabled: bool,
) -> CompletionLayout {
    let start = popup.scroll.min(popup.entries.len());
    let end = start.saturating_add(visible_rows).min(popup.entries.len());
    let visible = popup.entries[start..end].iter();
    let keyword_width = visible
        .clone()
        .map(|entry| text_width(&entry.keyword))
        .max()
        .unwrap_or(12)
        .min(36);
    let kind_width = visible
        .filter_map(|entry| completion_kind_display(entry.kind.as_deref(), icons_enabled))
        .map(text_width)
        .max()
        .unwrap_or(0)
        .min(12);
    let kind_gap = usize::from(kind_width > 0) * COMPLETION_KIND_GAP;
    let content_width = 1
        + COMPLETION_SELECTOR_GAP
        + keyword_width
        + kind_gap
        + kind_width
        + COMPLETION_TRAILING_PADDING;
    let available = term_w.saturating_sub(2).max(1) as usize;
    let width = content_width
        .saturating_add(2)
        .clamp(COMPLETION_MIN_WIDTH as usize, COMPLETION_MAX_WIDTH as usize)
        .min(available)
        .max(COMPLETION_MIN_WIDTH.min(term_w) as usize) as u16;

    let inner_available = width.saturating_sub(2) as usize;
    let fixed_width = completion_fixed_width(kind_width);
    let keyword_width = keyword_width
        .min(inner_available.saturating_sub(fixed_width))
        .max(COMPLETION_MIN_KEYWORD_WIDTH);

    CompletionLayout {
        width,
        keyword_width,
        kind_width,
    }
}

fn completion_fixed_width(kind_width: usize) -> usize {
    1 + COMPLETION_SELECTOR_GAP
        + usize::from(kind_width > 0) * COMPLETION_KIND_GAP
        + kind_width
        + COMPLETION_TRAILING_PADDING
}

fn completion_kind_display(kind: Option<&str>, icons_enabled: bool) -> Option<&str> {
    if icons_enabled {
        kind.and_then(completion_kind_icon)
    } else {
        kind
    }
}

fn popup_visible_len(popup: &CompletionPopup) -> usize {
    popup
        .entries
        .len()
        .saturating_sub(popup.scroll.min(popup.entries.len()))
        .min(COMPLETION_VISIBLE_ROWS)
}

fn draw_entries(
    window: &mut dyn Window,
    popup: &CompletionPopup,
    style: UiStyle,
    x: u16,
    y: u16,
    layout: CompletionLayout,
    capacity: usize,
) -> minui::Result<()> {
    let selected_style = style.finder.selected;
    let entries: &[CompletionEntry] = &popup.entries;
    let width = layout.width;
    let start = popup.scroll.min(entries.len());
    let end = (start + capacity).min(entries.len());

    for (visible_idx, entry) in entries[start..end].iter().enumerate() {
        let idx = start + visible_idx;
        let row = y + 1 + visible_idx as u16;
        let is_selected = idx == popup.selected;
        let kind_style = completion_kind_color(style, entry, is_selected);
        let dim_style = selection_aware_color(style.finder.dim, selected_style, is_selected);
        let row_background = if is_selected {
            selected_style.bg
        } else {
            style.finder.text.bg
        };
        let keyword_style = ColorPair::new(style.theme.light_gray, row_background);
        let match_style = ColorPair::new(style.theme.white, row_background);
        let marker = if is_selected { "›" } else { " " };
        if is_selected {
            window.write_str_colored(
                row,
                x + 1,
                &" ".repeat(width.saturating_sub(2) as usize),
                selected_style,
            )?;
        }
        window.write_str_colored(row, x + 1, marker, dim_style)?;

        let keyword_x = x + 2 + COMPLETION_SELECTOR_GAP as u16;
        let keyword = clip_text_to_cells(&entry.keyword, layout.keyword_width);
        draw_completion_keyword(
            window,
            row,
            keyword_x,
            &keyword,
            &entry.highlights,
            keyword_style,
            match_style,
        )?;

        if layout.kind_width > 0
            && let Some(kind) = completion_kind_display(entry.kind.as_deref(), style.icons_enabled)
        {
            let kind = clip_text_to_cells(kind, layout.kind_width);
            let kind_x = x + width
                .saturating_sub(1 + COMPLETION_TRAILING_PADDING as u16 + layout.kind_width as u16);
            window.write_str_colored(row, kind_x, &kind, kind_style)?;
        }
    }

    Ok(())
}

fn draw_completion_keyword(
    window: &mut dyn Window,
    row: u16,
    start_col: u16,
    keyword: &str,
    highlights: &[Range<usize>],
    base: ColorPair,
    highlighted: ColorPair,
) -> minui::Result<()> {
    let mut segment_start = 0usize;
    let mut segment_highlighted = None;
    let mut column = start_col;
    for (byte_index, grapheme) in keyword.grapheme_indices(true) {
        let grapheme_end = byte_index.saturating_add(grapheme.len());
        let is_highlighted = highlights
            .iter()
            .any(|range| byte_index < range.end && grapheme_end > range.start);
        if let Some(current) = segment_highlighted
            && current != is_highlighted
        {
            let segment = &keyword[segment_start..byte_index];
            window.write_str_colored(
                row,
                column,
                segment,
                if current { highlighted } else { base },
            )?;
            column = column.saturating_add(text_width(segment) as u16);
            segment_start = byte_index;
        }
        segment_highlighted = Some(is_highlighted);
    }
    if let Some(is_highlighted) = segment_highlighted {
        window.write_str_colored(
            row,
            column,
            &keyword[segment_start..],
            if is_highlighted { highlighted } else { base },
        )?;
    }
    Ok(())
}

fn completion_kind_color(style: UiStyle, entry: &CompletionEntry, is_selected: bool) -> ColorPair {
    let role = completion_keyword_role(entry.kind.as_deref());
    let mut color = style.syntax.color_for(role);
    color.bg = if is_selected {
        style.finder.selected.bg
    } else {
        style.finder.text.bg
    };
    color
}

fn completion_keyword_role(kind: Option<&str>) -> SyntaxRole {
    match kind {
        Some("keyword") | Some("operator") => SyntaxRole::Keyword,
        Some("class") | Some("interface") | Some("struct") | Some("type") => SyntaxRole::Type,
        Some("constructor") => SyntaxRole::Constructor,
        Some("function") => SyntaxRole::Function,
        Some("method") => SyntaxRole::FunctionMethod,
        Some("field") | Some("property") => SyntaxRole::Property,
        Some("constant") => SyntaxRole::Constant,
        Some("variable") => SyntaxRole::VariableParameter,
        Some("snippet") => SyntaxRole::FunctionMacro,
        Some("module") => SyntaxRole::KeywordImport,
        _ => SyntaxRole::VariableParameter,
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
    text.chars()
        .map(|ch| cell_width(&ch.to_string(), TabPolicy::Fixed(4)) as usize)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn popup_with_kind() -> CompletionPopup {
        CompletionPopup {
            entries: vec![CompletionEntry {
                kind: Some("function".to_string()),
                keyword: "very_long_completion_keyword".to_string(),
                highlights: vec![0..4],
            }],
            selected: 0,
            scroll: 0,
        }
    }

    #[test]
    fn completion_layout_keeps_keyword_and_kind_columns_separate() {
        let layout = completion_layout(&popup_with_kind(), 1, COMPLETION_MIN_WIDTH, false);
        let inner_available = layout.width.saturating_sub(2) as usize;
        let used = layout.keyword_width + completion_fixed_width(layout.kind_width);

        assert!(layout.keyword_width >= COMPLETION_MIN_KEYWORD_WIDTH);
        assert!(layout.kind_width > 0);
        assert!(used <= inner_available);
    }
}
