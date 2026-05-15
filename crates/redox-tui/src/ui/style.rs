//! Visual style and small layout config for `redox-tui`.

use minui::{Color, ColorPair};

use crate::app::{DiagnosticSeverity, GitFileStatusKind, GitGutterKind};

pub const STATUS_BAR_HEIGHT_ROWS: usize = 1;
pub const STATUS_BAR_HEIGHT_CELLS: u16 = STATUS_BAR_HEIGHT_ROWS as u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SyntaxRole {
    MarkdownCode,
    MarkdownEmphasis,
    MarkdownFrontmatter,
    MarkdownHeading,
    MarkdownHighlight,
    MarkdownLink,
    MarkdownListMarker,
    MarkdownStrong,
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
    pub mid_gray: Color,
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
            mid_gray: Color::Rgb {
                r: (80),
                g: (78),
                b: (84),
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
            coords: StatusModuleColors::solid(ColorPair::new(theme.black, theme.dark_gray)),
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
pub struct GitStyle {
    pub added: ColorPair,
    pub modified: ColorPair,
    pub conflict: ColorPair,
    pub removed: ColorPair,
}

impl GitStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            added: ColorPair::new(theme.green, theme.bg),
            modified: ColorPair::new(theme.yellow, theme.bg),
            conflict: ColorPair::new(theme.orange, theme.bg),
            removed: ColorPair::new(theme.red, theme.bg),
        }
    }

    pub fn file_status(self, status: GitFileStatusKind) -> ColorPair {
        match status {
            GitFileStatusKind::Added => self.added,
            GitFileStatusKind::Modified => self.modified,
            GitFileStatusKind::Conflict => self.conflict,
            GitFileStatusKind::Removed => self.removed,
        }
    }

    pub fn gutter_marker(self, kind: GitGutterKind) -> (&'static str, ColorPair) {
        match kind {
            GitGutterKind::Added => ("▍", self.added),
            GitGutterKind::Modified => ("▍", self.modified),
            GitGutterKind::Removed => ("▶", self.removed),
        }
    }
}

impl Default for GitStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub status_bar_bg: ColorPair,
    pub status_metadata: ColorPair,
    pub status_file_path: ColorPair,
    pub status_dirty: ColorPair,
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
            status_metadata: ColorPair::new(theme.black, theme.dark_gray),
            status_file_path: ColorPair::new(theme.mid_gray, theme.black),
            status_dirty: ColorPair::new(theme.mid_gray, theme.black),
            mode_normal: ColorPair::new(theme.black, theme.purple),
            mode_insert: ColorPair::new(theme.black, theme.blue),
            mode_command: ColorPair::new(theme.black, theme.red),
            mode_visual: ColorPair::new(theme.black, theme.orange),
            minimap: ColorPair::new(theme.light_gray, Color::Transparent),
            minimap_alt: ColorPair::new(Color::Transparent, theme.light_gray),
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
            top_margin_rows: 6,
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
pub struct DiagnosticInlineStyle {
    pub error: ColorPair,
    pub warning: ColorPair,
    pub information: ColorPair,
    pub hint: ColorPair,
}

impl DiagnosticInlineStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            error: ColorPair::new(
                theme.light_red,
                Color::Rgb {
                    // dim red background
                    r: 49,
                    g: 38,
                    b: 43,
                },
            ),
            warning: ColorPair::new(
                theme.light_orange,
                Color::Rgb {
                    // dim orange background
                    r: 49,
                    g: 43,
                    b: 42,
                },
            ),
            information: ColorPair::new(
                theme.light_gray,
                Color::Rgb {
                    // dim gray background
                    r: 35,
                    g: 34,
                    b: 38,
                },
            ),
            hint: ColorPair::new(
                theme.light_blue,
                Color::Rgb {
                    // dim blue background
                    r: 39,
                    g: 45,
                    b: 49,
                },
            ),
        }
    }

    pub fn colors(self, severity: DiagnosticSeverity) -> ColorPair {
        match severity {
            DiagnosticSeverity::Error => self.error,
            DiagnosticSeverity::Warning => self.warning,
            DiagnosticSeverity::Information => self.information,
            DiagnosticSeverity::Hint => self.hint,
        }
    }

    pub fn background(self, severity: DiagnosticSeverity) -> Color {
        self.colors(severity).bg
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FinderStyle {
    pub width_percent: u16,
    pub height_percent: u16,
    pub min_width: u16,
    pub min_height: u16,
    pub border: ColorPair,
    pub title: ColorPair,
    pub text: ColorPair,
    pub prompt: ColorPair,
    pub query_title: ColorPair,
    pub dim: ColorPair,
    pub match_highlight: ColorPair,
    pub selected: ColorPair,
    pub pinned_bg: ColorPair,
    pub pinned_marker: ColorPair,
    pub hotkey: ColorPair,
    pub preview_title: ColorPair,
    pub preview_path: ColorPair,
}

impl FinderStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 78,
            height_percent: 72,
            min_width: 52,
            min_height: 14,
            border: ColorPair::new(theme.light_gray, theme.bg),
            title: ColorPair::new(theme.light_blue, theme.bg),
            text: ColorPair::new(theme.white, theme.bg),
            prompt: ColorPair::new(theme.light_gray, theme.bg),
            query_title: ColorPair::new(theme.light_blue, theme.bg),
            dim: ColorPair::new(theme.light_gray, theme.bg),
            match_highlight: ColorPair::new(theme.orange, theme.bg),
            selected: ColorPair::new(theme.white, theme.black),
            pinned_bg: ColorPair::new(theme.white, theme.dark_gray),
            pinned_marker: ColorPair::new(theme.light_blue, theme.dark_gray),
            hotkey: ColorPair::new(theme.light_gray, theme.dark_gray),
            preview_title: ColorPair::new(theme.light_blue, theme.bg),
            preview_path: ColorPair::new(theme.light_gray, theme.bg),
        }
    }
}

impl Default for FinderStyle {
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
    pub markdown_code: ColorPair,
    pub markdown_emphasis: ColorPair,
    pub markdown_frontmatter: ColorPair,
    pub markdown_heading: ColorPair,
    pub markdown_highlight: ColorPair,
    pub markdown_link: ColorPair,
    pub markdown_list_marker: ColorPair,
    pub markdown_strong: ColorPair,
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
            markdown_code: ColorPair::new(theme.light_gray, bg),
            markdown_emphasis: ColorPair::new(theme.orange, bg),
            markdown_frontmatter: ColorPair::new(theme.dark_gray, bg),
            markdown_heading: ColorPair::new(theme.blue, bg),
            markdown_highlight: ColorPair::new(theme.black, theme.green),
            markdown_link: ColorPair::new(theme.purple, bg),
            markdown_list_marker: ColorPair::new(theme.light_gray, bg),
            markdown_strong: ColorPair::new(theme.red, bg),
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
            comment: ColorPair::new(theme.dark_gray, bg),
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
            SyntaxRole::MarkdownCode => self.markdown_code,
            SyntaxRole::MarkdownEmphasis => self.markdown_emphasis,
            SyntaxRole::MarkdownFrontmatter => self.markdown_frontmatter,
            SyntaxRole::MarkdownHeading => self.markdown_heading,
            SyntaxRole::MarkdownHighlight => self.markdown_highlight,
            SyntaxRole::MarkdownLink => self.markdown_link,
            SyntaxRole::MarkdownListMarker => self.markdown_list_marker,
            SyntaxRole::MarkdownStrong => self.markdown_strong,
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
    pub git: GitStyle,
    pub palette: Palette,
    pub layout: Layout,
    pub about: AboutStyle,
    pub command_line: CommandLineStyle,
    pub diagnostic_inline: DiagnosticInlineStyle,
    pub explorer: ExplorerStyle,
    pub finder: FinderStyle,
    pub perf: PerfStyle,
    pub syntax: SyntaxStyle,
}

impl Default for UiStyle {
    fn default() -> Self {
        let theme = BaseTheme::default();
        Self {
            theme,
            git: GitStyle::from_theme(theme),
            palette: Palette::from_theme(theme),
            layout: Layout::default(),
            about: AboutStyle::from_theme(theme),
            command_line: CommandLineStyle::from_theme(theme),
            diagnostic_inline: DiagnosticInlineStyle::from_theme(theme),
            explorer: ExplorerStyle::from_theme(theme),
            finder: FinderStyle::from_theme(theme),
            perf: PerfStyle::from_theme(theme),
            syntax: SyntaxStyle::from_theme(theme),
        }
    }
}

fn dim_foreground_color(color: Color, bg: Color) -> Color {
    const FOREGROUND_WEIGHT: u16 = 699;
    const BACKGROUND_WEIGHT: u16 = 301;

    match color {
        Color::Rgb { r, g, b } => {
            let Color::Rgb {
                r: bg_r,
                g: bg_g,
                b: bg_b,
            } = bg
            else {
                return color;
            };
            Color::Rgb {
                r: blend_channel(r, bg_r, FOREGROUND_WEIGHT, BACKGROUND_WEIGHT),
                g: blend_channel(g, bg_g, FOREGROUND_WEIGHT, BACKGROUND_WEIGHT),
                b: blend_channel(b, bg_b, FOREGROUND_WEIGHT, BACKGROUND_WEIGHT),
            }
        }
        Color::Transparent => Color::Transparent,
        other => other,
    }
}

fn blend_channel(fg: u8, bg: u8, fg_weight: u16, bg_weight: u16) -> u8 {
    let fg_weight = u32::from(fg_weight);
    let bg_weight = u32::from(bg_weight);
    let total = fg_weight.saturating_add(bg_weight).max(1);
    let value = u32::from(fg)
        .saturating_mul(fg_weight)
        .saturating_add(u32::from(bg).saturating_mul(bg_weight))
        .saturating_add(total / 2)
        / total;
    value.min(u32::from(u8::MAX)) as u8
}

impl BaseTheme {
    pub fn dimmed(self) -> Self {
        Self {
            bg: self.bg,
            color_column: self.color_column,
            scope: self.scope,
            selection_bg: self.selection_bg,
            selection_fg: dim_foreground_color(self.selection_fg, self.bg),
            white: dim_foreground_color(self.white, self.bg),
            black: dim_foreground_color(self.black, self.bg),
            red: dim_foreground_color(self.red, self.bg),
            green: dim_foreground_color(self.green, self.bg),
            yellow: dim_foreground_color(self.yellow, self.bg),
            blue: dim_foreground_color(self.blue, self.bg),
            purple: dim_foreground_color(self.purple, self.bg),
            orange: dim_foreground_color(self.orange, self.bg),
            light_red: dim_foreground_color(self.light_red, self.bg),
            light_green: dim_foreground_color(self.light_green, self.bg),
            light_yellow: dim_foreground_color(self.light_yellow, self.bg),
            light_blue: dim_foreground_color(self.light_blue, self.bg),
            light_purple: dim_foreground_color(self.light_purple, self.bg),
            light_orange: dim_foreground_color(self.light_orange, self.bg),
            dark_gray: dim_foreground_color(self.dark_gray, self.bg),
            mid_gray: dim_foreground_color(self.mid_gray, self.bg),
            light_gray: dim_foreground_color(self.light_gray, self.bg),
        }
    }
}

impl UiStyle {
    pub fn dimmed(self) -> Self {
        let theme = self.theme.dimmed();
        Self {
            theme,
            git: GitStyle::from_theme(theme),
            palette: Palette::from_theme(theme),
            layout: self.layout,
            about: AboutStyle::from_theme(theme),
            command_line: CommandLineStyle::from_theme(theme),
            diagnostic_inline: DiagnosticInlineStyle::from_theme(theme),
            explorer: ExplorerStyle::from_theme(theme),
            finder: FinderStyle::from_theme(theme),
            perf: PerfStyle::from_theme(theme),
            syntax: SyntaxStyle::from_theme(theme),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimmed_style_fades_foreground_without_changing_background() {
        let style = UiStyle::default();
        let dimmed = style.dimmed();

        assert_eq!(
            dimmed.theme.bg,
            Color::Rgb {
                r: 26,
                g: 25,
                b: 28,
            }
        );
        assert_eq!(
            dimmed.theme.purple,
            Color::Rgb {
                r: 134,
                g: 140,
                b: 186,
            }
        );
    }

    #[test]
    fn markdown_syntax_roles_use_requested_theme_colours() {
        let theme = BaseTheme::default();
        let style = SyntaxStyle::from_theme(theme);

        assert_eq!(style.color_for(SyntaxRole::MarkdownHeading).fg, theme.blue);
        assert_eq!(
            style.color_for(SyntaxRole::MarkdownEmphasis).fg,
            theme.orange
        );
        assert_eq!(style.color_for(SyntaxRole::MarkdownStrong).fg, theme.red);
        assert_eq!(
            style.color_for(SyntaxRole::MarkdownHighlight).fg,
            theme.black
        );
        assert_eq!(
            style.color_for(SyntaxRole::MarkdownHighlight).bg,
            theme.green
        );
        assert_eq!(
            style.color_for(SyntaxRole::MarkdownCode).fg,
            theme.light_gray
        );
        assert_eq!(
            style.color_for(SyntaxRole::MarkdownFrontmatter).fg,
            theme.dark_gray
        );
        assert_eq!(style.color_for(SyntaxRole::MarkdownLink).fg, theme.purple);
        assert_eq!(
            style.color_for(SyntaxRole::MarkdownListMarker).fg,
            theme.light_gray
        );
    }
}
