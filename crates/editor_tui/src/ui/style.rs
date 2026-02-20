//! Visual style and small layout config for `editor_tui`.

use minui::{Color, ColorPair};

pub const STATUS_BAR_HEIGHT_ROWS: usize = 1;
pub const STATUS_BAR_HEIGHT_CELLS: u16 = STATUS_BAR_HEIGHT_ROWS as u16;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub status_bar_bg: ColorPair,
    pub mode_normal: ColorPair,
    pub mode_insert: ColorPair,
    pub mode_command: ColorPair,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            status_bar_bg: ColorPair::new(Color::LightGray, Color::Black),
            mode_normal: ColorPair::new(Color::Black, Color::Red),
            mode_insert: ColorPair::new(Color::Black, Color::Blue),
            mode_command: ColorPair::new(Color::Black, Color::Cyan),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub status_left_min_width: u16,
    pub status_right_min_width: u16,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            status_left_min_width: 12,
            status_right_min_width: 18,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UiStyle {
    pub palette: Palette,
    pub layout: Layout,
}
