//! Visual style and small layout config for `editor_tui`.

use minui::{Color, ColorPair};

pub const STATUS_BAR_HEIGHT_ROWS: usize = 1;
pub const STATUS_BAR_HEIGHT_CELLS: u16 = STATUS_BAR_HEIGHT_ROWS as u16;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BaseTheme {
    pub bg: Color,
    pub white: Color,
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub purple: Color,
    pub orange: Color,
    pub dark_gray: Color,
    pub light_gray: Color,
}

impl Default for BaseTheme {
    fn default() -> Self {
        Self {
            bg: Color::Rgb {
                r: (26),
                g: (25),
                b: (28),
            },
            white: Color::Rgb {
                r: (226),
                g: (226),
                b: (227),
            },
            black: Color::Rgb {
                r: (34),
                g: (33),
                b: (37),
            },
            red: Color::Rgb {
                r: (252),
                g: (128),
                b: (143),
            },
            green: Color::Rgb {
                r: (188),
                g: (240),
                b: (146),
            },
            yellow: Color::Rgb {
                r: (243),
                g: (228),
                b: (140),
            },
            blue: Color::Rgb {
                r: (155),
                g: (227),
                b: (237),
            },
            purple: Color::Rgb {
                r: (180),
                g: (190),
                b: (254),
            },
            orange: Color::Rgb {
                r: (255),
                g: (172),
                b: (114),
            },
            dark_gray: Color::Rgb {
                r: (51),
                g: (49),
                b: (55),
            },
            light_gray: Color::Rgb {
                r: (104),
                g: (101),
                b: (111),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub status_bar_bg: ColorPair,
    pub mode_normal: ColorPair,
    pub mode_insert: ColorPair,
    pub mode_command: ColorPair,
    pub minimap: ColorPair,
    pub minimap_alt: ColorPair,
}

impl Palette {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            status_bar_bg: ColorPair::new(theme.light_gray, theme.black),
            mode_normal: ColorPair::new(theme.black, theme.purple),
            mode_insert: ColorPair::new(theme.black, theme.blue),
            mode_command: ColorPair::new(theme.black, theme.red),
            minimap: ColorPair::new(theme.white, Color::Transparent),
            minimap_alt: ColorPair::new(Color::Transparent, theme.white),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
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

impl ExplorerStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 64,
            height_percent: 60,
            min_width: 20,
            min_height: 6,
            border: ColorPair::new(theme.light_gray, theme.bg),
            title: ColorPair::new(theme.blue, theme.bg),
            file: ColorPair::new(theme.white, theme.bg),
            directory: ColorPair::new(theme.blue, theme.bg),
            executable: ColorPair::new(theme.yellow, theme.bg),
            hidden: ColorPair::new(theme.dark_gray, theme.bg),
        }
    }
}

impl Default for ExplorerStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UiStyle {
    pub theme: BaseTheme,
    pub palette: Palette,
    pub layout: Layout,
    pub explorer: ExplorerStyle,
}

impl Default for UiStyle {
    fn default() -> Self {
        let theme = BaseTheme::default();
        Self {
            theme,
            palette: Palette::from_theme(theme),
            layout: Layout::default(),
            explorer: ExplorerStyle::from_theme(theme),
        }
    }
}
