use tree_sitter::Language;

use crate::ui::syntax::SyntaxLanguage;

use super::{LanguageConfig, default_refine_role, general_capture_mapping};

pub(super) const JAVASCRIPT_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::JavaScript,
    grammar: javascript_language,
    highlights_queries: &[
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
    ],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["cjs", "js", "jsx", "mjs"],
    scope_kinds: &[],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

fn javascript_language() -> Language {
    tree_sitter_javascript::LANGUAGE.into()
}
