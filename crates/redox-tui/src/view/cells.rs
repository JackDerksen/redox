//! Mapping character ranges on a line to the visible cells they cover.

use minui::prelude::render::{TabPolicy, cell_width};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn selected_visible_cells(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    sel_start_char: usize,
    sel_end_char_exclusive: usize,
) -> Vec<bool> {
    highlighted_visible_cells(
        source_line,
        scroll_x,
        width_cells,
        std::slice::from_ref(&(sel_start_char..sel_end_char_exclusive)),
    )
}

pub(crate) fn visible_content_cell_width(
    source_line: &str,
    scroll_x: usize,
    max_cells: usize,
) -> usize {
    if max_cells == 0 || source_line.is_empty() {
        return 0;
    }

    let mut consumed_cells = 0usize;
    let mut visible_cells = 0usize;

    for grapheme in source_line.graphemes(true) {
        let grapheme_width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
        if consumed_cells.saturating_add(grapheme_width) <= scroll_x {
            consumed_cells = consumed_cells.saturating_add(grapheme_width);
            continue;
        }

        let remaining_cells = max_cells.saturating_sub(visible_cells);
        if remaining_cells == 0 {
            break;
        }

        visible_cells = visible_cells.saturating_add(grapheme_width.min(remaining_cells));
        if visible_cells >= max_cells {
            break;
        }
    }

    visible_cells
}

pub(crate) fn clipped_cell_width(text: &str, max_cells: usize) -> usize {
    let mut width = 0usize;
    for grapheme in text.graphemes(true) {
        let ch_width = cell_width(grapheme, TabPolicy::Fixed(4)) as usize;
        if width.saturating_add(ch_width) > max_cells {
            break;
        }
        width = width.saturating_add(ch_width);
    }
    width
}

pub(crate) fn search_highlight_cells(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    ranges: &[std::ops::Range<usize>],
) -> Vec<bool> {
    let mut cells = highlighted_visible_cells(source_line, scroll_x, width_cells, ranges);
    for range in ranges.iter().filter(|range| range.is_empty()) {
        let byte = source_line
            .char_indices()
            .nth(range.start)
            .map_or(source_line.len(), |(byte, _)| byte);
        let start = crate::ui::helpers::text_width(&source_line[..byte]);
        mark_visible_cell_range(&mut cells, start, start.saturating_add(1), scroll_x);
    }
    cells
}

pub(crate) fn highlighted_visible_cells(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    ranges: &[std::ops::Range<usize>],
) -> Vec<bool> {
    let mut selected = vec![false; width_cells];
    if width_cells == 0 || source_line.is_empty() || ranges.is_empty() {
        return selected;
    }

    let mut line_cells = 0usize;
    let mut char_idx = 0usize;
    let max_visible_cell = scroll_x.saturating_add(width_cells);

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
        if start_cell >= max_visible_cell {
            break;
        }
        if !ranges
            .iter()
            .any(|range| start_char < range.end && end_char > range.start)
        {
            continue;
        }

        let visible_start = start_cell.max(scroll_x).saturating_sub(scroll_x);
        let visible_end = end_cell.min(max_visible_cell).saturating_sub(scroll_x);
        selected[visible_start..visible_end].fill(true);
    }

    selected
}

pub(crate) fn visual_selection_visible_cells(
    buffer: &redox_core::TextBuffer,
    source_line: &str,
    selection: redox_core::Selection,
    mode: redox_core::VisualModeKind,
    line_idx: usize,
    scroll_x: usize,
    width_cells: usize,
) -> Option<Vec<bool>> {
    let cells = match mode {
        redox_core::VisualModeKind::Line => {
            let (start_line, end_line) = selection.line_range();
            if line_idx < start_line || line_idx > end_line {
                return None;
            }
            // Keep a single cell visible for selected empty lines.
            visible_cell_range(
                0,
                crate::ui::helpers::text_width(source_line).max(1),
                scroll_x,
                width_cells,
            )
        }
        redox_core::VisualModeKind::Block => {
            let (start, end) = selection.ordered();
            if line_idx < start.line || line_idx > end.line {
                return None;
            }
            let left = start.col.min(end.col);
            let right = start.col.max(end.col).saturating_add(1);
            visible_cell_range(left, right, scroll_x, width_cells)
        }
        redox_core::VisualModeKind::Char => {
            let (start, end) = selection.ordered();
            if line_idx < start.line || line_idx > end.line {
                return None;
            }

            if let Some(range) =
                buffer.visual_selection_char_range_on_line(selection, mode, line_idx)
            {
                selected_visible_cells(source_line, scroll_x, width_cells, range.start, range.end)
            } else if start.line < end.line && source_line.is_empty() {
                visible_cell_range(0, 1, scroll_x, width_cells)
            } else {
                return None;
            }
        }
    };

    cells.iter().any(|cell| *cell).then_some(cells)
}

pub(crate) fn visible_cell_range(
    start_cell: usize,
    end_cell: usize,
    scroll_x: usize,
    width_cells: usize,
) -> Vec<bool> {
    let mut cells = vec![false; width_cells];
    mark_visible_cell_range(&mut cells, start_cell, end_cell, scroll_x);
    cells
}

pub(crate) fn mark_visible_cell_range(
    cells: &mut [bool],
    start_cell: usize,
    end_cell: usize,
    scroll_x: usize,
) {
    if cells.is_empty() || start_cell >= end_cell {
        return;
    }

    let visible_start = start_cell.saturating_sub(scroll_x);
    let visible_end = end_cell.saturating_sub(scroll_x).min(cells.len());
    if visible_start >= visible_end {
        return;
    }

    for cell in &mut cells[visible_start..visible_end] {
        *cell = true;
    }
}

pub(crate) fn occupied_visible_cells(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
) -> Vec<bool> {
    let mut occupied = vec![false; width_cells];
    if width_cells == 0 || source_line.is_empty() {
        return occupied;
    }

    let mut line_cells = 0usize;
    let max_visible_cell = scroll_x.saturating_add(width_cells);

    for g in source_line.graphemes(true) {
        if g.chars().all(char::is_whitespace) {
            let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
            line_cells = line_cells.saturating_add(g_width.max(1));
            continue;
        }

        let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width.max(1));
        line_cells = end_cell;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell >= max_visible_cell {
            break;
        }

        let visible_start = start_cell.saturating_sub(scroll_x);
        let visible_end = end_cell.min(max_visible_cell).saturating_sub(scroll_x);
        for cell in visible_start..visible_end.min(occupied.len()) {
            occupied[cell] = true;
        }
    }

    occupied
}
