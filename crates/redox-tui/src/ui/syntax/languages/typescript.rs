use tree_sitter::Language;

use crate::ui::syntax::SyntaxLanguage;

use super::{default_refine_role, general_capture_mapping, LanguageConfig};

pub(super) const TYPESCRIPT_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::TypeScript,
    grammar: typescript_language,
    highlights_queries: &[
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
    ],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["cts", "mts", "ts"],
    scope_kinds: &[],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

pub(super) const TSX_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Tsx,
    grammar: tsx_language,
    highlights_queries: &[
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
    ],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["tsx"],
    scope_kinds: &[],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

fn typescript_language() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}

fn tsx_language() -> Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}
