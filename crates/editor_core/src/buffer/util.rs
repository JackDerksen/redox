//! Internal helper utilities for the Ropey-backed buffer.
//!
//! These helpers are kept in a dedicated module to keep higher-level buffer code
//! focused on the public API.

use super::{Pos, TextBuffer};

/// Returns whether a character is considered part of a "word".
///
/// NOTE: This is intentionally minimal and ASCII-focused for now.
/// I can make this configurable (Vim `'iskeyword'`-style) or Unicode-aware later.
#[inline]
pub(crate) fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Returns the smaller of two positions after clamping them into the buffer.
///
/// Clamping ensures comparisons behave sensibly even if callers pass out-of-range
/// positions.
#[inline]
pub(crate) fn min_pos(buf: &TextBuffer, a: Pos, b: Pos) -> Pos {
    let a = buf.clamp_pos(a);
    let b = buf.clamp_pos(b);
    if a <= b { a } else { b }
}

/// Returns the larger of two positions after clamping them into the buffer.
///
/// Clamping ensures comparisons behave sensibly even if callers pass out-of-range
/// positions.
#[inline]
pub(crate) fn max_pos(buf: &TextBuffer, a: Pos, b: Pos) -> Pos {
    let a = buf.clamp_pos(a);
    let b = buf.clamp_pos(b);
    if a >= b { a } else { b }
}
