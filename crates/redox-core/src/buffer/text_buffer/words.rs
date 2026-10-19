//! Word-motion helpers for `TextBuffer`.
//!
//! Vim-like lowercase word motions use three character classes:
//! - keyword chars (letters and digits in any script, and `_`)
//! - whitespace
//! - symbols (any non-whitespace, non-keyword char)
//!
//! This setup lets `w`/`b`/`e` visit punctuation such as `(`, `)`, `/`, `-`, etc.
//! Uppercase `W`/`B`/`E` treat each non-whitespace run as one WORD.

use super::super::util::is_word_char;
use super::TextBuffer;
use crate::buffer::Pos;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Keyword,
    Symbol,
}

#[inline]
fn classify(character: char, big_word: bool) -> CharClass {
    if character.is_whitespace() {
        CharClass::Whitespace
    } else if big_word || is_word_char(character) {
        CharClass::Keyword
    } else {
        CharClass::Symbol
    }
}

impl TextBuffer {
    /// Find the start of the previous/current whitespace-separated WORD (`B`).
    pub fn big_word_start_before(&self, pos: Pos) -> Pos {
        self.word_start_before_impl(pos, true)
    }

    /// Find the start of the next whitespace-separated WORD (`W`).
    pub fn big_word_start_after(&self, pos: Pos) -> Pos {
        self.word_start_after_impl(pos, true)
    }

    /// Find the end of the current/next whitespace-separated WORD (`E`).
    pub fn big_word_end_after(&self, pos: Pos) -> Pos {
        self.word_end_after_impl(pos, true)
    }

    /// Find the end of the previous whitespace-separated WORD (`gE`).
    pub fn big_word_end_before(&self, pos: Pos) -> Pos {
        self.word_end_before_impl(pos, true)
    }

    /// Find a WORD's end, including the character under the cursor (`cW`).
    pub fn big_word_end_at_or_after(&self, pos: Pos) -> Pos {
        self.word_end_at_or_after_impl(pos, true)
    }

    /// Find the start of the previous/current word-like run (`b`-style).
    pub fn word_start_before(&self, pos: Pos) -> Pos {
        self.word_start_before_impl(pos, false)
    }

    fn word_start_before_impl(&self, pos: Pos, big_word: bool) -> Pos {
        let mut character_index = self.pos_to_char(pos);
        if character_index == 0 {
            return Pos::zero();
        }

        while character_index > 0
            && classify(self.rope.char(character_index - 1), big_word) == CharClass::Whitespace
        {
            character_index -= 1;
        }

        if character_index == 0 {
            return Pos::zero();
        }

        let target = classify(self.rope.char(character_index - 1), big_word);
        while character_index > 0
            && classify(self.rope.char(character_index - 1), big_word) == target
        {
            character_index -= 1;
        }

        self.char_to_pos(character_index)
    }

    /// Find the end of the current/next word-like run (`e`-style).
    pub fn word_end_after(&self, pos: Pos) -> Pos {
        self.word_end_after_impl(pos, false)
    }

    /// Find the end of the previous word-like run (`ge`).
    pub fn word_end_before(&self, pos: Pos) -> Pos {
        self.word_end_before_impl(pos, false)
    }

    fn word_end_before_impl(&self, pos: Pos, big_word: bool) -> Pos {
        let mut character_index = self.pos_to_char(pos);
        if character_index < self.len_chars() {
            let class = classify(self.rope.char(character_index), big_word);
            while character_index > 0
                && class != CharClass::Whitespace
                && classify(self.rope.char(character_index - 1), big_word) == class
            {
                character_index -= 1;
            }
        }
        while character_index > 0
            && classify(self.rope.char(character_index - 1), big_word) == CharClass::Whitespace
        {
            character_index -= 1;
        }
        self.char_to_pos(character_index.saturating_sub(1))
    }

    fn word_end_after_impl(&self, pos: Pos, big_word: bool) -> Pos {
        let character_index = self.pos_to_char(pos);
        let character_count = self.len_chars();
        if character_index >= character_count {
            return self.char_to_pos(character_index);
        }

        let next_index = (character_index + 1).min(character_count - 1);
        self.word_end_at_or_after_impl(self.char_to_pos(next_index), big_word)
    }

    /// Find a word's end, including the character under the cursor (`cw`-style).
    pub fn word_end_at_or_after(&self, pos: Pos) -> Pos {
        self.word_end_at_or_after_impl(pos, false)
    }

    fn word_end_at_or_after_impl(&self, pos: Pos, big_word: bool) -> Pos {
        let mut character_index = self.pos_to_char(pos);
        let character_count = self.len_chars();
        if character_index >= character_count {
            return self.char_to_pos(character_index);
        }

        while character_index < character_count
            && classify(self.rope.char(character_index), big_word) == CharClass::Whitespace
        {
            character_index += 1;
        }
        if character_index >= character_count {
            return self.char_to_pos(character_count - 1);
        }

        let class = classify(self.rope.char(character_index), big_word);
        while character_index + 1 < character_count
            && classify(self.rope.char(character_index + 1), big_word) == class
        {
            character_index += 1;
        }

        self.char_to_pos(character_index)
    }

    /// Find the start of the next word-like run (`w`-style).
    pub fn word_start_after(&self, pos: Pos) -> Pos {
        self.word_start_after_impl(pos, false)
    }

    fn word_start_after_impl(&self, pos: Pos, big_word: bool) -> Pos {
        let mut character_index = self.pos_to_char(pos);
        let character_count = self.len_chars();
        if character_index >= character_count {
            return self.char_to_pos(character_index);
        }

        let class = classify(self.rope.char(character_index), big_word);
        if class != CharClass::Whitespace {
            while character_index < character_count
                && classify(self.rope.char(character_index), big_word) == class
            {
                character_index += 1;
            }
        }

        while character_index < character_count
            && classify(self.rope.char(character_index), big_word) == CharClass::Whitespace
        {
            character_index += 1;
        }

        self.char_to_pos(character_index)
    }
}
