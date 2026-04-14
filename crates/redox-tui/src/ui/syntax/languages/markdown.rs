use tree_sitter::Language;

use crate::ui::style::SyntaxRole;
use crate::ui::syntax::SyntaxCapture;
use crate::ui::syntax::SyntaxLanguage;

use super::{LanguageConfig, default_refine_role, general_capture_mapping};

const MARKDOWN_THEME_QUERY: &str = r#"
[
  (atx_heading)
  (setext_heading)
] @markup.heading

[
  (fenced_code_block)
  (indented_code_block)
] @markup.code

[
  (minus_metadata)
  (plus_metadata)
] @markup.frontmatter

[
  (link_destination)
  (link_label)
  (link_title)
] @markup.link

[
  (list_marker_plus)
  (list_marker_minus)
  (list_marker_star)
  (list_marker_dot)
  (list_marker_parenthesis)
  (task_list_marker_checked)
  (task_list_marker_unchecked)
] @markup.list
"#;

const MARKDOWN_INLINE_THEME_QUERY: &str = r#"
[
  (emphasis)
  (strong_emphasis)
] @markup.emphasis

[
  (code_span)
  (code_span_delimiter)
] @markup.code

[
  (inline_link)
  (shortcut_link)
  (full_reference_link)
  (collapsed_reference_link)
  (uri_autolink)
  (email_autolink)
] @markup.link
"#;

pub(super) const MARKDOWN_LANGUAGE: LanguageConfig = LanguageConfig {
    language: SyntaxLanguage::Markdown,
    grammar: markdown_language,
    highlights_queries: &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK, MARKDOWN_THEME_QUERY],
    inline_grammar: Some(markdown_inline_language),
    inline_highlights_queries: &[
        tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
        MARKDOWN_INLINE_THEME_QUERY,
    ],
    extensions: &["md", "markdown"],
    scope_kinds: &[
        "block_quote",
        "document",
        "fenced_code_block",
        "list",
        "section",
    ],
    capture_mapping: markdown_capture_mapping,
    refine_role: default_refine_role,
};

fn markdown_language() -> Language {
    tree_sitter_md::LANGUAGE.into()
}

fn markdown_inline_language() -> Language {
    tree_sitter_md::INLINE_LANGUAGE.into()
}

fn markdown_capture_mapping(capture: &str) -> Option<SyntaxCapture> {
    let role = match capture {
        "markup.code" => SyntaxRole::MarkdownCode,
        "markup.emphasis" => SyntaxRole::MarkdownEmphasis,
        "markup.frontmatter" => SyntaxRole::MarkdownFrontmatter,
        "markup.heading" => SyntaxRole::MarkdownHeading,
        "markup.link" => SyntaxRole::MarkdownLink,
        "markup.list" => SyntaxRole::MarkdownListMarker,
        _ => return general_capture_mapping(capture),
    };

    Some(SyntaxCapture {
        role,
        priority: 150,
    })
}
