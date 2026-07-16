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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
                r: (65),
                g: (62),
                b: (70),
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
pub struct StatusLinePalette {
    pub bar: ColorPair,
    pub path: ColorPair,
    pub dirty: ColorPair,
    pub mode_normal: ColorPair,
    pub mode_insert: ColorPair,
    pub mode_command: ColorPair,
    pub mode_visual: ColorPair,
    pub metadata: StatusModuleColors,
    pub coords: StatusModuleColors,
    pub minimap_module: StatusModuleColors,
    pub minimap: ColorPair,
    pub minimap_alt: ColorPair,
}

impl StatusLinePalette {
    pub fn from_theme(theme: BaseTheme) -> Self {
        let module_shell = ColorPair::new(theme.black, theme.dark_gray);
        let module_text = ColorPair::new(theme.black, theme.dark_gray);
        Self {
            bar: ColorPair::new(theme.light_gray, theme.black),
            path: ColorPair::new(theme.dark_gray, theme.black),
            dirty: ColorPair::new(theme.light_gray, theme.black),
            mode_normal: ColorPair::new(theme.black, theme.purple),
            mode_insert: ColorPair::new(theme.black, theme.blue),
            mode_command: ColorPair::new(theme.black, theme.red),
            mode_visual: ColorPair::new(theme.black, theme.orange),
            metadata: StatusModuleColors {
                wrapper: module_shell,
                content: module_text,
            },
            coords: StatusModuleColors {
                wrapper: module_shell,
                content: module_text,
            },
            minimap_module: StatusModuleColors::solid(module_shell),
            minimap: ColorPair::new(theme.light_gray, Color::Transparent),
            minimap_alt: ColorPair::new(Color::Transparent, theme.light_gray),
        }
    }
}

impl Default for StatusLinePalette {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub status_left_min_width: u16,
    pub status_right_min_width: u16,
    pub color_column: usize,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            status_left_min_width: 12,
            status_right_min_width: 18,
            color_column: 79,
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
            min_width: 52,
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
            width_percent: 65,
            height_percent: 60,
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
pub struct LspMarketplaceStyle {
    pub width_percent: u16,
    pub height_percent: u16,
    pub min_width: u16,
    pub min_height: u16,
}

impl LspMarketplaceStyle {
    pub fn from_theme(_theme: BaseTheme) -> Self {
        Self {
            width_percent: 65,
            height_percent: 60,
            min_width: 52,
            min_height: 12,
        }
    }
}

impl Default for LspMarketplaceStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UndoTreeStyle {
    pub width_percent: u16,
    pub min_width: u16,
    pub max_width: u16,
    pub preview_height_percent: u16,
    pub preview_min_height: u16,
    pub preview_max_height: u16,
    pub text: ColorPair,
    pub selected: ColorPair,
    pub selected_indicator: ColorPair,
    pub node: ColorPair,
    pub node_label: ColorPair,
    pub redo_marker: ColorPair,
    pub edge: ColorPair,
    pub timestamp: ColorPair,
    pub preview_title: ColorPair,
    pub preview_label: ColorPair,
    pub preview_text: ColorPair,
    pub preview_dim: ColorPair,
    pub preview_deleted: ColorPair,
    pub preview_inserted: ColorPair,
}

impl UndoTreeStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 32,
            min_width: 32,
            max_width: 56,
            preview_height_percent: 42,
            preview_min_height: 8,
            preview_max_height: 14,
            text: ColorPair::new(theme.white, theme.bg),
            selected: ColorPair::new(theme.white, theme.black),
            selected_indicator: ColorPair::new(theme.orange, theme.bg),
            node: ColorPair::new(theme.white, theme.bg),
            node_label: ColorPair::new(theme.blue, theme.bg),
            redo_marker: ColorPair::new(theme.purple, theme.bg),
            edge: ColorPair::new(theme.light_gray, theme.bg),
            timestamp: ColorPair::new(theme.dark_gray, theme.bg),
            preview_title: ColorPair::new(theme.blue, theme.bg),
            preview_label: ColorPair::new(theme.light_gray, theme.bg),
            preview_text: ColorPair::new(theme.white, theme.bg),
            preview_dim: ColorPair::new(theme.dark_gray, theme.bg),
            preview_deleted: ColorPair::new(theme.red, theme.bg),
            preview_inserted: ColorPair::new(theme.green, theme.bg),
        }
    }
}

impl Default for UndoTreeStyle {
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
            width_percent: 44,
            height_percent: 34,
            min_width: 40,
            min_height: 12,
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
    pub status_line: StatusLinePalette,
    pub layout: Layout,
    pub about: AboutStyle,
    pub command_line: CommandLineStyle,
    pub diagnostic_inline: DiagnosticInlineStyle,
    pub explorer: ExplorerStyle,
    pub finder: FinderStyle,
    pub lsp_marketplace: LspMarketplaceStyle,
    pub perf: PerfStyle,
    pub syntax: SyntaxStyle,
    pub undo_tree: UndoTreeStyle,
    pub dim_amount: f32,
}

impl Default for UiStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

impl UiStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            theme,
            git: GitStyle::from_theme(theme),
            status_line: StatusLinePalette::from_theme(theme),
            layout: Layout::default(),
            about: AboutStyle::from_theme(theme),
            command_line: CommandLineStyle::from_theme(theme),
            diagnostic_inline: DiagnosticInlineStyle::from_theme(theme),
            explorer: ExplorerStyle::from_theme(theme),
            finder: FinderStyle::from_theme(theme),
            lsp_marketplace: LspMarketplaceStyle::from_theme(theme),
            perf: PerfStyle::from_theme(theme),
            syntax: SyntaxStyle::from_theme(theme),
            undo_tree: UndoTreeStyle::from_theme(theme),
            dim_amount: 0.301,
        }
    }
}

fn dim_foreground_color(color: Color, bg: Color, amount: f32) -> Color {
    let background_weight = (amount.clamp(0.0, 1.0) * 1_000.0).round() as u16;
    let foreground_weight = 1_000u16.saturating_sub(background_weight);
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
                r: blend_channel(r, bg_r, foreground_weight, background_weight),
                g: blend_channel(g, bg_g, foreground_weight, background_weight),
                b: blend_channel(b, bg_b, foreground_weight, background_weight),
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
    pub fn dimmed(self, amount: f32) -> Self {
        Self {
            bg: self.bg,
            color_column: self.color_column,
            scope: self.scope,
            selection_bg: self.selection_bg,
            selection_fg: dim_foreground_color(self.selection_fg, self.bg, amount),
            white: dim_foreground_color(self.white, self.bg, amount),
            black: dim_foreground_color(self.black, self.bg, amount),
            red: dim_foreground_color(self.red, self.bg, amount),
            green: dim_foreground_color(self.green, self.bg, amount),
            yellow: dim_foreground_color(self.yellow, self.bg, amount),
            blue: dim_foreground_color(self.blue, self.bg, amount),
            purple: dim_foreground_color(self.purple, self.bg, amount),
            orange: dim_foreground_color(self.orange, self.bg, amount),
            light_red: dim_foreground_color(self.light_red, self.bg, amount),
            light_green: dim_foreground_color(self.light_green, self.bg, amount),
            light_yellow: dim_foreground_color(self.light_yellow, self.bg, amount),
            light_blue: dim_foreground_color(self.light_blue, self.bg, amount),
            light_purple: dim_foreground_color(self.light_purple, self.bg, amount),
            light_orange: dim_foreground_color(self.light_orange, self.bg, amount),
            dark_gray: dim_foreground_color(self.dark_gray, self.bg, amount),
            mid_gray: dim_foreground_color(self.mid_gray, self.bg, amount),
            light_gray: dim_foreground_color(self.light_gray, self.bg, amount),
        }
    }
}

fn dim_style_colour(
    colour: Color,
    theme: BaseTheme,
    dimmed: BaseTheme,
    bg: Color,
    amount: f32,
    is_foreground: bool,
) -> Color {
    let theme_colours = [
        (theme.bg, dimmed.bg),
        (theme.color_column, dimmed.color_column),
        (theme.scope, dimmed.scope),
        (theme.selection_bg, dimmed.selection_bg),
        (theme.selection_fg, dimmed.selection_fg),
        (theme.white, dimmed.white),
        (theme.black, dimmed.black),
        (theme.red, dimmed.red),
        (theme.green, dimmed.green),
        (theme.yellow, dimmed.yellow),
        (theme.blue, dimmed.blue),
        (theme.purple, dimmed.purple),
        (theme.orange, dimmed.orange),
        (theme.light_red, dimmed.light_red),
        (theme.light_green, dimmed.light_green),
        (theme.light_yellow, dimmed.light_yellow),
        (theme.light_blue, dimmed.light_blue),
        (theme.light_purple, dimmed.light_purple),
        (theme.light_orange, dimmed.light_orange),
        (theme.dark_gray, dimmed.dark_gray),
        (theme.mid_gray, dimmed.mid_gray),
        (theme.light_gray, dimmed.light_gray),
    ];
    if is_foreground
        && let Some((_, replacement)) = theme_colours
            .iter()
            .find(|(original, _)| *original == colour)
    {
        return *replacement;
    }
    if is_foreground {
        dim_foreground_color(colour, bg, amount)
    } else {
        colour
    }
}

impl UiStyle {
    pub fn dimmed(self) -> Self {
        let mut style = self;
        let bg = self.theme.bg;
        let dimmed_theme = self.theme.dimmed(self.dim_amount);
        style.theme = dimmed_theme;
        macro_rules! dim {
            ($($pair:expr),+ $(,)?) => { $(
                $pair.fg = dim_style_colour(
                    $pair.fg, self.theme, dimmed_theme, bg, self.dim_amount, true,
                );
                $pair.bg = dim_style_colour(
                    $pair.bg, self.theme, dimmed_theme, bg, self.dim_amount, false,
                );
            )+ };
        }
        dim!(
            style.git.added,
            style.git.modified,
            style.git.conflict,
            style.git.removed,
            style.status_line.bar,
            style.status_line.path,
            style.status_line.dirty,
            style.status_line.mode_normal,
            style.status_line.mode_insert,
            style.status_line.mode_command,
            style.status_line.mode_visual,
            style.status_line.metadata.wrapper,
            style.status_line.metadata.content,
            style.status_line.coords.wrapper,
            style.status_line.coords.content,
            style.status_line.minimap_module.wrapper,
            style.status_line.minimap_module.content,
            style.status_line.minimap,
            style.status_line.minimap_alt,
            style.about.border,
            style.about.title,
            style.about.text,
            style.about.logo_red,
            style.about.logo_white,
            style.about.logo_blue,
            style.command_line.border,
            style.command_line.title,
            style.command_line.text,
            style.command_line.prompt,
            style.diagnostic_inline.error,
            style.diagnostic_inline.warning,
            style.diagnostic_inline.information,
            style.diagnostic_inline.hint,
            style.explorer.border,
            style.explorer.title,
            style.explorer.file,
            style.explorer.directory,
            style.explorer.executable,
            style.explorer.hidden,
            style.finder.border,
            style.finder.title,
            style.finder.text,
            style.finder.prompt,
            style.finder.query_title,
            style.finder.dim,
            style.finder.match_highlight,
            style.finder.selected,
            style.finder.pinned_bg,
            style.finder.pinned_marker,
            style.finder.hotkey,
            style.finder.preview_title,
            style.finder.preview_path,
            style.perf.border,
            style.perf.title,
            style.perf.text,
            style.perf.label,
            style.perf.value,
            style.perf.dim,
            style.perf.good,
            style.perf.warn,
            style.perf.hot,
            style.perf.bar_bg,
            style.undo_tree.text,
            style.undo_tree.selected,
            style.undo_tree.selected_indicator,
            style.undo_tree.node,
            style.undo_tree.node_label,
            style.undo_tree.redo_marker,
            style.undo_tree.edge,
            style.undo_tree.timestamp,
            style.undo_tree.preview_title,
            style.undo_tree.preview_label,
            style.undo_tree.preview_text,
            style.undo_tree.preview_dim,
            style.undo_tree.preview_deleted,
            style.undo_tree.preview_inserted,
            style.syntax.markdown_code,
            style.syntax.markdown_emphasis,
            style.syntax.markdown_frontmatter,
            style.syntax.markdown_heading,
            style.syntax.markdown_highlight,
            style.syntax.markdown_link,
            style.syntax.markdown_list_marker,
            style.syntax.markdown_strong,
            style.syntax.variable_builtin,
            style.syntax.variable_parameter,
            style.syntax.keyword,
            style.syntax.keyword_operator,
            style.syntax.keyword_import,
            style.syntax.type_name,
            style.syntax.type_builtin,
            style.syntax.type_definition,
            style.syntax.function,
            style.syntax.function_macro,
            style.syntax.function_method,
            style.syntax.string,
            style.syntax.string_escape,
            style.syntax.character,
            style.syntax.number,
            style.syntax.boolean,
            style.syntax.float,
            style.syntax.comment,
            style.syntax.constant,
            style.syntax.constant_builtin,
            style.syntax.constant_macro,
            style.syntax.constructor,
            style.syntax.attribute,
            style.syntax.property,
            style.syntax.operator,
            style.syntax.punctuation_delimiter,
            style.syntax.punctuation_bracket,
            style.syntax.punctuation_special,
        );
        style
    }

    pub fn set_syntax_colour(&mut self, name: &str, colour: ColorPair) -> anyhow::Result<()> {
        let target = match name {
            "markdown_code" => &mut self.syntax.markdown_code,
            "markdown_emphasis" => &mut self.syntax.markdown_emphasis,
            "markdown_frontmatter" => &mut self.syntax.markdown_frontmatter,
            "markdown_heading" => &mut self.syntax.markdown_heading,
            "markdown_highlight" => &mut self.syntax.markdown_highlight,
            "markdown_link" => &mut self.syntax.markdown_link,
            "markdown_list_marker" => &mut self.syntax.markdown_list_marker,
            "markdown_strong" => &mut self.syntax.markdown_strong,
            "variable_builtin" => &mut self.syntax.variable_builtin,
            "variable_parameter" => &mut self.syntax.variable_parameter,
            "keyword" => &mut self.syntax.keyword,
            "keyword_operator" => &mut self.syntax.keyword_operator,
            "keyword_import" => &mut self.syntax.keyword_import,
            "type" | "type_name" => &mut self.syntax.type_name,
            "type_builtin" => &mut self.syntax.type_builtin,
            "type_definition" => &mut self.syntax.type_definition,
            "function" => &mut self.syntax.function,
            "function_macro" => &mut self.syntax.function_macro,
            "function_method" => &mut self.syntax.function_method,
            "string" => &mut self.syntax.string,
            "string_escape" => &mut self.syntax.string_escape,
            "character" => &mut self.syntax.character,
            "number" => &mut self.syntax.number,
            "boolean" => &mut self.syntax.boolean,
            "float" => &mut self.syntax.float,
            "comment" => &mut self.syntax.comment,
            "constant" => &mut self.syntax.constant,
            "constant_builtin" => &mut self.syntax.constant_builtin,
            "constant_macro" => &mut self.syntax.constant_macro,
            "constructor" => &mut self.syntax.constructor,
            "attribute" => &mut self.syntax.attribute,
            "property" => &mut self.syntax.property,
            "operator" => &mut self.syntax.operator,
            "punctuation_delimiter" => &mut self.syntax.punctuation_delimiter,
            "punctuation_bracket" => &mut self.syntax.punctuation_bracket,
            "punctuation_special" => &mut self.syntax.punctuation_special,
            _ => anyhow::bail!("unknown syntax colour {name:?}"),
        };
        *target = colour;
        Ok(())
    }

    pub fn set_ui_colour(&mut self, name: &str, colour: ColorPair) -> anyhow::Result<()> {
        macro_rules! colour_target {
            ($($name:literal => $target:expr),+ $(,)?) => {
                match name { $($name => &mut $target,)+ _ => anyhow::bail!("unknown UI colour {name:?}"), }
            };
        }
        let target = colour_target! {
            "git.added" => self.git.added, "git.modified" => self.git.modified,
            "git.conflict" => self.git.conflict, "git.removed" => self.git.removed,
            "status.bar" => self.status_line.bar, "status.path" => self.status_line.path,
            "status.dirty" => self.status_line.dirty, "status.mode_normal" => self.status_line.mode_normal,
            "status.mode_insert" => self.status_line.mode_insert, "status.mode_command" => self.status_line.mode_command,
            "status.mode_visual" => self.status_line.mode_visual, "status.metadata_wrapper" => self.status_line.metadata.wrapper,
            "status.metadata_content" => self.status_line.metadata.content, "status.coords_wrapper" => self.status_line.coords.wrapper,
            "status.coords_content" => self.status_line.coords.content, "status.minimap_wrapper" => self.status_line.minimap_module.wrapper,
            "status.minimap_content" => self.status_line.minimap_module.content, "status.minimap" => self.status_line.minimap,
            "status.minimap_alt" => self.status_line.minimap_alt,
            "about.border" => self.about.border, "about.title" => self.about.title, "about.text" => self.about.text,
            "about.logo_red" => self.about.logo_red, "about.logo_white" => self.about.logo_white, "about.logo_blue" => self.about.logo_blue,
            "command_line.border" => self.command_line.border, "command_line.title" => self.command_line.title,
            "command_line.text" => self.command_line.text, "command_line.prompt" => self.command_line.prompt,
            "diagnostic.error" => self.diagnostic_inline.error, "diagnostic.warning" => self.diagnostic_inline.warning,
            "diagnostic.information" => self.diagnostic_inline.information, "diagnostic.hint" => self.diagnostic_inline.hint,
            "explorer.border" => self.explorer.border, "explorer.title" => self.explorer.title, "explorer.file" => self.explorer.file,
            "explorer.directory" => self.explorer.directory, "explorer.executable" => self.explorer.executable, "explorer.hidden" => self.explorer.hidden,
            "finder.border" => self.finder.border, "finder.title" => self.finder.title, "finder.text" => self.finder.text,
            "finder.prompt" => self.finder.prompt, "finder.query_title" => self.finder.query_title, "finder.dim" => self.finder.dim,
            "finder.match_highlight" => self.finder.match_highlight, "finder.selected" => self.finder.selected,
            "finder.pinned_bg" => self.finder.pinned_bg, "finder.pinned_marker" => self.finder.pinned_marker,
            "finder.hotkey" => self.finder.hotkey, "finder.preview_title" => self.finder.preview_title, "finder.preview_path" => self.finder.preview_path,
            "perf.border" => self.perf.border, "perf.title" => self.perf.title, "perf.text" => self.perf.text,
            "perf.label" => self.perf.label, "perf.value" => self.perf.value, "perf.dim" => self.perf.dim,
            "perf.good" => self.perf.good, "perf.warn" => self.perf.warn, "perf.hot" => self.perf.hot, "perf.bar_bg" => self.perf.bar_bg,
            "undo_tree.text" => self.undo_tree.text, "undo_tree.selected" => self.undo_tree.selected,
            "undo_tree.selected_indicator" => self.undo_tree.selected_indicator, "undo_tree.node" => self.undo_tree.node,
            "undo_tree.node_label" => self.undo_tree.node_label, "undo_tree.redo_marker" => self.undo_tree.redo_marker,
            "undo_tree.edge" => self.undo_tree.edge, "undo_tree.timestamp" => self.undo_tree.timestamp,
            "undo_tree.preview_title" => self.undo_tree.preview_title, "undo_tree.preview_label" => self.undo_tree.preview_label,
            "undo_tree.preview_text" => self.undo_tree.preview_text, "undo_tree.preview_dim" => self.undo_tree.preview_dim,
            "undo_tree.preview_deleted" => self.undo_tree.preview_deleted, "undo_tree.preview_inserted" => self.undo_tree.preview_inserted
        };
        *target = colour;
        Ok(())
    }

    pub fn set_popup_size(
        &mut self,
        name: &str,
        width: Option<u16>,
        height: Option<u16>,
        min_width: Option<u16>,
        min_height: Option<u16>,
    ) {
        macro_rules! apply {
            ($popup:expr) => {{
                if let Some(v) = width {
                    $popup.width_percent = v;
                }
                if let Some(v) = height {
                    $popup.height_percent = v;
                }
                if let Some(v) = min_width {
                    $popup.min_width = v;
                }
                if let Some(v) = min_height {
                    $popup.min_height = v;
                }
            }};
        }
        match name {
            "about" => apply!(self.about),
            "explorer" => apply!(self.explorer),
            "finder" | "diagnostics" | "code_actions" => apply!(self.finder),
            "lsp_marketplace" => apply!(self.lsp_marketplace),
            "perf" => apply!(self.perf),
            "command_line" => {
                if let Some(v) = width {
                    self.command_line.width_percent = v;
                }
                if let Some(v) = min_width {
                    self.command_line.min_width = v;
                }
            }
            "undo_tree" => {
                if let Some(v) = width {
                    self.undo_tree.width_percent = v;
                }
                if let Some(v) = min_width {
                    self.undo_tree.min_width = v;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_status_modules_keep_the_original_dark_palette() {
        let style = UiStyle::default();

        assert_eq!(
            style.status_line.metadata.wrapper.fg,
            style.status_line.bar.bg
        );
        assert_eq!(
            style.status_line.metadata.content,
            ColorPair::new(style.theme.black, style.theme.dark_gray)
        );
        assert_eq!(
            style.status_line.coords.wrapper.fg,
            style.status_line.bar.bg
        );
        assert_eq!(
            style.status_line.coords.content,
            ColorPair::new(style.theme.black, style.theme.dark_gray)
        );
    }

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
    fn dimmed_style_preserves_matching_backgrounds_and_fades_custom_foregrounds() {
        let mut style = UiStyle::default();
        let background = style.theme.purple;
        style.finder.selected = ColorPair::new(
            Color::Rgb {
                r: 200,
                g: 100,
                b: 50,
            },
            background,
        );
        style.dim_amount = 0.5;
        let dimmed = style.dimmed();
        assert_eq!(dimmed.finder.selected.bg, background);
        assert_eq!(
            dimmed.finder.selected.fg,
            Color::Rgb {
                r: 113,
                g: 63,
                b: 39
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
