//! Position and selection types for the rope-backed buffer.
//!
//! These are **logical** positions: (line, column), both 0-based, where column is
//! a character offset within the line (not a visual column).

/// A stable identifier for positions within a buffer.
///
/// This is a *logical* position: line + column (both 0-based).
/// It is not a byte offset.
///
/// Column is a char offset within the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    #[inline]
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    #[inline]
    pub const fn zero() -> Self {
        Self { line: 0, col: 0 }
    }
}

/// A selection expressed as an anchor + active cursor.
///
/// If `anchor == cursor`, selection is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Selection {
    pub anchor: Pos,
    pub cursor: Pos,
}

impl Selection {
    #[inline]
    pub const fn new(anchor: Pos, cursor: Pos) -> Self {
        Self { anchor, cursor }
    }

    #[inline]
    pub const fn empty(at: Pos) -> Self {
        Self {
            anchor: at,
            cursor: at,
        }
    }

    /// Returns the ordered range (start <= end).
    #[inline]
    pub fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}
