//! Word-motion helpers for `TextBuffer`.
//!
//! Current behavior
//! - "Word characters" are defined by `buffer::util::is_word_char`.
//!   Right now that is ASCII-ish (`[A-Za-z0-9_]`), but it’s centralized so I
//!   can later swap it for Vim-like `'iskeyword'` rules, Unicode word
//!   segmentation, identifier rules, etc.
//! - Motions operate on **char indices** via Ropey.

use super::super::util::is_word_char;
use super::TextBuffer;
use crate::buffer::Pos;

impl TextBuffer {
    /// Find the start of the "word" before `pos`.
    ///
    /// Word characters are defined by `is_word_char`.
    ///
    /// Rough semantics:
    /// - If immediately left of `pos` is a delimiter, skip delimiters left.
    /// - Then skip word characters left.
    /// - Return the resulting position.
    ///
    /// This is meant to map cleanly to editor motions like "b".
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

    /// Find the end of the "word" after `pos`.
    ///
    /// Word characters are defined by `is_word_char`.
    ///
    /// Rough semantics:
    /// - From `pos`, skip delimiters right until a word character or EOF.
    /// - Then skip word characters right.
    /// - Return the resulting position.
    ///
    /// This is meant to map cleanly to editor motions like "w/e".
    pub fn word_end_after(&self, pos: Pos) -> Pos {
        let mut c = self.pos_to_char(pos);
        let maxc = self.len_chars();

        if c >= maxc {
            return self.char_to_pos(c);
        }

        // If we're on a word character, find the end of the current word.
        if is_word_char(self.rope.char(c)) {
            let mut end_of_current = c;
            while end_of_current < maxc {
                let ch = self.rope.char(end_of_current);
                if !is_word_char(ch) {
                    break;
                }
                end_of_current += 1;
            }

            // Check if we're already at the end of the word.
            if c == end_of_current - 1 {
                // At end of word, find next word's end.
                c = end_of_current;
            } else {
                // In the middle of a word, go to its end.
                return self.char_to_pos(end_of_current.saturating_sub(1));
            }
        }

        // Skip delimiters to find the next word.
        while c < maxc {
            let ch = self.rope.char(c);
            if is_word_char(ch) {
                break;
            }
            c += 1;
        }

        // Skip word chars to find the end of this word.
        while c < maxc {
            let ch = self.rope.char(c);
            if !is_word_char(ch) {
                break;
            }
            c += 1;
        }

        // Vim's 'e' motion lands on the last character of the word.
        self.char_to_pos(c.saturating_sub(1))
    }

    /// Find the start of the "word" after `pos`.
    ///
    /// Word characters are defined by `is_word_char`.
    ///
    /// Rough semantics:
    /// - From `pos`, skip word characters right.
    /// - Then skip delimiters right.
    /// - Return the resulting position.
    ///
    /// This is meant to map cleanly to editor motions like "w".
    pub fn word_start_after(&self, pos: Pos) -> Pos {
        let mut c = self.pos_to_char(pos);
        let maxc = self.len_chars();

        while c < maxc {
            let ch = self.rope.char(c);
            if !is_word_char(ch) {
                break;
            }
            c += 1;
        }

        while c < maxc {
            let ch = self.rope.char(c);
            if is_word_char(ch) {
                break;
            }
            c += 1;
        }

        self.char_to_pos(c)
    }
}
