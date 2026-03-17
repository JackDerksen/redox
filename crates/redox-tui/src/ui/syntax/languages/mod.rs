use std::path::Path;

use tree_sitter::Language;

use crate::ui::style::SyntaxRole;
use crate::ui::syntax::{SyntaxCapture, SyntaxLanguage};

mod rust;

pub(crate) struct LanguageConfig {
    pub(crate) language: SyntaxLanguage,
    pub(crate) grammar: fn() -> Language,
    pub(crate) highlights_query: &'static str,
    pub(crate) extensions: &'static [&'static str],
    pub(crate) capture_mapping: fn(&str) -> Option<SyntaxCapture>,
    pub(crate) refine_role: fn(SyntaxRole, &str) -> SyntaxRole,
}

const LANGUAGES: &[LanguageConfig] = &[rust::RUST_LANGUAGE];

pub(crate) fn language_config_for(language: SyntaxLanguage) -> Option<&'static LanguageConfig> {
    LANGUAGES.iter().find(|config| config.language == language)
}

pub(crate) fn language_for_path(path: Option<&Path>) -> Option<SyntaxLanguage> {
    let ext = path
        .and_then(Path::extension)
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())?;

    LANGUAGES.iter().find_map(|config| {
        config
            .extensions
            .iter()
            .any(|candidate| *candidate == ext)
            .then_some(config.language)
    })
}
