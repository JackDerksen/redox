//! Word-motion helpers for `TextBuffer`.
//!
//! Current behavior
//! - “Word characters” are defined by `buffer::util::is_word_char`.
//!   Right now that is ASCII-ish (`[A-Za-z0-9_]`), but it’s centralized so I
//!   can later swap it for Vim-like `'iskeyword'` rules, Unicode word
//!   segmentation, identifier rules, etc.
//! - Motions operate on **char indices** via Ropey.

use super::super::util::is_word_char;
use super::TextBuffer;
use crate::buffer::Pos;

impl TextBuffer {
    /// Find the start of the “word” before `pos`.
    ///
    /// Word characters are defined by `is_word_char`.
    ///
    /// Rough semantics:
    /// - If immediately left of `pos` is a delimiter, skip delimiters left.
    /// - Then skip word characters left.
    /// - Return the resulting position.
    ///
    /// This is meant to map cleanly to editor motions like “b”.
    pub fn word_start_before(&self, pos: Pos) -> Pos {
        let mut c = self.pos_to_char(pos);
        if c == 0 {
            return Pos::zero();
        }

        // If we're at a delimiter, first skip delimiters left.
        while c > 0 {
            let ch = self.rope.char(c - 1);
            if is_word_char(ch) {
                break;
            }
            c -= 1;
        }

        // ...then skip word chars left.
        while c > 0 {
            let ch = self.rope.char(c - 1);
            if !is_word_char(ch) {
                break;
            }
            c -= 1;
        }

        self.char_to_pos(c)
    }

    /// Find the end of the “word” after `pos`.
    ///
    /// Word characters are defined by `is_word_char`.
    ///
    /// Rough semantics:
    /// - From `pos`, skip delimiters right until a word character or EOF.
    /// - Then skip word characters right.
    /// - Return the resulting position.
    ///
    /// This is meant to map cleanly to editor motions like “w/e” depending on how
    /// I apply it.
    pub fn word_end_after(&self, pos: Pos) -> Pos {
        let mut c = self.pos_to_char(pos);
        let maxc = self.len_chars();

        // Skip delimiters right.
        while c < maxc {
            let ch = self.rope.char(c);
            if is_word_char(ch) {
                break;
            }
            c += 1;
        }

        // Skip word chars right.
        while c < maxc {
            let ch = self.rope.char(c);
            if !is_word_char(ch) {
                break;
            }
            c += 1;
        }

        self.char_to_pos(c)
    }
}
