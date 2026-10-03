use crate::ui::text_style::TextStyle;
use minui::{Color, Result, Style, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::state::{UndoTreeLineRole, UndoTreeLineSpan};
use crate::ui::icons::UNDO_TREE;
use crate::ui::render::LineViewport;
use crate::ui::style::UndoTreeStyle;
use crate::ui::widgets::popup::clip_text_to_cells;

// Credit to https://github.com/mbbill/undotree for the inspiration here.

const UNDO_TREE_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);
const PREVIEW_HEADER_ROWS: usize = 2;
pub const UNDO_TREE_HEADER_ROWS: u16 = 2;
const UNDO_TREE_TITLE: &str = "Undo tree";
const UNDO_TREE_TITLE_COL: u16 = 2;
const PREVIEW_CONTENT_COL: u16 = 4;

pub(crate) fn undo_tree_preview_content_width(width: usize) -> usize {
    width.saturating_sub(PREVIEW_CONTENT_COL as usize + 1)
}

pub fn draw_undo_tree_lines(
    window: &mut dyn Window,
    width: u16,
    style: UndoTreeStyle,
    lines: &[String],
    line_spans: &[Vec<UndoTreeLineSpan>],
    (first_line, selected_line): (usize, usize),
    icons_enabled: bool,
) -> Result<()> {
    draw_undo_tree_header(window, width, style, icons_enabled)?;
    for (row, line) in lines.iter().enumerate() {
        let is_selected = first_line.saturating_add(row) == selected_line;
        let spans = line_spans
            .get(first_line.saturating_add(row))
            .map(Vec::as_slice)
            .unwrap_or_default();
        draw_undo_tree_line(
            window,
            width,
            (row as u16).saturating_add(UNDO_TREE_HEADER_ROWS),
            line,
            spans,
            style,
            is_selected,
        )?;
    }
    Ok(())
}

fn draw_undo_tree_header(
    window: &mut dyn Window,
    width: u16,
    style: UndoTreeStyle,
    icons_enabled: bool,
) -> Result<()> {
    fill_row(window, width, 0, style.text)?;
    let title = undo_tree_title(icons_enabled);
    let title = clip_text_to_cells(&title, width.saturating_sub(UNDO_TREE_TITLE_COL) as usize);
    if width > UNDO_TREE_TITLE_COL {
        window.write_str_styled(0, UNDO_TREE_TITLE_COL, &title, Style::from(style.title))?;
    }
    if window.get_size().1 > 1 {
        fill_row(window, width, 1, style.text)?;
        if width > 2 {
            window.write_str_styled(1, 1, &"─".repeat(width as usize - 2), style.edge.into())?;
        }
    }
    Ok(())
}

fn undo_tree_title(icons_enabled: bool) -> String {
    if icons_enabled {
        format!("{UNDO_TREE} {UNDO_TREE_TITLE}")
    } else {
        UNDO_TREE_TITLE.to_owned()
    }
}

pub fn draw_undo_tree_preview_lines(
    window: &mut dyn Window,
    width: u16,
    scroll_x: usize,
    style: UndoTreeStyle,
    lines: &[String],
    (first_line, separator_row): (usize, Option<usize>),
) -> Result<()> {
    for (row, line) in lines.iter().enumerate() {
        draw_preview_line(
            window,
            width,
            scroll_x,
            (row, first_line + row),
            line,
            style,
            separator_row,
        )?;
    }
    Ok(())
}

fn draw_undo_tree_line(
    window: &mut dyn Window,
    width: u16,
    row: u16,
    line: &str,
    spans: &[UndoTreeLineSpan],
    style: UndoTreeStyle,
    is_selected: bool,
) -> Result<()> {
    let bg = if is_selected {
        style.selected.bg
    } else {
        style.text.bg
    };
    fill_row(
        window,
        width,
        row,
        style.text.with_colors(style.text.fg, bg),
    )?;

    let line = clip_text_to_cells(line, width as usize);
    let mut col = 0u16;
    for (byte_idx, ch) in line.char_indices() {
        let role = spans
            .iter()
            .find(|span| span.range.contains(&byte_idx))
            .map(|span| span.role);
        let colors = match role {
            Some(UndoTreeLineRole::Timestamp) => with_bg(style.timestamp, bg),
            Some(UndoTreeLineRole::Edge) => with_bg(style.edge, bg),
            Some(UndoTreeLineRole::RedoMarker) => with_bg(style.redo_marker, bg),
            Some(UndoTreeLineRole::NodeLabel) => with_bg(style.node_label, bg),
            Some(UndoTreeLineRole::SelectedIndicator) => with_bg(style.selected_indicator, bg),
            Some(UndoTreeLineRole::Node) => with_bg(style.node, bg),
            None => with_bg(style.text, bg),
        };
        let colors = if is_selected {
            colors.selected(style.selected)
        } else {
            colors
        };
        col = write_grapheme(window, width, row, col, &ch.to_string(), colors)?;
    }
    if is_selected && width > 0 {
        window.write_str_styled(
            row,
            0,
            "▎",
            with_bg(style.selected_indicator, bg)
                .selected(style.selected)
                .into(),
        )?;
    }
    Ok(())
}

fn draw_preview_line(
    window: &mut dyn Window,
    width: u16,
    scroll_x: usize,
    (row, source_row): (usize, usize),
    line: &str,
    style: UndoTreeStyle,
    separator_row: Option<usize>,
) -> Result<()> {
    let row_u16 = row as u16;
    fill_row(window, width, row_u16, style.preview_text)?;
    let marker = if separator_row
        .is_some_and(|separator| source_row >= PREVIEW_HEADER_ROWS && source_row < separator)
    {
        Some(("− ", style.preview_deleted))
    } else if separator_row.is_some_and(|separator| source_row > separator) {
        Some(("+ ", style.preview_inserted))
    } else {
        None
    };
    let colors = if source_row == 0 {
        style.preview_title
    } else if source_row == 1 {
        style.preview_label
    } else if separator_row == Some(source_row) {
        style.preview_separator
    } else if let Some((_, colors)) = marker {
        colors
    } else if line == "No changes to preview." {
        style.preview_dim
    } else {
        style.preview_text
    };
    let column = if let Some((marker, colors)) = marker {
        write_preview_text(
            window,
            LineViewport {
                row: row_u16,
                column: 2,
                scroll_x: 0,
                width: width.saturating_sub(2).min(2) as usize,
            },
            marker,
            colors.into(),
        )?;
        PREVIEW_CONTENT_COL
    } else {
        2
    };
    let clipped = write_preview_text(
        window,
        LineViewport {
            row: row_u16,
            column,
            scroll_x: if marker.is_some() { scroll_x } else { 0 },
            width: if marker.is_some() {
                undo_tree_preview_content_width(width as usize)
            } else {
                width.saturating_sub(column + 1) as usize
            },
        },
        line,
        colors.into(),
    )?;
    if clipped && width > column {
        window.write_str_styled(row_u16, width - 1, "›", style.preview_dim.into())?;
    }
    Ok(())
}

fn write_preview_text(
    window: &mut dyn Window,
    viewport: LineViewport,
    text: &str,
    style: Style,
) -> Result<bool> {
    let LineViewport {
        row,
        column,
        scroll_x,
        width,
    } = viewport;
    let mut cell = 0usize;
    let visible_end = scroll_x.saturating_add(width);
    for grapheme in text.graphemes(true) {
        let start = cell;
        cell = cell.saturating_add((cell_width(grapheme, UNDO_TREE_TAB_POLICY) as usize).max(1));
        if cell <= scroll_x {
            continue;
        }
        if start >= visible_end {
            return Ok(true);
        }
        let column = column.saturating_add(start.saturating_sub(scroll_x) as u16);
        if grapheme == "\t" || start < scroll_x || cell > visible_end {
            let padding = cell.min(visible_end) - start.max(scroll_x);
            window.write_str_styled(row, column, &" ".repeat(padding), style)?;
            if cell > visible_end {
                return Ok(true);
            }
        } else {
            window.write_str_styled(row, column, grapheme, style)?;
        }
    }
    Ok(false)
}

fn write_grapheme(
    window: &mut dyn Window,
    width_limit: u16,
    row: u16,
    col: u16,
    grapheme: &str,
    colors: TextStyle,
) -> Result<u16> {
    let width = (cell_width(grapheme, UNDO_TREE_TAB_POLICY) as u16).max(1);
    if col.saturating_add(width) > width_limit {
        return Ok(width_limit);
    }
    window.write_str_styled(row, col, grapheme, colors.into())?;
    Ok(col.saturating_add(width))
}

fn fill_row(window: &mut dyn Window, width: u16, row: u16, colors: TextStyle) -> Result<()> {
    if width == 0 {
        return Ok(());
    }
    window.write_str_styled(row, 0, &" ".repeat(width as usize), colors.into())
}

fn with_bg(colors: TextStyle, bg: Color) -> TextStyle {
    colors.with_colors(colors.fg, bg)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tests::TestWindow;

    #[test]
    fn narrow_preview_keeps_semantic_colours() {
        let style = UndoTreeStyle::default();
        let lines = [
            "Original state".to_string(),
            String::new(),
            "No changes to preview.".to_string(),
        ];
        let mut window = TestWindow::new(4, lines.len() as u16);

        draw_undo_tree_preview_lines(&mut window, 4, 0, style, &lines, (0, None))
            .expect("preview should render");

        assert_eq!(
            window.styles[0][2].colors,
            Some(style.preview_title.colors())
        );
        assert_eq!(window.styles[2][2].colors, Some(style.preview_dim.colors()));
        assert_eq!(window.row_text(0), "  O›");
    }

    #[test]
    fn preview_diff_lines_use_explicit_separator_colours() {
        let style = UndoTreeStyle {
            preview_separator: TextStyle::new(Color::Yellow, Color::Blue),
            ..UndoTreeStyle::default()
        };
        let lines = [
            "Change 12".to_string(),
            "Before · line 1".to_string(),
            "context".to_string(),
            "old".to_string(),
            "After · line 1".to_string(),
            "new".to_string(),
        ];
        let mut window = TestWindow::new(12, lines.len() as u16);

        assert_ne!(style.preview_deleted, style.preview_text);
        for (first_line, scroll_x) in [(0, 0), (2, 0), (4, 0), (5, 0), (2, 1)] {
            draw_undo_tree_preview_lines(
                &mut window,
                12,
                scroll_x,
                style,
                &lines[first_line..],
                (first_line, Some(4)),
            )
            .expect("preview should render");
            for (source_row, color) in [
                (2, style.preview_deleted),
                (3, style.preview_deleted),
                (4, style.preview_separator),
                (5, style.preview_inserted),
            ] {
                if source_row >= first_line {
                    assert_eq!(
                        window.styles[source_row - first_line][2].colors,
                        Some(color.colors())
                    );
                }
            }
            if first_line <= 3 {
                assert_eq!(
                    window.row_text((3 - first_line) as u16).chars().nth(2),
                    Some('−')
                );
            }
            if first_line <= 5 {
                assert_eq!(
                    window.row_text((5 - first_line) as u16).chars().nth(2),
                    Some('+')
                );
            }
        }
    }
}
