use tree_sitter::Language;

use crate::ui::style::SyntaxRole;
use crate::ui::syntax::{SyntaxCapture, SyntaxLanguage};

use super::LanguageConfig;

pub(super) const RUST_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Rust,
    grammar: rust_language,
    highlights_queries: &[tree_sitter_rust::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["rs"],
    scope_kinds: &[],
    capture_mapping: rust_capture_mapping,
    refine_role: refine_rust_role,
};

fn rust_language() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}

fn rust_capture_mapping(capture: &str) -> Option<SyntaxCapture> {
    let (role, priority) = match capture {
        "attribute" => (SyntaxRole::Attribute, 80),
        "comment" | "comment.documentation" => (SyntaxRole::Comment, 70),
        "constant" => (SyntaxRole::Constant, 50),
        "constant.builtin" => (SyntaxRole::ConstantBuiltin, 90),
        "constructor" => (SyntaxRole::Constructor, 55),
        "escape" => (SyntaxRole::StringEscape, 110),
        "function" => (SyntaxRole::Function, 60),
        "function.macro" => (SyntaxRole::FunctionMacro, 95),
        "function.method" => (SyntaxRole::FunctionMethod, 95),
        "keyword" => (SyntaxRole::Keyword, 60),
        "operator" => (SyntaxRole::Operator, 50),
        "property" => (SyntaxRole::Property, 60),
        "punctuation.bracket" => (SyntaxRole::PunctuationBracket, 40),
        "punctuation.delimiter" => (SyntaxRole::PunctuationDelimiter, 40),
        "string" => (SyntaxRole::String, 65),
        "type" => (SyntaxRole::Type, 55),
        "type.builtin" => (SyntaxRole::TypeBuiltin, 90),
        "variable.builtin" => (SyntaxRole::VariableBuiltin, 85),
        "variable.parameter" => (SyntaxRole::VariableParameter, 85),
        _ => return None,
    };

    Some(SyntaxCapture { role, priority })
}

fn refine_rust_role(role: SyntaxRole, node_kind: &str) -> SyntaxRole {
    match (role, node_kind) {
        (SyntaxRole::String, "char_literal") => SyntaxRole::Character,
        (SyntaxRole::ConstantBuiltin, "boolean_literal") => SyntaxRole::Boolean,
        (SyntaxRole::ConstantBuiltin, "float_literal") => SyntaxRole::Float,
        (SyntaxRole::ConstantBuiltin, "integer_literal") => SyntaxRole::Number,
        _ => role,
    }
}
