use minui::{ColorPair, TabPolicy, Window, cell_width};

use crate::app::{CompletionEntry, CompletionPopup};
use crate::ui::style::SyntaxRole;
use crate::ui::widgets::popup::wrap_text_to_cells;
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle};

const COMPLETION_VISIBLE_ROWS: usize = 8;
const COMPLETION_DOCUMENTATION_ROWS: usize = 4;
const COMPLETION_MIN_WIDTH: u16 = 32;
const COMPLETION_MIN_METADATA_WIDTH: u16 = 44;
const COMPLETION_MAX_WIDTH: u16 = 96;
const COMPLETION_SELECTOR_GAP: usize = 1;
const COMPLETION_TRAILING_PADDING: usize = 1;
const COMPLETION_COLUMN_GAP: usize = 4;
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
    let layout = completion_layout(popup, visible_rows, term_w);
    let width = layout.width;
    let documentation = completion_documentation_lines(popup, width);
    let doc_rows = if documentation.is_empty() {
        0
    } else {
        documentation.len().saturating_add(1)
    };
    let below_rows = text_bottom_y.saturating_sub(anchor_y.saturating_add(1)) as usize;
    let above_rows = anchor_y as usize;
    let frame_extra_rows = 2 + doc_rows;
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

    draw_frame(window, x, y, width, frame_h, style)?;
    draw_entries(window, popup, style, x, y, layout, capacity)?;
    if !documentation.is_empty() {
        draw_documentation(window, &documentation, style, x, y, width, capacity)?;
    }
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
    let color = ColorPair::new(style.theme.light_gray, style.theme.bg);
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
    type_width: usize,
    extra_width: usize,
}

fn completion_layout(
    popup: &CompletionPopup,
    visible_rows: usize,
    term_w: u16,
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
    let type_width = visible
        .clone()
        .filter_map(|entry| entry.type_label.as_deref())
        .map(text_width)
        .max()
        .unwrap_or(0)
        .min(30);
    let extra_width = visible
        .filter_map(|entry| entry.extra.as_deref())
        .map(text_width)
        .max()
        .unwrap_or(0)
        .min(18);

    let has_type = type_width > 0;
    let has_extra = extra_width > 0;
    let gaps = completion_metadata_gaps(type_width, extra_width);
    let content_width = 1
        + COMPLETION_SELECTOR_GAP
        + keyword_width
        + gaps
        + type_width
        + extra_width
        + COMPLETION_TRAILING_PADDING;
    let min_width = if has_type && has_extra {
        COMPLETION_MIN_METADATA_WIDTH
    } else {
        COMPLETION_MIN_WIDTH
    };
    let available = term_w.saturating_sub(2).max(1) as usize;
    let width = content_width
        .saturating_add(2)
        .clamp(min_width as usize, COMPLETION_MAX_WIDTH as usize)
        .min(available)
        .max(min_width.min(term_w) as usize) as u16;

    let inner_available = width.saturating_sub(2) as usize;
    let (keyword_width, type_width, extra_width) =
        fit_completion_columns(inner_available, keyword_width, type_width, extra_width);

    CompletionLayout {
        width,
        keyword_width,
        type_width,
        extra_width,
    }
}

fn fit_completion_columns(
    inner_available: usize,
    keyword_width: usize,
    type_width: usize,
    extra_width: usize,
) -> (usize, usize, usize) {
    let mut type_width = type_width;
    let mut extra_width = extra_width;

    loop {
        let base = 1 + COMPLETION_SELECTOR_GAP + COMPLETION_TRAILING_PADDING;
        let gaps = completion_metadata_gaps(type_width, extra_width);
        let metadata_budget =
            inner_available.saturating_sub(base + gaps + COMPLETION_MIN_KEYWORD_WIDTH);
        if type_width + extra_width <= metadata_budget {
            break;
        }

        if extra_width > 0 {
            let extra_budget = metadata_budget.saturating_sub(type_width);
            if extra_budget > 0 {
                extra_width = extra_width.min(extra_budget);
                break;
            } else {
                extra_width = 0;
            }
        } else if type_width > 0 {
            type_width = type_width.min(metadata_budget);
            if type_width == 0 {
                break;
            }
            break;
        } else {
            break;
        }
    }

    let fixed = completion_fixed_width(type_width, extra_width);
    let keyword_width = keyword_width
        .min(inner_available.saturating_sub(fixed))
        .max(COMPLETION_MIN_KEYWORD_WIDTH);

    (keyword_width, type_width, extra_width)
}

fn completion_fixed_width(type_width: usize, extra_width: usize) -> usize {
    1 + COMPLETION_SELECTOR_GAP
        + completion_metadata_gaps(type_width, extra_width)
        + type_width
        + extra_width
        + COMPLETION_TRAILING_PADDING
}

fn completion_metadata_gaps(type_width: usize, extra_width: usize) -> usize {
    usize::from(type_width > 0) * COMPLETION_COLUMN_GAP
        + usize::from(extra_width > 0) * COMPLETION_COLUMN_GAP
}

fn popup_visible_len(popup: &CompletionPopup) -> usize {
    popup
        .entries
        .len()
        .saturating_sub(popup.scroll.min(popup.entries.len()))
        .min(COMPLETION_VISIBLE_ROWS)
}

fn draw_frame(
    window: &mut dyn Window,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    style: UiStyle,
) -> minui::Result<()> {
    let border = style.finder.border;
    let fill = style.finder.text;
    window.write_str_colored(
        y,
        x,
        &format!("╭{}╮", "─".repeat(width.saturating_sub(2) as usize)),
        border,
    )?;
    for row in 1..height.saturating_sub(1) {
        window.write_str_colored(y + row, x, "│", border)?;
        window.write_str_colored(
            y + row,
            x + 1,
            &" ".repeat(width.saturating_sub(2) as usize),
            fill,
        )?;
        window.write_str_colored(y + row, x + width.saturating_sub(1), "│", border)?;
    }
    window.write_str_colored(
        y + height.saturating_sub(1),
        x,
        &format!("╰{}╯", "─".repeat(width.saturating_sub(2) as usize)),
        border,
    )?;
    Ok(())
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
        let keyword_style =
            completion_role_color(style, entry, is_selected, CompletionColumn::Keyword);
        let type_style = completion_role_color(style, entry, is_selected, CompletionColumn::Type);
        let extra_style = completion_role_color(style, entry, is_selected, CompletionColumn::Extra);
        let dim_style = selection_aware_color(style.finder.dim, selected_style, is_selected);
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
        window.write_str_colored(row, keyword_x, &keyword, keyword_style)?;

        let type_x = keyword_x + layout.keyword_width as u16 + COMPLETION_COLUMN_GAP as u16;
        if layout.type_width > 0
            && let Some(type_label) = &entry.type_label
        {
            let text = clip_text_to_cells(type_label, layout.type_width);
            window.write_str_colored(row, type_x, &text, type_style)?;
        }

        if layout.extra_width > 0
            && let Some(extra) = &entry.extra
        {
            let extra = clip_text_to_cells(extra, layout.extra_width);
            let extra_x = x + width
                .saturating_sub(1 + COMPLETION_TRAILING_PADDING as u16 + layout.extra_width as u16);
            window.write_str_colored(row, extra_x, &extra, extra_style)?;
        }
    }

    Ok(())
}

fn draw_documentation(
    window: &mut dyn Window,
    lines: &[String],
    style: UiStyle,
    x: u16,
    y: u16,
    width: u16,
    capacity: usize,
) -> minui::Result<()> {
    let row = y + capacity as u16 + 1;
    let separator = "─".repeat(width.saturating_sub(2) as usize);
    window.write_str_colored(row, x + 1, &separator, style.finder.border)?;
    let color = ColorPair::new(style.syntax.comment.fg, style.finder.text.bg);
    for (idx, line) in lines.iter().enumerate() {
        window.write_str_colored(row + 1 + idx as u16, x + 2, line, color)?;
    }
    Ok(())
}

fn completion_documentation_lines(popup: &CompletionPopup, width: u16) -> Vec<String> {
    let Some(documentation) = popup
        .entries
        .get(popup.selected)
        .and_then(|entry| entry.documentation.as_deref())
    else {
        return Vec::new();
    };
    wrap_text_to_cells(documentation, width.saturating_sub(4) as usize)
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .take(COMPLETION_DOCUMENTATION_ROWS)
        .collect()
}

#[derive(Debug, Clone, Copy)]
enum CompletionColumn {
    Keyword,
    Type,
    Extra,
}

fn completion_role_color(
    style: UiStyle,
    entry: &CompletionEntry,
    is_selected: bool,
    column: CompletionColumn,
) -> ColorPair {
    let role = match column {
        CompletionColumn::Keyword => completion_keyword_role(entry.kind.as_deref()),
        CompletionColumn::Type => SyntaxRole::Type,
        CompletionColumn::Extra => SyntaxRole::Comment,
    };
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

fn clip_text_to_cells(text: &str, max_cells: usize) -> String {
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = cell_width(&ch.to_string(), TabPolicy::Fixed(4)) as usize;
        if width.saturating_add(ch_width) > max_cells {
            break;
        }
        out.push(ch);
        width = width.saturating_add(ch_width);
    }
    out
}

fn text_width(text: &str) -> usize {
    text.chars()
        .map(|ch| cell_width(&ch.to_string(), TabPolicy::Fixed(4)) as usize)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn popup_with_metadata() -> CompletionPopup {
        CompletionPopup {
            entries: vec![CompletionEntry {
                kind: Some("function".to_string()),
                keyword: "very_long_completion_keyword".to_string(),
                type_label: Some("ExtremelyLongCompletionTypeName".to_string()),
                extra: Some("very_long_extra".to_string()),
                documentation: None,
            }],
            selected: 0,
            scroll: 0,
        }
    }

    #[test]
    fn completion_layout_drops_extra_before_overlapping_metadata_columns() {
        let layout = completion_layout(&popup_with_metadata(), 1, COMPLETION_MIN_METADATA_WIDTH);
        let inner_available = layout.width.saturating_sub(2) as usize;
        let used =
            layout.keyword_width + completion_fixed_width(layout.type_width, layout.extra_width);

        assert_eq!(layout.keyword_width, COMPLETION_MIN_KEYWORD_WIDTH);
        assert!(layout.type_width > 0);
        assert_eq!(layout.extra_width, 0);
        assert!(used <= inner_available);
    }
}
