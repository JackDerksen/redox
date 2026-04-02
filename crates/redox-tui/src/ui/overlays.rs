use std::collections::BTreeMap;

use minui::{ColorPair, Window};
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
struct DelimiterPair {
    start: Pos,
    end: Pos,
    kind: DelimiterKind,
}

pub(crate) fn draw_indent_guides(
    window: &mut dyn Window,
    row: u16,
    col: u16,
    visible_xs: &[usize],
    style: UiStyle,
    selected_cells: Option<&[bool]>,
) -> minui::Result<()> {
    for &visible_x in visible_xs {
        let is_selected = selected_cells
            .and_then(|cells| cells.get(visible_x))
            .copied()
            .unwrap_or(false);
        let bg = if is_selected {
            style.theme.selection_bg
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
) -> BTreeMap<usize, Vec<usize>> {
    let delimiter_pairs = delimiter_pairs(buffer);
    let scope = tree_sitter_scope
        .map(|pair| DelimiterPair {
            start: pair.start,
            end: pair.end,
            kind: DelimiterKind::Brace,
        })
        .or_else(|| find_active_scope_pair(buffer, cursor, &delimiter_pairs));
    let Some(scope) = scope else {
        return BTreeMap::new();
    };
    let Some(guide_cell) = active_scope_guide_cell(buffer, scope) else {
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

fn find_active_scope_pair(
    buffer: &TextBuffer,
    cursor: Pos,
    delimiter_pairs: &[DelimiterPair],
) -> Option<DelimiterPair> {
    find_active_pair_matching(buffer, cursor, delimiter_pairs, |pair| {
        pair.kind.is_structural() && pair.start.line < pair.end.line
    })
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
        let g_width = minui::cell_width(g, minui::prelude::TabPolicy::Fixed(4)) as usize;
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
) -> BTreeMap<usize, Vec<usize>> {
    let delimiter_pairs = delimiter_pairs(buffer);
    let Some(active_pair) = find_active_delimiter_pair(buffer, cursor, &delimiter_pairs) else {
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

fn find_active_delimiter_pair(
    buffer: &TextBuffer,
    cursor: Pos,
    delimiter_pairs: &[DelimiterPair],
) -> Option<DelimiterPair> {
    find_active_pair_matching(buffer, cursor, delimiter_pairs, |_| true)
}

fn find_active_pair_matching(
    buffer: &TextBuffer,
    cursor: Pos,
    delimiter_pairs: &[DelimiterPair],
    predicate: impl Fn(DelimiterPair) -> bool,
) -> Option<DelimiterPair> {
    let cursor = buffer.clamp_pos(cursor);

    if let Some(pair) = pair_for_cursor_char(buffer, cursor, delimiter_pairs, &predicate) {
        return Some(pair);
    }

    if cursor.col > 0 {
        let before = Pos::new(cursor.line, cursor.col - 1);
        if let Some(pair) = pair_for_cursor_char(buffer, before, delimiter_pairs, &predicate) {
            return Some(pair);
        }
    }

    smallest_enclosing_pair(buffer, cursor, delimiter_pairs, predicate)
}

fn pair_for_cursor_char(
    buffer: &TextBuffer,
    pos: Pos,
    delimiter_pairs: &[DelimiterPair],
    predicate: &impl Fn(DelimiterPair) -> bool,
) -> Option<DelimiterPair> {
    let ch = buffer.char_at(pos)?;
    delimiter_pairs.iter().copied().find_map(|pair| {
        if !predicate(pair) {
            return None;
        }
        let start_ch = buffer.char_at(pair.start)?;
        let end_ch = buffer.char_at(pair.end)?;
        if (pos == pair.start && ch == start_ch) || (pos == pair.end && ch == end_ch) {
            Some(pair)
        } else {
            None
        }
    })
}

fn smallest_enclosing_pair(
    buffer: &TextBuffer,
    cursor: Pos,
    delimiter_pairs: &[DelimiterPair],
    predicate: impl Fn(DelimiterPair) -> bool,
) -> Option<DelimiterPair> {
    let cursor_char = buffer.pos_to_char(cursor);
    delimiter_pairs
        .iter()
        .copied()
        .filter(|pair| predicate(*pair))
        .filter(|pair| {
            let start_char = buffer.pos_to_char(pair.start);
            let end_char = buffer.pos_to_char(pair.end);
            start_char < cursor_char && cursor_char <= end_char
        })
        .min_by_key(|pair| {
            let start_char = buffer.pos_to_char(pair.start);
            let end_char = buffer.pos_to_char(pair.end);
            end_char.saturating_sub(start_char)
        })
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
                    });
                }
            }
            '\'' if !is_escaped(prev_char) => {
                if let Some(start) = single_quotes.pop() {
                    pairs.push(DelimiterPair {
                        start: buffer.char_to_pos(start),
                        end: buffer.char_to_pos(char_idx),
                        kind: DelimiterKind::SingleQuote,
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

    use super::{
        active_scope_guide_cell, delimiter_pairs, filter_visible_indent_guides,
        find_active_delimiter_pair, find_active_scope_pair, leading_indent_guide_cells,
        visible_delimiter_cells,
    };

    #[test]
    fn highlights_pair_when_cursor_is_on_opening_delimiter() {
        let buffer = TextBuffer::from_str("(abc)");
        let pairs = delimiter_pairs(&buffer);
        let pair = find_active_delimiter_pair(&buffer, Pos::new(0, 0), &pairs);
        assert_eq!(
            pair.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 0), Pos::new(0, 4)))
        );
    }

    #[test]
    fn highlights_pair_when_cursor_is_on_closing_delimiter() {
        let buffer = TextBuffer::from_str("(abc)");
        let pairs = delimiter_pairs(&buffer);
        let pair = find_active_delimiter_pair(&buffer, Pos::new(0, 4), &pairs);
        assert_eq!(
            pair.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 0), Pos::new(0, 4)))
        );
    }

    #[test]
    fn highlights_nearest_surrounding_pair_when_cursor_is_inside_nested_pairs() {
        let buffer = TextBuffer::from_str("(\"  \")");
        let pairs = delimiter_pairs(&buffer);
        let pair = find_active_delimiter_pair(&buffer, Pos::new(0, 3), &pairs);
        assert_eq!(
            pair.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 1), Pos::new(0, 4)))
        );
    }

    #[test]
    fn cursor_after_delimiter_still_counts_as_on_it() {
        let buffer = TextBuffer::from_str("\"x\"");
        let pairs = delimiter_pairs(&buffer);
        let pair = find_active_delimiter_pair(&buffer, Pos::new(0, 1), &pairs);
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
        let pairs = delimiter_pairs(&buffer);
        let scope = find_active_scope_pair(&buffer, Pos::new(1, 11), &pairs);
        assert_eq!(
            scope.map(|pair| (pair.start, pair.end)),
            Some((Pos::new(0, 0), Pos::new(2, 0)))
        );
    }

    #[test]
    fn active_scope_guide_cell_uses_the_inner_indent_column() {
        let buffer = TextBuffer::from_str("if foo {\n\tif bar {\n\t\tbaz();\n\t}\n}\n");
        let pairs = delimiter_pairs(&buffer);
        let scope = find_active_scope_pair(&buffer, Pos::new(2, 2), &pairs).expect("scope");
        assert_eq!(active_scope_guide_cell(&buffer, scope), Some(4));
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
