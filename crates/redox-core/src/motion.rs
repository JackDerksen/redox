//! High-level editor navigation logic (motions).
//!
//! Motions are deterministic, side-effect-free cursor transformations over a
//! [`TextBuffer`]. They use logical [`Pos`] values: line and column are both
//! zero-based, and columns are character offsets, not visual cells.
//!
//! Frontends should apply these motions first, then project the result into
//! viewport, scrolling, and terminal-cell coordinates.

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

    /// Go to the first non-whitespace character on the line (`_`-ish).
    LineFirstNonWhitespace,

    /// Go to end of line (`$`-ish), i.e. `line_len_chars(line)`.
    LineEnd,

    /// Move to start of previous word (`b`-ish).
    WordStartBefore,

    /// Move to start of next word (`w`-ish).
    WordStartAfter,

    /// Move to end of next word (`e`-ish).
    WordEndAfter,

    /// Move onto the next matching character on the current line (`f`-ish).
    FindChar(char),

    /// Move just before the next matching character on the current line (`t`-ish).
    TillChar(char),

    /// Move onto the previous matching character on the current line (`F`-ish).
    FindCharBefore(char),

    /// Move just after the previous matching character on the current line (`T`-ish).
    TillCharBefore(char),

    /// Jump to the delimiter paired with the delimiter under the cursor (`%`-ish).
    MatchDelimiter,
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

        Motion::LineFirstNonWhitespace => {
            let line = buffer.clamp_line(cursor.line);
            Pos::new(line, buffer.line_first_non_whitespace_col(line))
        }

        Motion::LineEnd => {
            let line = buffer.clamp_line(cursor.line);
            let end_col = buffer.line_len_chars(line);
            Pos::new(line, end_col)
        }

        Motion::WordStartBefore => buffer.word_start_before(cursor),

        Motion::WordStartAfter => buffer.word_start_after(cursor),

        Motion::WordEndAfter => buffer.word_end_after(cursor),

        Motion::FindChar(needle) => buffer
            .find_char_after_on_line(cursor, needle)
            .unwrap_or(cursor),

        Motion::TillChar(needle) => buffer
            .find_char_after_on_line(cursor, needle)
            .map(|target| {
                if target.col > 0 {
                    Pos::new(target.line, target.col - 1)
                } else {
                    target
                }
            })
            .unwrap_or(cursor),

        Motion::FindCharBefore(needle) => buffer
            .find_char_before_on_line(cursor, needle)
            .unwrap_or(cursor),

        Motion::TillCharBefore(needle) => buffer
            .find_char_before_on_line(cursor, needle)
            .map(|target| Pos::new(target.line, target.col.saturating_add(1)))
            .unwrap_or(cursor),

        Motion::MatchDelimiter => buffer.matching_delimiter(cursor).unwrap_or(cursor),
    }
}

/// Apply a motion using operator semantics for the resulting half-open range end.
pub fn apply_motion_for_operator(
    buffer: &TextBuffer,
    cursor: Pos,
    motion: Motion,
    count: usize,
) -> Pos {
    match motion {
        Motion::FindChar(needle) => {
            let mut current = buffer.clamp_pos(cursor);
            let mut target = None;
            for _ in 0..count.max(1) {
                let Some(found) = buffer.find_char_after_on_line(current, needle) else {
                    return cursor;
                };
                target = Some(found);
                current = found;
            }
            target
                .map(|found| buffer.move_right(found))
                .unwrap_or(cursor)
        }
        Motion::TillChar(needle) => {
            let mut current = buffer.clamp_pos(cursor);
            let mut target = None;
            for _ in 0..count.max(1) {
                let Some(found) = buffer.find_char_after_on_line(current, needle) else {
                    return cursor;
                };
                target = Some(found);
                current = found;
            }
            target.unwrap_or(cursor)
        }
        Motion::FindCharBefore(needle) => {
            let mut current = buffer.clamp_pos(cursor);
            let mut target = None;
            for _ in 0..count.max(1) {
                let Some(found) = buffer.find_char_before_on_line(current, needle) else {
                    return cursor;
                };
                target = Some(found);
                current = found;
            }
            target.unwrap_or(cursor)
        }
        Motion::TillCharBefore(needle) => {
            let mut current = buffer.clamp_pos(cursor);
            let mut target = None;
            for _ in 0..count.max(1) {
                let Some(found) = buffer.find_char_before_on_line(current, needle) else {
                    return cursor;
                };
                let after_found = Pos::new(found.line, found.col.saturating_add(1));
                target = Some(after_found);
                current = found;
            }
            target.unwrap_or(cursor)
        }
        Motion::MatchDelimiter => {
            let target = apply_motion(buffer, cursor, motion);
            if target > cursor {
                buffer.move_right(target)
            } else {
                target
            }
        }
        _ => apply_motion_n(buffer, cursor, motion, count),
    }
}

/// Apply a motion with a Vim-style numeric count.
///
/// - If `count == 0`, this returns `cursor` unchanged.
/// - Left/right motions use direct char-index arithmetic.
/// - Other repeated motions are applied step by step so they can stop naturally
///   at document or line boundaries.
pub fn apply_motion_n(buffer: &TextBuffer, cursor: Pos, motion: Motion, count: usize) -> Pos {
    if count == 0 {
        return buffer.clamp_pos(cursor);
    }

    match motion {
        Motion::Left => {
            let at = buffer.pos_to_char(cursor);
            return buffer.char_to_pos(at.saturating_sub(count));
        }
        Motion::Right => {
            let at = buffer.pos_to_char(cursor);
            return buffer.char_to_pos(at.saturating_add(count).min(buffer.len_chars()));
        }
        _ => {}
    }

    if motion == Motion::MatchDelimiter {
        return apply_motion(buffer, cursor, motion);
    }

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
