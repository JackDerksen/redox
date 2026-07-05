use minui::{Color, ColorPair, Result, TabPolicy, Window, cell_width};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui::style::UndoTreeStyle;
use crate::ui::widgets::popup::clip_text_to_cells;

const UNDO_TREE_TAB_POLICY: TabPolicy = TabPolicy::Fixed(4);

pub fn draw_undo_tree_lines(
    window: &mut dyn Window,
    width: u16,
    style: UndoTreeStyle,
    lines: &[String],
    first_line: usize,
    selected_line: usize,
) -> Result<()> {
    for (row, line) in lines.iter().enumerate() {
        let is_selected = first_line.saturating_add(row) == selected_line;
        draw_undo_tree_line(window, width, row as u16, line, style, is_selected)?;
    }
    Ok(())
}

pub fn draw_undo_tree_preview_lines(
    window: &mut dyn Window,
    width: u16,
    style: UndoTreeStyle,
    lines: &[String],
) -> Result<()> {
    let separator_row = lines.iter().position(|line| line == "---");
    for (row, line) in lines.iter().enumerate() {
        draw_preview_line(window, width, row, line, style, separator_row)?;
    }
    Ok(())
}

fn draw_undo_tree_line(
    window: &mut dyn Window,
    width: u16,
    row: u16,
    line: &str,
    style: UndoTreeStyle,
    is_selected: bool,
) -> Result<()> {
    let bg = if is_selected {
        style.selected.bg
    } else {
        style.text.bg
    };
    fill_row(window, width, row, ColorPair::new(style.text.fg, bg))?;

    let line = clip_text_to_cells(line, width as usize);
    let timestamp_start = line.find('(').unwrap_or(line.len());
    let original_range = line
        .find("original")
        .map(|start| (start, start + "original".len()));
    let selected_indicator = line
        .find('>')
        .and_then(|start| line[start..].find('<').map(|end| (start, start + end + 1)));
    let mut col = 0u16;
    for (byte_idx, ch) in line.char_indices() {
        let is_original_label =
            original_range.is_some_and(|(start, end)| byte_idx >= start && byte_idx < end);
        let is_node_label = ch.is_ascii_digit() || is_original_label;
        let is_selected_indicator =
            selected_indicator.is_some_and(|(start, end)| byte_idx >= start && byte_idx < end);
        let colors = if byte_idx >= timestamp_start {
            with_bg(style.timestamp, bg)
        } else if is_undo_tree_edge_glyph(ch) {
            with_bg(style.edge, bg)
        } else if matches!(ch, '{' | '}') {
            with_bg(style.redo_marker, bg)
        } else if is_node_label {
            with_bg(style.node_label, bg)
        } else if is_selected_indicator {
            with_bg(style.selected_indicator, bg)
        } else if matches!(ch, '*' | '●' | '⦿') {
            with_bg(style.node, bg)
        } else {
            with_bg(style.text, bg)
        };
        col = write_grapheme(window, width, row, col, &ch.to_string(), colors)?;
    }
    Ok(())
}

fn draw_preview_line(
    window: &mut dyn Window,
    width: u16,
    row: usize,
    line: &str,
    style: UndoTreeStyle,
    separator_row: Option<usize>,
) -> Result<()> {
    let row_u16 = row as u16;
    fill_row(window, width, row_u16, style.preview_text)?;
    if let Some(rest) = line.strip_prefix("Node ") {
        return write_segments(
            window,
            width,
            row_u16,
            &[("Node ", style.preview_label), (rest, style.preview_title)],
        );
    }

    let colors = if line == "Original state" {
        style.preview_title
    } else if line == "---" {
        style.preview_label
    } else if separator_row.is_some_and(|separator| row > 1 && row < separator) {
        style.preview_deleted
    } else if separator_row.is_some_and(|separator| row > separator) {
        style.preview_inserted
    } else if line == "No edit is recorded for this point." {
        style.preview_dim
    } else {
        style.preview_text
    };
    let clipped = clip_text_to_cells(line, width as usize);
    write_segments(window, width, row_u16, &[(&clipped, colors)])
}

fn write_segments(
    window: &mut dyn Window,
    width: u16,
    row: u16,
    segments: &[(&str, ColorPair)],
) -> Result<()> {
    let mut col = 0u16;
    for (text, colors) in segments {
        for grapheme in text.graphemes(true) {
            col = write_grapheme(window, width, row, col, grapheme, *colors)?;
            if col >= width {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn write_grapheme(
    window: &mut dyn Window,
    width_limit: u16,
    row: u16,
    col: u16,
    grapheme: &str,
    colors: ColorPair,
) -> Result<u16> {
    let width = (cell_width(grapheme, UNDO_TREE_TAB_POLICY) as u16).max(1);
    if col.saturating_add(width) > width_limit {
        return Ok(width_limit);
    }
    window.write_str_colored(row, col, grapheme, colors)?;
    Ok(col.saturating_add(width))
}

fn fill_row(window: &mut dyn Window, width: u16, row: u16, colors: ColorPair) -> Result<()> {
    if width == 0 {
        return Ok(());
    }
    window.write_str_colored(row, 0, &" ".repeat(width as usize), colors)
}

fn with_bg(colors: ColorPair, bg: Color) -> ColorPair {
    ColorPair::new(colors.fg, bg)
}

fn is_undo_tree_edge_glyph(ch: char) -> bool {
    matches!(
        ch,
        '|' | '/' | '\\' | '│' | '─' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ColorWindow {
        width: u16,
        height: u16,
        cells: Vec<Vec<Option<ColorPair>>>,
    }

    impl ColorWindow {
        fn new(width: u16, height: u16) -> Self {
            Self {
                width,
                height,
                cells: vec![vec![None; width as usize]; height as usize],
            }
        }

        fn color_at(&self, row: u16, col: u16) -> Option<ColorPair> {
            self.cells
                .get(row as usize)
                .and_then(|row| row.get(col as usize))
                .copied()
                .flatten()
        }
    }

    impl Window for ColorWindow {
        fn write_str(&mut self, y: u16, x: u16, s: &str) -> Result<()> {
            self.write_str_colored(y, x, s, ColorPair::new(Color::Reset, Color::Reset))
        }

        fn write_str_colored(&mut self, y: u16, x: u16, s: &str, colors: ColorPair) -> Result<()> {
            if y >= self.height {
                return Ok(());
            }
            for (offset, _) in s.chars().enumerate() {
                let col = x as usize + offset;
                if col >= self.width as usize {
                    break;
                }
                self.cells[y as usize][col] = Some(colors);
            }
            Ok(())
        }

        fn flush(&mut self) -> Result<()> {
            Ok(())
        }

        fn set_cursor_position(&mut self, _x: u16, _y: u16) -> Result<()> {
            Ok(())
        }

        fn show_cursor(&mut self, _show: bool) -> Result<()> {
            Ok(())
        }

        fn get_size(&self) -> (u16, u16) {
            (self.width, self.height)
        }

        fn clear_screen(&mut self) -> Result<()> {
            for row in &mut self.cells {
                row.fill(None);
            }
            Ok(())
        }

        fn clear_line(&mut self, y: u16) -> Result<()> {
            if let Some(row) = self.cells.get_mut(y as usize) {
                row.fill(None);
            }
            Ok(())
        }

        fn clear_area(&mut self, y1: u16, x1: u16, y2: u16, x2: u16) -> Result<()> {
            let row_start = usize::from(y1.min(y2));
            let row_end = usize::from(y1.max(y2)).min(self.cells.len().saturating_sub(1));
            let col_start = usize::from(x1.min(x2));
            let col_end = usize::from(x1.max(x2)).min(self.width.saturating_sub(1) as usize);
            for row in row_start..=row_end {
                for col in col_start..=col_end {
                    self.cells[row][col] = None;
                }
            }
            Ok(())
        }
    }

    #[test]
    fn narrow_preview_keeps_semantic_colours() {
        let style = UndoTreeStyle::default();
        let lines = vec![
            "Node 12".to_string(),
            "Original state".to_string(),
            String::new(),
            "No edit is recorded for this point.".to_string(),
        ];
        let mut window = ColorWindow::new(4, lines.len() as u16);

        draw_undo_tree_preview_lines(&mut window, 4, style, &lines).expect("preview should render");

        assert_eq!(window.color_at(0, 0), Some(style.preview_label));
        assert_eq!(window.color_at(1, 0), Some(style.preview_title));
        assert_eq!(window.color_at(3, 0), Some(style.preview_dim));
    }
}
