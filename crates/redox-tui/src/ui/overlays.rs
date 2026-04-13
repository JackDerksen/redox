use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};

use minui::{Color, ColorPair, TabPolicy, Window, cell_width};
use redox_core::{Pos, TextBuffer};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui::{UiStyle, syntax::SyntaxScopePair};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterKind {
    Paren,
    Bracket,
    Brace,
    SingleQuote,
    DoubleQuote,
}

impl DelimiterKind {
    fn is_structural(self) -> bool {
        matches!(self, Self::Paren | Self::Bracket | Self::Brace)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DelimiterPair {
    start: Pos,
    end: Pos,
    kind: DelimiterKind,
    guide_cell: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexedDelimiterPair {
    pair: DelimiterPair,
    start_char: usize,
    end_char: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnclosingRange {
    start_char: usize,
    end_char: usize,
    pair_idx: usize,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct DelimiterAnalysis {
    pairs: Vec<IndexedDelimiterPair>,
    endpoint_index: HashMap<usize, Vec<usize>>,
    enclosing_ranges: Vec<EnclosingRange>,
    scope_ranges: Vec<EnclosingRange>,
}

#[derive(Debug, Default)]
pub(crate) struct DelimiterPairCache {
    analysis: Option<DelimiterAnalysis>,
}

impl DelimiterPairCache {
    pub(crate) fn get(&self) -> Option<&DelimiterAnalysis> {
        self.analysis.as_ref()
    }

    pub(crate) fn install(&mut self, analysis: DelimiterAnalysis) {
        self.analysis = Some(analysis);
    }
}

impl DelimiterAnalysis {
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.pairs.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    fn active_delimiter_pair(&self, buffer: &TextBuffer, cursor: Pos) -> Option<DelimiterPair> {
        let cursor = buffer.clamp_pos(cursor);
        let cursor_char = buffer.pos_to_char(cursor);

        if let Some(pair) = self.pair_at_endpoint(cursor_char, |_| true) {
            return Some(pair);
        }

        if cursor.col > 0
            && let Some(pair) = self.pair_at_endpoint(cursor_char - 1, |_| true)
        {
            return Some(pair);
        }

        self.pair_for_enclosing_range(cursor_char, &self.enclosing_ranges)
    }

    fn active_scope_pair(&self, buffer: &TextBuffer, cursor: Pos) -> Option<DelimiterPair> {
        let cursor = buffer.clamp_pos(cursor);
        let cursor_char = buffer.pos_to_char(cursor);
        let is_scope_pair =
            |pair: DelimiterPair| pair.kind.is_structural() && pair.start.line < pair.end.line;

        if let Some(pair) = self.pair_at_endpoint(cursor_char, is_scope_pair) {
            return Some(pair);
        }

        if cursor.col > 0
            && let Some(pair) = self.pair_at_endpoint(cursor_char - 1, is_scope_pair)
        {
            return Some(pair);
        }

        self.pair_for_enclosing_range(cursor_char, &self.scope_ranges)
    }

    fn pair_at_endpoint(
        &self,
        char_idx: usize,
        predicate: impl Fn(DelimiterPair) -> bool,
    ) -> Option<DelimiterPair> {
        self.endpoint_index.get(&char_idx)?.iter().find_map(|idx| {
            let pair = self.pairs.get(*idx)?.pair;
            predicate(pair).then_some(pair)
        })
    }

    fn pair_for_enclosing_range(
        &self,
        cursor_char: usize,
        ranges: &[EnclosingRange],
    ) -> Option<DelimiterPair> {
        let idx = ranges
            .partition_point(|range| range.start_char <= cursor_char)
            .checked_sub(1)?;
        let range = ranges[idx];
        (cursor_char <= range.end_char)
            .then(|| self.pairs.get(range.pair_idx).map(|indexed| indexed.pair))
            .flatten()
    }
}

pub(crate) fn compute_delimiter_analysis(buffer: &TextBuffer) -> DelimiterAnalysis {
    let pairs = delimiter_pairs(buffer)
        .into_iter()
        .map(|mut pair| {
            if pair.kind.is_structural() && pair.start.line < pair.end.line {
                pair.guide_cell = active_scope_guide_cell(buffer, pair);
            }
            IndexedDelimiterPair {
                start_char: buffer.pos_to_char(pair.start),
                end_char: buffer.pos_to_char(pair.end),
                pair,
            }
        })
        .collect::<Vec<_>>();
    DelimiterAnalysis {
        endpoint_index: delimiter_endpoint_index(&pairs),
        enclosing_ranges: delimiter_enclosing_ranges(&pairs, |_| true),
        scope_ranges: delimiter_enclosing_ranges(&pairs, |pair| {
            pair.kind.is_structural() && pair.start.line < pair.end.line
        }),
        pairs,
    }
}

pub(crate) fn draw_indent_guides(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    visible_xs: &[usize],
    occupied_cells: &[bool],
    style: UiStyle,
    selected_cells: Option<(&[bool], Color)>,
) -> minui::Result<()> {
    for &visible_x in visible_xs {
        if occupied_cells.get(visible_x).copied().unwrap_or(false) {
            continue;
        }
        let is_selected = selected_cells
            .and_then(|(cells, _)| cells.get(visible_x))
            .copied()
            .unwrap_or(false);
        let bg = if is_selected {
            selected_cells
                .map(|(_, bg)| bg)
                .unwrap_or(style.theme.selection_bg)
        } else {
            style.theme.bg
        };
        let color = ColorPair::new(style.theme.scope, bg);
        window.write_str_colored(row, col.saturating_add(visible_x as u16), "│", color)?;
    }

    Ok(())
}

pub(crate) fn active_scope_indent_guides(
    tree_sitter_scope: Option<SyntaxScopePair>,
    buffer: &TextBuffer,
    cursor: Pos,
    first_line: usize,
    line_count: usize,
    scroll_x: usize,
    width_cells: usize,
    cached_delimiter_analysis: Option<&DelimiterAnalysis>,
) -> BTreeMap<usize, Vec<usize>> {
    let scope = tree_sitter_scope
        .map(|pair| DelimiterPair {
            start: pair.start,
            end: pair.end,
            kind: DelimiterKind::Brace,
            guide_cell: None,
        })
        .or_else(|| {
            cached_delimiter_analysis
                .map(|analysis| analysis.active_scope_pair(buffer, cursor))
                .unwrap_or(None)
        });
    let Some(scope) = scope else {
        return BTreeMap::new();
    };
    let Some(guide_cell) = scope
        .guide_cell
        .or_else(|| active_scope_guide_cell(buffer, scope))
    else {
        return BTreeMap::new();
    };

    let visible = filter_visible_indent_guides(&[guide_cell], scroll_x, width_cells);
    if visible.is_empty() {
        return BTreeMap::new();
    }

    let mut guides = BTreeMap::new();
    let visible_xs = visible;
    let scope_start = scope.start.line.saturating_add(1).max(first_line);
    let scope_end = scope.end.line.min(first_line.saturating_add(line_count));
    for line_idx in scope_start..scope_end {
        guides.insert(line_idx, visible_xs.clone());
    }
    guides
}

fn active_scope_guide_cell(buffer: &TextBuffer, scope: DelimiterPair) -> Option<usize> {
    let opening_guides = leading_indent_guide_cells(&buffer.line_string(scope.start.line));
    for line_idx in scope.start.line.saturating_add(1)..scope.end.line {
        let source_line = buffer.line_string(line_idx);
        if source_line.trim().is_empty() {
            continue;
        }

        let inner_guides = leading_indent_guide_cells(&source_line);
        if inner_guides.len() > opening_guides.len() {
            return inner_guides.get(opening_guides.len()).copied();
        }
        if let Some(&last) = inner_guides.last() {
            return Some(last);
        }
    }
    None
}

fn leading_indent_guide_cells(source_line: &str) -> Vec<usize> {
    if source_line.is_empty() {
        return Vec::new();
    }

    let mut guides = Vec::new();
    let mut line_cells = 0usize;
    let mut consecutive_spaces = 0usize;
    let mut current_space_block_start = 0usize;

    for ch in source_line.chars() {
        match ch {
            '\t' => {
                consecutive_spaces = 0;
                guides.push(line_cells);
                line_cells = line_cells.saturating_add(4);
            }
            ' ' => {
                if consecutive_spaces % 4 == 0 {
                    current_space_block_start = line_cells;
                }
                consecutive_spaces = consecutive_spaces.saturating_add(1);
                line_cells = line_cells.saturating_add(1);
                if consecutive_spaces % 4 == 0 {
                    guides.push(current_space_block_start);
                }
            }
            _ => break,
        }
    }

    guides
}

fn filter_visible_indent_guides(
    absolute_guides: &[usize],
    scroll_x: usize,
    width_cells: usize,
) -> Vec<usize> {
    absolute_guides
        .iter()
        .copied()
        .filter(|cell| *cell >= scroll_x)
        .map(|cell| cell.saturating_sub(scroll_x))
        .take_while(|visible_x| *visible_x < width_cells)
        .collect()
}

pub(crate) fn draw_delimiter_highlights(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    delimiter_highlight_chars: &[usize],
    style: UiStyle,
) -> minui::Result<()> {
    let visible = visible_delimiter_cells(
        source_line,
        scroll_x,
        width_cells,
        delimiter_highlight_chars,
    );
    if visible.is_empty() {
        return Ok(());
    }

    for (visible_x, text) in visible {
        let color = ColorPair::new(style.theme.white, style.theme.scope);
        window.write_str_colored(row, col.saturating_add(visible_x as u16), &text, color)?;
    }

    Ok(())
}

fn visible_delimiter_cells(
    source_line: &str,
    scroll_x: usize,
    width_cells: usize,
    delimiter_highlight_chars: &[usize],
) -> Vec<(usize, String)> {
    if width_cells == 0 || delimiter_highlight_chars.is_empty() || source_line.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut line_cells = 0usize;
    let mut char_idx = 0usize;

    for g in source_line.graphemes(true) {
        let g_width = cell_width(g, TabPolicy::Fixed(4)) as usize;
        let g_chars = g.chars().count();
        let start_cell = line_cells;
        let end_cell = line_cells.saturating_add(g_width);
        let start_char = char_idx;

        line_cells = end_cell;
        char_idx = char_idx.saturating_add(g_chars);

        if !delimiter_highlight_chars.contains(&start_char) {
            continue;
        }
        if end_cell <= scroll_x || start_cell < scroll_x {
            continue;
        }

        let visible_x = start_cell.saturating_sub(scroll_x);
        if visible_x.saturating_add(g_width) > width_cells {
            break;
        }

        let text = if g == "\t" {
            " ".repeat(g_width.max(1))
        } else {
            g.to_owned()
        };
        out.push((visible_x, text));
    }

    out
}

pub(crate) fn active_delimiter_highlights(
    buffer: &TextBuffer,
    cursor: Pos,
    first_line: usize,
    line_count: usize,
    delimiter_analysis: &DelimiterAnalysis,
) -> BTreeMap<usize, Vec<usize>> {
    let Some(active_pair) = delimiter_analysis.active_delimiter_pair(buffer, cursor) else {
        return BTreeMap::new();
    };

    let mut highlights = BTreeMap::new();
    for pos in [active_pair.start, active_pair.end] {
        if pos.line >= first_line && pos.line < first_line.saturating_add(line_count) {
            highlights
                .entry(pos.line)
                .or_insert_with(Vec::new)
                .push(pos.col);
        }
    }
    highlights
}

fn delimiter_endpoint_index(pairs: &[IndexedDelimiterPair]) -> HashMap<usize, Vec<usize>> {
    let mut endpoints: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, pair) in pairs.iter().enumerate() {
        endpoints.entry(pair.start_char).or_default().push(idx);
        endpoints.entry(pair.end_char).or_default().push(idx);
    }
    endpoints
}

fn delimiter_enclosing_ranges(
    pairs: &[IndexedDelimiterPair],
    predicate: impl Fn(DelimiterPair) -> bool,
) -> Vec<EnclosingRange> {
    #[derive(Clone, Copy)]
    enum EventKind {
        Start,
        End,
    }

    #[derive(Clone, Copy)]
    struct Event {
        char_idx: usize,
        kind: EventKind,
        pair_idx: usize,
    }

    let mut events = Vec::new();
    for (pair_idx, indexed) in pairs.iter().enumerate() {
        if predicate(indexed.pair) {
            events.push(Event {
                char_idx: indexed.start_char,
                kind: EventKind::Start,
                pair_idx,
            });
            events.push(Event {
                char_idx: indexed.end_char,
                kind: EventKind::End,
                pair_idx,
            });
        }
    }
    events.sort_by_key(|event| {
        (
            event.char_idx,
            match event.kind {
                EventKind::End => 0usize,
                EventKind::Start => 1,
            },
        )
    });

    let mut ranges = Vec::new();
    let mut active = vec![false; pairs.len()];
    let mut innermost = BinaryHeap::<Reverse<(usize, usize)>>::new();
    let mut prev_char = None;
    let mut idx = 0;
    while idx < events.len() {
        let char_idx = events[idx].char_idx;
        if let Some(start_char) = prev_char
            && start_char <= char_idx
            && let Some(Reverse((_, pair_idx))) = innermost.peek().copied()
        {
            ranges.push(EnclosingRange {
                start_char,
                end_char: char_idx,
                pair_idx,
            });
        }

        while idx < events.len() && events[idx].char_idx == char_idx {
            match events[idx].kind {
                EventKind::Start => {
                    let pair_idx = events[idx].pair_idx;
                    active[pair_idx] = true;
                    innermost.push(Reverse((
                        pairs[pair_idx].end_char - pairs[pair_idx].start_char,
                        pair_idx,
                    )));
                }
                EventKind::End => active[events[idx].pair_idx] = false,
            }
            idx += 1;
        }

        while let Some(Reverse((_, pair_idx))) = innermost.peek().copied() {
            if active[pair_idx] {
                break;
            }
            innermost.pop();
        }
        prev_char = char_idx.checked_add(1);
    }

    ranges
}

fn delimiter_pairs(buffer: &TextBuffer) -> Vec<DelimiterPair> {
    let source = buffer.to_string();
    let mut pairs = Vec::new();
    let mut parens = Vec::new();
    let mut brackets = Vec::new();
    let mut braces = Vec::new();
    let mut single_quotes = Vec::new();
    let mut double_quotes = Vec::new();
    let mut prev_char: Option<char> = None;

    for (char_idx, ch) in source.chars().enumerate() {
        match ch {
            '(' => parens.push(char_idx),
            ')' => {
                if let Some(start) = parens.pop() {
                    pairs.push(DelimiterPair {
                        start: buffer.char_to_pos(start),
                        end: buffer.char_to_pos(char_idx),
                        kind: DelimiterKind::Paren,
                        guide_cell: None,
                    });
                }
            }
            '[' => brackets.push(char_idx),
            ']' => {
                if let Some(start) = brackets.pop() {
                    pairs.push(DelimiterPair {
                        start: buffer.char_to_pos(start),
                        end: buffer.char_to_pos(char_idx),
                        kind: DelimiterKind::Bracket,
                        guide_cell: None,
                    });
                }
            }
            '{' => braces.push(char_idx),
            '}' => {
                if let Some(start) = braces.pop() {
                    pairs.push(DelimiterPair {
                        start: buffer.char_to_pos(start),
                        end: buffer.char_to_pos(char_idx),
                        kind: DelimiterKind::Brace,
                        guide_cell: None,
                    });
                }
            }
            '\'' if !is_escaped(prev_char) => {
                if let Some(start) = single_quotes.pop() {
                    pairs.push(DelimiterPair {
                        start: buffer.char_to_pos(start),
                        end: buffer.char_to_pos(char_idx),
                        kind: DelimiterKind::SingleQuote,
                        guide_cell: None,
                    });
                } else {
                    single_quotes.push(char_idx);
                }
            }
            '"' if !is_escaped(prev_char) => {
                if let Some(start) = double_quotes.pop() {
                    pairs.push(DelimiterPair {
                        start: buffer.char_to_pos(start),
                        end: buffer.char_to_pos(char_idx),
                        kind: DelimiterKind::DoubleQuote,
                        guide_cell: None,
                    });
                } else {
                    double_quotes.push(char_idx);
                }
            }
            _ => {}
        }
        prev_char = Some(ch);
    }

    pairs
}

fn is_escaped(prev_char: Option<char>) -> bool {
    matches!(prev_char, Some('\\'))
}

#[cfg(test)]
mod tests {
    use redox_core::{Pos, TextBuffer};

    use crate::ui::syntax::SyntaxScopePair;

    use super::{
        active_scope_guide_cell, active_scope_indent_guides, compute_delimiter_analysis,
        filter_visible_indent_guides, leading_indent_guide_cells, visible_delimiter_cells,
    };

    #[test]
    fn highlights_pair_when_cursor_is_on_opening_delimiter() {
        let buffer = TextBuffer::from_str("(abc)");
        let analysis = compute_delimiter_analysis(&buffer);
        let pair = analysis.active_delimiter_pair(&buffer, Pos::new(0, 0));
        assert_eq!(
            pair.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 0), Pos::new(0, 4)))
        );
    }

    #[test]
    fn highlights_pair_when_cursor_is_on_closing_delimiter() {
        let buffer = TextBuffer::from_str("(abc)");
        let analysis = compute_delimiter_analysis(&buffer);
        let pair = analysis.active_delimiter_pair(&buffer, Pos::new(0, 4));
        assert_eq!(
            pair.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 0), Pos::new(0, 4)))
        );
    }

    #[test]
    fn highlights_nearest_surrounding_pair_when_cursor_is_inside_nested_pairs() {
        let buffer = TextBuffer::from_str("(\"  \")");
        let analysis = compute_delimiter_analysis(&buffer);
        let pair = analysis.active_delimiter_pair(&buffer, Pos::new(0, 3));
        assert_eq!(
            pair.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 1), Pos::new(0, 4)))
        );
    }

    #[test]
    fn cursor_after_delimiter_still_counts_as_on_it() {
        let buffer = TextBuffer::from_str("\"x\"");
        let analysis = compute_delimiter_analysis(&buffer);
        let pair = analysis.active_delimiter_pair(&buffer, Pos::new(0, 1));
        assert_eq!(
            pair.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 0), Pos::new(0, 2)))
        );
    }

    #[test]
    fn visible_delimiter_cells_map_to_the_expected_columns() {
        assert_eq!(
            visible_delimiter_cells("    if {", 0, 20, &[7]),
            vec![(7, "{".to_string())]
        );
        assert_eq!(
            visible_delimiter_cells("    }", 0, 20, &[4]),
            vec![(4, "}".to_string())]
        );
    }

    #[test]
    fn leading_indent_guides_follow_tabs_and_space_blocks() {
        assert_eq!(leading_indent_guide_cells("\t\tfoo()"), vec![0, 4]);
        assert_eq!(leading_indent_guide_cells("        foo()"), vec![0, 4]);
        assert_eq!(leading_indent_guide_cells("    \tfoo()"), vec![0, 4]);
        assert!(leading_indent_guide_cells("  foo()").is_empty());
    }

    #[test]
    fn active_scope_uses_structural_pair_outside_same_line_quotes() {
        let buffer = TextBuffer::from_str("{\n\tprintln(\"hi\");\n}\n");
        let analysis = compute_delimiter_analysis(&buffer);
        let scope = analysis.active_scope_pair(&buffer, Pos::new(1, 11));
        assert_eq!(
            scope.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 0), Pos::new(2, 0)))
        );
    }

    #[test]
    fn active_scope_guide_cell_uses_the_inner_indent_column() {
        let buffer = TextBuffer::from_str("if foo {\n\tif bar {\n\t\tbaz();\n\t}\n}\n");
        let analysis = compute_delimiter_analysis(&buffer);
        let scope = analysis
            .active_scope_pair(&buffer, Pos::new(2, 2))
            .expect("scope");
        assert_eq!(active_scope_guide_cell(&buffer, scope), Some(4));
    }

    #[test]
    fn active_scope_guides_use_tree_sitter_scope_without_delimiter_scan() {
        let buffer = TextBuffer::from_str("{\n    answer();\n}\n");
        let guides = active_scope_indent_guides(
            Some(SyntaxScopePair {
                start: Pos::new(0, 0),
                end: Pos::new(2, 0),
            }),
            &buffer,
            Pos::new(1, 4),
            0,
            3,
            0,
            20,
            None,
        );
        assert_eq!(guides.get(&1), Some(&vec![0]));
    }

    #[test]
    fn visible_indent_guides_filter_by_scroll() {
        assert_eq!(
            filter_visible_indent_guides(&[0, 4, 8], 0, 20),
            vec![0, 4, 8]
        );
        assert_eq!(filter_visible_indent_guides(&[0, 4, 8], 2, 20), vec![2, 6]);
    }
}
