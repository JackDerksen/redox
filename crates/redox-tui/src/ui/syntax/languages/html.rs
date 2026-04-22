use tree_sitter::Language;

use crate::ui::syntax::SyntaxLanguage;

use super::{LanguageConfig, default_refine_role, general_capture_mapping};

pub(super) const HTML_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Html,
    grammar: html_language,
    highlights_queries: &[tree_sitter_html::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["htm", "html"],
    scope_kinds: &["element", "script_element", "style_element"],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

fn html_language() -> Language {
    tree_sitter_html::LANGUAGE.into()
}
