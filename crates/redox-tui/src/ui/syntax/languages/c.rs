use tree_sitter::Language;

use crate::ui::style::SyntaxRole;
use crate::ui::syntax::SyntaxLanguage;

use super::{default_refine_role, general_capture_mapping, LanguageConfig};

pub(super) const C_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::C,
    grammar: c_language,
    highlights_queries: &[tree_sitter_c::HIGHLIGHT_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["c", "h"],
    scope_kinds: &[],
    capture_mapping: general_capture_mapping,
    refine_role: refine_c_role,
};

fn c_language() -> Language {
    tree_sitter_c::LANGUAGE.into()
}

fn refine_c_role(role: SyntaxRole, node_kind: &str) -> SyntaxRole {
    match (role, node_kind) {
        (SyntaxRole::Number, "char_literal") => SyntaxRole::Character,
        _ => default_refine_role(role, node_kind),
    }
}
