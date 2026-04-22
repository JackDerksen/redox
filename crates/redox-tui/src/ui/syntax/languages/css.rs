use tree_sitter::Language;

use crate::ui::syntax::SyntaxLanguage;

use super::{LanguageConfig, default_refine_role, general_capture_mapping};

pub(super) const CSS_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Css,
    grammar: css_language,
    highlights_queries: &[tree_sitter_css::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["css"],
    scope_kinds: &["block", "rule_set"],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

fn css_language() -> Language {
    tree_sitter_css::LANGUAGE.into()
}
