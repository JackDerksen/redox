//! Tree-sitter-backed syntax highlighting for the editor viewport.

mod languages;

use std::path::Path;

use minui::{cell_width, ColorPair, TabPolicy, Window};
use redox_core::{Pos, TextBuffer};
use tree_sitter::{Node, Parser, Query, QueryCursor, Range, StreamingIterator, Tree};
use unicode_segmentation::UnicodeSegmentation;

use self::languages::{
    language_config_for, language_for_path as config_language_for_path, LanguageConfig,
};
use super::style::{SyntaxRole, UiStyle};
use crate::ui::helpers::apply_color_column;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxLanguage {
    C,
    Cpp,
    Css,
    Go,
    Html,
    JavaScript,
    Json,
    Lua,
    Markdown,
    Python,
    Rust,
    Toml,
    TypeScript,
    Tsx,
    Yaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSyntaxSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub role: SyntaxRole,
    priority: u16,
}

#[derive(Default)]
pub struct SyntaxHighlighter {
    cache: Option<HighlightCache>,
    cache_stale: bool,
    active_scope_cache: Option<ActiveScopeCache>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxScopePair {
    pub start: Pos,
    pub end: Pos,
}

#[derive(Debug, Clone, Copy)]
struct ActiveScopeCache {
    language: SyntaxLanguage,
    analysis_version: u64,
    cursor_char: usize,
    scope: Option<SyntaxScopePair>,
}

#[derive(Debug, Clone, Copy)]
pub struct VisibleLineSyntaxSpans<'a> {
    line_spans: &'a [Vec<LineSyntaxSpan>],
    first_line: usize,
    line_count: usize,
}

struct QuerySyntaxEngine {
    language: SyntaxLanguage,
    parser: Parser,
    query: Query,
    capture_roles: Vec<Option<SyntaxCapture>>,
    refine_role: fn(SyntaxRole, &str) -> SyntaxRole,
    inline: Option<InlineSyntaxEngine>,
}

struct InlineSyntaxEngine {
    parser: Parser,
    query: Query,
    capture_roles: Vec<Option<SyntaxCapture>>,
}

impl std::fmt::Debug for SyntaxHighlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntaxHighlighter")
            .field("cache", &self.cache)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct HighlightCache {
    language: SyntaxLanguage,
    line_spans: Vec<Vec<LineSyntaxSpan>>,
    tree: Tree,
}

#[derive(Debug, Clone, Copy)]
struct TokenSpan {
    start_byte: usize,
    end_byte: usize,
    role: SyntaxRole,
    priority: u16,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SyntaxCapture {
    pub(crate) role: SyntaxRole,
    pub(crate) priority: u16,
}

/// Language-agnostic syntax highlighting engine with Treesitter.
impl QuerySyntaxEngine {
    fn new(config: &'static LanguageConfig) -> Option<Self> {
        let mut parser = Parser::new();
        let language = (config.grammar)();
        if parser.set_language(&language).is_err() {
            return None;
        }
        let query_source = config.highlights_queries.join("\n");
        let query = Query::new(&language, &query_source).ok()?;
        let capture_roles = query
            .capture_names()
            .iter()
            .map(|capture| (config.capture_mapping)(capture))
            .collect();

        let inline = config.inline_grammar.and_then(|grammar| {
            let mut parser = Parser::new();
            let language = grammar();
            if parser.set_language(&language).is_err() {
                return None;
            }
            let query_source = config.inline_highlights_queries.join("\n");
            let query = Query::new(&language, &query_source).ok()?;
            let capture_roles = query
                .capture_names()
                .iter()
                .map(|capture| (config.capture_mapping)(capture))
                .collect();

            Some(InlineSyntaxEngine {
                parser,
                query,
                capture_roles,
            })
        });

        Some(Self {
            language: config.language,
            parser,
            query,
            capture_roles,
            refine_role: config.refine_role,
            inline,
        })
    }

    fn parse_line_spans(&mut self, source: &str) -> Option<(Vec<Vec<LineSyntaxSpan>>, Tree)> {
        let line_starts = compute_line_start_bytes(source);
        let mut line_spans: Vec<Vec<LineSyntaxSpan>> = vec![Vec::new(); line_starts.len().max(1)];

        let Some(tree) = self.parser.parse(source, None) else {
            return None;
        };

        let mut tokens = Vec::new();
        collect_query_tokens(
            &self.query,
            &self.capture_roles,
            self.refine_role,
            tree.root_node(),
            source,
            &mut tokens,
        );
        if let Some(inline) = &mut self.inline {
            inline.parse_tokens(source, tree.root_node(), self.refine_role, &mut tokens);
        }
        if self.language == SyntaxLanguage::Markdown {
            collect_markdown_highlight_tokens(source, &line_starts, &mut tokens);
        }

        for token in tokens {
            push_token_to_lines(token, &line_starts, source.len(), line_spans.as_mut_slice());
        }

        for spans in &mut line_spans {
            spans.sort_by_key(|span| (span.start_byte, span.end_byte, span.priority));
        }

        Some((line_spans, tree))
    }
}

impl InlineSyntaxEngine {
    fn parse_tokens(
        &mut self,
        source: &str,
        root: Node<'_>,
        refine_role: fn(SyntaxRole, &str) -> SyntaxRole,
        tokens: &mut Vec<TokenSpan>,
    ) {
        let mut ranges = Vec::new();
        collect_inline_ranges(root, &mut ranges);
        for range in ranges {
            if self.parser.set_included_ranges(&[range]).is_err() {
                continue;
            }
            let Some(tree) = self.parser.parse(source, None) else {
                continue;
            };
            collect_query_tokens(
                &self.query,
                &self.capture_roles,
                refine_role,
                tree.root_node(),
                source,
                tokens,
            );
        }
    }
}

fn collect_query_tokens(
    query: &Query,
    capture_roles: &[Option<SyntaxCapture>],
    refine_role: fn(SyntaxRole, &str) -> SyntaxRole,
    node: Node<'_>,
    source: &str,
    tokens: &mut Vec<TokenSpan>,
) {
    let mut query_cursor = QueryCursor::new();
    let mut captures = query_cursor.captures(query, node, source.as_bytes());
    while {
        captures.advance();
        captures.get().is_some()
    } {
        let (query_match, capture_index) = captures
            .get()
            .expect("query capture should exist after advance");
        let capture = query_match.captures[*capture_index];
        let Some(syntax_capture) = capture_roles.get(capture.index as usize).copied().flatten()
        else {
            continue;
        };

        let role = refine_role(syntax_capture.role, capture.node.kind());
        tokens.push(TokenSpan {
            start_byte: capture.node.start_byte(),
            end_byte: capture.node.end_byte(),
            role,
            priority: syntax_capture.priority,
        });
    }
}

fn collect_inline_ranges(node: Node<'_>, ranges: &mut Vec<Range>) {
    match node.kind() {
        "inline" | "pipe_table_cell" => {
            ranges.push(node.range());
        }
        _ => {
            for idx in 0..node.named_child_count() {
                if let Some(child) = node.named_child(idx as u32) {
                    collect_inline_ranges(child, ranges);
                }
            }
        }
    }
}

impl SyntaxHighlighter {
    pub(crate) fn compute_cache(
        buffer: &TextBuffer,
        language: SyntaxLanguage,
    ) -> Option<HighlightCache> {
        let config = language_config_for(language)?;
        let mut engine = QuerySyntaxEngine::new(config)?;
        let source = buffer.to_string();
        let (spans, tree) = engine.parse_line_spans(&source)?;
        Some(HighlightCache {
            language,
            line_spans: spans,
            tree,
        })
    }

    pub fn visible_line_spans_cached(
        &self,
        language: Option<SyntaxLanguage>,
        first_line: usize,
        line_count: usize,
    ) -> Option<VisibleLineSyntaxSpans<'_>> {
        let language = language?;
        let cache = self
            .cache
            .as_ref()
            .filter(|cache| cache.language == language)?;
        Some(VisibleLineSyntaxSpans {
            line_spans: &cache.line_spans,
            first_line,
            line_count,
        })
    }

    pub(crate) fn has_cache_for(&self, language: SyntaxLanguage) -> bool {
        !self.cache_stale
            && self
                .cache
                .as_ref()
                .is_some_and(|cache| cache.language == language)
    }

    pub(crate) fn mark_cache_stale(&mut self) {
        if self.cache.is_some() {
            self.cache_stale = true;
        }
    }

    #[cfg(test)]
    pub(crate) fn has_stale_cache_for(&self, language: SyntaxLanguage) -> bool {
        self.cache_stale
            && self
                .cache
                .as_ref()
                .is_some_and(|cache| cache.language == language)
    }

    #[cfg(test)]
    pub(crate) fn has_any_cache_for(&self, language: SyntaxLanguage) -> bool {
        self.cache
            .as_ref()
            .is_some_and(|cache| cache.language == language)
    }

    pub fn active_scope_pair_cached(
        &mut self,
        buffer: &TextBuffer,
        language: Option<SyntaxLanguage>,
        analysis_version: u64,
        cursor: Pos,
    ) -> Option<SyntaxScopePair> {
        let language = language?;
        let cursor_char = buffer.pos_to_char(cursor);
        if let Some(cached) = self.active_scope_cache
            && cached.language == language
            && cached.analysis_version == analysis_version
            && cached.cursor_char == cursor_char
        {
            return cached.scope;
        }

        if self.cache_stale {
            return None;
        }

        let config = language_config_for(language)?;
        let cache = self
            .cache
            .as_ref()
            .filter(|cache| cache.language == language)?;
        let cursor_byte = buffer.rope().char_to_byte(cursor_char);
        let root = cache.tree.root_node();
        let scope = root
            .named_descendant_for_byte_range(cursor_byte, cursor_byte)
            .or_else(|| {
                cursor_byte
                    .checked_sub(1)
                    .and_then(|byte| root.named_descendant_for_byte_range(byte, byte))
            })
            .and_then(|node| active_scope_pair_for_node(buffer, config, node));
        self.active_scope_cache = Some(ActiveScopeCache {
            language,
            analysis_version,
            cursor_char,
            scope,
        });
        scope
    }

    pub fn active_scope_pair_for_display_cached(
        &mut self,
        buffer: &TextBuffer,
        language: Option<SyntaxLanguage>,
        analysis_version: u64,
        cursor: Pos,
    ) -> Option<SyntaxScopePair> {
        let stale_scope = if self.cache_stale {
            language
                .and_then(|language| {
                    self.active_scope_cache
                        .filter(|cached| cached.language == language)
                })
                .and_then(|cached| cached.scope)
        } else {
            None
        };
        self.active_scope_pair_cached(buffer, language, analysis_version, cursor)
            .or(stale_scope)
    }

    pub(crate) fn replace_cache(&mut self, cache: Option<HighlightCache>) {
        self.cache = cache;
        self.cache_stale = false;
        self.active_scope_cache = None;
    }
}

impl<'a> VisibleLineSyntaxSpans<'a> {
    pub fn get(&self, row: usize) -> Option<&'a [LineSyntaxSpan]> {
        if row >= self.line_count {
            return None;
        }

        Some(
            self.line_spans
                .get(self.first_line.saturating_add(row))
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        )
    }
}

impl std::ops::Index<usize> for VisibleLineSyntaxSpans<'_> {
    type Output = [LineSyntaxSpan];

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("visible syntax row out of bounds")
    }
}

fn active_scope_pair_for_node(
    buffer: &TextBuffer,
    config: &LanguageConfig,
    node: Node<'_>,
) -> Option<SyntaxScopePair> {
    let mut current = Some(node);
    while let Some(candidate) = current {
        if let Some(pair) = structural_scope_pair_for_node(buffer, config, candidate) {
            return Some(pair);
        }
        current = candidate.parent();
    }
    None
}

fn structural_scope_pair_for_node(
    buffer: &TextBuffer,
    config: &LanguageConfig,
    node: Node<'_>,
) -> Option<SyntaxScopePair> {
    if node.start_position().row >= node.end_position().row {
        return None;
    }

    if let Some(pair) = delimiter_wrapped_scope_pair(buffer, node) {
        return Some(pair);
    }

    config
        .scope_kinds
        .contains(&node.kind())
        .then(|| node_scope_pair(buffer, node))
        .flatten()
}

fn delimiter_wrapped_scope_pair(buffer: &TextBuffer, node: Node<'_>) -> Option<SyntaxScopePair> {
    let start_pos = byte_to_pos(buffer, node.start_byte())?;
    let end_pos = byte_to_pos(buffer, node.end_byte().checked_sub(1)?)?;
    let start_char = buffer.char_at(start_pos)?;
    let end_char = buffer.char_at(end_pos)?;
    matches!((start_char, end_char), ('{', '}') | ('[', ']') | ('(', ')')).then_some(
        SyntaxScopePair {
            start: start_pos,
            end: end_pos,
        },
    )
}

fn node_scope_pair(buffer: &TextBuffer, node: Node<'_>) -> Option<SyntaxScopePair> {
    let last_pos = byte_to_pos(buffer, node.end_byte().checked_sub(1)?)?;
    Some(SyntaxScopePair {
        start: byte_to_pos(buffer, node.start_byte())?,
        end: Pos::new(last_pos.line.saturating_add(1), 0),
    })
}

fn byte_to_pos(buffer: &TextBuffer, byte_idx: usize) -> Option<Pos> {
    if byte_idx > buffer.rope().len_bytes() {
        return None;
    }
    let char_idx = buffer.rope().byte_to_char(byte_idx);
    Some(buffer.char_to_pos(char_idx))
}

pub fn language_for_path(path: Option<&Path>) -> Option<SyntaxLanguage> {
    config_language_for_path(path)
}

pub fn scope_guides_enabled(language: Option<SyntaxLanguage>) -> bool {
    !matches!(language, Some(SyntaxLanguage::Markdown))
}

pub(crate) fn smart_newline_insert(
    buffer: &TextBuffer,
    language: Option<SyntaxLanguage>,
    cursor: Pos,
) -> Option<(String, Pos)> {
    let language = smart_indent_language(language)?;
    let source = buffer.to_string();
    let cursor = buffer.clamp_pos(cursor);
    let line = buffer.clamp_line(cursor.line);
    let line_text = buffer.line_string(line);
    let left = line_text.chars().take(cursor.col).collect::<String>();
    let right = line_text.chars().skip(cursor.col).collect::<String>();
    let virtual_source = if left == line_text {
        source
    } else {
        let mut source = source;
        let line_start = source_line_start_byte(&source, line)?;
        let cursor_byte = line_start + left.len();
        let line_end = line_start + line_text.len();
        source.replace_range(cursor_byte..line_end, &left);
        source
    };
    let tree = parse_tree(&virtual_source, language)?;

    let base_indent = floored_indent(leading_indent(&line_text));
    let inner_indent = indent_after_line(&virtual_source, &tree, language, line)
        .unwrap_or_else(|| base_indent.clone());
    let right_trimmed = right.trim_start();
    let quote_split = quote_delimiter_split(&left, right_trimmed);
    if delimiter_split(&virtual_source, &tree, line, right_trimmed) || quote_split {
        let split_indent = if quote_split {
            let mut indent = base_indent.clone();
            indent.push('\t');
            indent
        } else {
            inner_indent
        };
        let insert = format!("\n{split_indent}\n{base_indent}");
        let cursor = Pos::new(line + 1, split_indent.chars().count());
        return Some((insert, cursor));
    }

    let indent = desired_indent_for_line_source(&virtual_source, language, line + 1)?;
    Some((
        format!("\n{indent}"),
        Pos::new(line + 1, indent.chars().count()),
    ))
}

pub(crate) fn smart_open_line_insert(
    buffer: &TextBuffer,
    language: Option<SyntaxLanguage>,
    line: usize,
    above: bool,
) -> Option<(String, Pos)> {
    let language = smart_indent_language(language)?;
    let source = buffer.to_string();
    let line = buffer.clamp_line(line);
    let insert_pos = if above {
        Pos::new(line, 0)
    } else {
        Pos::new(line, buffer.line_len_chars(line))
    };
    let insert_byte = source_line_start_byte(&source, insert_pos.line)?
        + buffer
            .line_string(insert_pos.line)
            .chars()
            .take(insert_pos.col)
            .map(char::len_utf8)
            .sum::<usize>();

    let mut virtual_source = source;
    virtual_source.insert(insert_byte, '\n');
    let new_line = if above { line } else { line + 1 };
    let indent = desired_indent_for_line_source(&virtual_source, language, new_line)?;
    let insert = if above {
        format!("{indent}\n")
    } else {
        format!("\n{indent}")
    };

    Some((insert, Pos::new(new_line, indent.chars().count())))
}

pub(crate) fn desired_indent_for_line(
    buffer: &TextBuffer,
    language: Option<SyntaxLanguage>,
    line: usize,
) -> Option<String> {
    let language = smart_indent_language(language)?;
    desired_indent_for_line_source(&buffer.to_string(), language, line)
}

fn desired_indent_for_line_source(
    source: &str,
    language: SyntaxLanguage,
    line: usize,
) -> Option<String> {
    let tree = parse_tree(source, language)?;
    let lines = source.lines().collect::<Vec<_>>();
    let line_text = lines.get(line).copied().unwrap_or("");
    let Some(prev_line) = line.checked_sub(1) else {
        return Some(String::new());
    };
    let prev_text = lines.get(prev_line).copied().unwrap_or("");
    if prev_text.trim().is_empty() {
        if line_text.trim().is_empty()
            && let Some(next_text) = lines.get(line + 1).copied()
            && (starts_with_closing_delimiter(next_text.trim_start())
                || starts_with_html_closing(next_text.trim_start()))
        {
            let mut indent = floored_indent(leading_indent(next_text));
            indent.push('\t');
            return Some(indent);
        }
        return Some(String::new());
    }

    if language == SyntaxLanguage::Markdown
        && let Some(indent) = markdown_indent_after_line(prev_text)
    {
        return Some(indent);
    }
    let mut indent = indent_after_line(source, &tree, language, prev_line)
        .unwrap_or_else(|| floored_indent(leading_indent(prev_text)));

    let trimmed = line_text.trim_start();
    if starts_with_closing_delimiter(trimmed) || starts_with_html_closing(trimmed) {
        remove_one_indent_level(&mut indent);
    }

    Some(indent)
}

fn smart_indent_language(language: Option<SyntaxLanguage>) -> Option<SyntaxLanguage> {
    language
}

fn parse_tree(source: &str, language: SyntaxLanguage) -> Option<Tree> {
    let config = language_config_for(language)?;
    let mut parser = Parser::new();
    parser.set_language(&(config.grammar)()).ok()?;
    parser.parse(source, None)
}

fn indent_after_line(
    source: &str,
    tree: &Tree,
    language: SyntaxLanguage,
    line: usize,
) -> Option<String> {
    let lines = source.lines().collect::<Vec<_>>();
    let text = lines.get(line).copied()?;
    let mut indent = floored_indent(leading_indent(text));
    if opens_line(source, tree, language, line) {
        indent.push('\t');
    }
    Some(indent)
}

fn opens_line(source: &str, tree: &Tree, language: SyntaxLanguage, line: usize) -> bool {
    let Some((ch, byte)) = trailing_significant_char(source, tree, line) else {
        return false;
    };
    if paired_closer_for(ch).is_some() {
        return true;
    }
    if language == SyntaxLanguage::Python && ch == ':' {
        return true;
    }
    if matches!(language, SyntaxLanguage::Html | SyntaxLanguage::Tsx) {
        let lines = source.lines().collect::<Vec<_>>();
        if let Some(text) = lines.get(line) {
            return opens_html_tag(text);
        }
    }
    ch == ':' && node_kind_at_byte(tree, byte).is_some_and(|kind| kind.contains("mapping"))
}

fn delimiter_split(source: &str, tree: &Tree, line: usize, right_trimmed: &str) -> bool {
    let Some((ch, _)) = trailing_significant_char(source, tree, line) else {
        return false;
    };
    paired_closer_for(ch).is_some_and(|closer| right_trimmed.starts_with(closer))
}

fn quote_delimiter_split(left: &str, right_trimmed: &str) -> bool {
    let Some((quote, byte_idx)) = trailing_non_whitespace_char(left) else {
        return false;
    };
    if !matches!(quote, '"' | '\'' | '`') || !right_trimmed.starts_with(quote) {
        return false;
    }
    quote == '`' || !is_escaped_at(left, byte_idx)
}

fn trailing_non_whitespace_char(text: &str) -> Option<(char, usize)> {
    text.char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, ch)| (ch, idx))
}

fn is_escaped_at(text: &str, byte_idx: usize) -> bool {
    let mut backslashes = 0usize;
    for ch in text[..byte_idx].chars().rev() {
        if ch == '\\' {
            backslashes += 1;
        } else {
            break;
        }
    }
    backslashes % 2 == 1
}

fn trailing_significant_char(source: &str, tree: &Tree, line: usize) -> Option<(char, usize)> {
    let line_start = source_line_start_byte(source, line)?;
    let line_text = source.lines().nth(line)?;
    for (byte_offset, ch) in line_text.char_indices().rev() {
        if ch.is_whitespace() {
            continue;
        }
        let byte = line_start + byte_offset;
        if is_string_or_comment_node(tree, byte) {
            continue;
        }
        return Some((ch, byte));
    }
    None
}

fn node_kind_at_byte(tree: &Tree, byte: usize) -> Option<&str> {
    tree.root_node()
        .named_descendant_for_byte_range(byte, byte)
        .map(|node| node.kind())
}

fn is_string_or_comment_node(tree: &Tree, byte: usize) -> bool {
    let mut node = tree.root_node().named_descendant_for_byte_range(byte, byte);
    while let Some(current) = node {
        let kind = current.kind();
        if kind.contains("string") || kind.contains("comment") {
            return true;
        }
        node = current.parent();
    }
    false
}

fn leading_indent(text: &str) -> &str {
    let end = text
        .char_indices()
        .find_map(|(idx, ch)| (!matches!(ch, ' ' | '\t')).then_some(idx))
        .unwrap_or(text.len());
    &text[..end]
}

fn floored_indent(indent: &str) -> String {
    "\t".repeat(indent_width(indent) / 4)
}

fn indent_width(indent: &str) -> usize {
    let mut col = 0usize;
    for ch in indent.chars() {
        match ch {
            '\t' => col += 4 - (col % 4),
            ' ' => col += 1,
            _ => break,
        }
    }
    col
}

fn remove_one_indent_level(indent: &mut String) {
    if indent.ends_with('\t') {
        indent.pop();
        return;
    }

    let remove = indent
        .chars()
        .rev()
        .take_while(|ch| *ch == ' ')
        .take(4)
        .count();
    for _ in 0..remove {
        indent.pop();
    }
}

fn paired_closer_for(ch: char) -> Option<char> {
    match ch {
        '{' => Some('}'),
        '[' => Some(']'),
        '(' => Some(')'),
        '<' => Some('>'),
        _ => None,
    }
}

fn starts_with_closing_delimiter(trimmed: &str) -> bool {
    trimmed
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '}' | ']' | ')' | '>'))
}

fn starts_with_html_closing(trimmed: &str) -> bool {
    trimmed.starts_with("</")
}

fn opens_html_tag(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with('<')
        && !trimmed.starts_with("</")
        && !trimmed.starts_with("<!")
        && !trimmed.ends_with("/>")
        && trimmed.contains('>')
}

fn markdown_indent_after_line(text: &str) -> Option<String> {
    let base_width = indent_width(leading_indent(text));
    let mut rest = &text[leading_indent(text).len()..];
    let mut continuation_width = base_width;
    let mut saw_block_quote = false;

    while let Some(after_marker) = rest.strip_prefix('>') {
        saw_block_quote = true;
        continuation_width += 1;
        rest = after_marker;
        if let Some(after_space) = rest.strip_prefix(' ') {
            continuation_width += 1;
            rest = after_space;
        }
        let nested = leading_indent(rest);
        continuation_width += indent_width(nested);
        rest = &rest[nested.len()..];
    }

    if let Some(marker_len) = markdown_list_marker_len(rest) {
        continuation_width += marker_len;
        return Some("\t".repeat(continuation_width / 4));
    }

    saw_block_quote.then(|| "\t".repeat(continuation_width / 4))
}

fn markdown_list_marker_len(text: &str) -> Option<usize> {
    if let Some(ch) = text.chars().next()
        && matches!(ch, '-' | '*' | '+')
        && text.chars().nth(1).is_some_and(char::is_whitespace)
    {
        return Some(2);
    }

    let mut digit_end = 0usize;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            digit_end = idx + ch.len_utf8();
            continue;
        }
        break;
    }
    if digit_end == 0 {
        return None;
    }

    let mut chars = text[digit_end..].chars();
    let delimiter = chars.next()?;
    let space = chars.next()?;
    matches!(delimiter, '.' | ')')
        .then_some(())
        .filter(|_| space.is_whitespace())?;
    Some(digit_end + delimiter.len_utf8() + space.len_utf8())
}

fn source_line_start_byte(source: &str, line: usize) -> Option<usize> {
    compute_line_start_bytes(source).get(line).copied()
}

pub fn draw_line_with_syntax(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    base_color: ColorPair,
    color_column: Option<(usize, minui::Color)>,
    style: UiStyle,
    spans: &[LineSyntaxSpan],
) -> minui::Result<()> {
    if width_cells == 0 {
        return Ok(());
    }

    let mut line_cells = 0usize;
    let mut byte_idx = 0usize;
    let mut syntax_idx = 0usize;
    let visible_end = scroll_x.saturating_add(width_cells);
    let mut pending_start: Option<usize> = None;
    let mut pending_end = 0usize;
    let mut pending_visible_x = 0usize;
    let mut pending_colors = base_color;

    for g in source_line.graphemes(true) {
        let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width);
        let start_byte = byte_idx;
        let end_byte = byte_idx.saturating_add(g.len());

        line_cells = end_cell;
        byte_idx = end_byte;

        if end_cell <= scroll_x {
            continue;
        }
        if start_cell >= visible_end {
            break;
        }

        let clipped_start = start_cell.max(scroll_x);
        let clipped_end = end_cell.min(visible_end);
        if clipped_start >= clipped_end {
            continue;
        }

        while syntax_idx < spans.len() && spans[syntax_idx].end_byte <= start_byte {
            syntax_idx += 1;
        }

        let colors = syntax_color_for_range(
            base_color,
            style,
            &spans[syntax_idx..],
            start_byte,
            end_byte,
        );
        let colors = apply_color_column(colors, color_column, start_cell, end_cell);
        let visible_x = clipped_start.saturating_sub(scroll_x);

        if g == "\t" {
            flush_pending_syntax_span(
                window,
                row,
                col,
                source_line,
                pending_start.take(),
                pending_end,
                pending_visible_x,
                pending_colors,
            )?;
            let spaces = " ".repeat(clipped_end.saturating_sub(clipped_start));
            window.write_str_colored(row, col.saturating_add(visible_x as u16), &spaces, colors)?;
            continue;
        }

        if clipped_start != start_cell || clipped_end != end_cell {
            continue;
        }

        if pending_start.is_some() && pending_colors == colors && pending_end == start_byte {
            pending_end = end_byte;
            continue;
        }

        flush_pending_syntax_span(
            window,
            row,
            col,
            source_line,
            pending_start.take(),
            pending_end,
            pending_visible_x,
            pending_colors,
        )?;
        pending_start = Some(start_byte);
        pending_end = end_byte;
        pending_visible_x = visible_x;
        pending_colors = colors;
    }

    flush_pending_syntax_span(
        window,
        row,
        col,
        source_line,
        pending_start,
        pending_end,
        pending_visible_x,
        pending_colors,
    )?;

    draw_color_column_gap(
        window,
        row,
        col,
        source_line,
        scroll_x,
        width_cells,
        color_column,
    )
}

fn flush_pending_syntax_span(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    source_line: &str,
    start: Option<usize>,
    end: usize,
    visible_x: usize,
    colors: ColorPair,
) -> minui::Result<()> {
    let Some(start) = start else {
        return Ok(());
    };
    if start >= end {
        return Ok(());
    }

    window.write_str_colored(
        row,
        col.saturating_add(visible_x as u16),
        &source_line[start..end],
        colors,
    )
}

pub fn syntax_color_for_range(
    base_color: ColorPair,
    style: UiStyle,
    spans: &[LineSyntaxSpan],
    start_byte: usize,
    end_byte: usize,
) -> ColorPair {
    if let Some(span) = best_span_for_range(spans, start_byte, end_byte) {
        style.syntax.color_for(span.role)
    } else {
        base_color
    }
}

fn draw_color_column_gap(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    color_column: Option<(usize, minui::Color)>,
) -> minui::Result<()> {
    let Some((visible_col, bg)) = color_column else {
        return Ok(());
    };
    if visible_col >= width_cells {
        return Ok(());
    }

    let line_width = cell_width(source_line, TabPolicy::Fixed(4)) as usize;
    if line_width > scroll_x.saturating_add(visible_col) {
        return Ok(());
    }

    window.write_str_colored(
        row,
        col.saturating_add(visible_col as u16),
        " ",
        ColorPair::new(minui::Color::Transparent, bg),
    )
}

fn compute_line_start_bytes(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (byte_idx, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(byte_idx + 1);
        }
    }
    starts
}

fn collect_markdown_highlight_tokens(
    source: &str,
    line_starts: &[usize],
    tokens: &mut Vec<TokenSpan>,
) {
    for (line_idx, line_start) in line_starts.iter().copied().enumerate() {
        let line_end = line_starts
            .get(line_idx + 1)
            .copied()
            .map(|next| next.saturating_sub(1))
            .unwrap_or(source.len());
        let line = &source[line_start..line_end];
        let bytes = line.as_bytes();
        let mut cursor = 0;

        while let Some(open_offset) = find_markdown_highlight_delimiter(bytes, cursor) {
            let content_start = open_offset + 2;
            let Some(close_offset) = find_markdown_highlight_delimiter(bytes, content_start) else {
                break;
            };

            if close_offset > content_start {
                let start = line_start + open_offset;
                let end = line_start + close_offset + 2;
                if !source[line_start + content_start..line_start + close_offset]
                    .trim()
                    .is_empty()
                    && !token_overlaps_role(tokens.as_slice(), start, end, SyntaxRole::MarkdownCode)
                {
                    tokens.push(TokenSpan {
                        start_byte: start,
                        end_byte: end,
                        role: SyntaxRole::MarkdownHighlight,
                        priority: 180,
                    });
                }
            }

            cursor = close_offset + 2;
        }
    }
}

fn find_markdown_highlight_delimiter(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor + 1 < bytes.len() {
        if bytes[cursor] == b'='
            && bytes[cursor + 1] == b'='
            && !is_escaped_ascii_byte(bytes, cursor)
        {
            return Some(cursor);
        }
        cursor += 1;
    }
    None
}

fn is_escaped_ascii_byte(bytes: &[u8], idx: usize) -> bool {
    let mut slash_count = 0;
    let mut cursor = idx;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        slash_count += 1;
        cursor -= 1;
    }
    slash_count % 2 == 1
}

fn token_overlaps_role(
    tokens: &[TokenSpan],
    start_byte: usize,
    end_byte: usize,
    role: SyntaxRole,
) -> bool {
    tokens.iter().any(|token| {
        token.role == role && token.start_byte < end_byte && token.end_byte > start_byte
    })
}

fn line_index_for_byte(line_starts: &[usize], byte: usize) -> usize {
    match line_starts.binary_search(&byte) {
        Ok(idx) => idx,
        Err(0) => 0,
        Err(idx) => idx - 1,
    }
}

fn push_token_to_lines(
    token: TokenSpan,
    line_starts: &[usize],
    source_len: usize,
    line_spans: &mut [Vec<LineSyntaxSpan>],
) {
    if token.start_byte >= token.end_byte || token.start_byte >= source_len {
        return;
    }

    let mut cursor = token.start_byte;
    let token_end = token.end_byte.min(source_len);

    while cursor < token_end {
        let line_idx = line_index_for_byte(line_starts, cursor);
        let line_start = line_starts[line_idx];
        let next_line_start = line_starts.get(line_idx + 1).copied().unwrap_or(source_len);
        let line_content_end = if line_idx + 1 < line_starts.len() {
            next_line_start.saturating_sub(1)
        } else {
            next_line_start
        };

        if cursor >= line_content_end {
            cursor = next_line_start;
            continue;
        }

        let seg_end = token_end.min(line_content_end);
        if seg_end > cursor {
            line_spans[line_idx].push(LineSyntaxSpan {
                start_byte: cursor.saturating_sub(line_start),
                end_byte: seg_end.saturating_sub(line_start),
                role: token.role,
                priority: token.priority,
            });
        }

        if seg_end >= token_end {
            break;
        }
        cursor = next_line_start;
    }
}

fn best_span_for_range(
    spans: &[LineSyntaxSpan],
    start_byte: usize,
    end_byte: usize,
) -> Option<&LineSyntaxSpan> {
    spans
        .iter()
        .take_while(|span| span.start_byte < end_byte)
        .filter(|span| span.end_byte > start_byte)
        .max_by_key(|span| (span.priority, span.end_byte.saturating_sub(span.start_byte)))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use redox_core::{Pos, TextBuffer};

    use super::{language_for_path, SyntaxHighlighter, SyntaxLanguage};
    use crate::ui::style::SyntaxRole;

    #[test]
    fn detects_rust_paths() {
        assert_eq!(
            language_for_path(Some(Path::new("/tmp/main.rs"))),
            Some(SyntaxLanguage::Rust)
        );
        assert_eq!(
            language_for_path(Some(Path::new("/tmp/MAIN.RS"))),
            Some(SyntaxLanguage::Rust)
        );
        assert_eq!(
            language_for_path(Some(Path::new("/tmp/readme.md"))),
            Some(SyntaxLanguage::Markdown)
        );
        assert_eq!(language_for_path(None), None);
    }

    #[test]
    fn detects_new_tree_sitter_language_paths() {
        let cases = [
            ("/tmp/main.c", SyntaxLanguage::C),
            ("/tmp/main.h", SyntaxLanguage::C),
            ("/tmp/main.cc", SyntaxLanguage::Cpp),
            ("/tmp/main.cpp", SyntaxLanguage::Cpp),
            ("/tmp/main.cxx", SyntaxLanguage::Cpp),
            ("/tmp/style.css", SyntaxLanguage::Css),
            ("/tmp/main.go", SyntaxLanguage::Go),
            ("/tmp/index.html", SyntaxLanguage::Html),
            ("/tmp/index.htm", SyntaxLanguage::Html),
            ("/tmp/app.js", SyntaxLanguage::JavaScript),
            ("/tmp/app.jsx", SyntaxLanguage::JavaScript),
            ("/tmp/app.mjs", SyntaxLanguage::JavaScript),
            ("/tmp/config.json", SyntaxLanguage::Json),
            ("/tmp/init.lua", SyntaxLanguage::Lua),
            ("/tmp/readme.markdown", SyntaxLanguage::Markdown),
            ("/tmp/main.py", SyntaxLanguage::Python),
            ("/tmp/main.pyi", SyntaxLanguage::Python),
            ("/tmp/Cargo.toml", SyntaxLanguage::Toml),
            ("/tmp/app.ts", SyntaxLanguage::TypeScript),
            ("/tmp/app.tsx", SyntaxLanguage::Tsx),
            ("/tmp/config.yaml", SyntaxLanguage::Yaml),
            ("/tmp/config.yml", SyntaxLanguage::Yaml),
        ];

        for (path, language) in cases {
            assert_eq!(language_for_path(Some(Path::new(path))), Some(language));
        }
    }

    #[test]
    fn scope_guides_are_disabled_for_markdown_only() {
        assert!(!super::scope_guides_enabled(Some(SyntaxLanguage::Markdown)));
        assert!(super::scope_guides_enabled(Some(SyntaxLanguage::Python)));
        assert!(super::scope_guides_enabled(Some(SyntaxLanguage::Rust)));
        assert!(super::scope_guides_enabled(None));
    }

    #[test]
    fn comments_are_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("// comment\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Rust,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Rust), 0, 1)
            .expect("rust spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::Comment && span.start_byte == 0 && span.end_byte >= 10
        }));
    }

    #[test]
    fn string_quotes_are_highlighted_as_string() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("let s = \"hello\";\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Rust,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Rust), 0, 1)
            .expect("rust spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::String && span.start_byte <= 8 && span.end_byte >= 15
        }));
    }

    #[test]
    fn go_comments_are_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("// comment\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Go,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Go), 0, 1)
            .expect("go spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::Comment && span.start_byte == 0 && span.end_byte >= 10
        }));
    }

    #[test]
    fn c_comments_are_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("// comment\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(&buffer, SyntaxLanguage::C));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::C), 0, 1)
            .expect("c spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::Comment && span.start_byte == 0 && span.end_byte >= 10
        }));
    }

    #[test]
    fn cpp_uses_c_and_cpp_highlight_queries() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("auto value = nullptr; // comment\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Cpp,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Cpp), 0, 1)
            .expect("cpp spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::Type && span.start_byte == 0 && span.end_byte == 4
        }));
        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::Comment && span.start_byte <= 22 && span.end_byte >= 32
        }));
    }

    #[test]
    fn new_tree_sitter_languages_highlight_basic_tokens() {
        let cases = [
            (SyntaxLanguage::Css, "body { color: red; }\n"),
            (SyntaxLanguage::Html, "<div class=\"note\">hi</div>\n"),
            (SyntaxLanguage::JavaScript, "const value = true;\n"),
            (SyntaxLanguage::Json, "{\"enabled\": true}\n"),
            (SyntaxLanguage::Lua, "local value = true\nprint(value)\n"),
            (SyntaxLanguage::Toml, "[package]\nname = \"redox\"\n"),
            (
                SyntaxLanguage::TypeScript,
                "type User = { name: string };\n",
            ),
            (
                SyntaxLanguage::Tsx,
                "const el = <Button label=\"Save\" />;\n",
            ),
            (SyntaxLanguage::Yaml, "name: redox\n"),
        ];

        for (language, source) in cases {
            let mut highlighter = SyntaxHighlighter::default();
            let buffer = TextBuffer::from_str(source);
            highlighter.replace_cache(SyntaxHighlighter::compute_cache(&buffer, language));
            let spans = highlighter
                .visible_line_spans_cached(Some(language), 0, 1)
                .expect("spans");

            assert!(
                !spans[0].is_empty(),
                "{language:?} should produce at least one highlight span"
            );
        }
    }

    #[test]
    fn typescript_combines_javascript_and_typescript_highlight_queries() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str(
            "export type User = { name: string };\nconst label = format(user.name, \"ok\");\n",
        );
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::TypeScript,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::TypeScript), 0, 2)
            .expect("typescript spans");

        assert!(spans[0].iter().any(|span| span.role == SyntaxRole::Keyword));
        assert!(spans[0]
            .iter()
            .any(|span| span.role == SyntaxRole::TypeBuiltin));
        assert!(spans[1]
            .iter()
            .any(|span| span.role == SyntaxRole::Function));
        assert!(spans[1].iter().any(|span| span.role == SyntaxRole::String));
    }

    #[test]
    fn markdown_headings_are_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("# Title\n\nBody\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Markdown,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Markdown), 0, 1)
            .expect("markdown spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownHeading && span.start_byte == 0 && span.end_byte >= 7
        }));
    }

    #[test]
    fn markdown_inline_emphasis_is_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("This is *italic*, **bold**, and `code`.\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Markdown,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Markdown), 0, 1)
            .expect("markdown spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownEmphasis && span.start_byte <= 8 && span.end_byte >= 16
        }));
        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownStrong && span.start_byte <= 18 && span.end_byte >= 26
        }));
        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownCode && span.start_byte <= 32 && span.end_byte >= 38
        }));
    }

    #[test]
    fn markdown_highlight_marks_equal_delimited_text() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("Keep ==this== bright, not `==this==`.\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Markdown,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Markdown), 0, 1)
            .expect("markdown spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownHighlight
                && span.start_byte == 5
                && span.end_byte == 13
        }));
        assert_eq!(
            spans[0]
                .iter()
                .filter(|span| span.role == SyntaxRole::MarkdownHighlight)
                .count(),
            1
        );
    }

    #[test]
    fn markdown_code_blocks_are_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("```rust\nlet answer = 42;\n```\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Markdown,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Markdown), 1, 1)
            .expect("markdown spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownCode && span.start_byte == 0 && span.end_byte >= 16
        }));
    }

    #[test]
    fn markdown_frontmatter_is_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("---\ntitle: Redox\n---\n\n# Title\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Markdown,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Markdown), 0, 3)
            .expect("markdown spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownFrontmatter
                && span.start_byte == 0
                && span.end_byte >= 3
        }));
        assert!(spans[1].iter().any(|span| {
            span.role == SyntaxRole::MarkdownFrontmatter
                && span.start_byte == 0
                && span.end_byte >= 12
        }));
    }

    #[test]
    fn markdown_links_are_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("[Redox](https://example.com)\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Markdown,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Markdown), 0, 1)
            .expect("markdown spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownLink && span.start_byte == 0 && span.end_byte >= 28
        }));
    }

    #[test]
    fn markdown_list_markers_are_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("* item\n- other\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Markdown,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Markdown), 0, 2)
            .expect("markdown spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::MarkdownListMarker
                && span.start_byte == 0
                && span.end_byte >= 1
        }));
        assert!(spans[1].iter().any(|span| {
            span.role == SyntaxRole::MarkdownListMarker
                && span.start_byte == 0
                && span.end_byte >= 1
        }));
    }

    #[test]
    fn python_comments_are_highlighted() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("# comment\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Python,
        ));
        let spans = highlighter
            .visible_line_spans_cached(Some(SyntaxLanguage::Python), 0, 1)
            .expect("python spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::Comment && span.start_byte == 0 && span.end_byte >= 9
        }));
    }

    #[test]
    fn active_scope_pair_uses_multiline_structural_node() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("fn main() {\n    println!(\"hi\");\n}\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Rust,
        ));
        let scope = highlighter
            .active_scope_pair_cached(&buffer, Some(SyntaxLanguage::Rust), 0, Pos::new(1, 15))
            .expect("scope");

        assert_eq!(scope.start, Pos::new(0, 10));
        assert_eq!(scope.end, Pos::new(2, 0));
    }

    #[test]
    fn new_brace_languages_use_tree_sitter_scope_nodes() {
        let cases = [
            (
                SyntaxLanguage::C,
                "int main() {\n    return 0;\n}\n",
                Pos::new(0, 11),
            ),
            (
                SyntaxLanguage::Cpp,
                "int main() {\n    return 0;\n}\n",
                Pos::new(0, 11),
            ),
            (
                SyntaxLanguage::Go,
                "func main() {\n    println(\"hi\")\n}\n",
                Pos::new(0, 12),
            ),
            (
                SyntaxLanguage::Css,
                "body {\n    color: red;\n}\n",
                Pos::new(0, 5),
            ),
            (
                SyntaxLanguage::JavaScript,
                "if (value) {\n    run();\n}\n",
                Pos::new(0, 11),
            ),
            (
                SyntaxLanguage::Json,
                "{\n  \"enabled\": true\n}\n",
                Pos::new(0, 0),
            ),
            (
                SyntaxLanguage::TypeScript,
                "if (value) {\n    run();\n}\n",
                Pos::new(0, 11),
            ),
            (
                SyntaxLanguage::Tsx,
                "if (value) {\n    run();\n}\n",
                Pos::new(0, 11),
            ),
            (
                SyntaxLanguage::Toml,
                "values = [\n  \"redox\"\n]\n",
                Pos::new(0, 9),
            ),
        ];

        for (language, source, expected_start) in cases {
            let mut highlighter = SyntaxHighlighter::default();
            let buffer = TextBuffer::from_str(source);
            highlighter.replace_cache(SyntaxHighlighter::compute_cache(&buffer, language));
            let scope = highlighter
                .active_scope_pair_cached(&buffer, Some(language), 0, Pos::new(1, 4))
                .expect("scope");

            assert_eq!(scope.start, expected_start);
            assert_eq!(scope.end, Pos::new(2, 0));
        }
    }

    #[test]
    fn lua_uses_tree_sitter_scope_nodes() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("function main()\n    print(\"hi\")\nend\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Lua,
        ));
        let scope = highlighter
            .active_scope_pair_cached(&buffer, Some(SyntaxLanguage::Lua), 0, Pos::new(1, 4))
            .expect("lua scope");

        assert_eq!(scope.start, Pos::new(0, 0));
        assert_eq!(scope.end, Pos::new(3, 0));
    }

    #[test]
    fn tag_and_whitespace_languages_use_tree_sitter_scope_nodes() {
        let cases = [
            (
                SyntaxLanguage::Html,
                "<main>\n  <p>Text</p>\n</main>\n",
                Pos::new(0, 0),
                3,
            ),
            (
                SyntaxLanguage::Yaml,
                "root:\n  child: value\n",
                Pos::new(1, 2),
                2,
            ),
        ];

        for (language, source, expected_start, expected_end_line) in cases {
            let mut highlighter = SyntaxHighlighter::default();
            let buffer = TextBuffer::from_str(source);
            highlighter.replace_cache(SyntaxHighlighter::compute_cache(&buffer, language));
            let scope = highlighter
                .active_scope_pair_cached(&buffer, Some(language), 0, Pos::new(1, 4))
                .expect("scope");

            assert_eq!(scope.start, expected_start);
            assert_eq!(scope.end.line, expected_end_line);
        }
    }

    #[test]
    fn python_active_scope_pair_uses_tree_sitter_node_range() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("def main():\n    print(\"hi\")\n    return 1\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Python,
        ));
        let scope = highlighter
            .active_scope_pair_cached(&buffer, Some(SyntaxLanguage::Python), 0, Pos::new(1, 8))
            .expect("scope");

        assert_eq!(scope.start, Pos::new(0, 0));
        assert_eq!(scope.end.line, 3);
    }

    #[test]
    fn markdown_active_scope_pair_uses_tree_sitter_section() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("# Title\n\nBody\n");
        highlighter.replace_cache(SyntaxHighlighter::compute_cache(
            &buffer,
            SyntaxLanguage::Markdown,
        ));
        let scope = highlighter
            .active_scope_pair_cached(&buffer, Some(SyntaxLanguage::Markdown), 0, Pos::new(2, 1))
            .expect("scope");

        assert_eq!(scope.start, Pos::new(0, 0));
        assert_eq!(scope.end.line, 3);
    }
}
