use minui::widgets::WindowView;
use minui::{Color, ColorPair, TabPolicy, Window, cell_width, window::CursorSpec};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{FinderPopup, FinderPreview, PinSelectorPopup};
use crate::ui::UiStyle;
use crate::ui::style::{FinderStyle, StatusModuleKind};
use crate::ui::widgets::popup::{
    PopupChrome, clip_text_to_cells, draw_popup_frame_at, popup_inner_size, popup_window_view,
};
use crate::ui::widgets::status_bar::{
    STATUS_MODULE_EDGE_LEFT, STATUS_MODULE_EDGE_RIGHT, scroll_minimap_cell,
};

const FINDER_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);
const PREVIEW_THRESHOLD_COLS: u16 = 130;
const QUERY_TITLE: &str = "Finder";
const PREVIEW_TITLE: &str = "File Preview";
const PIN_SELECTOR_TITLE: &str = "Pinboard";
const PIN_MARKER: &str = "↦ ";
const SELECTED_MARKER: &str = "›";
const VACANT_SLOT_LABEL: &str = "<empty>";
const QUERY_GAP_ROWS: u16 = 0;
const FINDER_POPUP_EXPAND_CELLS: u16 = 1;
const ENTRY_MARKER_COL: u16 = 1;
const ENTRY_LABEL_COL: u16 = 3;
const PIN_SELECTOR_HORIZONTAL_PADDING: u16 = 1;
const PINBOARD_MIN_WIDTH: u16 = 24;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FinderFrameLayout {
    x: u16,
    y: u16,
    inner_w: u16,
    inner_h: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FinderPopupLayout {
    results: FinderFrameLayout,
    query: FinderFrameLayout,
    preview: Option<FinderFrameLayout>,
}

struct HighlightedText<'a> {
    row: u16,
    col: u16,
    text: &'a str,
    highlights: &'a [Range<usize>],
    max_cells: usize,
    base: ColorPair,
    highlighted: ColorPair,
}

struct FinderFooterIndicator {
    left_edge: &'static str,
    glyph: &'static str,
    right_edge: &'static str,
    wrapper_colors: ColorPair,
    content_colors: ColorPair,
}

struct FinderRightFooter {
    text: String,
    text_colors: ColorPair,
    indicator: Option<FinderFooterIndicator>,
}

impl FinderRightFooter {
    fn width(&self) -> u16 {
        text_width(&self.text) as u16 + self.indicator.as_ref().map_or(0, |_| 3)
    }
}

pub fn draw_finder_popup(
    popup: &FinderPopup,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let (term_w, term_h) = window.get_size();
    let (combined_inner_w, combined_inner_h) = popup_inner_size(
        term_w,
        term_h,
        style.finder.width_percent,
        style.finder.height_percent,
        style.finder.min_width,
        style.finder.min_height,
    );
    let combined_inner_w =
        expanded_finder_inner_size(combined_inner_w, term_w, FINDER_POPUP_EXPAND_CELLS);
    let combined_inner_h =
        expanded_finder_inner_size(combined_inner_h, term_h, FINDER_POPUP_EXPAND_CELLS);
    let show_preview =
        term_w >= PREVIEW_THRESHOLD_COLS && popup.preview.is_some() && combined_inner_w >= 64;
    let layout = compute_finder_popup_layout(
        term_w,
        term_h,
        combined_inner_w,
        combined_inner_h,
        show_preview,
    );

    let list_layout = draw_popup_frame_at(
        window,
        layout.results.x,
        layout.results.y,
        layout.results.inner_w,
        layout.results.inner_h,
        "",
        PopupChrome {
            border: style.finder.border,
            title: style.finder.title,
            fill: style.finder.text,
        },
    )?;
    let mut list_view = popup_window_view(window, list_layout);
    draw_entries(&mut list_view, popup, style.finder)?;

    if let Some(preview_frame) = layout.preview {
        let preview_layout = draw_popup_frame_at(
            window,
            preview_frame.x,
            preview_frame.y,
            preview_frame.inner_w,
            preview_frame.inner_h,
            PREVIEW_TITLE,
            PopupChrome {
                border: style.finder.border,
                title: style.finder.preview_title,
                fill: style.finder.text,
            },
        )?;
        let mut preview_view = popup_window_view(window, preview_layout);
        if let Some(preview) = &popup.preview {
            draw_preview(&mut preview_view, preview, style.finder)?;
        }
    }

    let query_layout = draw_popup_frame_at(
        window,
        layout.query.x,
        layout.query.y,
        layout.query.inner_w,
        layout.query.inner_h,
        QUERY_TITLE,
        PopupChrome {
            border: style.command_line.border,
            title: style.finder.query_title,
            fill: style.command_line.text,
        },
    )?;
    let mut query_view = popup_window_view(window, query_layout);
    let right_footer = finder_right_footer(popup, style);
    let input_col = draw_query_row(&mut query_view, popup, style, &right_footer)?;
    let right_w = visible_right_footer_width(&right_footer, layout.query.inner_w);
    let input_w = layout
        .query
        .inner_w
        .saturating_sub(right_w)
        .saturating_sub(input_col.saturating_add(1))
        .max(1) as usize;
    let cursor_offset = finder_input_cursor_offset(&popup.query, input_w);
    window.request_cursor(CursorSpec {
        x: query_layout
            .x
            .saturating_add(1)
            .saturating_add(input_col)
            .saturating_add(cursor_offset as u16),
        y: query_layout.y.saturating_add(1),
        visible: true,
    });

    Ok(())
}

fn compute_finder_popup_layout(
    term_w: u16,
    term_h: u16,
    combined_inner_w: u16,
    combined_inner_h: u16,
    show_preview: bool,
) -> FinderPopupLayout {
    let query_inner_h = 1u16;
    let preview_gap = 0u16;
    let combined_outer_w = combined_inner_w.saturating_add(2);
    let combined_outer_h = combined_inner_h.saturating_add(2);
    let x = term_w.saturating_sub(combined_outer_w) / 2;
    let y = term_h.saturating_sub(combined_outer_h) / 2;

    let left_total_inner_w = if show_preview {
        let split_inner_w = combined_outer_w
            .saturating_sub(preview_gap)
            .saturating_sub(4)
            .max(2);
        let preview_inner_w = (split_inner_w.saturating_mul(2) / 5).max(1);
        split_inner_w.saturating_sub(preview_inner_w).max(1)
    } else {
        combined_inner_w.max(1)
    };

    let left_total_outer_h = combined_outer_h;
    let results_inner_h = left_total_outer_h
        .saturating_sub(QUERY_GAP_ROWS)
        .saturating_sub(query_inner_h)
        .saturating_sub(4)
        .max(1);

    let results = FinderFrameLayout {
        x,
        y,
        inner_w: left_total_inner_w,
        inner_h: results_inner_h,
    };
    let query = FinderFrameLayout {
        x,
        y: y.saturating_add(results_inner_h)
            .saturating_add(2)
            .saturating_add(QUERY_GAP_ROWS),
        inner_w: left_total_inner_w,
        inner_h: query_inner_h,
    };

    let preview = if show_preview {
        let preview_x = x
            .saturating_add(left_total_inner_w)
            .saturating_add(2)
            .saturating_add(preview_gap);
        let preview_inner_w = combined_outer_w
            .saturating_sub(preview_x.saturating_sub(x))
            .saturating_sub(2)
            .max(1);
        Some(FinderFrameLayout {
            x: preview_x,
            y,
            inner_w: preview_inner_w,
            inner_h: combined_inner_h.max(1),
        })
    } else {
        None
    };

    FinderPopupLayout {
        results,
        query,
        preview,
    }
}

fn expanded_finder_inner_size(inner: u16, terminal_cells: u16, expand_cells: u16) -> u16 {
    let max_inner = terminal_cells.saturating_sub(2).max(1);
    inner
        .saturating_add(expand_cells.saturating_mul(2))
        .min(max_inner)
        .max(1)
}

pub fn draw_pin_selector_popup(
    popup: &PinSelectorPopup,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let (term_w, term_h) = window.get_size();
    let inner_w = compute_pin_selector_inner_width(popup, term_w);
    let inner_h = popup.slots.len() as u16 + 2;

    let layout = draw_popup_frame_at(
        window,
        term_w.saturating_sub(inner_w.saturating_add(2)) / 2,
        term_h.saturating_sub(inner_h.saturating_add(2)) / 2,
        inner_w,
        inner_h,
        PIN_SELECTOR_TITLE,
        PopupChrome {
            border: style.finder.border,
            title: style.finder.title,
            fill: ColorPair::new(style.finder.text.fg, Color::Transparent),
        },
    )?;
    let mut view = popup_window_view(window, layout);
    view.write_str_colored(
        0,
        PIN_SELECTOR_HORIZONTAL_PADDING,
        &clip_text_to_cells(
            &popup.path_label,
            inner_w.saturating_sub(PIN_SELECTOR_HORIZONTAL_PADDING.saturating_mul(2)) as usize,
        ),
        ColorPair::new(style.finder.dim.fg, Color::Transparent),
    )?;

    for (idx, slot) in popup.slots.iter().enumerate() {
        let row = idx as u16 + 1;
        let selected = idx == popup.selected;
        let row_colors = if selected {
            style.finder.selected
        } else if slot.path_label.is_some() {
            style.finder.text
        } else {
            style.finder.dim
        };
        let bg = if selected {
            style.finder.selected
        } else {
            ColorPair::new(row_colors.fg, Color::Transparent)
        };
        let blank = " ".repeat(inner_w as usize);
        view.write_str_colored(row, 0, &blank, bg)?;
        if selected {
            view.write_str_colored(row, ENTRY_MARKER_COL, SELECTED_MARKER, bg)?;
        }
        let value = slot.path_label.as_deref().unwrap_or(VACANT_SLOT_LABEL);
        let hotkey = format!("Ctrl+{}", slot.slot + 1);
        let hotkey_w = hotkey.chars().count() as u16;
        let hotkey_col = inner_w
            .saturating_sub(PIN_SELECTOR_HORIZONTAL_PADDING)
            .saturating_sub(hotkey_w);
        let value_w = hotkey_col
            .saturating_sub(ENTRY_LABEL_COL)
            .saturating_sub(1)
            .max(1) as usize;
        view.write_str_colored(
            row,
            ENTRY_LABEL_COL,
            &clip_text_to_cells(value, value_w),
            bg,
        )?;
        if hotkey_w.saturating_add(PIN_SELECTOR_HORIZONTAL_PADDING) < inner_w {
            let hotkey_color = if selected {
                bg
            } else if slot.path_label.is_some() {
                ColorPair::new(style.finder.hotkey.fg, Color::Transparent)
            } else {
                ColorPair::new(style.finder.dim.fg, Color::Transparent)
            };
            view.write_str_colored(row, hotkey_col, &hotkey, hotkey_color)?;
        }
    }

    window.request_cursor(CursorSpec {
        x: 0,
        y: 0,
        visible: false,
    });
    Ok(())
}

fn compute_pin_selector_inner_width(popup: &PinSelectorPopup, term_w: u16) -> u16 {
    compute_pin_selector_inner_width_from_labels(
        &popup.path_label,
        popup.slots.iter().map(|slot| slot.path_label.as_deref()),
        term_w,
    )
}

fn compute_pin_selector_inner_width_from_labels<'a>(
    path_label: &str,
    slot_labels: impl IntoIterator<Item = Option<&'a str>>,
    term_w: u16,
) -> u16 {
    let max_inner_w = term_w.saturating_sub(4).max(1);
    let slot_w = slot_labels
        .into_iter()
        .map(|label| {
            let label = label.unwrap_or(VACANT_SLOT_LABEL);
            text_width(label)
                .saturating_add(12)
                .saturating_add(PIN_SELECTOR_HORIZONTAL_PADDING as usize * 2)
        })
        .max()
        .unwrap_or(20)
        .min(max_inner_w as usize) as u16;
    let candidate_w = text_width(path_label)
        .saturating_add(PIN_SELECTOR_HORIZONTAL_PADDING as usize * 2)
        .min(max_inner_w as usize) as u16;

    slot_w
        .max(candidate_w)
        .max(PINBOARD_MIN_WIDTH)
        .min(max_inner_w)
}

fn draw_entries(
    view: &mut WindowView<'_>,
    popup: &FinderPopup,
    style: FinderStyle,
) -> minui::Result<()> {
    if popup.entries.is_empty() {
        view.write_str_colored(0, 0, "<no matches>", style.dim)?;
        return Ok(());
    }

    let pinned_count = popup
        .entries
        .iter()
        .take_while(|entry| entry.is_pinned)
        .count();
    let rows = visible_entry_rows(
        popup.entries.len(),
        pinned_count,
        popup.selected,
        view.height as usize,
    );
    for (actual_index, screen_row) in rows {
        let entry = &popup.entries[actual_index];
        let selected = actual_index == popup.selected;
        let row = screen_row as u16;
        let base = if selected {
            style.selected
        } else if entry.is_pinned {
            style.pinned_bg
        } else {
            style.text
        };
        let blank = " ".repeat(view.width as usize);
        view.write_str_colored(row, 0, &blank, base)?;

        let marker = if selected {
            SELECTED_MARKER
        } else if entry.is_pinned {
            PIN_MARKER
        } else {
            " "
        };
        let marker_color = if selected {
            base
        } else if entry.is_pinned {
            style.pinned_marker
        } else {
            style.dim
        };
        view.write_str_colored(row, ENTRY_MARKER_COL, marker, marker_color)?;

        let hotkey_w = entry
            .hotkey
            .as_ref()
            .map(|hotkey| hotkey.chars().count() as u16 + 1)
            .unwrap_or(0);
        if let Some(hotkey) = &entry.hotkey
            && hotkey_w < view.width
        {
            view.write_str_colored(
                row,
                view.width.saturating_sub(hotkey_w),
                hotkey,
                if selected { base } else { style.hotkey },
            )?;
        }

        let text_w = view
            .width
            .saturating_sub(ENTRY_LABEL_COL)
            .saturating_sub(hotkey_w)
            .max(1) as usize;
        draw_highlighted_text(
            view,
            HighlightedText {
                row,
                col: ENTRY_LABEL_COL,
                text: &entry.label,
                highlights: &entry.highlights,
                max_cells: text_w,
                base,
                highlighted: ColorPair::new(style.match_highlight.fg, base.bg),
            },
        )?;
    }

    Ok(())
}

fn visible_entry_rows(
    entry_count: usize,
    pinned_count: usize,
    selected: usize,
    visible_rows: usize,
) -> Vec<(usize, usize)> {
    if entry_count == 0 || visible_rows == 0 {
        return Vec::new();
    }

    let visible_pinned = pinned_count.min(visible_rows);
    let remaining_rows = visible_rows.saturating_sub(visible_pinned);
    let file_count = entry_count.saturating_sub(pinned_count);

    let mut rows = Vec::with_capacity(entry_count.min(visible_rows));

    for idx in 0..visible_pinned {
        rows.push((idx, idx));
    }

    if remaining_rows == 0 || file_count == 0 {
        return rows;
    }

    let selected_file_idx = selected.checked_sub(pinned_count);
    let file_window_start = match selected_file_idx {
        Some(selected_file_idx) if file_count > remaining_rows => {
            let max_start = file_count.saturating_sub(remaining_rows);
            selected_file_idx
                .saturating_sub(remaining_rows.saturating_sub(1).min(remaining_rows / 2))
                .min(max_start)
        }
        _ if file_count > remaining_rows => file_count.saturating_sub(remaining_rows),
        _ => 0,
    };

    let visible_file_count = file_count
        .saturating_sub(file_window_start)
        .min(remaining_rows);
    let file_row_start = visible_rows.saturating_sub(visible_file_count);

    for visible_idx in 0..visible_file_count {
        rows.push((
            pinned_count + file_window_start + visible_idx,
            file_row_start + visible_idx,
        ));
    }

    rows
}

fn draw_preview(
    view: &mut WindowView<'_>,
    preview: &FinderPreview,
    style: FinderStyle,
) -> minui::Result<()> {
    view.write_str_colored(
        0,
        0,
        &clip_text_to_cells(&preview.title, view.width as usize),
        style.preview_path,
    )?;
    for (idx, line) in preview
        .lines
        .iter()
        .take(view.height.saturating_sub(1) as usize)
        .enumerate()
    {
        view.write_str_colored(
            idx as u16 + 1,
            0,
            &clip_text_to_cells(line, view.width as usize),
            style.text,
        )?;
    }
    Ok(())
}

fn draw_query_row(
    view: &mut WindowView<'_>,
    popup: &FinderPopup,
    style: UiStyle,
    right_footer: &FinderRightFooter,
) -> minui::Result<u16> {
    let prompt_col = 1u16.min(view.width.saturating_sub(1));
    view.write_str_colored(0, prompt_col, "❯", style.finder.prompt)?;
    let right_w = visible_right_footer_width(right_footer, view.width);
    if right_w > 0 {
        let footer_col = view.width.saturating_sub(right_w);
        view.write_str_colored(0, footer_col, &right_footer.text, right_footer.text_colors)?;
        if let Some(indicator) = &right_footer.indicator {
            let module_col = footer_col.saturating_add(text_width(&right_footer.text) as u16);
            view.write_str_colored(0, module_col, indicator.left_edge, indicator.wrapper_colors)?;
            view.write_str_colored(
                0,
                module_col.saturating_add(1),
                indicator.glyph,
                indicator.content_colors,
            )?;
            view.write_str_colored(
                0,
                module_col.saturating_add(2),
                indicator.right_edge,
                indicator.wrapper_colors,
            )?;
        }
    }
    let input_col = prompt_col.saturating_add(2);
    let input_w = view
        .width
        .saturating_sub(right_w)
        .saturating_sub(input_col.saturating_add(1))
        .max(1) as usize;
    let clipped = finder_input_view(&popup.query, input_w);
    view.write_str_colored(0, input_col, &clipped, style.command_line.text)?;
    Ok(input_col)
}

fn visible_right_footer_width(right_footer: &FinderRightFooter, available_w: u16) -> u16 {
    let right_w = right_footer.width();
    if right_w < available_w { right_w } else { 0 }
}

fn draw_highlighted_text(
    view: &mut WindowView<'_>,
    spec: HighlightedText<'_>,
) -> minui::Result<()> {
    if spec.max_cells == 0 {
        return Ok(());
    }

    let clipped = clip_text_to_cells(spec.text, spec.max_cells);
    if spec.highlights.is_empty() {
        view.write_str_colored(spec.row, spec.col, &clipped, spec.base)?;
        return Ok(());
    }

    let mut cursor_col = spec.col;
    let mut byte_idx = 0usize;
    for grapheme in clipped.graphemes(true) {
        let next_byte = byte_idx.saturating_add(grapheme.len());
        let is_highlighted = spec
            .highlights
            .iter()
            .any(|range| byte_idx < range.end && next_byte > range.start);
        view.write_str_colored(
            spec.row,
            cursor_col,
            grapheme,
            if is_highlighted {
                spec.highlighted
            } else {
                spec.base
            },
        )?;
        cursor_col = cursor_col.saturating_add(text_width(grapheme) as u16);
        byte_idx = next_byte;
    }
    Ok(())
}

fn finder_input_view(text: &str, max_cells: usize) -> String {
    if text_width(text) <= max_cells {
        return clip_text_to_cells(text, max_cells);
    }

    let mut used = 0usize;
    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    let mut start = graphemes.len();
    while start > 0 {
        let grapheme = graphemes[start - 1];
        let grapheme_w = text_width(grapheme).max(1);
        if used + grapheme_w > max_cells {
            break;
        }
        used += grapheme_w;
        start -= 1;
    }
    graphemes[start..].concat()
}

fn finder_input_cursor_offset(text: &str, max_cells: usize) -> usize {
    let visible = finder_input_view(text, max_cells);
    text_width(&visible)
}

fn finder_right_footer(popup: &FinderPopup, style: UiStyle) -> FinderRightFooter {
    let text = format!("{}/{}", popup.result_count, popup.total_count);
    let text_colors = style.finder.dim;
    let indicator = {
        let minimap_module_colors = style
            .palette
            .status_modules
            .colors(StatusModuleKind::Minimap);
        let pinned_count = popup
            .entries
            .iter()
            .take_while(|entry| entry.is_pinned)
            .count();
        let selected_file_index =
            finder_file_scroll_position(popup.selected, pinned_count, popup.result_count);
        let (glyph, colors) = scroll_minimap_cell(
            selected_file_index,
            popup.result_count,
            style.palette.minimap,
            style.palette.minimap_alt,
            minimap_module_colors.wrapper.bg,
        );
        FinderFooterIndicator {
            left_edge: STATUS_MODULE_EDGE_RIGHT, // I know this looks wrong, but it looks nice
            glyph,
            right_edge: STATUS_MODULE_EDGE_LEFT,
            wrapper_colors: ColorPair::new(minimap_module_colors.wrapper.bg, style.finder.text.bg),
            content_colors: colors,
        }
    };

    FinderRightFooter {
        text,
        text_colors,
        indicator: Some(indicator),
    }
}

fn finder_file_scroll_position(selected: usize, pinned_count: usize, file_count: usize) -> usize {
    if file_count <= 1 {
        return 0;
    }

    selected.saturating_sub(pinned_count).min(file_count - 1)
}

fn text_width(text: &str) -> usize {
    text.graphemes(true)
        .map(|grapheme| (cell_width(grapheme, FINDER_TAB_POLICY) as usize).max(1))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{
        FinderFrameLayout, PIN_SELECTOR_HORIZONTAL_PADDING, PINBOARD_MIN_WIDTH, QUERY_GAP_ROWS,
        compute_finder_popup_layout, compute_pin_selector_inner_width_from_labels,
        finder_file_scroll_position, finder_input_cursor_offset, finder_input_view,
        finder_right_footer, text_width, visible_entry_rows, visible_right_footer_width,
    };
    use crate::app::FinderPopup;
    use crate::ui::UiStyle;

    #[test]
    fn visible_entry_rows_bottom_justifies_files_below_pins() {
        let rows = visible_entry_rows(4, 2, 3, 8);

        assert_eq!(rows, vec![(0, 0), (1, 1), (2, 6), (3, 7)]);
    }

    #[test]
    fn visible_entry_rows_scrolls_file_region_under_pins() {
        let rows = visible_entry_rows(8, 2, 2, 5);

        assert_eq!(rows, vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)]);
    }

    #[test]
    fn finder_layout_with_preview_keeps_query_under_results_and_preview_full_height() {
        let layout = compute_finder_popup_layout(160, 40, 96, 24, true);
        let preview = layout.preview.expect("preview layout");

        assert_eq!(layout.query.x, layout.results.x);
        assert_eq!(layout.query.inner_w, layout.results.inner_w);
        assert_eq!(preview.y, layout.results.y);
        assert_eq!(preview.inner_h, 24);
        assert_eq!(
            layout.query.y,
            layout.results.y + layout.results.inner_h + 2 + QUERY_GAP_ROWS
        );
        assert_eq!(preview.x, layout.results.x + layout.results.inner_w + 2);
    }

    #[test]
    fn finder_layout_without_preview_uses_single_column_width() {
        let layout = compute_finder_popup_layout(100, 30, 70, 18, false);

        assert_eq!(layout.preview, None);
        assert_eq!(
            layout.results,
            FinderFrameLayout {
                x: 14,
                y: 5,
                inner_w: 70,
                inner_h: 15,
            }
        );
        assert_eq!(
            layout.query,
            FinderFrameLayout {
                x: 14,
                y: 22,
                inner_w: 70,
                inner_h: 1,
            }
        );
    }

    #[test]
    fn pinboard_width_respects_candidate_path_label() {
        let path_label = "src/some/deeply/nested/current_candidate_file.rs";

        assert_eq!(
            compute_pin_selector_inner_width_from_labels(path_label, [Some("a.rs")], 120),
            text_width(path_label) as u16 + PIN_SELECTOR_HORIZONTAL_PADDING * 2
        );
    }

    #[test]
    fn pinboard_width_keeps_default_minimum_when_candidate_is_short() {
        assert_eq!(
            compute_pin_selector_inner_width_from_labels("a.rs", [Some("b.rs")], 120),
            PINBOARD_MIN_WIDTH
        );
    }

    #[test]
    fn finder_footer_reserves_counter_padding_and_scroll_indicator() {
        let popup = FinderPopup {
            entries: Vec::new(),
            query: "main".to_string(),
            selected: 0,
            result_count: 84,
            total_count: 84,
            preview: None,
        };
        let footer = finder_right_footer(&popup, UiStyle::default());

        assert!(footer.indicator.is_some());
        assert_eq!(footer.width(), text_width("84/84") as u16 + 3);
    }

    #[test]
    fn finder_footer_does_not_reserve_width_when_it_cannot_fit() {
        let popup = FinderPopup {
            entries: Vec::new(),
            query: "abcdef".to_string(),
            selected: 0,
            result_count: 84,
            total_count: 84,
            preview: None,
        };
        let footer = finder_right_footer(&popup, UiStyle::default());
        let prompt_col = 1u16;
        let input_col = prompt_col + 2;
        let query_w = footer.width();
        let right_w = visible_right_footer_width(&footer, query_w);
        let input_w = query_w
            .saturating_sub(right_w)
            .saturating_sub(input_col.saturating_add(1))
            .max(1) as usize;
        let incorrectly_reserved_input_w = query_w
            .saturating_sub(footer.width())
            .saturating_sub(input_col.saturating_add(1))
            .max(1) as usize;

        assert_eq!(right_w, 0);
        assert!(input_w > incorrectly_reserved_input_w);
        assert_eq!(finder_input_view(&popup.query, input_w), "cdef");
        assert_eq!(
            finder_input_view(&popup.query, incorrectly_reserved_input_w),
            "f"
        );
        assert_eq!(finder_input_cursor_offset(&popup.query, input_w), 4);
    }

    #[test]
    fn finder_scroll_position_ignores_pinned_entries() {
        assert_eq!(finder_file_scroll_position(0, 2, 3), 0);
        assert_eq!(finder_file_scroll_position(2, 2, 3), 0);
        assert_eq!(finder_file_scroll_position(3, 2, 3), 1);
        assert_eq!(finder_file_scroll_position(4, 2, 3), 2);
    }
}
