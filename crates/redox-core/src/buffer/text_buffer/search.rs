//! Plain-text search helpers for `TextBuffer`.
//!
//! These helpers intentionally implement only literal substring and same-line
//! character search semantics for now. Regex-aware search can layer on later
//! for stuff like `:s/` commands.

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
            match self.rope().char(idx) {
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
            match self.rope().char(idx) {
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
            if self.rope().char(idx) == delimiter && !self.char_is_escaped_for_pairing(idx) {
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
            if self.rope().char(idx) != '\\' {
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
        let mut matches = Vec::new();
        let mut overlap = String::new();
        let mut processed_chars = 0usize;
        let mut last_emitted_end = 0usize;

        for chunk in self.rope().chunks() {
            let chunk_chars = chunk.chars().count();

            if overlap.is_empty() {
                collect_segment_matches(
                    self,
                    chunk,
                    processed_chars,
                    processed_chars,
                    needle,
                    needle_chars,
                    &mut matches,
                    &mut last_emitted_end,
                );

                if overlap_char_limit > 0 {
                    overlap = trailing_chars(chunk, overlap_char_limit);
                }
            } else {
                let overlap_chars = overlap.chars().count();
                let segment_start_char = processed_chars.saturating_sub(overlap_chars);
                let mut segment = String::with_capacity(overlap.len().saturating_add(chunk.len()));
                segment.push_str(&overlap);
                segment.push_str(chunk);

                collect_segment_matches(
                    self,
                    &segment,
                    segment_start_char,
                    processed_chars,
                    needle,
                    needle_chars,
                    &mut matches,
                    &mut last_emitted_end,
                );

                overlap = trailing_chars(&segment, overlap_char_limit);
            }

            processed_chars = processed_chars.saturating_add(chunk_chars);
        }

        matches
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

fn collect_segment_matches(
    buffer: &TextBuffer,
    segment: &str,
    segment_start_char: usize,
    emit_after_char: usize,
    needle: &str,
    needle_chars: usize,
    out: &mut Vec<(Pos, Pos)>,
    last_emitted_end: &mut usize,
) {
    let segment_scan_start_char = last_emitted_end.saturating_sub(segment_start_char);
    let segment_scan_start_byte = byte_idx_for_char(segment, segment_scan_start_char);
    let mut scan_start_byte = segment_scan_start_byte;
    let mut scan_start_chars = segment_scan_start_char;

    for (match_start_byte_rel, _) in segment[segment_scan_start_byte..].match_indices(needle) {
        let match_start_byte = segment_scan_start_byte.saturating_add(match_start_byte_rel);
        scan_start_chars = scan_start_chars
            .saturating_add(segment[scan_start_byte..match_start_byte].chars().count());

        let start_char = segment_start_char.saturating_add(scan_start_chars);
        let end_char = start_char.saturating_add(needle_chars);
        if start_char >= *last_emitted_end && end_char > emit_after_char {
            out.push((buffer.char_to_pos(start_char), buffer.char_to_pos(end_char)));
            *last_emitted_end = end_char;
        }

        scan_start_byte = match_start_byte.saturating_add(needle.len());
        scan_start_chars = scan_start_chars.saturating_add(needle_chars);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_find_matches(buffer: &TextBuffer, needle: &str) -> Vec<(Pos, Pos)> {
        let source = buffer.to_string();
        source
            .match_indices(needle)
            .map(|(start_byte, matched)| {
                let start_char = buffer.rope().byte_to_char(start_byte);
                let end_char = buffer
                    .rope()
                    .byte_to_char(start_byte.saturating_add(matched.len()));
                (buffer.char_to_pos(start_char), buffer.char_to_pos(end_char))
            })
            .collect()
    }

    #[test]
    fn find_matches_matches_naive_search_across_rope_chunks() {
        let mut text = String::new();
        for i in 0..40_000usize {
            text.push_str(&format!("{i:05x}|"));
        }
        let buffer = TextBuffer::from_str(&text);

        let mut chunks = buffer.rope().chunks();
        let first_chunk = chunks.next().expect("expected at least one chunk");
        let second_chunk = chunks.next().expect("expected multiple rope chunks");
        let boundary_needle = format!(
            "{}{}",
            trailing_chars(first_chunk, 6),
            second_chunk.chars().take(6).collect::<String>()
        );

        assert_eq!(
            buffer.find_matches(&boundary_needle),
            naive_find_matches(&buffer, &boundary_needle)
        );
    }

    #[test]
    fn find_matches_matches_naive_search_for_unicode_across_rope_chunks() {
        let mut text = String::new();
        for i in 0..20_000usize {
            text.push_str(&format!("α{i:05x}🙂β"));
        }
        let buffer = TextBuffer::from_str(&text);

        let mut chunks = buffer.rope().chunks();
        let first_chunk = chunks.next().expect("expected at least one chunk");
        let second_chunk = chunks.next().expect("expected multiple rope chunks");
        let boundary_needle = format!(
            "{}{}",
            trailing_chars(first_chunk, 4),
            second_chunk.chars().take(4).collect::<String>()
        );

        assert_eq!(
            buffer.find_matches(&boundary_needle),
            naive_find_matches(&buffer, &boundary_needle)
        );
    }

    #[test]
    fn find_matches_preserves_non_overlapping_semantics_across_chunk_boundaries() {
        let text = "a".repeat(120_000);
        let buffer = TextBuffer::from_str(&text);

        // Ensure the buffer actually spans multiple chunks
        assert!(
            buffer.rope().chunks().count() > 1,
            "expected multiple rope chunks"
        );

        assert_eq!(
            buffer.find_matches("aaa"),
            naive_find_matches(&buffer, "aaa")
        );
    }

    #[test]
    fn matching_delimiter_ignores_escaped_asymmetric_delimiter_under_cursor() {
        let buffer = TextBuffer::from_str(r"\(alpha)");

        assert_eq!(buffer.matching_delimiter(Pos::new(0, 1)), None);
    }

    #[test]
    fn matching_delimiter_skips_escaped_asymmetric_delimiters_while_scanning() {
        let buffer = TextBuffer::from_str(r"(alpha \( beta))");

        assert_eq!(
            buffer.matching_delimiter(Pos::new(0, 0)),
            Some(Pos::new(0, 14))
        );
        assert_eq!(
            buffer.matching_delimiter(Pos::new(0, 14)),
            Some(Pos::new(0, 0))
        );
    }
}
