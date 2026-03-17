//! Small shared UI helpers for editor rendering paths.

use minui::{Color, ColorPair};

pub fn apply_color_column(
    colors: ColorPair,
    color_column: Option<(usize, Color)>,
    start_cell: usize,
    end_cell: usize,
) -> ColorPair {
    let Some((column, bg)) = color_column else {
        return colors;
    };
    if start_cell <= column && column < end_cell {
        ColorPair::new(colors.fg, bg)
    } else {
        colors
    }
}
