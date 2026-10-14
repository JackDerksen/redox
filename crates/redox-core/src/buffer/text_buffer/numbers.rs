//! Vim-style number increments, with sequences for selected columns of numbers.

use crate::{Pos, Selection, TextBuffer, VisualModeKind};

/// How a selection's numbers change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionIncrement {
    /// A column of equal numbers becomes a sequence starting at that value.
    /// Any other number changes by one step.
    Smart,
    /// Vim's `g ctrl+a`: the nth number in each column changes by n steps.
    Progressive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Radix {
    Decimal,
    Hex,
    Binary,
}

impl Radix {
    fn base(self) -> u32 {
        match self {
            Self::Decimal => 10,
            Self::Hex => 16,
            Self::Binary => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NumberToken {
    /// Char columns, including any sign or radix prefix.
    start: usize,
    end: usize,
    radix: Radix,
    prefix: String,
    value: i128,
    digits: usize,
    zero_padded: bool,
    uppercase: bool,
}

impl NumberToken {
    fn format(&self, value: i128) -> String {
        let width = if self.zero_padded { self.digits } else { 0 };
        match self.radix {
            Radix::Decimal => {
                let digits = format!("{:0width$}", value.unsigned_abs());
                if value < 0 {
                    format!("-{digits}")
                } else {
                    digits
                }
            }
            // Prefixed numbers are unsigned and wrap like Vim's 64-bit values.
            Radix::Hex | Radix::Binary => {
                let value = value.rem_euclid(1 << 64) as u64;
                let digits = match (self.radix, self.uppercase) {
                    (Radix::Hex, true) => format!("{value:0width$X}"),
                    (Radix::Hex, false) => format!("{value:0width$x}"),
                    _ => format!("{value:0width$b}"),
                };
                format!("{}{digits}", self.prefix)
            }
        }
    }
}

/// Find numbers on a line. `strict` skips digits attached to identifiers (`x1`)
/// and fractional parts (`1.5`), which selections should leave alone.
fn parse_numbers(chars: &[char], strict: bool) -> Vec<NumberToken> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let prev = i.checked_sub(1).map(|index| chars[index]);
        let before_prev = i.checked_sub(2).map(|index| chars[index]);
        let prefixed = |marker: [char; 2], digit: fn(&char) -> bool| {
            chars[i] == '0'
                && chars.get(i + 1).is_some_and(|c| marker.contains(c))
                && chars.get(i + 2).is_some_and(digit)
        };
        let radix = if prefixed(['x', 'X'], char::is_ascii_hexdigit) {
            Radix::Hex
        } else if prefixed(['b', 'B'], |c| matches!(c, '0' | '1')) {
            Radix::Binary
        } else {
            Radix::Decimal
        };
        let digits_start = if radix == Radix::Decimal { i } else { i + 2 };
        let mut end = digits_start;
        while chars.get(end).is_some_and(|c| c.is_digit(radix.base())) {
            end += 1;
        }
        let start = i;
        i = end;

        let attached = prev.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let fraction = prev == Some('.') && before_prev.is_some_and(|c| c.is_ascii_digit());
        if strict && (attached || fraction) {
            continue;
        }
        let digits = chars[digits_start..end].iter().collect::<String>();
        let Ok(magnitude) = i128::from_str_radix(&digits, radix.base()) else {
            continue;
        };
        let negative = radix == Radix::Decimal
            && prev == Some('-')
            && !before_prev.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | ')' | ']'));
        tokens.push(NumberToken {
            start: start - usize::from(negative),
            end,
            radix,
            prefix: chars[start..digits_start].iter().collect(),
            value: if negative { -magnitude } else { magnitude },
            digits: digits.chars().count(),
            zero_padded: digits.len() > 1 && digits.starts_with('0'),
            uppercase: digits.chars().any(|c| c.is_ascii_uppercase()),
        });
    }
    tokens
}

impl TextBuffer {
    /// Add `delta` to the number under or after the cursor on its line, as Vim's
    /// `ctrl+a` does. Return the position of the new number's last character.
    pub fn increment_number_at(&mut self, cursor: Pos, delta: i128) -> Option<Pos> {
        let line = self.clamp_line(cursor.line);
        let chars = self.line_string(line).chars().collect::<Vec<_>>();
        let token = parse_numbers(&chars, false)
            .into_iter()
            .find(|token| token.end > cursor.col)?;
        let text = token.format(token.value.saturating_add(delta));
        let line_start = self.line_to_char(line);
        let end = self.replace_char_range(line_start + token.start, line_start + token.end, &text);
        Some(self.move_left(end))
    }

    /// Change the numbers a visual selection touches. Numbers are grouped into
    /// columns: the nth number on each selected line, or every number when the
    /// selection covers one line. Return whether anything changed.
    pub fn increment_numbers_in_selection(
        &mut self,
        selection: Selection,
        mode: VisualModeKind,
        delta: i128,
        kind: SelectionIncrement,
    ) -> bool {
        let plan = self.visual_selection_edit_plan(selection, mode);
        let mut rows = Vec::new();
        for (a, b) in plan.delete_ranges {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            for line in start.line..=end.line {
                if line == end.line && end.col == 0 && line > start.line {
                    continue;
                }
                let from = if line == start.line { start.col } else { 0 };
                let to = if line == end.line {
                    end.col
                } else {
                    self.line_len_chars(line)
                };
                let chars = self.line_string(line).chars().collect::<Vec<_>>();
                let row = parse_numbers(&chars, true)
                    .into_iter()
                    .filter(|token| token.end > from && token.start < to)
                    .map(|token| (line, token))
                    .collect::<Vec<_>>();
                if !row.is_empty() {
                    rows.push(row);
                }
            }
        }

        let columns = if rows.len() == 1 {
            rows
        } else {
            let width = rows.iter().map(Vec::len).max().unwrap_or(0);
            (0..width)
                .map(|index| {
                    rows.iter()
                        .filter_map(|row| row.get(index).cloned())
                        .collect::<Vec<_>>()
                })
                .collect()
        };

        let mut edits = Vec::new();
        for column in columns {
            let sequence = column.len() > 1
                && column
                    .iter()
                    .all(|(_, token)| token.value == column[0].1.value);
            for (index, (line, token)) in column.into_iter().enumerate() {
                let steps = match kind {
                    SelectionIncrement::Smart if sequence => index,
                    SelectionIncrement::Smart => 1,
                    SelectionIncrement::Progressive => index + 1,
                };
                if steps == 0 {
                    continue;
                }
                let value = token
                    .value
                    .saturating_add(delta.saturating_mul(steps as i128));
                edits.push((line, token.start, token.end, token.format(value)));
            }
        }
        edits.sort_by_key(|&(line, start, _, _)| std::cmp::Reverse((line, start)));
        for (line, start, end, text) in &edits {
            let line_start = self.line_to_char(*line);
            self.replace_char_range(line_start + start, line_start + end, text);
        }
        !edits.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn increment_at(text: &str, cursor: Pos, delta: i128) -> (String, Option<Pos>) {
        let mut buffer = TextBuffer::from_text(text);
        let landed = buffer.increment_number_at(cursor, delta);
        (buffer.to_string(), landed)
    }

    fn increment_selection(
        text: &str,
        selection: Selection,
        mode: VisualModeKind,
        delta: i128,
        kind: SelectionIncrement,
    ) -> String {
        let mut buffer = TextBuffer::from_text(text);
        buffer.increment_numbers_in_selection(selection, mode, delta, kind);
        buffer.to_string()
    }

    #[test]
    fn cursor_increments_follow_vim_number_formats() {
        for (text, col, delta, expected, landed) in [
            ("x = 41;", 0, 1, "x = 42;", 5),
            ("x = 41;", 5, 1, "x = 42;", 5),
            ("x = 9", 4, 1, "x = 10", 5),
            ("x = -1", 0, 1, "x = 0", 4),
            ("x = 0", 0, -3, "x = -3", 5),
            ("a-1", 0, 1, "a-2", 2),
            ("007", 0, 1, "008", 2),
            ("0099", 0, 1, "0100", 3),
            ("0x0f", 0, 1, "0x10", 3),
            ("0xFE", 2, 1, "0xFF", 3),
            ("0x0", 0, -1, "0xffffffffffffffff", 17),
            ("0b0111", 0, 1, "0b1000", 5),
            ("word1", 0, 1, "word2", 4),
            ("1.5", 2, 1, "1.6", 2),
        ] {
            assert_eq!(
                increment_at(text, Pos::new(0, col), delta),
                (expected.to_string(), Some(Pos::new(0, landed))),
                "{text} at {col}"
            );
        }
        assert_eq!(increment_at("x = 1;", Pos::new(0, 5), 1).1, None);
        assert_eq!(increment_at("no digits", Pos::zero(), 1).1, None);
    }

    #[test]
    fn selections_turn_equal_columns_into_sequences() {
        let lines = Selection::new(Pos::zero(), Pos::new(5, 0));
        let smart = SelectionIncrement::Smart;
        assert_eq!(
            increment_selection("1\n1\n1\n1\n1\n1\n", lines, VisualModeKind::Line, 1, smart),
            "1\n2\n3\n4\n5\n6\n"
        );
        assert_eq!(
            increment_selection("1\n2\n3\n", lines, VisualModeKind::Line, 1, smart),
            "2\n3\n4\n"
        );
        assert_eq!(
            increment_selection("5\n5\n5\n", lines, VisualModeKind::Line, -2, smart),
            "5\n3\n1\n"
        );
        assert_eq!(
            increment_selection(
                "a[0] = 0;\na[0] = 0;\na[0] = 0;\n",
                lines,
                VisualModeKind::Line,
                1,
                smart
            ),
            "a[0] = 0;\na[1] = 1;\na[2] = 2;\n"
        );
        assert_eq!(
            increment_selection(
                "let xs = [0, 0, 0, 0];",
                Selection::new(Pos::new(0, 10), Pos::new(0, 19)),
                VisualModeKind::Char,
                1,
                smart
            ),
            "let xs = [0, 1, 2, 3];"
        );
        // Identifiers and fractional parts are not counted.
        assert_eq!(
            increment_selection(
                "v1 = 1.5\nv1 = 1.5\n",
                lines,
                VisualModeKind::Line,
                1,
                smart
            ),
            "v1 = 1.5\nv1 = 2.5\n"
        );
        assert_eq!(
            increment_selection(
                "1 1\n1 1\n",
                Selection::new(Pos::new(0, 2), Pos::new(1, 2)),
                VisualModeKind::Block,
                1,
                smart
            ),
            "1 1\n1 2\n"
        );
        assert_eq!(
            increment_selection("7\n", lines, VisualModeKind::Line, 1, smart),
            "8\n"
        );
    }

    #[test]
    fn progressive_selection_matches_vim_g_ctrl_a() {
        let lines = Selection::new(Pos::zero(), Pos::new(3, 0));
        assert_eq!(
            increment_selection(
                "0\n0\n0\n",
                lines,
                VisualModeKind::Line,
                2,
                SelectionIncrement::Progressive
            ),
            "2\n4\n6\n"
        );
    }

    #[test]
    fn selection_increments_keep_marks_attached() {
        let mut buffer = TextBuffer::from_text("9\n9 end\n");
        buffer.set_mark('a', Pos::new(1, 2));
        buffer.increment_numbers_in_selection(
            Selection::new(Pos::zero(), Pos::new(1, 0)),
            VisualModeKind::Line,
            1,
            SelectionIncrement::Smart,
        );
        assert_eq!(buffer.to_string(), "9\n10 end\n");
        assert_eq!(buffer.mark('a'), Some(Pos::new(1, 3)));
    }
}
