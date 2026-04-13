//! Visual style and small layout config for `redox-tui`.

use minui::{Color, ColorPair};

pub const STATUS_BAR_HEIGHT_ROWS: usize = 1;
pub const STATUS_BAR_HEIGHT_CELLS: u16 = STATUS_BAR_HEIGHT_ROWS as u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SyntaxRole {
    VariableBuiltin,
    VariableParameter,
    Keyword,
    KeywordOperator,
    KeywordImport,
    Type,
    TypeBuiltin,
    TypeDefinition,
    Function,
    FunctionMacro,
    FunctionMethod,
    String,
    StringEscape,
    Character,
    Number,
    Boolean,
    Float,
    Comment,
    Constant,
    ConstantBuiltin,
    ConstantMacro,
    Constructor,
    Attribute,
    Property,
    Operator,
    PunctuationDelimiter,
    PunctuationBracket,
    PunctuationSpecial,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BaseTheme {
    pub bg: Color,
    pub color_column: Color,
    pub scope: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub white: Color,
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub purple: Color,
    pub orange: Color,
    pub light_red: Color,
    pub light_green: Color,
    pub light_yellow: Color,
    pub light_blue: Color,
    pub light_purple: Color,
    pub light_orange: Color,
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
            color_column: Color::Rgb {
                r: (24),
                g: (23),
                b: (26),
            },
            scope: Color::Rgb {
                r: (45),
                g: (44),
                b: (47),
            },
            selection_bg: Color::Rgb {
                r: (45),
                g: (43),
                b: (48),
            },
            selection_fg: Color::Rgb {
                r: (226),
                g: (226),
                b: (227),
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
            light_red: Color::Rgb {
                r: (255),
                g: (157),
                b: (177),
            },
            light_green: Color::Rgb {
                r: (207),
                g: (238),
                b: (194),
            },
            light_yellow: Color::Rgb {
                r: (245),
                g: (232),
                b: (175),
            },
            light_blue: Color::Rgb {
                r: (187),
                g: (232),
                b: (238),
            },
            light_purple: Color::Rgb {
                r: (214),
                g: (217),
                b: (252),
            },
            light_orange: Color::Rgb {
                r: (255),
                g: (199),
                b: (158),
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
pub struct StatusModuleColors {
    pub wrapper: ColorPair,
    pub content: ColorPair,
}

impl StatusModuleColors {
    pub fn solid(colors: ColorPair) -> Self {
        Self {
            wrapper: colors,
            content: colors,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusModuleKind {
    Coords,
    Minimap,
}

#[derive(Debug, Clone, Copy)]
pub struct StatusModuleTheme {
    pub coords: StatusModuleColors,
    pub minimap: StatusModuleColors,
}

impl StatusModuleTheme {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            coords: StatusModuleColors::solid(ColorPair::new(theme.black, theme.light_gray)),
            minimap: StatusModuleColors::solid(ColorPair::new(theme.black, theme.dark_gray)),
        }
    }

    pub fn colors(self, kind: StatusModuleKind) -> StatusModuleColors {
        match kind {
            StatusModuleKind::Coords => self.coords,
            StatusModuleKind::Minimap => self.minimap,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub status_bar_bg: ColorPair,
    pub mode_normal: ColorPair,
    pub mode_insert: ColorPair,
    pub mode_command: ColorPair,
    pub mode_visual: ColorPair,
    pub minimap: ColorPair,
    pub minimap_alt: ColorPair,
    pub status_modules: StatusModuleTheme,
}

impl Palette {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            status_bar_bg: ColorPair::new(theme.light_gray, theme.black),
            mode_normal: ColorPair::new(theme.black, theme.purple),
            mode_insert: ColorPair::new(theme.black, theme.blue),
            mode_command: ColorPair::new(theme.black, theme.red),
            mode_visual: ColorPair::new(theme.black, theme.orange),
            minimap: ColorPair::new(theme.white, Color::Transparent),
            minimap_alt: ColorPair::new(Color::Transparent, theme.white),
            status_modules: StatusModuleTheme::from_theme(theme),
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
    pub status_module_gap_width: u16,
    pub popup_dim_amount: u8,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            status_left_min_width: 12,
            status_right_min_width: 18,
            status_module_gap_width: 0,
            popup_dim_amount: 5,
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
            width_percent: 65,
            height_percent: 60,
            min_width: 20,
            min_height: 6,
            border: ColorPair::new(theme.light_gray, theme.bg),
            title: ColorPair::new(theme.light_gray, theme.bg),
            file: ColorPair::new(theme.white, theme.bg),
            directory: ColorPair::new(theme.blue, theme.bg),
            executable: ColorPair::new(theme.red, theme.bg),
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
pub struct AboutStyle {
    pub width_percent: u16,
    pub height_percent: u16,
    pub min_width: u16,
    pub min_height: u16,
    pub border: ColorPair,
    pub title: ColorPair,
    pub text: ColorPair,
    pub logo_red: ColorPair,
    pub logo_white: ColorPair,
    pub logo_blue: ColorPair,
}

impl AboutStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 65,
            height_percent: 52,
            min_width: 56,
            min_height: 12,
            border: ColorPair::new(theme.light_gray, theme.bg),
            title: ColorPair::new(theme.light_gray, theme.bg),
            text: ColorPair::new(theme.white, theme.bg),
            logo_red: ColorPair::new(theme.red, theme.bg),
            logo_white: ColorPair::new(theme.white, theme.bg),
            logo_blue: ColorPair::new(theme.blue, theme.bg),
        }
    }
}

impl Default for AboutStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CommandLineStyle {
    pub width_percent: u16,
    pub min_width: u16,
    pub top_margin_rows: u16,
    pub inner_height_rows: u16,
    pub border: ColorPair,
    pub title: ColorPair,
    pub text: ColorPair,
    pub prompt: ColorPair,
}

impl CommandLineStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 65,
            min_width: 24,
            top_margin_rows: 2,
            inner_height_rows: 1,
            border: ColorPair::new(theme.light_gray, theme.bg),
            title: ColorPair::new(theme.red, theme.bg),
            text: ColorPair::new(theme.white, theme.bg),
            prompt: ColorPair::new(theme.light_gray, theme.bg),
        }
    }
}

impl Default for CommandLineStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PerfStyle {
    pub width_percent: u16,
    pub height_percent: u16,
    pub min_width: u16,
    pub min_height: u16,
    pub border: ColorPair,
    pub title: ColorPair,
    pub text: ColorPair,
    pub label: ColorPair,
    pub value: ColorPair,
    pub dim: ColorPair,
    pub good: ColorPair,
    pub warn: ColorPair,
    pub hot: ColorPair,
    pub bar_bg: ColorPair,
}

impl PerfStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 56,
            height_percent: 48,
            min_width: 50,
            min_height: 14,
            border: ColorPair::new(theme.light_gray, theme.bg),
            title: ColorPair::new(theme.yellow, theme.bg),
            text: ColorPair::new(theme.white, theme.bg),
            label: ColorPair::new(theme.light_gray, theme.bg),
            value: ColorPair::new(theme.white, theme.bg),
            dim: ColorPair::new(theme.dark_gray, theme.bg),
            good: ColorPair::new(theme.green, theme.bg),
            warn: ColorPair::new(theme.yellow, theme.bg),
            hot: ColorPair::new(theme.red, theme.bg),
            bar_bg: ColorPair::new(theme.dark_gray, theme.bg),
        }
    }
}

impl Default for PerfStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SyntaxStyle {
    pub variable_builtin: ColorPair,
    pub variable_parameter: ColorPair,
    pub keyword: ColorPair,
    pub keyword_operator: ColorPair,
    pub keyword_import: ColorPair,
    pub type_name: ColorPair,
    pub type_builtin: ColorPair,
    pub type_definition: ColorPair,
    pub function: ColorPair,
    pub function_macro: ColorPair,
    pub function_method: ColorPair,
    pub string: ColorPair,
    pub string_escape: ColorPair,
    pub character: ColorPair,
    pub number: ColorPair,
    pub boolean: ColorPair,
    pub float: ColorPair,
    pub comment: ColorPair,
    pub constant: ColorPair,
    pub constant_builtin: ColorPair,
    pub constant_macro: ColorPair,
    pub constructor: ColorPair,
    pub attribute: ColorPair,
    pub property: ColorPair,
    pub operator: ColorPair,
    pub punctuation_delimiter: ColorPair,
    pub punctuation_bracket: ColorPair,
    pub punctuation_special: ColorPair,
}

impl SyntaxStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        let bg = theme.bg;
        Self {
            variable_builtin: ColorPair::new(theme.purple, bg),
            variable_parameter: ColorPair::new(theme.orange, bg),
            keyword: ColorPair::new(theme.red, bg),
            keyword_operator: ColorPair::new(theme.white, bg),
            keyword_import: ColorPair::new(theme.white, bg),
            type_name: ColorPair::new(theme.light_blue, bg),
            type_builtin: ColorPair::new(theme.light_orange, bg),
            type_definition: ColorPair::new(theme.light_orange, bg),
            function: ColorPair::new(theme.blue, bg),
            function_macro: ColorPair::new(theme.purple, bg),
            function_method: ColorPair::new(theme.blue, bg),
            string: ColorPair::new(theme.green, bg),
            string_escape: ColorPair::new(theme.orange, bg),
            character: ColorPair::new(theme.green, bg),
            number: ColorPair::new(theme.purple, bg),
            boolean: ColorPair::new(theme.purple, bg),
            float: ColorPair::new(theme.light_purple, bg),
            comment: ColorPair::new(theme.light_gray, bg),
            constant: ColorPair::new(theme.purple, bg),
            constant_builtin: ColorPair::new(theme.purple, bg),
            constant_macro: ColorPair::new(theme.yellow, bg),
            constructor: ColorPair::new(theme.white, bg),
            attribute: ColorPair::new(theme.orange, bg),
            property: ColorPair::new(theme.orange, bg),
            operator: ColorPair::new(theme.white, bg),
            punctuation_delimiter: ColorPair::new(theme.white, bg),
            punctuation_bracket: ColorPair::new(theme.white, bg),
            punctuation_special: ColorPair::new(theme.orange, bg),
        }
    }

    pub fn color_for(self, role: SyntaxRole) -> ColorPair {
        match role {
            SyntaxRole::VariableBuiltin => self.variable_builtin,
            SyntaxRole::VariableParameter => self.variable_parameter,
            SyntaxRole::Keyword => self.keyword,
            SyntaxRole::KeywordOperator => self.keyword_operator,
            SyntaxRole::KeywordImport => self.keyword_import,
            SyntaxRole::Type => self.type_name,
            SyntaxRole::TypeBuiltin => self.type_builtin,
            SyntaxRole::TypeDefinition => self.type_definition,
            SyntaxRole::Function => self.function,
            SyntaxRole::FunctionMacro => self.function_macro,
            SyntaxRole::FunctionMethod => self.function_method,
            SyntaxRole::String => self.string,
            SyntaxRole::StringEscape => self.string_escape,
            SyntaxRole::Character => self.character,
            SyntaxRole::Number => self.number,
            SyntaxRole::Boolean => self.boolean,
            SyntaxRole::Float => self.float,
            SyntaxRole::Comment => self.comment,
            SyntaxRole::Constant => self.constant,
            SyntaxRole::ConstantBuiltin => self.constant_builtin,
            SyntaxRole::ConstantMacro => self.constant_macro,
            SyntaxRole::Constructor => self.constructor,
            SyntaxRole::Attribute => self.attribute,
            SyntaxRole::Property => self.property,
            SyntaxRole::Operator => self.operator,
            SyntaxRole::PunctuationDelimiter => self.punctuation_delimiter,
            SyntaxRole::PunctuationBracket => self.punctuation_bracket,
            SyntaxRole::PunctuationSpecial => self.punctuation_special,
        }
    }
}

impl Default for SyntaxStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UiStyle {
    pub theme: BaseTheme,
    pub palette: Palette,
    pub layout: Layout,
    pub about: AboutStyle,
    pub command_line: CommandLineStyle,
    pub explorer: ExplorerStyle,
    pub perf: PerfStyle,
    pub syntax: SyntaxStyle,
}

impl Default for UiStyle {
    fn default() -> Self {
        let theme = BaseTheme::default();
        Self {
            theme,
            palette: Palette::from_theme(theme),
            layout: Layout::default(),
            about: AboutStyle::from_theme(theme),
            command_line: CommandLineStyle::from_theme(theme),
            explorer: ExplorerStyle::from_theme(theme),
            perf: PerfStyle::from_theme(theme),
            syntax: SyntaxStyle::from_theme(theme),
        }
    }
}

fn dim_color(color: Color, amount: u8) -> Color {
    match color {
        Color::Rgb { r, g, b } => Color::Rgb {
            r: r.saturating_sub(amount),
            g: g.saturating_sub(amount),
            b: b.saturating_sub(amount),
        },
        Color::Transparent => Color::Transparent,
        other => other,
    }
}

impl BaseTheme {
    pub fn dimmed(self, amount: u8) -> Self {
        Self {
            bg: dim_color(self.bg, amount),
            color_column: dim_color(self.color_column, amount),
            scope: dim_color(self.scope, amount),
            selection_bg: dim_color(self.selection_bg, amount),
            selection_fg: dim_color(self.selection_fg, amount),
            white: dim_color(self.white, amount),
            black: dim_color(self.black, amount),
            red: dim_color(self.red, amount),
            green: dim_color(self.green, amount),
            yellow: dim_color(self.yellow, amount),
            blue: dim_color(self.blue, amount),
            purple: dim_color(self.purple, amount),
            orange: dim_color(self.orange, amount),
            light_red: dim_color(self.light_red, amount),
            light_green: dim_color(self.light_green, amount),
            light_yellow: dim_color(self.light_yellow, amount),
            light_blue: dim_color(self.light_blue, amount),
            light_purple: dim_color(self.light_purple, amount),
            light_orange: dim_color(self.light_orange, amount),
            dark_gray: dim_color(self.dark_gray, amount),
            light_gray: dim_color(self.light_gray, amount),
        }
    }
}

impl UiStyle {
    pub fn dimmed(self) -> Self {
        let theme = self.theme.dimmed(self.layout.popup_dim_amount);
        Self {
            theme,
            palette: Palette::from_theme(theme),
            layout: self.layout,
            about: AboutStyle::from_theme(theme),
            command_line: CommandLineStyle::from_theme(theme),
            explorer: ExplorerStyle::from_theme(theme),
            perf: PerfStyle::from_theme(theme),
            syntax: SyntaxStyle::from_theme(theme),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimmed_style_reduces_rgb_channels_by_popup_amount() {
        let style = UiStyle::default();
        let dimmed = style.dimmed();

        assert_eq!(
            dimmed.theme.bg,
            Color::Rgb {
                r: 21,
                g: 20,
                b: 23,
            }
        );
        assert_eq!(dimmed.layout.popup_dim_amount, 5);
    }
}
