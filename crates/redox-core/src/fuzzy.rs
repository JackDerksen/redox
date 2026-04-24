use std::cmp::Ordering;
use std::ops::Range;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FuzzyQuery {
    tokens: Vec<String>,
}

impl FuzzyQuery {
    pub fn new(query: &str) -> Self {
        Self {
            tokens: query
                .split_whitespace()
                .filter(|token| !token.is_empty())
                .map(|token| token.to_lowercase())
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
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

    for token in &query.tokens {
        let token_match = fuzzy_match_token(label, token)?;
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
    let file_name = Path::new(label)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(label);
    let filename_stats = filename_match_stats(file_name, query);

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

fn filename_match_stats(file_name: &str, query: &FuzzyQuery) -> FilenameMatchStats {
    let mut match_count = 0usize;
    let mut contiguous_match_count = 0usize;
    let mut first_match = usize::MAX;

    for token in &query.tokens {
        let Some(matched) = fuzzy_match_token(file_name, token) else {
            continue;
        };
        match_count += 1;
        first_match = first_match.min(matched.first_match);
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

fn fuzzy_match_token(label: &str, token: &str) -> Option<FuzzyTokenMatch> {
    if token.is_empty() {
        return Some(FuzzyTokenMatch {
            highlights: Vec::new(),
            first_match: usize::MAX,
            last_match_end: 0,
            gap_count: usize::MAX,
            is_contiguous: false,
        });
    }

    let file_name_offset = file_name_byte_offset(label);
    if let Some(range) = contiguous_match_range(&label[file_name_offset..], token) {
        let range = range.start + file_name_offset..range.end + file_name_offset;
        return Some(FuzzyTokenMatch {
            first_match: range.start,
            last_match_end: range.end,
            highlights: vec![range],
            gap_count: 0,
            is_contiguous: true,
        });
    }

    if let Some(range) = contiguous_match_range(label, token) {
        return Some(FuzzyTokenMatch {
            first_match: range.start,
            last_match_end: range.end,
            highlights: vec![range],
            gap_count: 0,
            is_contiguous: true,
        });
    }

    let label_chars = label
        .char_indices()
        .map(|(byte_idx, ch)| (byte_idx, ch, ch.len_utf8(), ch.to_lowercase().to_string()))
        .collect::<Vec<_>>();
    let token_chars = token.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();

    let mut search_start = 0usize;
    let mut highlights = Vec::with_capacity(token_chars.len());
    let mut matched_char_indices = Vec::with_capacity(token_chars.len());

    for token_ch in token_chars {
        let next = label_chars
            .iter()
            .enumerate()
            .skip(search_start)
            .find(|(_, (_, _, _, label_ch))| label_ch == &token_ch)?;
        let (char_idx, (byte_idx, _, byte_len, _)) = next;
        highlights.push(*byte_idx..*byte_idx + *byte_len);
        matched_char_indices.push(char_idx);
        search_start = char_idx + 1;
    }

    let first_match = highlights
        .first()
        .map(|range| range.start)
        .unwrap_or(usize::MAX);
    let last_match_end = highlights.last().map(|range| range.end).unwrap_or(0);
    let gap_count = matched_char_indices
        .windows(2)
        .map(|window| window[1].saturating_sub(window[0]).saturating_sub(1))
        .sum();

    Some(FuzzyTokenMatch {
        highlights,
        first_match,
        last_match_end,
        gap_count,
        is_contiguous: false,
    })
}

fn contiguous_match_range(label: &str, token: &str) -> Option<Range<usize>> {
    let label_chars = label
        .char_indices()
        .map(|(byte_idx, ch)| (byte_idx, ch.len_utf8(), ch.to_lowercase().to_string()))
        .collect::<Vec<_>>();
    let token_chars = token
        .chars()
        .map(|ch| ch.to_lowercase().to_string())
        .collect::<Vec<_>>();

    if token_chars.is_empty() || token_chars.len() > label_chars.len() {
        return None;
    }

    label_chars
        .windows(token_chars.len())
        .find(|window| {
            window
                .iter()
                .zip(&token_chars)
                .all(|((_, _, label_ch), token_ch)| label_ch == token_ch)
        })
        .map(|window| {
            let (start, _, _) = window[0];
            let (end_start, end_len, _) = window[window.len() - 1];
            start..end_start + end_len
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
    use super::*;

    #[test]
    fn merge_ranges_combines_overlaps() {
        let merged = merge_ranges(vec![0..2, 1..4, 5..7, 6..9]);
        assert_eq!(merged, vec![0..4, 5..9]);
    }

    #[test]
    fn fuzzy_match_is_case_insensitive_and_non_contiguous() {
        let query = FuzzyQuery::new("SRMA");
        let matched = fuzzy_match_ranges("src/main.rs", &query).expect("fuzzy match");

        assert_eq!(matched.highlights, vec![0..2, 4..6]);
    }

    #[test]
    fn fuzzy_match_treats_regex_metacharacters_literally() {
        let bracket_query = FuzzyQuery::new("[");
        let bracket_match =
            fuzzy_match_ranges("src/[draft].rs", &bracket_query).expect("bracket match");
        assert_eq!(bracket_match.highlights, vec![4..5]);

        let dot_query = FuzzyQuery::new(".");
        let dot_match = fuzzy_match_ranges("src/main.rs", &dot_query).expect("dot match");
        assert_eq!(dot_match.highlights, vec![8..9]);
    }

    #[test]
    fn fuzzy_match_prefers_contiguous_token_over_earliest_scattered_match() {
        let query = FuzzyQuery::new("tui");
        let label = "crates/redox-tui/src/app/state.rs";
        let matched = fuzzy_match_ranges(label, &query).expect("contiguous token match");
        let start = label.find("tui").expect("tui occurrence");

        assert_eq!(matched.highlights, vec![start..start + "tui".len()]);
        assert_eq!(matched.gap_count, 0);
    }

    #[test]
    fn fuzzy_match_prefers_leaf_contiguous_token() {
        let query = FuzzyQuery::new("state");
        let label = "stateful/src/app/state.rs";
        let matched = fuzzy_match_ranges(label, &query).expect("leaf token match");
        let start = label.rfind("state").expect("leaf state occurrence");

        assert_eq!(matched.highlights, vec![start..start + "state".len()]);
    }

    #[test]
    fn path_helpers_use_path_components() {
        assert_eq!(path_depth("src/main.rs"), 1);
        assert_eq!(file_name_byte_offset("src/main.rs"), "src/".len());
    }

    #[cfg(windows)]
    #[test]
    fn path_helpers_use_windows_path_components() {
        assert_eq!(path_depth(r"src\main.rs"), 1);
        assert_eq!(file_name_byte_offset(r"src\main.rs"), r"src\".len());
    }

    #[test]
    fn path_match_score_prefers_filename_matches() {
        let query = FuzzyQuery::new("main");
        let direct_match = fuzzy_match_ranges("src/main.rs", &query).expect("direct match");
        let indirect_match = fuzzy_match_ranges("main/src/lib.rs", &query).expect("indirect match");
        let direct = path_match_score("src/main.rs", &direct_match, &query);
        let indirect = path_match_score("main/src/lib.rs", &indirect_match, &query);

        assert_eq!(
            compare_path_match_scores(&direct, &indirect),
            Ordering::Less
        );
    }

    #[test]
    fn path_match_score_prefers_contiguous_path_matches_over_earlier_scattered_matches() {
        let query = FuzzyQuery::new("tui");
        let contiguous_match =
            fuzzy_match_ranges("crates/redox-tui/src/lib.rs", &query).expect("contiguous match");
        let scattered_match =
            fuzzy_match_ranges("toplevel/utility/index.rs", &query).expect("scattered match");
        let contiguous = path_match_score("crates/redox-tui/src/lib.rs", &contiguous_match, &query);
        let scattered = path_match_score("toplevel/utility/index.rs", &scattered_match, &query);

        assert_eq!(
            compare_path_match_scores(&contiguous, &scattered),
            Ordering::Less
        );
    }
}
