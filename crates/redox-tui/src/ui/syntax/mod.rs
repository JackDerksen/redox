//! Tree-sitter-backed syntax highlighting for the editor viewport.

mod languages;

use std::path::Path;

use minui::prelude::TabPolicy;
use minui::{ColorPair, ColoredSpan, Window, cell_width};
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
    engine: Option<QuerySyntaxEngine>,
    cache: Option<HighlightCache>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxScopePair {
    pub start: Pos,
    pub end: Pos,
}

struct QuerySyntaxEngine {
    language: SyntaxLanguage,
    parser: Parser,
    query: Query,
    capture_roles: Vec<Option<SyntaxCapture>>,
    refine_role: fn(SyntaxRole, &str) -> SyntaxRole,
}

impl std::fmt::Debug for SyntaxHighlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntaxHighlighter")
            .field("engine_loaded", &self.engine.is_some())
            .field("cache", &self.cache)
            .finish()
    }
}

#[derive(Debug)]
struct HighlightCache {
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

#[derive(Debug)]
struct OwnedColoredSpan {
    text: String,
    colors: ColorPair,
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
            language: config.language,
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
    fn ensure_cache(
        &mut self,
        buffer: &TextBuffer,
        language: SyntaxLanguage,
    ) -> Option<&HighlightCache> {
        let needs_engine = self
            .engine
            .as_ref()
            .map(|engine| engine.language != language)
            .unwrap_or(true);
        if needs_engine {
            let config = language_config_for(language)?;
            self.engine = QuerySyntaxEngine::new(config);
        }

        let needs_rebuild = self
            .cache
            .as_ref()
            .map(|cache| cache.language != language)
            .unwrap_or(true);

        if needs_rebuild {
            let engine = self.engine.as_mut()?;
            let source = buffer.to_string();
            let (spans, tree) = engine.parse_line_spans(&source)?;
            self.cache = Some(HighlightCache {
                language,
                line_spans: spans,
                tree,
            });
        }

        self.cache.as_ref()
    }

    pub fn visible_line_spans(
        &mut self,
        buffer: &TextBuffer,
        language: Option<SyntaxLanguage>,
        first_line: usize,
        line_count: usize,
    ) -> Option<Vec<Vec<LineSyntaxSpan>>> {
        let language = language?;
        let cache = self.ensure_cache(buffer, language)?;
        let mut out = Vec::with_capacity(line_count);
        for line in first_line..first_line.saturating_add(line_count) {
            out.push(cache.line_spans.get(line).cloned().unwrap_or_default());
        }
        Some(out)
    }

    pub fn active_scope_pair(
        &mut self,
        buffer: &TextBuffer,
        language: Option<SyntaxLanguage>,
        cursor: Pos,
    ) -> Option<SyntaxScopePair> {
        let language = language?;
        let cache = self.ensure_cache(buffer, language)?;
        let cursor_byte = buffer.rope().char_to_byte(buffer.pos_to_char(cursor));
        let root = cache.tree.root_node();
        let node = root
            .named_descendant_for_byte_range(cursor_byte, cursor_byte)
            .or_else(|| {
                cursor_byte
                    .checked_sub(1)
                    .and_then(|byte| root.named_descendant_for_byte_range(byte, byte))
            })?;

        active_scope_pair_for_node(buffer, node)
    }

    pub fn invalidate(&mut self) {
        self.cache = None;
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

    let owned_spans = collect_visible_spans(
        source_line,
        scroll_x,
        width_cells,
        base_color,
        color_column,
        style,
        spans,
    );
    if !owned_spans.is_empty() {
        let spans_ref: Vec<ColoredSpan<'_>> = owned_spans
            .iter()
            .map(|span| ColoredSpan::new(&span.text, span.colors))
            .collect();
        window.write_spans_colored(row, col, &spans_ref)?;
    }

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

fn collect_visible_spans(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    base_color: ColorPair,
    color_column: Option<(usize, minui::Color)>,
    style: UiStyle,
    spans: &[LineSyntaxSpan],
) -> Vec<OwnedColoredSpan> {
    let mut owned_spans: Vec<OwnedColoredSpan> = Vec::new();
    let mut line_cells = 0usize;
    let mut byte_idx = 0usize;
    let mut syntax_idx = 0usize;
    let visible_end = scroll_x.saturating_add(width_cells);

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

        let text = if g == "\t" {
            " ".repeat(clipped_end.saturating_sub(clipped_start))
        } else if clipped_start == start_cell && clipped_end == end_cell {
            g.to_owned()
        } else {
            // Terminal graphemes are atomic; only tab expansion can be partially clipped.
            continue;
        };
        push_colored_text(&mut owned_spans, &text, colors);
    }

    owned_spans
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

fn push_colored_text(spans: &mut Vec<OwnedColoredSpan>, text: &str, colors: ColorPair) {
    if text.is_empty() {
        return;
    }

    if let Some(last) = spans.last_mut()
        && last.colors == colors
    {
        last.text.push_str(text);
        return;
    }

    spans.push(OwnedColoredSpan {
        text: text.to_string(),
        colors,
    });
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

    use super::{SyntaxHighlighter, SyntaxLanguage, collect_visible_spans, language_for_path};
    use crate::ui::UiStyle;
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
        let spans = highlighter
            .visible_line_spans(&buffer, Some(SyntaxLanguage::Rust), 0, 1)
            .expect("rust spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::Comment && span.start_byte == 0 && span.end_byte >= 10
        }));
    }

    #[test]
    fn string_quotes_are_highlighted_as_string() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("let s = \"hello\";\n");
        let spans = highlighter
            .visible_line_spans(&buffer, Some(SyntaxLanguage::Rust), 0, 1)
            .expect("rust spans");

        assert!(spans[0].iter().any(|span| {
            span.role == SyntaxRole::String && span.start_byte <= 8 && span.end_byte >= 15
        }));
    }

    #[test]
    fn partially_scrolled_tab_keeps_visible_tail() {
        let owned = collect_visible_spans(
            "\tX",
            2,
            3,
            UiStyle::default().syntax.operator,
            None,
            UiStyle::default(),
            &[],
        );

        let text = owned.into_iter().map(|span| span.text).collect::<String>();
        assert_eq!(text, "  X");
    }

    #[test]
    fn active_scope_pair_uses_multiline_structural_node() {
        let mut highlighter = SyntaxHighlighter::default();
        let buffer = TextBuffer::from_str("fn main() {\n    println!(\"hi\");\n}\n");
        let scope = highlighter
            .active_scope_pair(&buffer, Some(SyntaxLanguage::Rust), Pos::new(1, 15))
            .expect("scope");

        assert_eq!(scope.start, Pos::new(0, 10));
        assert_eq!(scope.end, Pos::new(2, 0));
    }
}
