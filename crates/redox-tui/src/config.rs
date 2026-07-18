//! User configuration loading and validation.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use minui::{Color, ColorPair};
use serde::Deserialize;

use crate::input::cursor::DEFAULT_SCROLLOFF_ROWS;
use crate::ui::UiStyle;

pub const DEFAULT_DIM_AMOUNT: f32 = 0.301;
pub const DEFAULT_UNDO_HISTORY_SIZE: usize = usize::MAX;
pub const DEFAULT_WHICH_KEY_DELAY_MS: u64 = 3_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: String,
    pub icons_enabled: bool,
    pub background_dimming: f32,
    pub undo_tree_history_size: usize,
    pub scrolloff: usize,
    pub color_column: usize,
    pub leader: String,
    pub which_key: WhichKeyConfig,
    pub popups: BTreeMap<String, PopupSize>,
    pub keybindings: BTreeMap<String, BTreeMap<String, String>>,
    pub bind: Vec<BindConfig>,
    pub themes: BTreeMap<String, ThemeConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            icons_enabled: false,
            background_dimming: DEFAULT_DIM_AMOUNT,
            undo_tree_history_size: DEFAULT_UNDO_HISTORY_SIZE,
            scrolloff: DEFAULT_SCROLLOFF_ROWS,
            color_column: 79,
            leader: " ".to_string(),
            which_key: WhichKeyConfig::default(),
            popups: BTreeMap::new(),
            keybindings: BTreeMap::new(),
            bind: Vec::new(),
            themes: BTreeMap::new(),
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
pub struct ThemeConfig {
    pub palette: BTreeMap<String, String>,
    pub syntax: BTreeMap<String, ColourValue>,
    pub ui: BTreeMap<String, ColourValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ColourValue {
    Foreground(String),
    Pair(ColourPairConfig),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColourPairConfig {
    fg: String,
    bg: String,
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

    pub fn style(&self) -> anyhow::Result<UiStyle> {
        self.style_for_theme(&self.theme)
    }

    pub fn style_for_theme(&self, name: &str) -> anyhow::Result<UiStyle> {
        if !self.has_theme(name) {
            bail!("unknown colorscheme {name:?}");
        }
        let mut style = UiStyle::default();
        style.icons_enabled = self.icons_enabled;
        style.layout.color_column = self.color_column;
        let Some(theme) = self.themes.get(name) else {
            self.apply_popup_sizes(&mut style);
            style.dim_amount = self.background_dimming;
            return Ok(style);
        };

        apply_palette(&mut style, &theme.palette)?;
        // Re-derive every default role after changing the base palette.
        style = UiStyle::from_theme(style.theme);
        style.icons_enabled = self.icons_enabled;
        style.layout.color_column = self.color_column;
        for (name, value) in &theme.syntax {
            let pair = colour_pair(value, style.theme.bg)
                .with_context(|| format!("invalid syntax colour {name:?}"))?;
            style.set_syntax_colour(name, pair)?;
        }
        for (name, value) in &theme.ui {
            let pair = colour_pair(value, style.theme.bg)
                .with_context(|| format!("invalid UI colour {name:?}"))?;
            style.set_ui_colour(name, pair)?;
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
        let colour =
            parse_colour(value).with_context(|| format!("invalid palette colour {name:?}"))?;
        match name.as_str() {
            "background" | "bg" => style.theme.bg = colour,
            "color_column" => style.theme.color_column = colour,
            "scope" => style.theme.scope = colour,
            "selection_bg" => style.theme.selection_bg = colour,
            "selection_fg" => style.theme.selection_fg = colour,
            "white" => style.theme.white = colour,
            "black" => style.theme.black = colour,
            "red" => style.theme.red = colour,
            "green" => style.theme.green = colour,
            "yellow" => style.theme.yellow = colour,
            "blue" => style.theme.blue = colour,
            "purple" => style.theme.purple = colour,
            "orange" => style.theme.orange = colour,
            "light_red" => style.theme.light_red = colour,
            "light_green" => style.theme.light_green = colour,
            "light_yellow" => style.theme.light_yellow = colour,
            "light_blue" => style.theme.light_blue = colour,
            "light_purple" => style.theme.light_purple = colour,
            "light_orange" => style.theme.light_orange = colour,
            "dark_gray" => style.theme.dark_gray = colour,
            "mid_gray" => style.theme.mid_gray = colour,
            "light_gray" => style.theme.light_gray = colour,
            _ => bail!("unknown palette colour {name:?}"),
        }
    }
    Ok(())
}

fn colour_pair(value: &ColourValue, default_bg: Color) -> anyhow::Result<ColorPair> {
    match value {
        ColourValue::Foreground(fg) => Ok(ColorPair::new(parse_colour(fg)?, default_bg)),
        ColourValue::Pair(pair) => Ok(ColorPair::new(
            parse_colour(&pair.fg)?,
            parse_colour(&pair.bg)?,
        )),
    }
}

fn parse_colour(value: &str) -> anyhow::Result<Color> {
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
    fn omitted_configuration_preserves_current_defaults() {
        let config: Config = toml::from_str("").expect("empty configuration should parse");
        assert_eq!(config.scrolloff, DEFAULT_SCROLLOFF_ROWS);
        assert_eq!(config.color_column, 79);
        assert!(!config.icons_enabled);
        assert_eq!(config.leader(), ' ');
        assert!(config.which_key.enabled);
        assert_eq!(config.which_key.delay_ms, DEFAULT_WHICH_KEY_DELAY_MS);
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
