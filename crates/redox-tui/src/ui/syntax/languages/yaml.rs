use tree_sitter::Language;

use crate::ui::syntax::SyntaxLanguage;

use super::{default_refine_role, general_capture_mapping, LanguageConfig};

pub(super) const YAML_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Yaml,
    grammar: yaml_language,
    highlights_queries: &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
    inline_grammar: None,
    inline_highlights_queries: &[],
    extensions: &["yaml", "yml"],
    scope_kinds: &["block_mapping", "block_sequence"],
    capture_mapping: general_capture_mapping,
    refine_role: default_refine_role,
};

fn yaml_language() -> Language {
    tree_sitter_yaml::LANGUAGE.into()
}
