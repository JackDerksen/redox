//! Text-object resolution for operator-pending commands.
//!
//! These helpers resolve reusable targets like `iw`, `ap`, and `i]` into
//! mode-aware edit plans so delete/change/yank can share the same core logic.

use super::selection::VisualModeKind;
use super::TextBuffer;
use crate::buffer::{Pos, Selection};
use crate::buffer::util::is_word_char;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectScope {
    Inner,
    Around,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelimiterKind {
    Parentheses,
    Brackets,
    Braces,
    SingleQuotes,
    DoubleQuotes,
    Backticks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectKind {
    Word,
    BigWord,
    Paragraph,
    Delimiter(DelimiterKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextObjectSpec {
    pub scope: TextObjectScope,
    pub kind: TextObjectKind,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextObjectEditPlan {
    pub delete_ranges: Vec<(Pos, Pos)>,
    pub text: String,
    pub mode: VisualModeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeMode {
    Char,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextObjectRange {
    start: usize,
    end: usize,
    mode: RangeMode,
}

impl TextBuffer {
    pub fn text_object_selection(
        &self,
        cursor: Pos,
        spec: TextObjectSpec,
    ) -> Option<(Selection, VisualModeKind)> {
        let range = self.text_object_range(cursor, spec)?;

        match range.mode {
            RangeMode::Char => {
                if range.start >= range.end {
                    return None;
                }

                Some((
                    Selection::new(
                        self.char_to_pos(range.start),
                        self.char_to_pos(range.end.saturating_sub(1)),
                    ),
                    VisualModeKind::Char,
                ))
            }
            RangeMode::Line => {
                let start_line = self.char_to_line(range.start);
                let end_line = self.char_to_line(range.end.saturating_sub(1));
                Some((
                    Selection::new(Pos::new(start_line, 0), Pos::new(end_line, 0)),
                    VisualModeKind::Line,
                ))
            }
        }
    }

    pub fn text_object_edit_plan(&self, cursor: Pos, spec: TextObjectSpec) -> Option<TextObjectEditPlan> {
        let range = self.text_object_range(cursor, spec)?;

        Some(match range.mode {
            RangeMode::Char => TextObjectEditPlan {
                delete_ranges: vec![(
                    self.char_to_pos(range.start),
                    self.char_to_pos(range.end),
                )],
                text: self.slice_chars(range.start, range.end),
                mode: VisualModeKind::Char,
            },
            RangeMode::Line => {
                let start_line = self.char_to_line(range.start);
                let end_line = self.char_to_line(range.end.saturating_sub(1));
                let (start, end) = self.line_span_pos_range(start_line, end_line);
                TextObjectEditPlan {
                    delete_ranges: vec![(start, end)],
                    text: self.line_span_text_linewise_register(start_line, end_line),
                    mode: VisualModeKind::Line,
                }
            }
        })
    }

    fn text_object_range(&self, cursor: Pos, spec: TextObjectSpec) -> Option<TextObjectRange> {
        match spec.kind {
            TextObjectKind::Word => self.word_text_object_range(cursor, spec.scope, spec.count),
            TextObjectKind::BigWord => {
                self.big_word_text_object_range(cursor, spec.scope, spec.count)
            }
            TextObjectKind::Paragraph => {
                self.paragraph_text_object_range(cursor, spec.scope, spec.count)
            }
            TextObjectKind::Delimiter(kind) => {
                self.delimiter_text_object_range(cursor, kind, spec.scope, spec.count)
            }
        }
    }

    fn word_text_object_range(
        &self,
        cursor: Pos,
        scope: TextObjectScope,
        count: usize,
    ) -> Option<TextObjectRange> {
        self.run_text_object_range(cursor, scope, count, is_word_char)
    }

    fn big_word_text_object_range(
        &self,
        cursor: Pos,
        scope: TextObjectScope,
        count: usize,
    ) -> Option<TextObjectRange> {
        self.run_text_object_range(cursor, scope, count, |ch| !ch.is_whitespace())
    }

    fn run_text_object_range(
        &self,
        cursor: Pos,
        scope: TextObjectScope,
        count: usize,
        predicate: impl Fn(char) -> bool + Copy,
    ) -> Option<TextObjectRange> {
        let count = count.max(1);
        let mut start = self.find_seed_char(self.pos_to_char(cursor), predicate)?;
        let mut end = self.run_end(start, predicate);
        start = self.run_start(start, predicate);

        for _ in 1..count {
            let next_start = self.find_next_run_start(end, predicate)?;
            end = self.run_end(next_start, predicate);
        }

        if scope == TextObjectScope::Around {
            let trailing_end = self.scan_whitespace_forward(end);
            if trailing_end > end {
                end = trailing_end;
            } else {
                start = self.scan_whitespace_backward(start);
            }
        }

        Some(TextObjectRange {
            start,
            end,
            mode: RangeMode::Char,
        })
    }

    fn paragraph_text_object_range(
        &self,
        cursor: Pos,
        scope: TextObjectScope,
        count: usize,
    ) -> Option<TextObjectRange> {
        let count = count.max(1);
        let mut start_line = self.clamp_line(cursor.line);

        if self.line_is_blank(start_line) {
            return Some(TextObjectRange {
                start: self.line_to_char(start_line),
                end: self.line_full_end_char(start_line),
                mode: RangeMode::Line,
            });
        }

        while start_line > 0 && !self.line_is_blank(start_line - 1) {
            start_line -= 1;
        }

        let mut end_line = self.clamp_line(cursor.line);
        while end_line + 1 < self.len_lines() && !self.line_is_blank(end_line + 1) {
            end_line += 1;
        }

        for _ in 1..count {
            let mut next_line = end_line.saturating_add(1);
            while next_line < self.len_lines() && self.line_is_blank(next_line) {
                next_line += 1;
            }
            if next_line >= self.len_lines() || self.line_is_blank(next_line) {
                break;
            }
            end_line = next_line;
            while end_line + 1 < self.len_lines() && !self.line_is_blank(end_line + 1) {
                end_line += 1;
            }
        }

        if scope == TextObjectScope::Around {
            let mut trailing = end_line.saturating_add(1);
            let mut extended = false;
            while trailing < self.len_lines() && self.line_is_blank(trailing) {
                end_line = trailing;
                trailing += 1;
                extended = true;
            }
            if !extended {
                while start_line > 0 && self.line_is_blank(start_line - 1) {
                    start_line -= 1;
                }
            }
        }

        Some(TextObjectRange {
            start: self.line_to_char(start_line),
            end: self.line_full_end_char(end_line),
            mode: RangeMode::Line,
        })
    }

    fn delimiter_text_object_range(
        &self,
        cursor: Pos,
        kind: DelimiterKind,
        scope: TextObjectScope,
        count: usize,
    ) -> Option<TextObjectRange> {
        let count = count.max(1);
        let cursor_char = self.pos_to_char(cursor);
        let anchor_before = cursor_char.saturating_sub(1);
        let (open, close) = delimiter_chars(kind);
        if open == close {
            return self.symmetric_delimiter_text_object_range(cursor, open, scope, count);
        }

        let cursor_line = self.clamp_line(cursor.line);

        let mut containing_pairs = Vec::new();
        let mut same_line_pairs = Vec::new();
        let mut stack = Vec::new();
        for char_idx in 0..self.len_chars() {
            let ch = self.rope().char(char_idx);
            if ch == open {
                stack.push(char_idx);
            } else if ch == close
                && let Some(start) = stack.pop()
            {
                if pair_contains_cursor(start, char_idx, cursor_char, anchor_before) {
                    containing_pairs.push((start, char_idx));
                } else if pair_is_on_line(self, start, char_idx, cursor_line) {
                    same_line_pairs.push((start, char_idx));
                }
            }
        }

        let (start, end_inclusive) = if !containing_pairs.is_empty() {
            containing_pairs.sort_by_key(|(start, end)| end.saturating_sub(*start));
            *containing_pairs.get(count.saturating_sub(1))?
        } else {
            same_line_pairs.sort_by_key(|(start, end)| {
                (
                    delimiter_pair_distance(*start, *end, cursor_char),
                    end.saturating_sub(*start),
                    *start,
                )
            });
            *same_line_pairs.get(count.saturating_sub(1))?
        };

        let (range_start, range_end) = match scope {
            TextObjectScope::Inner => (start.saturating_add(1), end_inclusive),
            TextObjectScope::Around => (start, end_inclusive.saturating_add(1)),
        };

        Some(TextObjectRange {
            start: range_start.min(self.len_chars()),
            end: range_end.min(self.len_chars()),
            mode: RangeMode::Char,
        })
    }

    fn symmetric_delimiter_text_object_range(
        &self,
        cursor: Pos,
        delimiter: char,
        scope: TextObjectScope,
        count: usize,
    ) -> Option<TextObjectRange> {
        let cursor_line = self.clamp_line(cursor.line);
        let cursor_char = self.pos_to_char(cursor);
        let anchor_before = cursor_char.saturating_sub(1);
        let line_range = self.line_char_range(cursor_line);
        let mut quote_chars = Vec::new();

        for char_idx in line_range.clone() {
            if self.rope().char(char_idx) == delimiter && !self.char_is_escaped(char_idx) {
                quote_chars.push(char_idx);
            }
        }

        let mut containing_pairs = Vec::new();
        let mut same_line_pairs = Vec::new();
        for pair in quote_chars.chunks_exact(2) {
            let start = pair[0];
            let end_inclusive = pair[1];
            if pair_contains_cursor(start, end_inclusive, cursor_char, anchor_before) {
                containing_pairs.push((start, end_inclusive));
            } else {
                same_line_pairs.push((start, end_inclusive));
            }
        }

        let (start, end_inclusive) = if !containing_pairs.is_empty() {
            containing_pairs.sort_by_key(|(start, end)| end.saturating_sub(*start));
            *containing_pairs.get(count.saturating_sub(1))?
        } else {
            same_line_pairs.sort_by_key(|(start, end)| {
                (
                    delimiter_pair_distance(*start, *end, cursor_char),
                    end.saturating_sub(*start),
                    *start,
                )
            });
            *same_line_pairs.get(count.saturating_sub(1))?
        };

        let (range_start, range_end) = match scope {
            TextObjectScope::Inner => (start.saturating_add(1), end_inclusive),
            TextObjectScope::Around => (start, end_inclusive.saturating_add(1)),
        };

        Some(TextObjectRange {
            start: range_start.min(self.len_chars()),
            end: range_end.min(self.len_chars()),
            mode: RangeMode::Char,
        })
    }

    fn find_seed_char(
        &self,
        cursor_char: usize,
        predicate: impl Fn(char) -> bool + Copy,
    ) -> Option<usize> {
        let maxc = self.len_chars();
        if maxc == 0 {
            return None;
        }

        let clamped = cursor_char.min(maxc.saturating_sub(1));
        if predicate(self.rope().char(clamped)) {
            return Some(clamped);
        }

        if let Some(next) = self.find_next_run_start(clamped, predicate) {
            return Some(next);
        }

        if clamped > 0 && predicate(self.rope().char(clamped - 1)) {
            return Some(clamped - 1);
        }

        self.find_prev_run_start(clamped, predicate)
    }

    fn run_start(&self, mut char_idx: usize, predicate: impl Fn(char) -> bool + Copy) -> usize {
        while char_idx > 0 && predicate(self.rope().char(char_idx - 1)) {
            char_idx -= 1;
        }
        char_idx
    }

    fn run_end(&self, mut char_idx: usize, predicate: impl Fn(char) -> bool + Copy) -> usize {
        while char_idx < self.len_chars() && predicate(self.rope().char(char_idx)) {
            char_idx += 1;
        }
        char_idx
    }

    fn find_next_run_start(
        &self,
        mut char_idx: usize,
        predicate: impl Fn(char) -> bool + Copy,
    ) -> Option<usize> {
        while char_idx < self.len_chars() {
            if predicate(self.rope().char(char_idx)) {
                return Some(char_idx);
            }
            char_idx += 1;
        }
        None
    }

    fn find_prev_run_start(
        &self,
        mut char_idx: usize,
        predicate: impl Fn(char) -> bool + Copy,
    ) -> Option<usize> {
        char_idx = char_idx.min(self.len_chars());
        while char_idx > 0 {
            char_idx -= 1;
            if predicate(self.rope().char(char_idx)) {
                return Some(self.run_start(char_idx, predicate));
            }
        }
        None
    }

    fn scan_whitespace_forward(&self, mut char_idx: usize) -> usize {
        while char_idx < self.len_chars() && self.rope().char(char_idx).is_whitespace() {
            char_idx += 1;
        }
        char_idx
    }

    fn scan_whitespace_backward(&self, mut char_idx: usize) -> usize {
        while char_idx > 0 && self.rope().char(char_idx - 1).is_whitespace() {
            char_idx -= 1;
        }
        char_idx
    }

    fn line_is_blank(&self, line_idx: usize) -> bool {
        self.line_string(line_idx).trim().is_empty()
    }

    fn char_is_escaped(&self, char_idx: usize) -> bool {
        let mut backslashes = 0;
        let mut idx = char_idx;
        while idx > 0 {
            idx -= 1;
            if self.rope().char(idx) != '\\' {
                break;
            }
            backslashes += 1;
        }
        backslashes % 2 == 1
    }
}

fn delimiter_chars(kind: DelimiterKind) -> (char, char) {
    match kind {
        DelimiterKind::Parentheses => ('(', ')'),
        DelimiterKind::Brackets => ('[', ']'),
        DelimiterKind::Braces => ('{', '}'),
        DelimiterKind::SingleQuotes => ('\'', '\''),
        DelimiterKind::DoubleQuotes => ('"', '"'),
        DelimiterKind::Backticks => ('`', '`'),
    }
}

fn pair_contains_cursor(start: usize, end_inclusive: usize, cursor_char: usize, before: usize) -> bool {
    (start <= cursor_char && cursor_char <= end_inclusive)
        || (cursor_char > 0 && start <= before && before <= end_inclusive)
}

fn pair_is_on_line(buf: &TextBuffer, start: usize, end_inclusive: usize, line: usize) -> bool {
    buf.char_to_line(start) == line && buf.char_to_line(end_inclusive) == line
}

fn delimiter_pair_distance(start: usize, end_inclusive: usize, cursor_char: usize) -> usize {
    if cursor_char < start {
        start - cursor_char
    } else if cursor_char > end_inclusive {
        cursor_char - end_inclusive
    } else {
        0
    }
}
