use std::path::Path;

use tree_sitter::Language;

use crate::ui::style::SyntaxRole;
use crate::ui::syntax::{SyntaxCapture, SyntaxLanguage};

mod c;
mod cpp;
mod css;
mod go;
mod html;
mod javascript;
mod json;
mod markdown;
mod python;
mod rust;
mod toml;
mod typescript;
mod yaml;

pub(crate) struct LanguageConfig {
    pub(crate) language: SyntaxLanguage,
    pub(crate) grammar: fn() -> Language,
    pub(crate) highlights_queries: &'static [&'static str],
    pub(crate) inline_grammar: Option<fn() -> Language>,
    pub(crate) inline_highlights_queries: &'static [&'static str],
    pub(crate) extensions: &'static [&'static str],
    pub(crate) scope_kinds: &'static [&'static str],
    pub(crate) capture_mapping: fn(&str) -> Option<SyntaxCapture>,
    pub(crate) refine_role: fn(SyntaxRole, &str) -> SyntaxRole,
}

const LANGUAGES: &[LanguageConfig] = &[
    c::C_LANGUAGE,
    cpp::CPP_LANGUAGE,
    css::CSS_LANGUAGE,
    go::GO_LANGUAGE,
    html::HTML_LANGUAGE,
    javascript::JAVASCRIPT_LANGUAGE,
    json::JSON_LANGUAGE,
    markdown::MARKDOWN_LANGUAGE,
    python::PYTHON_LANGUAGE,
    rust::RUST_LANGUAGE,
    toml::TOML_LANGUAGE,
    typescript::TYPESCRIPT_LANGUAGE,
    typescript::TSX_LANGUAGE,
    yaml::YAML_LANGUAGE,
];

pub(crate) fn language_config_for(language: SyntaxLanguage) -> Option<&'static LanguageConfig> {
    LANGUAGES.iter().find(|config| config.language == language)
}

pub(crate) fn language_for_path(path: Option<&Path>) -> Option<SyntaxLanguage> {
    let ext = path
        .and_then(Path::extension)
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())?;

    LANGUAGES.iter().find_map(|config| {
        config
            .extensions
            .iter()
            .any(|candidate| *candidate == ext)
            .then_some(config.language)
    })
}

pub(super) fn general_capture_mapping(capture: &str) -> Option<SyntaxCapture> {
    let (role, priority) = match capture {
        "attribute" => (SyntaxRole::Attribute, 80),
        "comment" | "comment.documentation" => (SyntaxRole::Comment, 70),
        "constant" => (SyntaxRole::Constant, 50),
        "constant.builtin" => (SyntaxRole::ConstantBuiltin, 90),
        "constant.macro" => (SyntaxRole::ConstantMacro, 90),
        "constructor" => (SyntaxRole::Constructor, 55),
        "delimiter" | "punctuation.delimiter" => (SyntaxRole::PunctuationDelimiter, 40),
        "embedded" => (SyntaxRole::String, 45),
        "escape" | "string.escape" => (SyntaxRole::StringEscape, 110),
        "function" => (SyntaxRole::Function, 60),
        "function.builtin" => (SyntaxRole::FunctionMacro, 90),
        "function.macro" | "function.special" => (SyntaxRole::FunctionMacro, 95),
        "function.method" => (SyntaxRole::FunctionMethod, 95),
        "keyword" => (SyntaxRole::Keyword, 60),
        "keyword.import" => (SyntaxRole::KeywordImport, 80),
        "keyword.operator" => (SyntaxRole::KeywordOperator, 80),
        "label" => (SyntaxRole::Property, 55),
        "number" => (SyntaxRole::Number, 70),
        "operator" => (SyntaxRole::Operator, 50),
        "property" | "property.builtin" => (SyntaxRole::Property, 60),
        "punctuation.bracket" => (SyntaxRole::PunctuationBracket, 40),
        "punctuation.special" => (SyntaxRole::PunctuationSpecial, 40),
        "string" => (SyntaxRole::String, 65),
        "string.special" => (SyntaxRole::StringEscape, 80),
        "tag" => (SyntaxRole::Type, 55),
        "tag.attribute" => (SyntaxRole::Attribute, 80),
        "tag.delimiter" => (SyntaxRole::PunctuationDelimiter, 40),
        "text.emphasis" => (SyntaxRole::Attribute, 55),
        "text.literal" => (SyntaxRole::String, 65),
        "text.reference" => (SyntaxRole::Property, 60),
        "text.strong" => (SyntaxRole::Keyword, 60),
        "text.title" => (SyntaxRole::TypeDefinition, 70),
        "text.uri" => (SyntaxRole::StringEscape, 65),
        "type" => (SyntaxRole::Type, 55),
        "type.builtin" => (SyntaxRole::TypeBuiltin, 90),
        "variable" | "variable.member" => (SyntaxRole::Property, 55),
        "variable.builtin" => (SyntaxRole::VariableBuiltin, 85),
        "variable.parameter" => (SyntaxRole::VariableParameter, 85),
        _ => return None,
    };

    Some(SyntaxCapture { role, priority })
}

pub(super) fn default_refine_role(role: SyntaxRole, _node_kind: &str) -> SyntaxRole {
    role
}
