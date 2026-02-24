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
    pub minimap: ColorPair,
    pub minimap_alt: ColorPair,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            status_bar_bg: ColorPair::new(Color::LightGray, Color::Black),
            mode_normal: ColorPair::new(Color::Black, Color::Red),
            mode_insert: ColorPair::new(Color::Black, Color::Blue),
            mode_command: ColorPair::new(Color::Black, Color::Cyan),
            minimap: ColorPair::new(Color::White, Color::Transparent),
            minimap_alt: ColorPair::new(Color::Transparent, Color::White),
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

#[derive(Debug, Clone, Copy)]
pub struct ExplorerStyle {
    pub width_percent: u16,
    pub height_percent: u16,
    pub min_width: u16,
    pub min_height: u16,
    pub border: ColorPair,
    pub title: ColorPair,
    pub file: ColorPair,
    pub directory: ColorPair,
    pub executable: ColorPair,
    pub hidden: ColorPair,
}

impl Default for ExplorerStyle {
    fn default() -> Self {
        Self {
            width_percent: 64,
            height_percent: 60,
            min_width: 20,
            min_height: 6,
            border: ColorPair::new(Color::DarkGray, Color::Transparent),
            title: ColorPair::new(Color::Blue, Color::Transparent),
            file: ColorPair::new(Color::Reset, Color::Transparent),
            directory: ColorPair::new(Color::Blue, Color::Transparent),
            executable: ColorPair::new(Color::Red, Color::Transparent),
            hidden: ColorPair::new(Color::DarkGray, Color::Transparent),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UiStyle {
    pub palette: Palette,
    pub layout: Layout,
    pub explorer: ExplorerStyle,
}
