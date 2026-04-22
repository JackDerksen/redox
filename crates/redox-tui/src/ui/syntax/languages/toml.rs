use tree_sitter::Language;

use crate::ui::syntax::SyntaxLanguage;

use super::{LanguageConfig, default_refine_role, general_capture_mapping};

pub(super) const TOML_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Toml,
    grammar: toml_language,
    highlights_queries: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["toml"],
    scope_kinds: &["array", "inline_table", "table", "table_array_element"],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

fn toml_language() -> Language {
    tree_sitter_toml_ng::LANGUAGE.into()
}
