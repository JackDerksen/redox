use tree_sitter::Language;

use crate::ui::syntax::SyntaxLanguage;

use super::{default_refine_role, general_capture_mapping, LanguageConfig};

pub(super) const JSON_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Json,
    grammar: json_language,
    highlights_queries: &[tree_sitter_json::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["json"],
    scope_kinds: &[],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

fn json_language() -> Language {
    tree_sitter_json::LANGUAGE.into()
}
