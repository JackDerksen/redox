//! Literal search and structural delimiter matching.

use super::TextBuffer;
use crate::buffer::Pos;

impl TextBuffer {
    /// Find the next occurrence of `needle` after `pos` on the same line.
    pub fn find_char_after_on_line(&self, pos: Pos, needle: char) -> Option<Pos> {
        let pos = self.clamp_pos(pos);
        let line = self.clamp_line(pos.line);
        let line_text = self.line_slice(line);

        for (col, ch) in line_text
            .chars()
            .enumerate()
            .skip(pos.col.saturating_add(1))
        {
            if ch == needle {
                return Some(Pos::new(line, col));
            }
        }

        None
    }

    /// Find the previous occurrence of `needle` before `pos` on the same line.
    pub fn find_char_before_on_line(&self, pos: Pos, needle: char) -> Option<Pos> {
        let pos = self.clamp_pos(pos);
        let line = self.clamp_line(pos.line);
        let line_text = self.line_slice(line);
        let mut found = None;

        for (col, ch) in line_text.chars().enumerate().take(pos.col) {
            if ch == needle {
                found = Some(Pos::new(line, col));
            }
        }

        found
    }

    /// Find the delimiter paired with the delimiter under `pos`.
    pub fn matching_delimiter(&self, pos: Pos) -> Option<Pos> {
        let pos = self.clamp_pos(pos);
        let char_idx = self.pos_to_char(pos);
        let ch = self.char_at(pos)?;
        if self.char_is_escaped_for_pairing(char_idx) {
            return None;
        }

        match delimiter_pair_for(ch)? {
            DelimiterPairKind::Asymmetric { open, close } if ch == open => {
                self.match_asymmetric_delimiter_forward(char_idx, open, close)
            }
            DelimiterPairKind::Asymmetric { open, close } if ch == close => {
                self.match_asymmetric_delimiter_backward(char_idx, open, close)
            }
            DelimiterPairKind::Symmetric { delimiter } => {
                self.match_symmetric_delimiter(char_idx, delimiter)
            }
            DelimiterPairKind::Asymmetric { .. } => None,
        }
    }

    fn match_asymmetric_delimiter_forward(
        &self,
        start_idx: usize,
        open: char,
        close: char,
    ) -> Option<Pos> {
        let mut depth = 0usize;
        for idx in start_idx.saturating_add(1)..self.len_chars() {
            if self.char_is_escaped_for_pairing(idx) {
                continue;
            }
            match self.char_at_index(idx) {
                ch if ch == open => depth = depth.saturating_add(1),
                ch if ch == close => {
                    if depth == 0 {
                        return Some(self.char_to_pos(idx));
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
        None
    }

    fn match_asymmetric_delimiter_backward(
        &self,
        end_idx: usize,
        open: char,
        close: char,
    ) -> Option<Pos> {
        let mut depth = 0usize;
        for idx in (0..end_idx).rev() {
            if self.char_is_escaped_for_pairing(idx) {
                continue;
            }
            match self.char_at_index(idx) {
                ch if ch == close => depth = depth.saturating_add(1),
                ch if ch == open => {
                    if depth == 0 {
                        return Some(self.char_to_pos(idx));
                    }
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
        None
    }

    fn match_symmetric_delimiter(&self, char_idx: usize, delimiter: char) -> Option<Pos> {
        if self.char_is_escaped_for_pairing(char_idx) {
            return None;
        }

        let line = self.char_to_line(char_idx);
        let line_range = self.line_char_range(line);
        let mut delimiters = Vec::new();
        for idx in line_range {
            if self.char_at_index(idx) == delimiter && !self.char_is_escaped_for_pairing(idx) {
                delimiters.push(idx);
            }
        }

        delimiters.chunks_exact(2).find_map(|pair| {
            if char_idx == pair[0] {
                Some(self.char_to_pos(pair[1]))
            } else if char_idx == pair[1] {
                Some(self.char_to_pos(pair[0]))
            } else {
                None
            }
        })
    }

    fn char_is_escaped_for_pairing(&self, char_idx: usize) -> bool {
        let mut backslashes = 0;
        let mut idx = char_idx;
        while idx > 0 {
            idx -= 1;
            if self.char_at_index(idx) != '\\' {
                break;
            }
            backslashes += 1;
        }
        backslashes % 2 == 1
    }

    /// Find all non-overlapping literal matches of `needle` in the buffer.
    ///
    /// Returned ranges are half-open `(start, end)` position pairs.
    pub fn find_matches(&self, needle: &str) -> Vec<(Pos, Pos)> {
        if needle.is_empty() {
            return Vec::new();
        }

        let needle_chars = needle.chars().count();
        let overlap_char_limit = needle_chars.saturating_sub(1);
        let mut collector = MatchCollector {
            buffer: self,
            needle,
            needle_chars,
            matches: Vec::new(),
            last_emitted_end: 0,
        };
        let mut overlap = String::new();
        let mut processed_chars = 0usize;

        for chunk in self.chunks() {
            let chunk_chars = chunk.chars().count();

            if overlap.is_empty() {
                collector.collect_segment(chunk, processed_chars, processed_chars);

                if overlap_char_limit > 0 {
                    overlap = trailing_chars(chunk, overlap_char_limit);
                }
            } else {
                let overlap_chars = overlap.chars().count();
                let segment_start_char = processed_chars.saturating_sub(overlap_chars);
                let mut segment = String::with_capacity(overlap.len().saturating_add(chunk.len()));
                segment.push_str(&overlap);
                segment.push_str(chunk);

                collector.collect_segment(&segment, segment_start_char, processed_chars);

                overlap = trailing_chars(&segment, overlap_char_limit);
            }

            processed_chars = processed_chars.saturating_add(chunk_chars);
        }

        collector.matches
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterPairKind {
    Asymmetric { open: char, close: char },
    Symmetric { delimiter: char },
}

fn delimiter_pair_for(ch: char) -> Option<DelimiterPairKind> {
    match ch {
        '(' | ')' => Some(DelimiterPairKind::Asymmetric {
            open: '(',
            close: ')',
        }),
        '[' | ']' => Some(DelimiterPairKind::Asymmetric {
            open: '[',
            close: ']',
        }),
        '{' | '}' => Some(DelimiterPairKind::Asymmetric {
            open: '{',
            close: '}',
        }),
        '<' | '>' => Some(DelimiterPairKind::Asymmetric {
            open: '<',
            close: '>',
        }),
        '\'' | '"' | '`' => Some(DelimiterPairKind::Symmetric { delimiter: ch }),
        _ => None,
    }
}

struct MatchCollector<'a> {
    buffer: &'a TextBuffer,
    needle: &'a str,
    needle_chars: usize,
    matches: Vec<(Pos, Pos)>,
    last_emitted_end: usize,
}

impl MatchCollector<'_> {
    fn collect_segment(
        &mut self,
        segment: &str,
        segment_start_char: usize,
        emit_after_char: usize,
    ) {
        let segment_scan_start_char = self.last_emitted_end.saturating_sub(segment_start_char);
        let segment_scan_start_byte = byte_idx_for_char(segment, segment_scan_start_char);
        let mut scan_start_byte = segment_scan_start_byte;
        let mut scan_start_chars = segment_scan_start_char;

        for (match_start_byte_rel, _) in
            segment[segment_scan_start_byte..].match_indices(self.needle)
        {
            let match_start_byte = segment_scan_start_byte.saturating_add(match_start_byte_rel);
            scan_start_chars = scan_start_chars
                .saturating_add(segment[scan_start_byte..match_start_byte].chars().count());

            let start_char = segment_start_char.saturating_add(scan_start_chars);
            let end_char = start_char.saturating_add(self.needle_chars);
            if start_char >= self.last_emitted_end && end_char > emit_after_char {
                self.matches.push((
                    self.buffer.char_to_pos(start_char),
                    self.buffer.char_to_pos(end_char),
                ));
                self.last_emitted_end = end_char;
            }

            scan_start_byte = match_start_byte.saturating_add(self.needle.len());
            scan_start_chars = scan_start_chars.saturating_add(self.needle_chars);
        }
    }
}

fn byte_idx_for_char(text: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }

    text.char_indices()
        .nth(char_idx)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(text.len())
}

fn trailing_chars(text: &str, char_limit: usize) -> String {
    if char_limit == 0 || text.is_empty() {
        return String::new();
    }

    let total_chars = text.chars().count();
    if total_chars <= char_limit {
        return text.to_string();
    }

    let skip_chars = total_chars.saturating_sub(char_limit);
    let start_byte = text
        .char_indices()
        .nth(skip_chars)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(text.len());
    text[start_byte..].to_string()
}
