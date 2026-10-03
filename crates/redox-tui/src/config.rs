//! User configuration loading and validation.

use crate::ui::text_style::TextStyle;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use minui::Color;
use serde::de::{MapAccess, Visitor, value::MapAccessDeserializer};
use serde::{Deserialize, Deserializer};

use crate::input::cursor::DEFAULT_SCROLLOFF_ROWS;
use crate::ui::UiStyle;
use crate::ui::style::{LineNumbers, Underline};

pub const DEFAULT_DIM_AMOUNT: f32 = 0.301;
pub const DEFAULT_UNDO_HISTORY_SIZE: usize = usize::MAX;
pub const DEFAULT_WHICH_KEY_DELAY_MS: u64 = 3_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: String,
    pub icons_enabled: bool,
    pub text_formatting: bool,
    pub animations: AnimationConfig,
    pub check_updates: bool,
    pub mouse: bool,
    pub mouse_invert_vertical: bool,
    pub mouse_invert_horizontal: bool,
    pub mouse_scroll_step_vertical: u16,
    pub mouse_scroll_step_horizontal: u16,
    pub logging: LoggingConfig,
    pub background_dimming: f32,
    pub undo_tree_history_size: usize,
    pub scrolloff: usize,
    pub color_column: usize,
    pub line_numbers: LineNumbers,
    pub leader: String,
    pub which_key: WhichKeyConfig,
    pub zen: ZenConfig,
    pub popups: BTreeMap<String, PopupSize>,
    pub keybindings: BTreeMap<String, BTreeMap<String, String>>,
    pub bind: Vec<BindConfig>,
    themes: BTreeMap<String, ThemeConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            icons_enabled: false,
            text_formatting: true,
            animations: AnimationConfig::default(),
            check_updates: true,
            mouse: true,
            mouse_invert_vertical: false,
            mouse_invert_horizontal: false,
            mouse_scroll_step_vertical: 3,
            mouse_scroll_step_horizontal: 3,
            logging: LoggingConfig::default(),
            background_dimming: DEFAULT_DIM_AMOUNT,
            undo_tree_history_size: DEFAULT_UNDO_HISTORY_SIZE,
            scrolloff: DEFAULT_SCROLLOFF_ROWS,
            color_column: 79,
            line_numbers: LineNumbers::default(),
            leader: " ".to_string(),
            which_key: WhichKeyConfig::default(),
            zen: ZenConfig::default(),
            popups: BTreeMap::new(),
            keybindings: BTreeMap::new(),
            bind: Vec::new(),
            themes: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnimationConfig {
    pub enabled: bool,
    #[serde(alias = "yank_ripple_ms")]
    pub yank_highlight_ms: u64,
    #[serde(alias = "jump_pulse_ms")]
    pub jump_highlight_ms: u64,
    #[serde(alias = "undo_redo_ms")]
    pub undo_redo_highlight_ms: u64,
    pub delimiter_blink_ms: u64,
    pub save_confirmation_ms: u64,
    pub save_fade_ms: u64,
    pub toast_fade_ms: u64,
    pub focus_fade_ms: u64,
    pub dashboard_logo_ms: u64,
    pub spinner_frame_ms: u64,
    pub rain_fps: u16,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            yank_highlight_ms: 150,
            jump_highlight_ms: 150,
            undo_redo_highlight_ms: 150,
            delimiter_blink_ms: 150,
            save_confirmation_ms: 600,
            save_fade_ms: 150,
            toast_fade_ms: 150,
            focus_fade_ms: 150,
            dashboard_logo_ms: 500,
            spinner_frame_ms: 100,
            rain_fps: 60,
        }
    }
}

impl AnimationConfig {
    pub fn duration(self, milliseconds: u64) -> Duration {
        Duration::from_millis(if self.enabled { milliseconds } else { 0 })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub enabled: bool,
    pub max_events: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_events: 5_000,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ZenConfig {
    pub enabled: bool,
    pub width_percent: u16,
    pub min_width: u16,
    pub hide_gutter: bool,
    pub hide_color_column: bool,
    pub focus_scope: bool,
    pub hide_diagnostics: bool,
    pub minimal_statusline: bool,
    pub show_toast: bool,
}

impl Default for ZenConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            width_percent: 80,
            min_width: 80,
            hide_gutter: true,
            hide_color_column: true,
            focus_scope: true,
            hide_diagnostics: true,
            minimal_statusline: true,
            show_toast: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BindConfig {
    pub mode: String,
    pub keys: String,
    pub sequence: Option<String>,
    pub command: Option<String>,
    pub desc: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WhichKeyConfig {
    pub enabled: bool,
    pub delay_ms: u64,
}

impl Default for WhichKeyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            delay_ms: DEFAULT_WHICH_KEY_DELAY_MS,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PopupSize {
    pub width_percent: Option<u16>,
    pub height_percent: Option<u16>,
    pub min_width: Option<u16>,
    pub min_height: Option<u16>,
    pub stacked_padding: Option<u16>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ThemeConfig {
    palette: BTreeMap<String, String>,
    syntax: BTreeMap<String, StyleValue>,
    ui: BTreeMap<String, StyleValue>,
}

#[derive(Debug, Clone)]
enum StyleValue {
    Foreground(String),
    Properties(StyleProperties),
}

impl<'de> Deserialize<'de> for StyleValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StyleValueVisitor;

        impl<'de> Visitor<'de> for StyleValueVisitor {
            type Value = StyleValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a colour string or style table")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StyleValue::Foreground(value.to_owned()))
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                StyleProperties::deserialize(MapAccessDeserializer::new(map))
                    .map(StyleValue::Properties)
            }
        }

        deserializer.deserialize_any(StyleValueVisitor)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StyleProperties {
    fg: Option<String>,
    bg: Option<String>,
    bold: Option<bool>,
    italic: Option<bool>,
    dim: Option<bool>,
    reverse: Option<bool>,
    strikethrough: Option<bool>,
    underline: Option<Underline>,
    underline_color: Option<String>,
}

impl Config {
    pub fn path(explicit_path: Option<&Path>) -> PathBuf {
        explicit_path
            .map(Path::to_path_buf)
            .unwrap_or_else(default_config_path)
    }

    pub fn load(explicit_path: Option<&Path>) -> anyhow::Result<(Self, Option<PathBuf>)> {
        let path = Self::path(explicit_path);
        if !path.exists() {
            if explicit_path.is_some() || env::var_os("REDOX_CONFIG").is_some() {
                bail!("configuration file does not exist: {}", path.display());
            }
            return Ok((Self::default(), None));
        }

        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read configuration: {}", path.display()))?;
        let config: Self = toml::from_str(&source)
            .with_context(|| format!("failed to parse configuration: {}", path.display()))?;
        config.validate()?;
        Ok((config, Some(path)))
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.animations.rain_fps > 60 {
            bail!("animations.rain_fps must be between 0 and 60");
        }
        for (name, step) in [
            (
                "mouse_scroll_step_vertical",
                self.mouse_scroll_step_vertical,
            ),
            (
                "mouse_scroll_step_horizontal",
                self.mouse_scroll_step_horizontal,
            ),
        ] {
            if step == 0 {
                bail!("{name} must be at least 1");
            }
        }
        if self.logging.max_events == 0 {
            bail!("logging.max_events must be at least 1");
        }
        validate_percent(Some(self.zen.width_percent), "zen.width_percent")?;
        if self.zen.min_width == 0 {
            bail!("zen.min_width must be at least 1");
        }
        if !(0.0..=1.0).contains(&self.background_dimming) {
            bail!("background_dimming must be between 0.0 and 1.0");
        }
        if self.undo_tree_history_size == 0 {
            bail!("undo_tree_history_size must be at least 1");
        }
        if self.leader.chars().count() != 1 {
            bail!("leader must contain exactly one character");
        }
        if !self.has_theme(&self.theme) {
            bail!(
                "theme {:?} is not defined in [themes.{}]",
                self.theme,
                self.theme
            );
        }
        for (name, popup) in &self.popups {
            if !matches!(
                name.as_str(),
                "about"
                    | "command_line"
                    | "explorer"
                    | "finder"
                    | "diagnostics"
                    | "code_actions"
                    | "lsp_marketplace"
                    | "perf"
                    | "undo_tree"
            ) {
                bail!("unknown popup {name:?}");
            }
            validate_percent(popup.width_percent, &format!("popups.{name}.width_percent"))?;
            validate_percent(
                popup.height_percent,
                &format!("popups.{name}.height_percent"),
            )?;
        }
        for name in self.themes.keys() {
            self.style_for_theme(name)
                .with_context(|| format!("invalid theme {name:?}"))?;
        }
        Ok(())
    }

    pub fn leader(&self) -> char {
        self.leader.chars().next().unwrap_or(' ')
    }

    pub fn has_theme(&self, name: &str) -> bool {
        name == "default" || self.themes.contains_key(name)
    }

    pub fn theme_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once("default").chain(
            self.themes
                .keys()
                .map(String::as_str)
                .filter(|name| *name != "default"),
        )
    }

    pub fn style(&self) -> anyhow::Result<UiStyle> {
        self.style_for_theme(&self.theme)
    }

    pub fn style_for_theme(&self, name: &str) -> anyhow::Result<UiStyle> {
        if !self.has_theme(name) {
            bail!("unknown colorscheme {name:?}");
        }
        let mut style = UiStyle::default();
        if let Some(theme) = self.themes.get(name) {
            apply_palette(&mut style, &theme.palette)?;
            style = UiStyle::from_theme(style.theme);
            let background = style.theme.bg;
            for (name, value) in &theme.syntax {
                value
                    .apply(style.syntax_style_mut(name)?, background)
                    .with_context(|| format!("invalid syntax style {name:?}"))?;
            }
            if let Some(value) = theme.ui.get("which_key.background") {
                let background = value.color_only(style.which_key.background)?;
                style.which_key.background = background;
                for (name, role) in style.ui_roles_mut() {
                    if name.starts_with("which_key.") {
                        role.bg = background;
                    }
                }
            }
            for (name, value) in &theme.ui {
                if matches!(
                    name.as_str(),
                    "diagnostic.error_range" | "status.language_icon"
                ) {
                    continue;
                }
                let color_target = match name.as_str() {
                    "zen.margin" => Some(&mut style.zen_margin),
                    "zen.ghost" => Some(&mut style.zen_ghost),
                    "which_key.background" => Some(&mut style.which_key.background),
                    _ => None,
                };
                if let Some(target) = color_target {
                    *target = value
                        .color_only(*target)
                        .with_context(|| format!("invalid UI colour {name:?}"))?;
                } else {
                    let role_background = if name.starts_with("which_key.") {
                        style.which_key.background
                    } else {
                        background
                    };
                    value
                        .apply(style.ui_style_mut(name)?, role_background)
                        .with_context(|| format!("invalid UI style {name:?}"))?;
                }
            }
            style.status_line.language_icon =
                TextStyle::new(style.status_line.metadata.content.fg, Color::Transparent);
            if let Some(value) = theme.ui.get("status.language_icon") {
                value
                    .apply(&mut style.status_line.language_icon, background)
                    .context("invalid UI style 'status.language_icon'")?;
            }
        }
        style.error_range.format.underline_color = Some(style.diagnostic_inline.error.fg);
        if let Some(value) = self
            .themes
            .get(name)
            .and_then(|theme| theme.ui.get("diagnostic.error_range"))
        {
            let background = style.theme.bg;
            value
                .apply(&mut style.error_range, background)
                .context("invalid UI style 'diagnostic.error_range'")?;
        }
        style.icons_enabled = self.icons_enabled;
        style.layout.color_column = Some(self.color_column);
        style.layout.line_numbers = self.line_numbers;
        if !self.text_formatting {
            style.disable_text_formatting();
        }
        self.apply_popup_sizes(&mut style);
        style.dim_amount = self.background_dimming;
        Ok(style)
    }

    fn apply_popup_sizes(&self, style: &mut UiStyle) {
        for (name, size) in &self.popups {
            style.set_popup_size(
                name,
                size.width_percent,
                size.height_percent,
                size.min_width,
                size.min_height,
                size.stacked_padding,
            );
        }
    }
}

fn default_config_path() -> PathBuf {
    if let Some(path) = env::var_os("REDOX_CONFIG") {
        return PathBuf::from(path);
    }
    crate::storage::config_root().join("config.toml")
}

fn validate_percent(value: Option<u16>, name: &str) -> anyhow::Result<()> {
    if value.is_some_and(|value| !(1..=100).contains(&value)) {
        bail!("{name} must be between 1 and 100");
    }
    Ok(())
}

fn apply_palette(style: &mut UiStyle, palette: &BTreeMap<String, String>) -> anyhow::Result<()> {
    for (name, value) in palette {
        let color =
            parse_color(value).with_context(|| format!("invalid palette colour {name:?}"))?;
        match name.as_str() {
            "background" | "bg" => style.theme.bg = color,
            "color_column" => style.theme.color_column = color,
            "scope" => style.theme.scope = color,
            "selection_bg" => style.theme.selection_bg = color,
            "selection_fg" => style.theme.selection_fg = color,
            "white" => style.theme.white = color,
            "black" => style.theme.black = color,
            "red" => style.theme.red = color,
            "green" => style.theme.green = color,
            "yellow" => style.theme.yellow = color,
            "blue" => style.theme.blue = color,
            "purple" => style.theme.purple = color,
            "orange" => style.theme.orange = color,
            "light_red" => style.theme.light_red = color,
            "light_green" => style.theme.light_green = color,
            "light_yellow" => style.theme.light_yellow = color,
            "light_blue" => style.theme.light_blue = color,
            "light_purple" => style.theme.light_purple = color,
            "light_orange" => style.theme.light_orange = color,
            "dark_gray" => style.theme.dark_gray = color,
            "mid_gray" => style.theme.mid_gray = color,
            "light_gray" => style.theme.light_gray = color,
            _ => bail!("unknown palette colour {name:?}"),
        }
    }
    Ok(())
}

impl StyleValue {
    fn apply(&self, target: &mut TextStyle, background: Color) -> anyhow::Result<()> {
        match self {
            Self::Foreground(value) => {
                target.fg = parse_color(value)?;
                if target.bg != Color::Transparent {
                    target.bg = background;
                }
            }
            Self::Properties(properties) => {
                if let Some(value) = &properties.fg {
                    target.fg = parse_color(value)?;
                }
                if let Some(value) = &properties.bg {
                    target.bg = parse_color(value)?;
                }
                macro_rules! apply {
                    ($($field:ident),+ $(,)?) => { $(
                        if let Some(value) = properties.$field { target.format.$field = value; }
                    )+ };
                }
                apply!(bold, italic, dim, reverse, strikethrough, underline);
                if let Some(value) = &properties.underline_color {
                    target.format.underline_color = Some(parse_color(value)?);
                }
            }
        }
        Ok(())
    }

    fn color_only(&self, current: Color) -> anyhow::Result<Color> {
        if let Self::Properties(properties) = self
            && (properties.bold.is_some()
                || properties.italic.is_some()
                || properties.dim.is_some()
                || properties.reverse.is_some()
                || properties.strikethrough.is_some()
                || properties.underline.is_some()
                || properties.underline_color.is_some())
        {
            bail!("this role is a colour only; text formatting is not supported");
        }
        let mut style = TextStyle::new(current, Color::Transparent);
        self.apply(&mut style, Color::Transparent)?;
        Ok(style.fg)
    }
}

fn parse_color(value: &str) -> anyhow::Result<Color> {
    if value.eq_ignore_ascii_case("transparent") {
        return Ok(Color::Transparent);
    }
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("colours must use #RRGGBB or 'transparent'");
    }
    Ok(Color::Rgb {
        r: u8::from_str_radix(&hex[0..2], 16)?,
        g: u8::from_str_radix(&hex[2..4], 16)?,
        b: u8::from_str_radix(&hex[4..6], 16)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_overrides_inherit_defaults_and_validate_properties() {
        let config: Config = toml::from_str(r##"
[themes.default.syntax]
comment = { italic = false }
keyword = { fg = "#123456", bold = true, italic = true, dim = true, reverse = true, strikethrough = true, underline = "curl", underline_color = "#abcdef" }
markdown_heading = { underline = "none" }
type_name = "#112233"
[themes.default.ui]
"about.title" = "#010203"
"status.bar" = { bg = "#445566" }
"status.dirty" = "#778899"
"finder.directory" = { bold = false, italic = true }
"finder.selected" = { fg = "#040506", bg = "#070809", bold = true }
"command_line.ghost" = { italic = true }
"which_key.background" = "#112233"
"which_key.key" = { italic = true }
"which_key.text" = "#445566"
"which_key.prefix" = { bg = "#778899", bold = false }
"diagnostic.error_range" = { underline_color = "transparent" }
"##).unwrap();
        config.validate().unwrap();
        let style = config.style().unwrap();
        let defaults = UiStyle::default();
        assert_eq!(
            style.syntax.comment.colors(),
            defaults.syntax.comment.colors()
        );
        assert!(!style.syntax.comment.format.italic);
        assert!(style.syntax.markdown_heading.format.bold);
        assert_eq!(
            style.syntax.markdown_heading.format.underline,
            Underline::None
        );
        assert_eq!(style.syntax.type_name.fg, Color::rgb(17, 34, 51));
        assert!(style.about.title.format.bold);
        assert_eq!(style.about.title.bg, style.theme.bg);
        assert_eq!(style.status_line.dirty.fg, Color::rgb(119, 136, 153));
        assert_eq!(style.status_line.dirty.bg, Color::Transparent);
        let format = style.syntax.keyword.format;
        assert!(
            format.bold && format.italic && format.dim && format.reverse && format.strikethrough
        );
        assert_eq!(format.underline, Underline::Curl);
        assert_eq!(format.underline_color, Some(Color::rgb(171, 205, 239)));
        assert!(!style.finder.directory.format.bold);
        assert!(style.finder.directory.format.italic);
        assert!(style.command_line.ghost.format.italic);
        assert_eq!(style.which_key.key.bg, Color::rgb(17, 34, 51));
        assert_eq!(style.which_key.text.bg, style.which_key.key.bg);
        assert_eq!(style.which_key.prefix.bg, Color::rgb(119, 136, 153));
        assert_eq!(minui::Style::from(style.error_range).underline_color, None);
        let dimmed = style.dimmed();
        assert_eq!(
            dimmed.command_line.ghost.format,
            style.command_line.ghost.format
        );
        assert_ne!(dimmed.command_line.ghost.fg, style.command_line.ghost.fg);
        assert_ne!(
            dimmed.syntax.keyword.format.underline_color,
            format.underline_color
        );

        for source in [
            "[themes.default.syntax]\nkeyword = { bold = 'yes' }",
            "[themes.default.syntax]\nkeyword = { underline = 'wavy' }",
            "[themes.default.syntax]\nkeyword = { italics = true }",
            "[themes.default.syntax]\nkeyword = { underline_color = 'red' }",
            "[themes.default.ui]\n'unknown.role' = { bold = true }",
            "[themes.default.ui]\n'zen.margin' = { italic = false }",
            "[animations]\nyank_highlight_ms = -1",
            "[animations]\nfocus_fade_ms = 1.5",
            "[animations]\ndashboard_logo_ms = 'fast'",
            "[animations]\nunknown_effect_ms = 100",
            "[animations]\nrain_fps = 61",
        ] {
            assert!(
                toml::from_str::<Config>(source)
                    .map_err(anyhow::Error::from)
                    .and_then(|config| config.validate())
                    .is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn example_and_reference_cover_roles_and_plain_mode_preserves_colours() {
        let example = include_str!("../../../config.example.toml");
        let reference = include_str!("../../../CONFIGURATION.md");
        let mut config: Config = toml::from_str(example).unwrap();
        config.validate().unwrap();
        let theme = &config.themes["vague"];
        let mut style = config.style_for_theme("vague").unwrap();
        for (name, _) in style.syntax_roles_mut() {
            assert!(
                theme.syntax.contains_key(name),
                "missing syntax example: {name}"
            );
            assert!(
                reference.contains(&format!("`{name}`")),
                "missing syntax reference: {name}"
            );
        }
        for (name, _) in style.ui_roles_mut() {
            assert!(theme.ui.contains_key(name), "missing UI example: {name}");
            assert!(
                reference.contains(&format!("`{name}`")),
                "missing UI reference: {name}"
            );
        }
        for name in ["default", "vague"] {
            config.text_formatting = true;
            let mut formatted = config.style_for_theme(name).unwrap();
            config.text_formatting = false;
            let mut plain = config.style_for_theme(name).unwrap();
            assert!(!plain.text_formatting);
            for ((_, before), (_, after)) in
                formatted.syntax_roles_mut().zip(plain.syntax_roles_mut())
            {
                assert_eq!(before.colors(), after.colors());
                assert_eq!(after.format, crate::ui::style::TextFormat::default());
            }
            for ((_, before), (_, after)) in formatted.ui_roles_mut().zip(plain.ui_roles_mut()) {
                assert_eq!(before.colors(), after.colors());
                assert_eq!(after.format, crate::ui::style::TextFormat::default());
            }
        }
    }

    #[test]
    fn omitted_configuration_preserves_current_defaults() {
        assert!(!Config::default().logging.enabled);
        assert_eq!(Config::default().logging.max_events, 5_000);
        let invalid: Config = toml::from_str("[logging]\nmax_events = 0").unwrap();
        assert!(invalid.validate().is_err());
        let config: Config = toml::from_str("").expect("empty configuration should parse");
        assert_eq!(config.scrolloff, DEFAULT_SCROLLOFF_ROWS);
        assert_eq!(config.color_column, 79);
        assert_eq!(config.line_numbers, LineNumbers::Relative);
        assert!(!config.icons_enabled);
        assert!(config.check_updates);
        assert!(config.mouse);
        assert!(!config.mouse_invert_vertical);
        assert!(!config.mouse_invert_horizontal);
        assert_eq!(config.mouse_scroll_step_vertical, 3);
        assert_eq!(config.mouse_scroll_step_horizontal, 3);
        let mouse_config: Config =
            toml::from_str("mouse_invert_vertical = true\nmouse_invert_horizontal = true").unwrap();
        assert!(mouse_config.mouse_invert_vertical);
        assert!(mouse_config.mouse_invert_horizontal);
        assert!(!toml::from_str::<Config>("mouse = false").unwrap().mouse);
        assert!(
            !toml::from_str::<Config>("check_updates = false")
                .unwrap()
                .check_updates
        );
        assert_eq!(config.leader(), ' ');
        assert!(config.which_key.enabled);
        assert_eq!(config.which_key.delay_ms, DEFAULT_WHICH_KEY_DELAY_MS);
        assert!(!config.zen.enabled);
        assert_eq!(config.zen.width_percent, 80);
        assert_eq!(config.zen.min_width, 80);
        let style = config.style().unwrap();
        assert_eq!(style.theme, UiStyle::default().theme);
        assert_eq!(style.which_key.edge, UiStyle::default().which_key.edge);
    }

    #[test]
    fn scrolloff_is_configurable() {
        let config: Config = toml::from_str("scrolloff = 2").expect("scrolloff should parse");
        assert_eq!(config.scrolloff, 2);
    }

    #[test]
    fn nerd_font_icons_are_opt_in() {
        let config: Config = toml::from_str("icons_enabled = true").expect("icons should parse");
        assert!(config.icons_enabled);
        assert!(config.style().unwrap().icons_enabled);
    }
}
