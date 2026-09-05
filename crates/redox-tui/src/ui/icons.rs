//! Built-in Nerd Font icon catalogue.
//!
//! Icons are deliberately not configurable. Callers must gate them on
//! `UiStyle::icons_enabled` so the default UI remains safe for ordinary fonts.

use std::path::Path;

use super::syntax::{SyntaxLanguage, language_for_path};

pub const PREFIX_WIDTH: u16 = 2;
pub const GIT_BRANCH: &str = "";
pub const UNDO_TREE: &str = "";
pub const DIAGNOSTIC_ERROR: &str = "";
pub const DIAGNOSTIC_WARNING: &str = "";
pub const DIAGNOSTIC_INFORMATION: &str = "";
pub const DIAGNOSTIC_HINT: &str = "";
pub const DIAGNOSTIC_ICONS: [&str; 4] = [
    DIAGNOSTIC_ERROR,
    DIAGNOSTIC_WARNING,
    DIAGNOSTIC_INFORMATION,
    DIAGNOSTIC_HINT,
];
pub const DIAGNOSTIC_FALLBACKS: [&str; 4] = ["×", "△", "•", "⚬"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    About,
    CodeActions,
    Command,
    Diagnostics,
    Explorer,
    FilePreview,
    Finder,
    Information,
    LanguageTools,
    Performance,
    Pinboard,
    Search,
}

pub fn popup_title(kind: PopupKind, title: &str, enabled: bool) -> String {
    if !enabled {
        return title.to_owned();
    }
    format!("{} {title}", popup_icon(kind))
}

pub fn file_icon(path: &Path) -> &'static str {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    match file_name.to_ascii_lowercase().as_str() {
        "cargo.toml" | "cargo.lock" => "",
        "dockerfile" | "compose.yaml" | "compose.yml" => "",
        ".gitignore" | ".gitattributes" | ".gitmodules" => "",
        "package.json" | "package-lock.json" => "",
        "makefile" | "gnumakefile" => "",
        _ => language_for_path(Some(path))
            .map(language_icon)
            .unwrap_or(""),
    }
}

pub fn filetype_icon(path: &Path) -> Option<&'static str> {
    language_for_path(Some(path)).map(language_icon)
}

pub fn folder_icon(is_open: bool) -> &'static str {
    if is_open { "" } else { "" }
}

pub fn completion_kind_icon(kind: &str) -> Option<&'static str> {
    match kind {
        "text" => Some("󰉿"),
        "method" => Some("󰆧"),
        "function" => Some("󰊕"),
        "constructor" => Some(""),
        "field" | "property" => Some("󰜢"),
        "variable" => Some("󰀫"),
        "class" => Some(""),
        "interface" => Some(""),
        "module" => Some("󰅩"),
        "keyword" => Some("󰌋"),
        "snippet" => Some(""),
        "constant" => Some("󰏿"),
        "struct" => Some("󰙅"),
        "event" => Some(""),
        "operator" => Some("󰆕"),
        "type" => Some(""),
        "item" => Some("•"),
        _ => None,
    }
}

fn language_icon(language: SyntaxLanguage) -> &'static str {
    match language {
        SyntaxLanguage::C => "",
        SyntaxLanguage::Cpp => "",
        SyntaxLanguage::Css => "",
        SyntaxLanguage::Go => "󰟓",
        SyntaxLanguage::Html => "",
        SyntaxLanguage::JavaScript => "",
        SyntaxLanguage::Json => "",
        SyntaxLanguage::Lua => "",
        SyntaxLanguage::Markdown => "",
        SyntaxLanguage::Python => "",
        SyntaxLanguage::Rust => "",
        SyntaxLanguage::Toml => "",
        SyntaxLanguage::TypeScript => "",
        SyntaxLanguage::Tsx => "",
        SyntaxLanguage::Yaml => "",
    }
}

fn popup_icon(kind: PopupKind) -> &'static str {
    match kind {
        PopupKind::About => "",
        PopupKind::CodeActions => "",
        PopupKind::Command => "",
        PopupKind::Diagnostics => "",
        PopupKind::Explorer => "",
        PopupKind::FilePreview => "",
        PopupKind::Finder => "",
        PopupKind::Information => "",
        PopupKind::LanguageTools => "",
        PopupKind::Performance => "",
        PopupKind::Pinboard => "",
        PopupKind::Search => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_titles_are_unchanged() {
        assert_eq!(popup_title(PopupKind::Finder, "Finder", false), "Finder");
    }

    #[test]
    fn file_icons_use_language_and_special_file_mappings() {
        assert_eq!(file_icon(Path::new("src/main.rs")), "");
        assert_eq!(file_icon(Path::new("Dockerfile")), "");
        assert_eq!(file_icon(Path::new("notes.unknown")), "");
    }

    #[test]
    fn completion_icons_cover_common_symbol_kinds() {
        assert_eq!(completion_kind_icon("function"), Some("󰊕"));
        assert_eq!(completion_kind_icon("variable"), Some("󰀫"));
        assert_eq!(completion_kind_icon("unknown"), None);
    }
}
