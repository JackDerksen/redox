use tree_sitter::Language;

use crate::ui::style::SyntaxRole;
use crate::ui::syntax::SyntaxLanguage;

use super::{default_refine_role, general_capture_mapping, LanguageConfig};

pub(super) const PYTHON_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Python,
    grammar: python_language,
    highlights_queries: &[tree_sitter_python::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["py", "pyi", "pyw"],
    scope_kinds: &[
        "class_definition",
        "for_statement",
        "function_definition",
        "if_statement",
        "match_statement",
        "try_statement",
        "while_statement",
        "with_statement",
    ],
    capture_mapping: general_capture_mapping,
    refine_role: refine_python_role,
};

fn python_language() -> Language {
    tree_sitter_python::LANGUAGE.into()
}

fn refine_python_role(role: SyntaxRole, node_kind: &str) -> SyntaxRole {
    match (role, node_kind) {
        (SyntaxRole::ConstantBuiltin, "false" | "true") => SyntaxRole::Boolean,
        (SyntaxRole::ConstantBuiltin, "none") => SyntaxRole::ConstantBuiltin,
        (SyntaxRole::Number, "float") => SyntaxRole::Float,
        _ => default_refine_role(role, node_kind),
    }
}
