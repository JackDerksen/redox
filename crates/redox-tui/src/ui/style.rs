//! Visual style and small layout config for `redox-tui`.

pub use crate::ui::text_style::{TextFormat, TextStyle, Underline};

use minui::Color;

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
                r: (255),
                g: (218),
                b: (132),
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
                r: (255),
                g: (233),
                b: (162),
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
    pub wrapper: TextStyle,
    pub content: TextStyle,
}

impl StatusModuleColors {
    pub fn solid(colors: TextStyle) -> Self {
        Self {
            wrapper: colors,
            content: colors,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GitStyle {
    pub added: TextStyle,
    pub modified: TextStyle,
    pub conflict: TextStyle,
    pub removed: TextStyle,
}

impl GitStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            added: TextStyle::new(theme.green, theme.bg),
            modified: TextStyle::new(theme.yellow, theme.bg),
            conflict: TextStyle::new(theme.orange, theme.bg),
            removed: TextStyle::new(theme.red, theme.bg),
        }
    }

    pub fn file_status(self, status: GitFileStatusKind) -> TextStyle {
        match status {
            GitFileStatusKind::Added => self.added,
            GitFileStatusKind::Modified => self.modified,
            GitFileStatusKind::Conflict => self.conflict,
            GitFileStatusKind::Removed => self.removed,
        }
    }

    pub fn gutter_marker(self, kind: GitGutterKind) -> (&'static str, TextStyle) {
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
    pub bar: TextStyle,
    pub path: TextStyle,
    pub dirty: TextStyle,
    pub saved: TextStyle,
    pub mode_normal: TextStyle,
    pub mode_insert: TextStyle,
    pub mode_command: TextStyle,
    pub mode_visual: TextStyle,
    pub metadata: StatusModuleColors,
    pub language_icon: TextStyle,
    pub coords: StatusModuleColors,
    pub minimap_module: StatusModuleColors,
    pub minimap: TextStyle,
    pub minimap_alt: TextStyle,
}

impl StatusLinePalette {
    pub fn from_theme(theme: BaseTheme) -> Self {
        // Muted text at approximately 4:1 contrast on the default status backgrounds.
        let bar_text = dim_foreground_color(theme.white, theme.black, 0.515);
        let module_wrapper = TextStyle::new(theme.black, theme.dark_gray);
        let module_text = TextStyle::new(
            dim_foreground_color(theme.white, theme.dark_gray, 0.466),
            theme.dark_gray,
        );
        Self {
            bar: TextStyle::new(bar_text, theme.black),
            path: TextStyle::new(bar_text, theme.black),
            dirty: TextStyle::new(bar_text, Color::Transparent),
            saved: TextStyle::new(theme.green, theme.black),
            mode_normal: TextStyle::new(theme.black, theme.purple),
            mode_insert: TextStyle::new(theme.black, theme.blue),
            mode_command: TextStyle::new(theme.black, theme.red),
            mode_visual: TextStyle::new(theme.black, theme.orange),
            metadata: StatusModuleColors {
                wrapper: module_wrapper,
                content: module_text,
            },
            language_icon: TextStyle::new(module_text.fg, Color::Transparent),
            coords: StatusModuleColors {
                wrapper: module_wrapper,
                content: module_text,
            },
            minimap_module: StatusModuleColors::solid(module_wrapper),
            minimap: TextStyle::new(module_text.fg, Color::Transparent),
            minimap_alt: TextStyle::new(Color::Transparent, module_text.fg),
        }
    }
}

impl Default for StatusLinePalette {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineNumbers {
    #[default]
    Relative,
    Absolute,
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub status_left_min_width: u16,
    pub status_right_min_width: u16,
    pub color_column: Option<usize>,
    pub line_numbers: LineNumbers,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            status_left_min_width: 12,
            status_right_min_width: 18,
            color_column: Some(79),
            line_numbers: LineNumbers::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExplorerStyle {
    pub width_percent: u16,
    pub height_percent: u16,
    pub min_width: u16,
    pub min_height: u16,
    pub border: TextStyle,
    pub title: TextStyle,
    pub file: TextStyle,
    pub directory: TextStyle,
    pub executable: TextStyle,
    pub hidden: TextStyle,
}

impl ExplorerStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 65,
            height_percent: 60,
            min_width: 20,
            min_height: 6,
            border: TextStyle::new(theme.light_gray, theme.bg),
            title: TextStyle::new(theme.light_gray, theme.bg).bold(),
            file: TextStyle::new(theme.white, theme.bg),
            directory: TextStyle::new(theme.blue, theme.bg).bold(),
            executable: TextStyle::new(theme.red, theme.bg),
            hidden: TextStyle::new(theme.dark_gray, theme.bg),
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
    pub border: TextStyle,
    pub title: TextStyle,
    pub text: TextStyle,
    pub logo_red: TextStyle,
    pub logo_white: TextStyle,
    pub logo_blue: TextStyle,
}

impl AboutStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 65,
            height_percent: 52,
            min_width: 52,
            min_height: 12,
            border: TextStyle::new(theme.light_gray, theme.bg),
            title: TextStyle::new(theme.light_gray, theme.bg).bold(),
            text: TextStyle::new(theme.white, theme.bg),
            logo_red: TextStyle::new(theme.red, theme.bg).bold(),
            logo_white: TextStyle::new(theme.white, theme.bg).bold(),
            logo_blue: TextStyle::new(theme.blue, theme.bg).bold(),
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
    pub error: TextStyle,
    pub inactive_title: TextStyle,
    pub width_percent: u16,
    pub min_width: u16,
    pub inner_height_rows: u16,
    pub stacked_padding: u16,
    pub border: TextStyle,
    pub title: TextStyle,
    pub text: TextStyle,
    pub ghost: TextStyle,
    pub prompt: TextStyle,
}

impl CommandLineStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            error: TextStyle::new(theme.red, theme.bg),
            inactive_title: TextStyle::new(theme.light_gray, theme.bg).bold(),
            width_percent: 65,
            min_width: 24,
            inner_height_rows: 1,
            stacked_padding: 0,
            border: TextStyle::new(theme.light_gray, theme.bg),
            title: TextStyle::new(theme.red, theme.bg).bold(),
            text: TextStyle::new(theme.white, theme.bg),
            ghost: TextStyle::new(theme.dark_gray, theme.bg),
            prompt: TextStyle::new(theme.light_gray, theme.bg),
        }
    }
}

impl Default for CommandLineStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WhichKeyStyle {
    pub background: Color,
    pub edge: TextStyle,
    pub prefix: TextStyle,
    pub key: TextStyle,
    pub arrow: TextStyle,
    pub text: TextStyle,
}

impl WhichKeyStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            background: theme.color_column,
            edge: TextStyle::new(theme.light_purple, theme.color_column),
            prefix: TextStyle::new(theme.light_purple, theme.color_column).bold(),
            key: TextStyle::new(theme.light_blue, theme.color_column),
            arrow: TextStyle::new(theme.light_gray, theme.color_column),
            text: TextStyle::new(theme.white, theme.color_column),
        }
    }
}

impl Default for WhichKeyStyle {
    fn default() -> Self {
        Self::from_theme(BaseTheme::default())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticInlineStyle {
    pub error: TextStyle,
    pub warning: TextStyle,
    pub information: TextStyle,
    pub hint: TextStyle,
}

impl DiagnosticInlineStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            error: TextStyle::new(
                theme.light_red,
                Color::Rgb {
                    r: 49,
                    g: 38,
                    b: 43,
                },
            ),
            warning: TextStyle::new(
                theme.light_orange,
                Color::Rgb {
                    r: 49,
                    g: 43,
                    b: 42,
                },
            ),
            information: TextStyle::new(
                theme.light_gray,
                Color::Rgb {
                    r: 35,
                    g: 34,
                    b: 38,
                },
            ),
            hint: TextStyle::new(
                theme.light_blue,
                Color::Rgb {
                    r: 39,
                    g: 45,
                    b: 49,
                },
            ),
        }
    }

    pub fn colors(self, severity: DiagnosticSeverity) -> TextStyle {
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
    pub directory: TextStyle,
    pub pinned: TextStyle,
    pub width_percent: u16,
    pub height_percent: u16,
    pub min_width: u16,
    pub min_height: u16,
    pub border: TextStyle,
    pub title: TextStyle,
    pub text: TextStyle,
    pub prompt: TextStyle,
    pub query_title: TextStyle,
    pub dim: TextStyle,
    pub match_highlight: TextStyle,
    pub selected: TextStyle,
    pub pinned_bg: TextStyle,
    pub pinned_marker: TextStyle,
    pub hotkey: TextStyle,
    pub preview_title: TextStyle,
    pub preview_path: TextStyle,
}

impl FinderStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            directory: TextStyle::new(Color::Transparent, Color::Transparent).bold(),
            pinned: TextStyle::new(Color::Transparent, Color::Transparent).italic(),
            width_percent: 65,
            height_percent: 60,
            min_width: 52,
            min_height: 14,
            border: TextStyle::new(theme.light_gray, theme.bg),
            title: TextStyle::new(theme.light_blue, theme.bg).bold(),
            text: TextStyle::new(theme.white, theme.bg),
            prompt: TextStyle::new(theme.light_gray, theme.bg),
            query_title: TextStyle::new(theme.light_blue, theme.bg).bold(),
            dim: TextStyle::new(theme.light_gray, theme.bg),
            match_highlight: TextStyle::new(theme.orange, theme.bg).underlined(),
            selected: TextStyle::new(theme.white, theme.black),
            pinned_bg: TextStyle::new(theme.white, theme.dark_gray),
            pinned_marker: TextStyle::new(theme.light_blue, theme.dark_gray),
            hotkey: TextStyle::new(theme.light_gray, theme.dark_gray),
            preview_title: TextStyle::new(theme.light_blue, theme.bg).bold(),
            preview_path: TextStyle::new(theme.light_gray, theme.bg).bold(),
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
    pub title: TextStyle,
    pub text: TextStyle,
    pub selected: TextStyle,
    pub selected_indicator: TextStyle,
    pub node: TextStyle,
    pub node_label: TextStyle,
    pub redo_marker: TextStyle,
    pub edge: TextStyle,
    pub timestamp: TextStyle,
    pub preview_title: TextStyle,
    pub preview_label: TextStyle,
    pub preview_text: TextStyle,
    pub preview_dim: TextStyle,
    pub preview_separator: TextStyle,
    pub preview_deleted: TextStyle,
    pub preview_inserted: TextStyle,
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
            title: TextStyle::new(theme.blue, theme.bg).bold(),
            text: TextStyle::new(theme.white, theme.bg),
            selected: TextStyle::new(theme.white, theme.black),
            selected_indicator: TextStyle::new(theme.orange, theme.bg),
            node: TextStyle::new(theme.white, theme.bg),
            node_label: TextStyle::new(theme.light_gray, theme.bg),
            redo_marker: TextStyle::new(theme.purple, theme.bg),
            edge: TextStyle::new(theme.light_gray, theme.bg),
            timestamp: TextStyle::new(theme.dark_gray, theme.bg),
            preview_title: TextStyle::new(theme.light_gray, theme.bg).bold(),
            preview_label: TextStyle::new(theme.light_gray, theme.bg).bold(),
            preview_text: TextStyle::new(theme.white, theme.bg),
            preview_dim: TextStyle::new(theme.dark_gray, theme.bg),
            preview_separator: TextStyle::new(theme.dark_gray, theme.bg),
            preview_deleted: TextStyle::new(theme.red, theme.bg),
            preview_inserted: TextStyle::new(theme.green, theme.bg),
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
    pub border: TextStyle,
    pub title: TextStyle,
    pub text: TextStyle,
    pub label: TextStyle,
    pub value: TextStyle,
    pub dim: TextStyle,
    pub good: TextStyle,
    pub warn: TextStyle,
    pub hot: TextStyle,
    pub bar_bg: TextStyle,
}

impl PerfStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            width_percent: 44,
            height_percent: 34,
            min_width: 40,
            min_height: 12,
            border: TextStyle::new(theme.light_gray, theme.bg),
            title: TextStyle::new(theme.yellow, theme.bg).bold(),
            text: TextStyle::new(theme.white, theme.bg),
            label: TextStyle::new(theme.light_gray, theme.bg),
            value: TextStyle::new(theme.white, theme.bg),
            dim: TextStyle::new(theme.dark_gray, theme.bg),
            good: TextStyle::new(theme.green, theme.bg),
            warn: TextStyle::new(theme.yellow, theme.bg),
            hot: TextStyle::new(theme.red, theme.bg),
            bar_bg: TextStyle::new(theme.dark_gray, theme.bg),
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
    pub markdown_code: TextStyle,
    pub markdown_emphasis: TextStyle,
    pub markdown_frontmatter: TextStyle,
    pub markdown_heading: TextStyle,
    pub markdown_highlight: TextStyle,
    pub markdown_link: TextStyle,
    pub markdown_list_marker: TextStyle,
    pub markdown_strong: TextStyle,
    pub variable_builtin: TextStyle,
    pub variable_parameter: TextStyle,
    pub keyword: TextStyle,
    pub keyword_operator: TextStyle,
    pub keyword_import: TextStyle,
    pub type_name: TextStyle,
    pub type_builtin: TextStyle,
    pub type_definition: TextStyle,
    pub function: TextStyle,
    pub function_macro: TextStyle,
    pub function_method: TextStyle,
    pub string: TextStyle,
    pub string_escape: TextStyle,
    pub character: TextStyle,
    pub number: TextStyle,
    pub boolean: TextStyle,
    pub float: TextStyle,
    pub comment: TextStyle,
    pub constant: TextStyle,
    pub constant_builtin: TextStyle,
    pub constant_macro: TextStyle,
    pub constructor: TextStyle,
    pub attribute: TextStyle,
    pub property: TextStyle,
    pub operator: TextStyle,
    pub punctuation_delimiter: TextStyle,
    pub punctuation_bracket: TextStyle,
    pub punctuation_special: TextStyle,
}

impl SyntaxStyle {
    pub fn from_theme(theme: BaseTheme) -> Self {
        let bg = theme.bg;
        Self {
            markdown_code: TextStyle::new(theme.light_gray, bg),
            markdown_emphasis: TextStyle::new(theme.orange, bg).italic(),
            markdown_frontmatter: TextStyle::new(theme.dark_gray, bg),
            markdown_heading: TextStyle::new(theme.blue, bg).bold().underlined(),
            markdown_highlight: TextStyle::new(theme.black, theme.green),
            markdown_link: TextStyle::new(theme.purple, bg),
            markdown_list_marker: TextStyle::new(theme.light_gray, bg),
            markdown_strong: TextStyle::new(theme.red, bg).bold(),
            variable_builtin: TextStyle::new(theme.purple, bg),
            variable_parameter: TextStyle::new(theme.orange, bg),
            keyword: TextStyle::new(theme.red, bg),
            keyword_operator: TextStyle::new(theme.white, bg),
            keyword_import: TextStyle::new(theme.white, bg),
            type_name: TextStyle::new(theme.light_blue, bg),
            type_builtin: TextStyle::new(theme.light_orange, bg),
            type_definition: TextStyle::new(theme.light_orange, bg),
            function: TextStyle::new(theme.blue, bg),
            function_macro: TextStyle::new(theme.purple, bg),
            function_method: TextStyle::new(theme.blue, bg),
            string: TextStyle::new(theme.green, bg),
            string_escape: TextStyle::new(theme.orange, bg),
            character: TextStyle::new(theme.green, bg),
            number: TextStyle::new(theme.purple, bg),
            boolean: TextStyle::new(theme.purple, bg),
            float: TextStyle::new(theme.light_purple, bg),
            comment: TextStyle::new(theme.dark_gray, bg).italic(),
            constant: TextStyle::new(theme.purple, bg),
            constant_builtin: TextStyle::new(theme.purple, bg),
            constant_macro: TextStyle::new(theme.yellow, bg),
            constructor: TextStyle::new(theme.white, bg),
            attribute: TextStyle::new(theme.orange, bg),
            property: TextStyle::new(theme.orange, bg),
            operator: TextStyle::new(theme.white, bg),
            punctuation_delimiter: TextStyle::new(theme.white, bg),
            punctuation_bracket: TextStyle::new(theme.white, bg),
            punctuation_special: TextStyle::new(theme.orange, bg),
        }
    }

    pub fn color_for(self, role: SyntaxRole) -> TextStyle {
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
pub struct DashboardStyle {
    pub text: TextStyle,
    pub selected: TextStyle,
    pub hotkey: TextStyle,
    pub version: TextStyle,
    pub icon: TextStyle,
    pub logo_red: TextStyle,
    pub logo_white: TextStyle,
    pub logo_blue: TextStyle,
}

impl DashboardStyle {
    fn from_theme(theme: BaseTheme) -> Self {
        Self {
            text: TextStyle::new(theme.light_gray, theme.bg),
            selected: TextStyle::new(theme.white, theme.bg),
            hotkey: TextStyle::new(theme.blue, theme.bg).bold(),
            version: TextStyle::new(theme.light_gray, theme.bg).italic(),
            icon: TextStyle::new(theme.red, theme.bg),
            logo_red: TextStyle::new(theme.red, theme.bg),
            logo_white: TextStyle::new(theme.white, theme.bg),
            logo_blue: TextStyle::new(theme.blue, theme.bg),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UiStyle {
    pub snippet: TextStyle,
    pub gutter_current_line_number: TextStyle,
    pub gutter_line_number: TextStyle,
    pub completion_match: TextStyle,
    pub completion_keyword: TextStyle,
    pub completion_ghost: TextStyle,
    pub text_formatting: bool,
    pub dashboard: DashboardStyle,
    pub editor_text: TextStyle,
    pub pane_title: TextStyle,
    pub section_title: TextStyle,
    pub search_match: TextStyle,
    pub search_current: TextStyle,
    pub error_range: TextStyle,
    pub theme: BaseTheme,
    pub zen_margin: Color,
    pub zen_ghost: Color,
    pub icons_enabled: bool,
    pub git: GitStyle,
    pub status_line: StatusLinePalette,
    pub layout: Layout,
    pub about: AboutStyle,
    pub command_line: CommandLineStyle,
    pub which_key: WhichKeyStyle,
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

const SUBSTITUTE_BACKGROUND_DIM_AMOUNT: f32 = 0.9;

impl UiStyle {
    pub(crate) fn substitute_colors(self, replacing: bool) -> TextStyle {
        if replacing {
            TextStyle::new(
                self.theme.light_green,
                dim_foreground_color(
                    self.theme.light_green,
                    self.theme.bg,
                    SUBSTITUTE_BACKGROUND_DIM_AMOUNT,
                ),
            )
        } else {
            self.diagnostic_inline.error
        }
    }

    pub fn from_theme(theme: BaseTheme) -> Self {
        Self {
            completion_ghost: TextStyle::new(theme.dark_gray, theme.bg),
            completion_keyword: TextStyle::new(theme.light_gray, theme.bg),
            completion_match: TextStyle::new(theme.white, theme.bg),
            gutter_line_number: TextStyle::new(theme.dark_gray, theme.bg),
            gutter_current_line_number: TextStyle::new(theme.white, theme.bg),
            snippet: TextStyle::new(theme.dark_gray, theme.bg),
            text_formatting: true,
            dashboard: DashboardStyle::from_theme(theme),
            editor_text: TextStyle::new(theme.white, theme.bg),
            pane_title: TextStyle::new(theme.light_gray, theme.black).bold(),
            section_title: TextStyle::new(Color::Transparent, Color::Transparent).bold(),
            search_match: TextStyle::new(Color::Transparent, theme.selection_bg).underlined(),
            search_current: TextStyle::new(Color::Transparent, theme.light_gray).underlined(),
            error_range: TextStyle {
                format: TextFormat {
                    underline: Underline::Curl,
                    underline_color: Some(theme.light_red),
                    ..TextFormat::default()
                },
                ..TextStyle::new(Color::Transparent, Color::Transparent)
            },
            theme,
            zen_margin: dim_foreground_color(theme.bg, Color::Rgb { r: 0, g: 0, b: 0 }, 0.12),
            zen_ghost: theme.dark_gray,
            icons_enabled: false,
            git: GitStyle::from_theme(theme),
            status_line: StatusLinePalette::from_theme(theme),
            layout: Layout::default(),
            about: AboutStyle::from_theme(theme),
            command_line: CommandLineStyle::from_theme(theme),
            which_key: WhichKeyStyle::from_theme(theme),
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

pub(crate) fn dim_foreground_color(color: Color, bg: Color, amount: f32) -> Color {
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

fn dim_style_color(
    color: Color,
    theme: BaseTheme,
    dimmed: BaseTheme,
    bg: Color,
    amount: f32,
) -> Color {
    let theme_colors = [
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
    if let Some((_, replacement)) = theme_colors.iter().find(|(original, _)| *original == color) {
        return *replacement;
    }
    dim_foreground_color(color, bg, amount)
}

impl UiStyle {
    #[cfg(test)]
    pub fn dimmed(self) -> Self {
        self.dimmed_by(self.dim_amount)
    }

    pub(crate) fn dimmed_by(self, amount: f32) -> Self {
        if amount == 0.0 {
            return self;
        }
        let mut style = self;
        let bg = self.theme.bg;
        let dimmed_theme = self.theme.dimmed(amount);
        style.theme = dimmed_theme;
        style.zen_ghost = dim_foreground_color(self.zen_ghost, bg, amount);
        let dim = |pair: &mut TextStyle| {
            pair.fg = dim_style_color(pair.fg, self.theme, dimmed_theme, bg, amount);
            if let Some(color) = pair.format.underline_color.as_mut() {
                *color = dim_style_color(*color, self.theme, dimmed_theme, bg, amount);
            }
        };
        for (_, pair) in style.syntax_roles_mut() {
            dim(pair);
        }
        for (_, pair) in style.ui_roles_mut() {
            dim(pair);
        }
        style
    }

    pub(crate) fn syntax_roles_mut(
        &mut self,
    ) -> impl Iterator<Item = (&'static str, &mut TextStyle)> {
        [
            ("markdown_code", &mut self.syntax.markdown_code),
            ("markdown_emphasis", &mut self.syntax.markdown_emphasis),
            (
                "markdown_frontmatter",
                &mut self.syntax.markdown_frontmatter,
            ),
            ("markdown_heading", &mut self.syntax.markdown_heading),
            ("markdown_highlight", &mut self.syntax.markdown_highlight),
            ("markdown_link", &mut self.syntax.markdown_link),
            (
                "markdown_list_marker",
                &mut self.syntax.markdown_list_marker,
            ),
            ("markdown_strong", &mut self.syntax.markdown_strong),
            ("variable_builtin", &mut self.syntax.variable_builtin),
            ("variable_parameter", &mut self.syntax.variable_parameter),
            ("keyword", &mut self.syntax.keyword),
            ("keyword_operator", &mut self.syntax.keyword_operator),
            ("keyword_import", &mut self.syntax.keyword_import),
            ("type", &mut self.syntax.type_name),
            ("type_builtin", &mut self.syntax.type_builtin),
            ("type_definition", &mut self.syntax.type_definition),
            ("function", &mut self.syntax.function),
            ("function_macro", &mut self.syntax.function_macro),
            ("function_method", &mut self.syntax.function_method),
            ("string", &mut self.syntax.string),
            ("string_escape", &mut self.syntax.string_escape),
            ("character", &mut self.syntax.character),
            ("number", &mut self.syntax.number),
            ("boolean", &mut self.syntax.boolean),
            ("float", &mut self.syntax.float),
            ("comment", &mut self.syntax.comment),
            ("constant", &mut self.syntax.constant),
            ("constant_builtin", &mut self.syntax.constant_builtin),
            ("constant_macro", &mut self.syntax.constant_macro),
            ("constructor", &mut self.syntax.constructor),
            ("attribute", &mut self.syntax.attribute),
            ("property", &mut self.syntax.property),
            ("operator", &mut self.syntax.operator),
            (
                "punctuation_delimiter",
                &mut self.syntax.punctuation_delimiter,
            ),
            ("punctuation_bracket", &mut self.syntax.punctuation_bracket),
            ("punctuation_special", &mut self.syntax.punctuation_special),
        ]
        .into_iter()
    }

    pub(crate) fn ui_roles_mut(&mut self) -> impl Iterator<Item = (&'static str, &mut TextStyle)> {
        [
            ("git.added", &mut self.git.added),
            ("git.modified", &mut self.git.modified),
            ("git.conflict", &mut self.git.conflict),
            ("git.removed", &mut self.git.removed),
            ("status.bar", &mut self.status_line.bar),
            ("status.path", &mut self.status_line.path),
            ("status.dirty", &mut self.status_line.dirty),
            ("status.saved", &mut self.status_line.saved),
            ("status.mode_normal", &mut self.status_line.mode_normal),
            ("status.mode_insert", &mut self.status_line.mode_insert),
            ("status.mode_command", &mut self.status_line.mode_command),
            ("status.mode_visual", &mut self.status_line.mode_visual),
            (
                "status.metadata_wrapper",
                &mut self.status_line.metadata.wrapper,
            ),
            (
                "status.metadata_content",
                &mut self.status_line.metadata.content,
            ),
            ("status.language_icon", &mut self.status_line.language_icon),
            (
                "status.coords_wrapper",
                &mut self.status_line.coords.wrapper,
            ),
            (
                "status.coords_content",
                &mut self.status_line.coords.content,
            ),
            (
                "status.minimap_wrapper",
                &mut self.status_line.minimap_module.wrapper,
            ),
            (
                "status.minimap_content",
                &mut self.status_line.minimap_module.content,
            ),
            ("status.minimap", &mut self.status_line.minimap),
            ("status.minimap_alt", &mut self.status_line.minimap_alt),
            ("about.border", &mut self.about.border),
            ("about.title", &mut self.about.title),
            ("about.text", &mut self.about.text),
            ("about.logo_red", &mut self.about.logo_red),
            ("about.logo_white", &mut self.about.logo_white),
            ("about.logo_blue", &mut self.about.logo_blue),
            ("command_line.border", &mut self.command_line.border),
            ("command_line.title", &mut self.command_line.title),
            ("command_line.text", &mut self.command_line.text),
            ("command_line.prompt", &mut self.command_line.prompt),
            ("diagnostic.error", &mut self.diagnostic_inline.error),
            ("diagnostic.warning", &mut self.diagnostic_inline.warning),
            (
                "diagnostic.information",
                &mut self.diagnostic_inline.information,
            ),
            ("diagnostic.hint", &mut self.diagnostic_inline.hint),
            ("explorer.border", &mut self.explorer.border),
            ("explorer.title", &mut self.explorer.title),
            ("explorer.file", &mut self.explorer.file),
            ("explorer.directory", &mut self.explorer.directory),
            ("explorer.executable", &mut self.explorer.executable),
            ("explorer.hidden", &mut self.explorer.hidden),
            ("finder.border", &mut self.finder.border),
            ("finder.title", &mut self.finder.title),
            ("finder.text", &mut self.finder.text),
            ("finder.prompt", &mut self.finder.prompt),
            ("finder.query_title", &mut self.finder.query_title),
            ("finder.dim", &mut self.finder.dim),
            ("finder.match_highlight", &mut self.finder.match_highlight),
            ("finder.selected", &mut self.finder.selected),
            ("finder.pinned_bg", &mut self.finder.pinned_bg),
            ("finder.pinned_marker", &mut self.finder.pinned_marker),
            ("finder.hotkey", &mut self.finder.hotkey),
            ("finder.preview_title", &mut self.finder.preview_title),
            ("finder.preview_path", &mut self.finder.preview_path),
            ("perf.border", &mut self.perf.border),
            ("perf.title", &mut self.perf.title),
            ("perf.text", &mut self.perf.text),
            ("perf.label", &mut self.perf.label),
            ("perf.value", &mut self.perf.value),
            ("perf.dim", &mut self.perf.dim),
            ("perf.good", &mut self.perf.good),
            ("perf.warn", &mut self.perf.warn),
            ("perf.hot", &mut self.perf.hot),
            ("perf.bar_bg", &mut self.perf.bar_bg),
            ("undo_tree.title", &mut self.undo_tree.title),
            ("undo_tree.text", &mut self.undo_tree.text),
            ("undo_tree.selected", &mut self.undo_tree.selected),
            (
                "undo_tree.selected_indicator",
                &mut self.undo_tree.selected_indicator,
            ),
            ("undo_tree.node", &mut self.undo_tree.node),
            ("undo_tree.node_label", &mut self.undo_tree.node_label),
            ("undo_tree.redo_marker", &mut self.undo_tree.redo_marker),
            ("undo_tree.edge", &mut self.undo_tree.edge),
            ("undo_tree.timestamp", &mut self.undo_tree.timestamp),
            ("undo_tree.preview_title", &mut self.undo_tree.preview_title),
            ("undo_tree.preview_label", &mut self.undo_tree.preview_label),
            ("undo_tree.preview_text", &mut self.undo_tree.preview_text),
            ("undo_tree.preview_dim", &mut self.undo_tree.preview_dim),
            (
                "undo_tree.preview_separator",
                &mut self.undo_tree.preview_separator,
            ),
            (
                "undo_tree.preview_deleted",
                &mut self.undo_tree.preview_deleted,
            ),
            (
                "undo_tree.preview_inserted",
                &mut self.undo_tree.preview_inserted,
            ),
            ("command_line.error", &mut self.command_line.error),
            (
                "command_line.inactive_title",
                &mut self.command_line.inactive_title,
            ),
            ("command_line.ghost", &mut self.command_line.ghost),
            ("finder.directory", &mut self.finder.directory),
            ("finder.pinned", &mut self.finder.pinned),
            ("which_key.edge", &mut self.which_key.edge),
            ("which_key.prefix", &mut self.which_key.prefix),
            ("which_key.key", &mut self.which_key.key),
            ("which_key.arrow", &mut self.which_key.arrow),
            ("which_key.text", &mut self.which_key.text),
            ("dashboard.text", &mut self.dashboard.text),
            ("dashboard.selected", &mut self.dashboard.selected),
            ("dashboard.hotkey", &mut self.dashboard.hotkey),
            ("dashboard.version", &mut self.dashboard.version),
            ("dashboard.icon", &mut self.dashboard.icon),
            ("dashboard.logo_red", &mut self.dashboard.logo_red),
            ("dashboard.logo_white", &mut self.dashboard.logo_white),
            ("dashboard.logo_blue", &mut self.dashboard.logo_blue),
            ("completion.ghost", &mut self.completion_ghost),
            ("completion.keyword", &mut self.completion_keyword),
            ("completion.match_highlight", &mut self.completion_match),
            ("gutter.line_number", &mut self.gutter_line_number),
            (
                "gutter.current_line_number",
                &mut self.gutter_current_line_number,
            ),
            ("editor.snippet", &mut self.snippet),
            ("editor.text", &mut self.editor_text),
            ("pane.title", &mut self.pane_title),
            ("popup.section_title", &mut self.section_title),
            ("search.match", &mut self.search_match),
            ("search.current", &mut self.search_current),
            ("diagnostic.error_range", &mut self.error_range),
        ]
        .into_iter()
    }

    pub(crate) fn syntax_style_mut(&mut self, name: &str) -> anyhow::Result<&mut TextStyle> {
        let name = if name == "type_name" { "type" } else { name };
        self.syntax_roles_mut()
            .find(|(role, _)| *role == name)
            .map(|(_, style)| style)
            .ok_or_else(|| anyhow::anyhow!("unknown syntax style {name:?}"))
    }

    pub(crate) fn ui_style_mut(&mut self, name: &str) -> anyhow::Result<&mut TextStyle> {
        self.ui_roles_mut()
            .find(|(role, _)| *role == name)
            .map(|(_, style)| style)
            .ok_or_else(|| anyhow::anyhow!("unknown UI style {name:?}"))
    }

    pub(crate) fn disable_text_formatting(&mut self) {
        self.text_formatting = false;
        for (_, style) in self.syntax_roles_mut() {
            style.format = TextFormat::default();
        }
        for (_, style) in self.ui_roles_mut() {
            style.format = TextFormat::default();
        }
    }

    pub fn set_popup_size(
        &mut self,
        name: &str,
        width: Option<u16>,
        height: Option<u16>,
        min_width: Option<u16>,
        min_height: Option<u16>,
        stacked_padding: Option<u16>,
    ) {
        macro_rules! apply {
            ($popup:expr) => {{
                if let Some(value) = width {
                    $popup.width_percent = value;
                }
                if let Some(value) = height {
                    $popup.height_percent = value;
                }
                if let Some(value) = min_width {
                    $popup.min_width = value;
                }
                if let Some(value) = min_height {
                    $popup.min_height = value;
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
                if let Some(value) = width {
                    self.command_line.width_percent = value;
                }
                if let Some(value) = min_width {
                    self.command_line.min_width = value;
                }
                if let Some(value) = stacked_padding {
                    self.command_line.stacked_padding = value;
                }
            }
            "undo_tree" => {
                if let Some(value) = width {
                    self.undo_tree.width_percent = value;
                }
                if let Some(value) = min_width {
                    self.undo_tree.min_width = value;
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
    fn default_status_modules_keep_the_original_dark_backgrounds() {
        let style = UiStyle::default();

        assert_eq!(
            style.status_line.metadata.wrapper.fg,
            style.status_line.bar.bg
        );
        assert_eq!(style.status_line.metadata.content.bg, style.theme.dark_gray);
        assert_eq!(
            style.status_line.coords.wrapper.fg,
            style.status_line.bar.bg
        );
        assert_eq!(style.status_line.coords.content.bg, style.theme.dark_gray);
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
        style.finder.selected = TextStyle::new(
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
