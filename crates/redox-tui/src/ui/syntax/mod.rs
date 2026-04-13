//! Tree-sitter-backed syntax highlighting for the editor viewport.

mod languages;

use std::path::Path;

use minui::{ColorPair, TabPolicy, Window, cell_width};
use redox_core::{Pos, TextBuffer};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};
use unicode_segmentation::UnicodeSegmentation;

use self::languages::{
    LanguageConfig, language_config_for, language_for_path as config_language_for_path,
};
use super::style::{SyntaxRole, UiStyle};
use crate::ui::helpers::apply_color_column;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxLanguage {
    Rust,
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
    parser: Parser,
    query: Query,
    capture_roles: Vec<Option<SyntaxCapture>>,
    refine_role: fn(SyntaxRole, &str) -> SyntaxRole,
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
        let query = Query::new(&language, config.highlights_query).ok()?;
        let capture_roles = query
            .capture_names()
            .iter()
            .map(|capture| (config.capture_mapping)(capture))
            .collect();

        Some(Self {
            parser,
            query,
            capture_roles,
            refine_role: config.refine_role,
        })
    }

    fn parse_line_spans(&mut self, source: &str) -> Option<(Vec<Vec<LineSyntaxSpan>>, Tree)> {
        let line_starts = compute_line_start_bytes(source);
        let mut line_spans: Vec<Vec<LineSyntaxSpan>> = vec![Vec::new(); line_starts.len().max(1)];

        let Some(tree) = self.parser.parse(source, None) else {
            return None;
        };

        let mut query_cursor = QueryCursor::new();
        let mut tokens = Vec::new();
        let mut captures = query_cursor.captures(&self.query, tree.root_node(), source.as_bytes());
        while {
            captures.advance();
            captures.get().is_some()
        } {
            let (query_match, capture_index) = captures
                .get()
                .expect("query capture should exist after advance");
            let capture = query_match.captures[*capture_index];
            let Some(syntax_capture) = self
                .capture_roles
                .get(capture.index as usize)
                .copied()
                .flatten()
            else {
                continue;
            };

            let role = (self.refine_role)(syntax_capture.role, capture.node.kind());
            tokens.push(TokenSpan {
                start_byte: capture.node.start_byte(),
                end_byte: capture.node.end_byte(),
                role,
                priority: syntax_capture.priority,
            });
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
            .and_then(|node| active_scope_pair_for_node(buffer, node));
        self.active_scope_cache = Some(ActiveScopeCache {
            language,
            analysis_version,
            cursor_char,
            scope,
        });
        scope
    }

    pub(crate) fn replace_cache(&mut self, cache: Option<HighlightCache>) {
        self.cache = cache;
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

fn active_scope_pair_for_node(buffer: &TextBuffer, node: Node<'_>) -> Option<SyntaxScopePair> {
    let mut current = Some(node);
    while let Some(candidate) = current {
        if let Some(pair) = structural_scope_pair_for_node(buffer, candidate) {
            return Some(pair);
        }
        current = candidate.parent();
    }
    None
}

fn structural_scope_pair_for_node(buffer: &TextBuffer, node: Node<'_>) -> Option<SyntaxScopePair> {
    if node.start_position().row >= node.end_position().row {
        return None;
    }

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

    use super::{SyntaxHighlighter, SyntaxLanguage, language_for_path};
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
        assert_eq!(language_for_path(Some(Path::new("/tmp/readme.md"))), None);
        assert_eq!(language_for_path(None), None);
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
}
