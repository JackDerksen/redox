use tree_sitter::Language;

use crate::ui::style::SyntaxRole;
use crate::ui::syntax::{SyntaxCapture, SyntaxLanguage};

use super::{default_refine_role, general_capture_mapping, LanguageConfig};

pub(super) const LUA_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Lua,
    grammar: lua_language,
    highlights_queries: &[tree_sitter_lua::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["lua"],
    scope_kinds: &[
        "do_statement",
        "for_statement",
        "function_declaration",
        "function_definition",
        "if_statement",
        "repeat_statement",
        "table_constructor",
        "while_statement",
    ],
    capture_mapping: lua_capture_mapping,
    refine_role: default_refine_role,
};

fn lua_language() -> Language {
    tree_sitter_lua::LANGUAGE.into()
}

fn lua_capture_mapping(capture: &str) -> Option<SyntaxCapture> {
    let role = match capture {
        "boolean" => SyntaxRole::Boolean,
        "conditional" | "keyword.function" | "keyword.return" | "repeat" => SyntaxRole::Keyword,
        "field" => SyntaxRole::Property,
        "function.call" => SyntaxRole::Function,
        "label" => SyntaxRole::Property,
        "method" | "method.call" => SyntaxRole::FunctionMethod,
        "parameter" => SyntaxRole::VariableParameter,
        "preproc" => SyntaxRole::Attribute,
        _ => return general_capture_mapping(capture),
    };

    Some(SyntaxCapture {
        role,
        priority: 100,
    })
}
