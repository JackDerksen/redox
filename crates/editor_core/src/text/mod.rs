//! Shared text position types and helpers used by the buffer.
//!
//! This module is intentionally "rope-agnostic" (it operates on indices and
//! provides conversions given line-start information), so it can be reused
//! whether the backing store is a Rope, a gap buffer, etc.
//!
//! When used with `ropey::Rope`, you typically pair these helpers with:
//! - `Rope::len_chars()`
//! - `Rope::line_to_char(line)`
//! - `Rope::char_to_line(char_idx)`
//! - `Rope::line(line).len_chars()` (includes newline if present)

use core::cmp::{max, min};
use core::fmt;

/// A 0-based character index (Unicode scalar value index).
///
/// In ropey, most cursor-safe indexing is done in **char indices** (not bytes).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CharIdx(pub usize);

impl CharIdx {
    #[inline]
    pub const fn new(v: usize) -> Self {
        Self(v)
    }

    #[inline]
    pub const fn get(self) -> usize {
        self.0
    }

    #[inline]
    pub const fn saturating_add(self, delta: usize) -> Self {
        Self(self.0.saturating_add(delta))
    }

    #[inline]
    pub const fn saturating_sub(self, delta: usize) -> Self {
        Self(self.0.saturating_sub(delta))
    }
}

impl fmt::Debug for CharIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CharIdx").field(&self.0).finish()
    }
}

/// A 0-based line index.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LineIdx(pub usize);

impl LineIdx {
    #[inline]
    pub const fn new(v: usize) -> Self {
        Self(v)
    }

    #[inline]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl fmt::Debug for LineIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LineIdx").field(&self.0).finish()
    }
}

/// A 0-based column index in **characters** within a line.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ColIdx(pub usize);

impl ColIdx {
    #[inline]
    pub const fn new(v: usize) -> Self {
        Self(v)
    }

    #[inline]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl fmt::Debug for ColIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ColIdx").field(&self.0).finish()
    }
}

/// A (line, column) location in the buffer.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LineCol {
    pub line: LineIdx,
    pub col: ColIdx,
}

impl LineCol {
    #[inline]
    pub const fn new(line: usize, col: usize) -> Self {
        Self {
            line: LineIdx(line),
            col: ColIdx(col),
        }
    }
}

impl fmt::Debug for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LineCol")
            .field("line", &self.line.0)
            .field("col", &self.col.0)
            .finish()
    }
}

/// A half-open character range: `[start, end)`.
///
/// Invariant expected by users:
/// - `start <= end`
///
/// When working with ropey, indices are in **chars** (not bytes).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CharRange {
    pub start: CharIdx,
    pub end: CharIdx,
}

impl CharRange {
    #[inline]
    pub const fn new(start: CharIdx, end: CharIdx) -> Self {
        Self { start, end }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.start.0 >= self.end.0
    }

    #[inline]
    pub const fn len(self) -> usize {
        self.end.0.saturating_sub(self.start.0)
    }

    /// Normalizes so that `start <= end`.
    #[inline]
    pub const fn normalized(self) -> Self {
        if self.start.0 <= self.end.0 {
            self
        } else {
            Self {
                start: self.end,
                end: self.start,
            }
        }
    }

    /// Clamp the range to `[0, max]`.
    #[inline]
    pub fn clamp_to_len(self, max_len: usize) -> Self {
        let s = min(self.start.0, max_len);
        let e = min(self.end.0, max_len);
        Self {
            start: CharIdx(s),
            end: CharIdx(e),
        }
        .normalized()
    }
}

impl fmt::Debug for CharRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CharRange")
            .field("start", &self.start.0)
            .field("end", &self.end.0)
            .finish()
    }
}

/// A small helper for "preferred column" behavior (vim-like vertical motion).
///
/// When you move up/down, you usually want to keep the *goal* column even if a
/// particular line is shorter. Store the goal separately from the actual column.
///
/// Typical usage:
/// - Update `goal_col` when moving left/right or after inserting text.
/// - When moving up/down, clamp to the target line length, but keep `goal_col`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct GoalCol {
    pub goal_col: ColIdx,
}

impl GoalCol {
    #[inline]
    pub const fn new(goal_col: usize) -> Self {
        Self {
            goal_col: ColIdx(goal_col),
        }
    }
}

/// Computes a `(line, col)` for a given `char_idx` using a provided `line_to_char`
/// function.
///
/// - `line_to_char(line)` must return the char index of the start of `line`.
/// - `line_count` is the number of lines in the document.
/// - The result is clamped to valid bounds.
///
/// This is rope-agnostic: you can pass ropey’s `Rope::line_to_char` directly.
pub fn char_to_line_col(
    char_idx: CharIdx,
    line_count: usize,
    mut line_to_char: impl FnMut(usize) -> usize,
) -> LineCol {
    if line_count == 0 {
        return LineCol::new(0, 0);
    }

    // Binary search the greatest line whose start <= char_idx
    let target = char_idx.0;
    let mut lo = 0usize;
    let mut hi = line_count - 1;

    while lo < hi {
        // bias upwards to avoid infinite loop
        let mid = (lo + hi + 1) / 2;
        let mid_start = line_to_char(mid);
        if mid_start <= target {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    let line = lo;
    let line_start = line_to_char(line);
    let col = target.saturating_sub(line_start);

    LineCol {
        line: LineIdx(line),
        col: ColIdx(col),
    }
}

/// Computes a char index for a given `(line, col)` using a provided `line_to_char`
/// function and a `line_len_chars` function.
///
/// - `line_to_char(line)` must return the char index of the start of `line`.
/// - `line_len_chars(line)` must return the length of that line in chars.
///
/// This clamps:
/// - `line` to `[0, line_count-1]` (when `line_count > 0`)
/// - `col` to `[0, line_len]`
pub fn line_col_to_char(
    pos: LineCol,
    line_count: usize,
    mut line_to_char: impl FnMut(usize) -> usize,
    mut line_len_chars: impl FnMut(usize) -> usize,
) -> CharIdx {
    if line_count == 0 {
        return CharIdx(0);
    }

    let line = min(pos.line.0, line_count - 1);
    let line_start = line_to_char(line);
    let line_len = line_len_chars(line);
    let col = min(pos.col.0, line_len);

    CharIdx(line_start.saturating_add(col))
}

/// Clamps a char index into `[0, len_chars]`.
#[inline]
pub fn clamp_char(char_idx: CharIdx, len_chars: usize) -> CharIdx {
    CharIdx(min(char_idx.0, len_chars))
}

/// Normalizes and clamps a range into `[0, len_chars]`.
#[inline]
pub fn clamp_range(range: CharRange, len_chars: usize) -> CharRange {
    range.normalized().clamp_to_len(len_chars)
}

/// Given a line length in chars, clamp a goal column to the line.
#[inline]
pub fn clamp_col_to_line(goal: ColIdx, line_len_chars: usize) -> ColIdx {
    ColIdx(min(goal.0, line_len_chars))
}

/// Computes a safe "visual" line length in chars, excluding a trailing `\n`
/// if present (common for ropey lines).
///
/// Many editors treat the newline as not part of the line's editable columns.
/// If you want newline-inclusive semantics, don't use this helper.
#[inline]
pub fn line_len_without_newline(
    line_len_chars_including_newline: usize,
    ends_with_newline: bool,
) -> usize {
    if ends_with_newline {
        line_len_chars_including_newline.saturating_sub(1)
    } else {
        line_len_chars_including_newline
    }
}

/// Compute the next/prev character index with clamping.
///
/// This is useful for cursor movement where you never want to go out of bounds.
#[inline]
pub fn move_char_clamped(current: CharIdx, delta: isize, len_chars: usize) -> CharIdx {
    if delta == 0 {
        return clamp_char(current, len_chars);
    }

    if delta > 0 {
        let d = delta as usize;
        CharIdx(min(current.0.saturating_add(d), len_chars))
    } else {
        let d = (-delta) as usize;
        CharIdx(current.0.saturating_sub(d))
    }
}

/// Returns `(min, max)` ordering of two char indices.
#[inline]
pub fn ordered_pair(a: CharIdx, b: CharIdx) -> (CharIdx, CharIdx) {
    if a.0 <= b.0 { (a, b) } else { (b, a) }
}

/// Updates `(actual_col, goal_col)` when moving vertically.
///
/// Pass:
/// - `goal_col`: previously remembered preferred column
/// - `target_line_len`: length of target line in chars (already excluding newline if desired)
///
/// Returns the actual column to place the cursor at (clamped), while keeping the
/// goal column unchanged.
#[inline]
pub fn apply_goal_col(goal_col: ColIdx, target_line_len: usize) -> ColIdx {
    clamp_col_to_line(goal_col, target_line_len)
}

/// Computes the common "cursor line start" and "cursor line end" bounds.
///
/// Inputs are char indices:
/// - `line_start`: start of the line containing the cursor
/// - `line_len_chars`: length of that line in chars (including newline if present)
///
/// Outputs are clamped half-open bounds of the line's editable area:
/// - `editable_start = line_start`
/// - `editable_end = line_start + line_len_without_newline(...)`
#[inline]
pub fn line_editable_bounds(
    line_start: CharIdx,
    line_len_chars_including_newline: usize,
    ends_with_newline: bool,
) -> (CharIdx, CharIdx) {
    let editable_len =
        line_len_without_newline(line_len_chars_including_newline, ends_with_newline);
    let start = line_start;
    let end = CharIdx(line_start.0.saturating_add(editable_len));
    (start, end)
}

/// Clamp a cursor char index into the editable bounds of its line.
///
/// If `cursor` lies past the editable end (e.g. on newline), it will be clamped back.
#[inline]
pub fn clamp_cursor_to_line_editable(
    cursor: CharIdx,
    line_start: CharIdx,
    editable_end: CharIdx,
) -> CharIdx {
    CharIdx(max(line_start.0, min(cursor.0, editable_end.0)))
}
