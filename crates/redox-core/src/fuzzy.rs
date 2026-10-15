use std::cmp::Ordering;
use std::ops::Range;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FuzzyQuery {
    tokens: Vec<QueryToken>,
}

#[derive(Debug, Clone)]
struct QueryToken {
    /// The lowercased token's chars, matched one by one in subsequences.
    chars: Vec<char>,
    /// Each of `chars` lowercased again, for contiguous matches.
    lower: Vec<Lower>,
}

/// A char's lowercase form. Only a few chars lowercase to several chars.
#[derive(Debug, Clone, Copy)]
enum Lower {
    One(char),
    /// Holds the original char, whose lowercase has several chars.
    Many(char),
}

impl PartialEq for Lower {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::One(left), Self::One(right)) => left == right,
            (Self::Many(left), Self::Many(right)) => left.to_lowercase().eq(right.to_lowercase()),
            _ => false,
        }
    }
}

impl Lower {
    fn of(ch: char) -> Self {
        if ch.is_ascii() {
            return Self::One(ch.to_ascii_lowercase());
        }
        let mut lower = ch.to_lowercase();
        match (lower.next(), lower.next()) {
            (Some(single), None) => Self::One(single),
            _ => Self::Many(ch),
        }
    }
}

/// A label char with its byte span, lowercased once per label.
#[derive(Debug, Clone, Copy)]
struct LabelChar {
    byte: usize,
    len: usize,
    lower: Lower,
}

fn lower_label(label: &str) -> Vec<LabelChar> {
    label
        .char_indices()
        .map(|(byte, ch)| LabelChar {
            byte,
            len: ch.len_utf8(),
            lower: Lower::of(ch),
        })
        .collect()
}

/// The label chars starting at `byte`, which must be a char boundary.
fn chars_from(chars: &[LabelChar], byte: usize) -> &[LabelChar] {
    &chars[chars.partition_point(|ch| ch.byte < byte)..]
}

impl FuzzyQuery {
    pub fn new(query: &str) -> Self {
        Self {
            tokens: query
                .split_whitespace()
                .filter(|token| !token.is_empty())
                .map(|token| {
                    let chars = token.to_lowercase().chars().collect::<Vec<_>>();
                    let lower = chars.iter().map(|&ch| Lower::of(ch)).collect();
                    QueryToken { chars, lower }
                })
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Whether every label this query matches is also matched by `broader`,
    /// so its results can be filtered from `broader`'s instead of from scratch.
    ///
    /// Holds when each broader token is a subsequence of one of these tokens
    /// and lowercasing its chars again leaves them unchanged. Then any match
    /// of the longer token also contains the broader token as a subsequence.
    pub fn refines(&self, broader: &FuzzyQuery) -> bool {
        broader.tokens.iter().all(|broader_token| {
            broader_token
                .chars
                .iter()
                .all(|&ch| Lower::of(ch) == Lower::One(ch))
                && self
                    .tokens
                    .iter()
                    .any(|token| is_subsequence(&broader_token.chars, &token.chars))
        })
    }
}

fn is_subsequence(needle: &[char], haystack: &[char]) -> bool {
    let mut haystack = haystack.iter();
    needle
        .iter()
        .all(|ch| haystack.any(|candidate| candidate == ch))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub highlights: Vec<Range<usize>>,
    pub first_match: usize,
    pub match_span: usize,
    pub gap_count: usize,
    pub contiguous_match_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathMatchScore {
    pub filename_match_count: usize,
    pub filename_contiguous_match_count: usize,
    pub contiguous_match_count: usize,
    pub filename_first_match: usize,
    pub path_first_match: usize,
    pub match_span: usize,
    pub gap_count: usize,
    pub depth: usize,
    pub len: usize,
}

#[derive(Debug, Clone)]
struct FuzzyTokenMatch {
    highlights: Vec<Range<usize>>,
    first_match: usize,
    last_match_end: usize,
    gap_count: usize,
    is_contiguous: bool,
}

pub fn fuzzy_match_ranges(label: &str, query: &FuzzyQuery) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(empty_query_match());
    }
    fuzzy_match_lowered(label, &lower_label(label), query)
}

/// Match a path label and score it, lowercasing the label only once.
pub fn fuzzy_path_match(label: &str, query: &FuzzyQuery) -> Option<(FuzzyMatch, PathMatchScore)> {
    if query.is_empty() {
        let matched = empty_query_match();
        let score = path_match_score_lowered(label, &[], &matched, query);
        return Some((matched, score));
    }
    let chars = lower_label(label);
    let matched = fuzzy_match_lowered(label, &chars, query)?;
    let score = path_match_score_lowered(label, &chars, &matched, query);
    Some((matched, score))
}

fn empty_query_match() -> FuzzyMatch {
    FuzzyMatch {
        highlights: Vec::new(),
        first_match: usize::MAX,
        match_span: usize::MAX,
        gap_count: usize::MAX,
        contiguous_match_count: 0,
    }
}

fn fuzzy_match_lowered(label: &str, chars: &[LabelChar], query: &FuzzyQuery) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch {
            highlights: Vec::new(),
            first_match: usize::MAX,
            match_span: usize::MAX,
            gap_count: usize::MAX,
            contiguous_match_count: 0,
        });
    }

    let mut ranges = Vec::new();
    let mut first_match = usize::MAX;
    let mut last_match_end = 0;
    let mut gap_count = 0usize;
    let mut contiguous_match_count = 0usize;

    let file_name_chars = chars_from(chars, file_name_byte_offset(label));
    for token in &query.tokens {
        let token_match = fuzzy_match_token(file_name_chars, chars, token)?;
        if first_match == usize::MAX {
            first_match = token_match.first_match;
        } else if token_match.first_match > last_match_end {
            gap_count += token_match.first_match - last_match_end;
        }
        last_match_end = token_match.last_match_end;
        gap_count += token_match.gap_count;
        if token_match.is_contiguous {
            contiguous_match_count += 1;
        }
        ranges.extend(token_match.highlights);
    }

    let highlights = merge_ranges(ranges);
    let match_span = highlights
        .first()
        .zip(highlights.last())
        .map(|(first, last)| last.end.saturating_sub(first.start))
        .unwrap_or(usize::MAX);

    Some(FuzzyMatch {
        highlights,
        first_match,
        match_span,
        gap_count,
        contiguous_match_count,
    })
}

pub fn path_match_score(label: &str, matched: &FuzzyMatch, query: &FuzzyQuery) -> PathMatchScore {
    let chars = if query.is_empty() {
        Vec::new()
    } else {
        lower_label(label)
    };
    path_match_score_lowered(label, &chars, matched, query)
}

fn path_match_score_lowered(
    label: &str,
    chars: &[LabelChar],
    matched: &FuzzyMatch,
    query: &FuzzyQuery,
) -> PathMatchScore {
    let file_name = Path::new(label)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(label);
    // `file_name` borrows from `label`, so its address gives its offset.
    let file_name_start = file_name.as_ptr() as usize - label.as_ptr() as usize;
    let file_name_chars = chars_from(chars, file_name_start);
    let file_name_chars = &file_name_chars
        [..file_name_chars.partition_point(|ch| ch.byte < file_name_start + file_name.len())];
    let filename_stats = filename_match_stats(file_name_chars, file_name_start, query);

    PathMatchScore {
        filename_match_count: filename_stats.match_count,
        filename_contiguous_match_count: filename_stats.contiguous_match_count,
        contiguous_match_count: matched.contiguous_match_count,
        filename_first_match: filename_stats.first_match,
        path_first_match: matched.first_match,
        match_span: matched.match_span,
        gap_count: matched.gap_count,
        depth: path_depth(label),
        len: label.len(),
    }
}

fn path_depth(label: &str) -> usize {
    Path::new(label).components().count().saturating_sub(1)
}

struct FilenameMatchStats {
    match_count: usize,
    contiguous_match_count: usize,
    first_match: usize,
}

/// Match each token against the file name alone; offsets are relative to it.
fn filename_match_stats(
    file_name: &[LabelChar],
    file_name_start: usize,
    query: &FuzzyQuery,
) -> FilenameMatchStats {
    let mut match_count = 0usize;
    let mut contiguous_match_count = 0usize;
    let mut first_match = usize::MAX;

    for token in &query.tokens {
        let Some(matched) = fuzzy_match_token(file_name, file_name, token) else {
            continue;
        };
        match_count += 1;
        first_match = first_match.min(matched.first_match.saturating_sub(file_name_start));
        if matched.is_contiguous {
            contiguous_match_count += 1;
        }
    }

    FilenameMatchStats {
        match_count,
        contiguous_match_count,
        first_match,
    }
}

pub fn compare_path_match_scores(left: &PathMatchScore, right: &PathMatchScore) -> Ordering {
    right
        .filename_match_count
        .cmp(&left.filename_match_count)
        .then(
            right
                .filename_contiguous_match_count
                .cmp(&left.filename_contiguous_match_count),
        )
        .then(
            right
                .contiguous_match_count
                .cmp(&left.contiguous_match_count),
        )
        .then(left.filename_first_match.cmp(&right.filename_first_match))
        .then(left.path_first_match.cmp(&right.path_first_match))
        .then(left.match_span.cmp(&right.match_span))
        .then(left.gap_count.cmp(&right.gap_count))
        .then(left.depth.cmp(&right.depth))
        .then(left.len.cmp(&right.len))
}

/// Prefer a contiguous match in the file name, then anywhere, then a subsequence.
fn fuzzy_match_token(
    file_name: &[LabelChar],
    label: &[LabelChar],
    token: &QueryToken,
) -> Option<FuzzyTokenMatch> {
    if token.chars.is_empty() {
        return Some(FuzzyTokenMatch {
            highlights: Vec::new(),
            first_match: usize::MAX,
            last_match_end: 0,
            gap_count: usize::MAX,
            is_contiguous: false,
        });
    }

    if let Some(range) = contiguous_match_range(file_name, &token.lower)
        .or_else(|| contiguous_match_range(label, &token.lower))
    {
        return Some(FuzzyTokenMatch {
            first_match: range.start,
            last_match_end: range.end,
            highlights: vec![range],
            gap_count: 0,
            is_contiguous: true,
        });
    }

    let mut search_start = 0usize;
    let mut highlights = Vec::with_capacity(token.chars.len());
    let mut gap_count = 0usize;
    let mut previous_index = None;
    for &token_ch in &token.chars {
        let offset = label[search_start..]
            .iter()
            .position(|label_ch| label_ch.lower == Lower::One(token_ch))?;
        let char_idx = search_start + offset;
        let matched = label[char_idx];
        highlights.push(matched.byte..matched.byte + matched.len);
        if let Some(previous) = previous_index {
            gap_count += char_idx - previous - 1;
        }
        previous_index = Some(char_idx);
        search_start = char_idx + 1;
    }

    Some(FuzzyTokenMatch {
        first_match: highlights.first().map_or(usize::MAX, |range| range.start),
        last_match_end: highlights.last().map_or(0, |range| range.end),
        highlights,
        gap_count,
        is_contiguous: false,
    })
}

fn contiguous_match_range(label: &[LabelChar], token: &[Lower]) -> Option<Range<usize>> {
    if token.is_empty() || token.len() > label.len() {
        return None;
    }

    label
        .windows(token.len())
        .find(|window| {
            window
                .iter()
                .zip(token)
                .all(|(label_ch, token_ch)| label_ch.lower == *token_ch)
        })
        .map(|window| {
            let last = window[window.len() - 1];
            window[0].byte..last.byte + last.len
        })
}

fn file_name_byte_offset(label: &str) -> usize {
    let Some(file_name) = Path::new(label).file_name().and_then(|name| name.to_str()) else {
        return 0;
    };

    label.rfind(file_name).unwrap_or(0)
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    if ranges.is_empty() {
        return ranges;
    }

    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
            continue;
        }
        merged.push(range);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::{FuzzyQuery, fuzzy_match_ranges, fuzzy_path_match, path_match_score};

    #[test]
    fn matching_ignores_case_and_reports_byte_ranges() {
        let query = FuzzyQuery::new("ÉT rs");
        let matched = fuzzy_match_ranges("src/été.rs", &query).unwrap();
        assert_eq!(matched.highlights, vec![4..7, 10..12]);
        assert!(fuzzy_match_ranges("src/main.rs", &query).is_none());
    }

    #[test]
    fn path_match_reuses_one_lowercase_pass_for_both_results() {
        for label in ["crates/core/src/fuzzy.rs", "README.md", "a/b/", ""] {
            for query in ["", "fz", "src rs", "md read", "zz"] {
                let query = FuzzyQuery::new(query);
                let separate = fuzzy_match_ranges(label, &query)
                    .map(|matched| (path_match_score(label, &matched, &query), matched));
                let combined =
                    fuzzy_path_match(label, &query).map(|(matched, score)| (score, matched));
                assert_eq!(combined, separate, "{label:?}");
            }
        }
    }

    #[test]
    fn refining_queries_only_add_non_space_chars() {
        let refines =
            |query: &str, broader: &str| FuzzyQuery::new(query).refines(&FuzzyQuery::new(broader));
        assert!(refines("abc", ""));
        assert!(refines("abc", "ab"));
        assert!(refines("axbc", "abc"));
        assert!(refines("ab x", "ab"));
        assert!(refines("Ab", "ab"));
        assert!(!refines("ab", "abc"));
        assert!(!refines("a bc", "abc"));
        // Lowercasing depends on context: a final sigma changes form.
        assert!(!refines("ΑΣΒ", "ΑΣ"));
    }
}
