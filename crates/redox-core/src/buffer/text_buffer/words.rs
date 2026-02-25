//! Word-motion helpers for `TextBuffer`.
//!
//! Vim-like lowercase word motions use three character classes:
//! - keyword chars (`[A-Za-z0-9_]` for now)
//! - whitespace
//! - symbols (any non-whitespace, non-keyword char)
//!
//! This setup lets `w`/`b`/`e` visit punctuation such as `(`, `)`, `/`, `-`, etc.

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
fn classify(ch: char) -> CharClass {
    if ch.is_whitespace() {
        CharClass::Whitespace
    } else if is_word_char(ch) {
        CharClass::Keyword
    } else {
        CharClass::Symbol
    }
}

impl TextBuffer {
    /// Find the start of the previous/current word-like run (`b`-style).
    pub fn word_start_before(&self, pos: Pos) -> Pos {
        let mut c = self.pos_to_char(pos);
        if c == 0 {
            return Pos::zero();
        }

        while c > 0 && classify(self.rope.char(c - 1)) == CharClass::Whitespace {
            c -= 1;
        }

        if c == 0 {
            return Pos::zero();
        }

        let target = classify(self.rope.char(c - 1));
        while c > 0 && classify(self.rope.char(c - 1)) == target {
            c -= 1;
        }

        self.char_to_pos(c)
    }

    /// Find the end of the current/next word-like run (`e`-style).
    pub fn word_end_after(&self, pos: Pos) -> Pos {
        let mut c = self.pos_to_char(pos);
        let maxc = self.len_chars();
        if c >= maxc {
            return self.char_to_pos(c);
        }

        let mut class = classify(self.rope.char(c));
        if class == CharClass::Whitespace {
            while c < maxc && classify(self.rope.char(c)) == CharClass::Whitespace {
                c += 1;
            }
            if c >= maxc {
                return self.char_to_pos(c.saturating_sub(1));
            }
            class = classify(self.rope.char(c));
        } else {
            let at_end = c + 1 >= maxc || classify(self.rope.char(c + 1)) != class;
            if at_end {
                c += 1;
                while c < maxc && classify(self.rope.char(c)) == CharClass::Whitespace {
                    c += 1;
                }
                if c >= maxc {
                    return self.char_to_pos(c.saturating_sub(1));
                }
                class = classify(self.rope.char(c));
            }
        }

        while c + 1 < maxc && classify(self.rope.char(c + 1)) == class {
            c += 1;
        }

        self.char_to_pos(c)
    }

    /// Find the start of the next word-like run (`w`-style).
    pub fn word_start_after(&self, pos: Pos) -> Pos {
        let mut c = self.pos_to_char(pos);
        let maxc = self.len_chars();
        if c >= maxc {
            return self.char_to_pos(c);
        }

        let class = classify(self.rope.char(c));
        if class != CharClass::Whitespace {
            while c < maxc && classify(self.rope.char(c)) == class {
                c += 1;
            }
        }

        while c < maxc && classify(self.rope.char(c)) == CharClass::Whitespace {
            c += 1;
        }

        self.char_to_pos(c)
    }
}
