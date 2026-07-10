use minui::{ColorPair, Window};

use crate::app::PaneRect;
use crate::ui::style::UiStyle;

pub fn draw_pane_split_lines(
    window: &mut dyn Window,
    style: UiStyle,
    rects: &[PaneRect],
    width: u16,
    height: u16,
) -> minui::Result<()> {
    let line_color = ColorPair::new(style.theme.light_gray, style.theme.bg);
    let mut line_cells = vec![false; width as usize * height as usize];
    for rect in rects {
        if rect.x > 0 {
            let x = rect.x - 1;
            if x < width {
                let start_y = rect.y.saturating_sub(1);
                for y in start_y..rect.y.saturating_add(rect.height).min(height) {
                    line_cells[y as usize * width as usize + x as usize] = true;
                }
            }
        }
        if rect.y > 0 {
            let y = rect.y - 1;
            if y < height {
                let start_x = rect.x.saturating_sub(1);
                for x in start_x..rect.x.saturating_add(rect.width).min(width) {
                    line_cells[y as usize * width as usize + x as usize] = true;
                }
            }
        }
    }

    for y in 0..height {
        for x in 0..width {
            let idx = y as usize * width as usize + x as usize;
            if !line_cells[idx] {
                continue;
            }
            let up = y > 0 && line_cells[(y - 1) as usize * width as usize + x as usize];
            let down = y + 1 < height && line_cells[(y + 1) as usize * width as usize + x as usize];
            let left = x > 0 && line_cells[y as usize * width as usize + (x - 1) as usize];
            let right = x + 1 < width && line_cells[y as usize * width as usize + (x + 1) as usize];
            let glyph = pane_split_line_glyph(up, down, left, right);
            window.write_str_colored(y, x, glyph, line_color)?;
        }
    }
    Ok(())
}

fn pane_split_line_glyph(up: bool, down: bool, left: bool, right: bool) -> &'static str {
    match (up, down, left, right) {
        (true, true, true, true) => "┼",
        (true, true, true, false) => "┤",
        (true, true, false, true) => "├",
        (true, false, true, true) => "┴",
        (false, true, true, true) => "┬",
        (true, false, false, true) => "└",
        (true, false, true, false) => "┘",
        (false, true, false, true) => "┌",
        (false, true, true, false) => "┐",
        (true, true, _, _) => "│",
        (_, _, true, true) => "─",
        (true, false, false, false) | (false, true, false, false) => "│",
        (false, false, true, false) | (false, false, false, true) => "─",
        _ => " ",
    }
}
