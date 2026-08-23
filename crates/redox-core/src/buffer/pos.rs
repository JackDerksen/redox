//! Logical positions and selections.

/// Logical position within a buffer.
///
/// Both fields are zero-based; `col` counts Unicode scalar values.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
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

/// A selection expressed as a fixed anchor and active cursor.
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

    #[inline]
    pub fn line_range(&self) -> (usize, usize) {
        let (start, end) = self.ordered();
        (start.line, end.line)
    }
}
