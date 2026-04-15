use tree_sitter::Language;

use crate::ui::style::SyntaxRole;
use crate::ui::syntax::SyntaxLanguage;

use super::{default_refine_role, general_capture_mapping, LanguageConfig};

pub(super) const GO_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Go,
    grammar: go_language,
    highlights_queries: &[tree_sitter_go::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["go"],
    scope_kinds: &[],
    capture_mapping: general_capture_mapping,
    refine_role: refine_go_role,
};

fn go_language() -> Language {
    tree_sitter_go::LANGUAGE.into()
}

fn refine_go_role(role: SyntaxRole, node_kind: &str) -> SyntaxRole {
    match (role, node_kind) {
        (SyntaxRole::String, "rune_literal") => SyntaxRole::Character,
        (SyntaxRole::Number, "float_literal") => SyntaxRole::Float,
        (SyntaxRole::ConstantBuiltin, "false" | "true") => SyntaxRole::Boolean,
        _ => default_refine_role(role, node_kind),
    }
}
