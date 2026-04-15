use tree_sitter::Language;

use crate::ui::syntax::SyntaxLanguage;

use super::{default_refine_role, general_capture_mapping, LanguageConfig};

pub(super) const CPP_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Cpp,
    grammar: cpp_language,
    highlights_queries: &[
        tree_sitter_c::HIGHLIGHT_QUERY,
        tree_sitter_cpp::HIGHLIGHT_QUERY,
    ],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &[
        "cc", "cpp", "cxx", "c++", "hh", "hpp", "hxx", "h++", "ipp", "tpp",
    ],
    scope_kinds: &[],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

fn cpp_language() -> Language {
    tree_sitter_cpp::LANGUAGE.into()
}
