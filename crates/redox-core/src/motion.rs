//! High-level editor navigation logic (motions).
//!
//! This module is intentionally **UI-agnostic** and depends only on core editor
//! types like [`TextBuffer`] and [`Pos`]. It provides a stable API to build
//! Vim-like behavior on top of (e.g. `w`, `gg`, `G`, `0`, `$`, etc.).
//!
//! Design goals:
//! - Keep motions deterministic and side-effect-free.
//! - Keep indexing consistent with `redox-core`: `Pos { line, col }` where `col`
//!   is in **char units** (Ropey model).
//! - Centralize motion semantics here so frontends (TUI/GUI) only project the
//!   resulting document cursor into their own viewport/cell coordinate systems.
//!
//! Notes:
//! - Word motions here currently use `TextBuffer`'s existing word helpers
//!   (`word_start_before`, `word_end_after`), which in turn use `buffer::util::is_word_char`.
//! - This module keeps motion semantics centralized so frontends remain thin.
//!
//! This file defines:
//! - [`Motion`] enum: the set of supported navigation intents.
//! - [`apply_motion`] / [`apply_motion_n`]: apply motions to a cursor position.

use crate::{Pos, TextBuffer};

/// A navigation intent (motion) that transforms a document cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    /// Move left by one char.
    Left,
    /// Move right by one char.
    Right,
    /// Move up by one line.
    Up,
    /// Move down by one line.
    Down,

    /// Go to first line of file (`gg`).
    FileStart,

    /// Go to last line of file (`G`). Column is clamped to that line.
    FileEnd,

    /// Go to start of line (`0`-ish).
    LineStart,

    /// Go to end of line (`$`-ish), i.e. `line_len_chars(line)`.
    LineEnd,

    /// Move to start of previous word (`b`-ish).
    WordStartBefore,

    /// Move to start of next word (`w`-ish).
    WordStartAfter,

    /// Move to end of next word (`e`-ish).
    WordEndAfter,
}

/// Apply a single `Motion` to a given cursor position.
///
/// This function is pure: it never mutates the buffer, and always returns a
/// position clamped to valid buffer bounds.
#[inline]
pub fn apply_motion(buffer: &TextBuffer, cursor: Pos, motion: Motion) -> Pos {
    let cursor = buffer.clamp_pos(cursor);

    match motion {
        Motion::Left => buffer.move_left(cursor),
        Motion::Right => buffer.move_right(cursor),
        Motion::Up => buffer.move_up(cursor),
        Motion::Down => buffer.move_down(cursor),

        Motion::FileStart => {
            let target_col = if cursor.line == 0 { 0 } else { cursor.col };
            buffer.clamp_pos(Pos::new(0, target_col))
        }

        Motion::FileEnd => {
            let last = buffer.len_lines().saturating_sub(1);
            buffer.clamp_pos(Pos::new(last, cursor.col))
        }

        Motion::LineStart => Pos::new(cursor.line, 0),

        Motion::LineEnd => {
            let line = buffer.clamp_line(cursor.line);
            let end_col = buffer.line_len_chars(line);
            Pos::new(line, end_col)
        }

        Motion::WordStartBefore => buffer.word_start_before(cursor),

        Motion::WordStartAfter => buffer.word_start_after(cursor),

        Motion::WordEndAfter => buffer.word_end_after(cursor),
    }
}

/// Apply a motion `count` times (Vim-style numeric prefix).
///
/// - If `count == 0`, this returns `cursor` unchanged.
/// - Motions are applied iteratively so they can clamp naturally at boundaries.
pub fn apply_motion_n(buffer: &TextBuffer, cursor: Pos, motion: Motion, count: usize) -> Pos {
    let mut cur = buffer.clamp_pos(cursor);
    for _ in 0..count {
        let next = apply_motion(buffer, cur, motion);
        // If the motion stops making progress (EOF/top/etc.), stop early.
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

/// Convenience helpers for motions that take a count.
pub mod helpers {
    use super::{Motion, apply_motion_n};
    use crate::{Pos, TextBuffer};

    /// Move forward by words (`w`-ish) by applying `WordStartAfter` repeatedly.
    #[inline]
    pub fn word_forward(buffer: &TextBuffer, cursor: Pos, count: usize) -> Pos {
        apply_motion_n(buffer, cursor, Motion::WordStartAfter, count)
    }

    /// Move backward by words (`b`-ish) by applying `WordStartBefore` repeatedly.
    #[inline]
    pub fn word_backward(buffer: &TextBuffer, cursor: Pos, count: usize) -> Pos {
        apply_motion_n(buffer, cursor, Motion::WordStartBefore, count)
    }

    /// Move to the first line (`gg`).
    #[inline]
    pub fn gg(buffer: &TextBuffer, cursor: Pos) -> Pos {
        super::apply_motion(buffer, cursor, Motion::FileStart)
    }

    /// Move to the last line (`G`).
    #[inline]
    pub fn file_end(buffer: &TextBuffer, cursor: Pos) -> Pos {
        super::apply_motion(buffer, cursor, Motion::FileEnd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_count_zero_is_noop() {
        let b = TextBuffer::from_str("abc\n");
        let p = Pos::new(0, 2);
        assert_eq!(apply_motion_n(&b, p, Motion::Left, 0), p);
        assert_eq!(apply_motion_n(&b, p, Motion::WordEndAfter, 0), p);
    }

    #[test]
    fn gg_goes_to_first_line_and_clamps_column() {
        let b = TextBuffer::from_str("a\nbb\nccc\n");
        let p = Pos::new(2, 2);
        let p2 = apply_motion(&b, p, Motion::FileStart);
        assert_eq!(p2.line, 0);
        // first line is "a" so col clamps to 1
        assert_eq!(p2.col, 1);
    }

    #[test]
    fn file_end_goes_to_last_line_and_clamps_column() {
        // NOTE: With a trailing '\n', Ropey reports an extra empty final line.
        // So "a\nbb\nccc\n" has 4 lines: "a", "bb", "ccc", "".
        let b = TextBuffer::from_str("a\nbb\nccc\n");
        let p = Pos::new(0, 10);
        let p2 = apply_motion(&b, p, Motion::FileEnd);

        // Last line is the empty line after the trailing newline.
        assert_eq!(p2.line, 3);
        assert_eq!(p2.col, 0);
    }

    #[test]
    fn line_start_and_line_end_work() {
        let b = TextBuffer::from_str("hello\nworld!\n");
        let p = Pos::new(1, 2);

        let start = apply_motion(&b, p, Motion::LineStart);
        assert_eq!(start, Pos::new(1, 0));

        let end = apply_motion(&b, p, Motion::LineEnd);
        assert_eq!(end, Pos::new(1, 6));
    }

    #[test]
    fn left_right_clamp_at_bounds() {
        // NOTE: With a trailing '\n', Ropey reports an extra empty final line.
        // Right at end-of-line can advance onto that empty line.
        let b = TextBuffer::from_str("ab\n");
        let p0 = Pos::new(0, 0);
        assert_eq!(apply_motion(&b, p0, Motion::Left), Pos::new(0, 0));

        // Moving right from end-of-line steps onto the newline, which maps to the next (empty) line at col 0.
        let p_end = Pos::new(0, 2);
        assert_eq!(apply_motion(&b, p_end, Motion::Right), Pos::new(1, 0));
    }

    #[test]
    fn up_down_preserve_column_when_possible() {
        let b = TextBuffer::from_str("aaaa\nb\ncccccc\n");
        let p = Pos::new(0, 3);

        let down = apply_motion(&b, p, Motion::Down);
        // line 1 is "b" so col clamps to 1
        assert_eq!(down, Pos::new(1, 1));

        let down2 = apply_motion(&b, down, Motion::Down);
        // line 2 is long enough, so col stays 1
        assert_eq!(down2, Pos::new(2, 1));

        let up = apply_motion(&b, down2, Motion::Up);
        assert_eq!(up, Pos::new(1, 1));
    }

    #[test]
    fn word_motions_ascii_smoke() {
        let b = TextBuffer::from_str("abc  def_12!\n");
        let p = Pos::new(0, 6); // in "def_12"

        let start = apply_motion(&b, p, Motion::WordStartBefore);
        assert_eq!(start, Pos::new(0, 5));

        let end = apply_motion(&b, start, Motion::WordEndAfter);
        assert_eq!(end, Pos::new(0, 10));
    }

    #[test]
    fn repeated_word_forward_stops_at_eof() {
        let b = TextBuffer::from_str("a b c\n");
        let p = Pos::new(0, 0);
        let p2 = apply_motion_n(&b, p, Motion::WordEndAfter, 100);

        // Vim's 'e' motion stops at the last character of the last word.
        assert_eq!(p2, Pos::new(0, 5));
    }

    #[test]
    fn word_start_after_visits_symbol_tokens() {
        let b = TextBuffer::from_str("(normal/insert/command)\n");
        let mut p = Pos::new(0, 0);

        p = apply_motion(&b, p, Motion::WordStartAfter);
        assert_eq!(p, Pos::new(0, 1)); // normal

        p = apply_motion(&b, p, Motion::WordStartAfter);
        assert_eq!(p, Pos::new(0, 7)); // /

        p = apply_motion(&b, p, Motion::WordStartAfter);
        assert_eq!(p, Pos::new(0, 8)); // insert

        p = apply_motion(&b, p, Motion::WordStartAfter);
        assert_eq!(p, Pos::new(0, 14)); // /

        p = apply_motion(&b, p, Motion::WordStartAfter);
        assert_eq!(p, Pos::new(0, 15)); // command

        p = apply_motion(&b, p, Motion::WordStartAfter);
        assert_eq!(p, Pos::new(0, 22)); // )
    }

    #[test]
    fn word_start_before_stops_on_symbol_token() {
        let b = TextBuffer::from_str("(normal/insert)\n");
        let p = Pos::new(0, 15); // after ')'
        let p2 = apply_motion(&b, p, Motion::WordStartBefore);
        assert_eq!(p2, Pos::new(0, 14)); // )
    }

    #[test]
    fn word_end_after_can_land_on_symbol_token() {
        let b = TextBuffer::from_str("alpha / beta\n");

        let p = Pos::new(0, 0);
        let p = apply_motion(&b, p, Motion::WordEndAfter);
        assert_eq!(p, Pos::new(0, 4)); // alpha

        let p = apply_motion(&b, p, Motion::WordEndAfter);
        assert_eq!(p, Pos::new(0, 6)); // /
    }
}
