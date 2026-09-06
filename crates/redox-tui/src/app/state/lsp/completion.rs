use std::collections::HashMap;
use std::ops::Range;
use std::time::{Duration, Instant};

use redox_core::Pos;
use redox_lsp::{CompletionCandidate, SnippetExpansion};

use super::{COMPLETION_AUTO_TRIGGER_DEBOUNCE, COMPLETION_TRIGGER_CHARACTER_DEBOUNCE};

#[derive(Debug, Clone)]
pub(super) struct CompletionState {
    pub(super) selected: usize,
    pub(super) requested_at: Pos,
    pub(super) context: super::RequestContext,
    pub(super) items: Vec<CompletionCandidate>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AutoCompletionRequest {
    pub(super) requested_at: Pos,
    pub(super) due_at: Instant,
}

#[derive(Debug, Clone)]
pub(super) struct ActiveSnippet {
    pub(super) buffer_id: redox_core::BufferId,
    pub(super) placeholders: Vec<ActiveSnippetPlaceholder>,
    pub(super) current: usize,
    pub(super) selected: bool,
    pub(super) final_char: Option<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct ActiveSnippetPlaceholder {
    pub(super) tabstop: usize,
    pub(super) start_char: usize,
    pub(super) end_char: usize,
    pub(super) filled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompletionContextKind {
    General,
    Member,
    Path,
    Module,
    Type,
    Function,
    StatementStart,
}

#[derive(Debug, Clone)]
pub(super) struct CompletionContext {
    pub(super) kind: CompletionContextKind,
    pub(super) nearby_text: String,
}

pub(super) fn should_auto_trigger_completion(ch: char) -> bool {
    ch == '_' || ch == '.' || ch == ':' || ch == '>' || ch.is_alphanumeric()
}

pub(super) fn completion_auto_trigger_delay(ch: char) -> Duration {
    if matches!(ch, '.' | ':' | '>') {
        COMPLETION_TRIGGER_CHARACTER_DEBOUNCE
    } else {
        COMPLETION_AUTO_TRIGGER_DEBOUNCE
    }
}

pub(super) fn filter_and_sort_completion_items(
    mut items: Vec<CompletionCandidate>,
    prefix: &str,
    context: &CompletionContext,
    recent: &HashMap<String, u32>,
) -> Vec<CompletionCandidate> {
    if prefix.is_empty() {
        items.sort_by(|left, right| {
            completion_rank_score(right, context, recent)
                .cmp(&completion_rank_score(left, context, recent))
                .then_with(|| compare_completion_candidates(left, right))
        });
        return items;
    }

    let mut scored = items
        .into_iter()
        .filter_map(|item| {
            completion_match_score(&item, prefix, context, recent).map(|score| (score, item))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| compare_completion_candidates(left, right))
    });
    scored.into_iter().map(|(_, item)| item).collect()
}

fn compare_completion_candidates(
    left: &CompletionCandidate,
    right: &CompletionCandidate,
) -> std::cmp::Ordering {
    left.sort_text
        .as_deref()
        .unwrap_or(&left.label)
        .cmp(right.sort_text.as_deref().unwrap_or(&right.label))
        .then_with(|| left.label.cmp(&right.label))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CompletionMatchScore {
    quality: u8,
    score: i32,
}

fn completion_match_score(
    item: &CompletionCandidate,
    prefix: &str,
    context: &CompletionContext,
    recent: &HashMap<String, u32>,
) -> Option<CompletionMatchScore> {
    let mut best = [
        item.label.as_str(),
        item.filter_text.as_deref().unwrap_or(&item.label),
        item.insert_text.as_str(),
    ]
    .into_iter()
    .filter_map(|candidate| text_match_score(candidate, prefix))
    .max()?;
    best.score += completion_rank_score(item, context, recent);
    Some(best)
}

fn completion_rank_score(
    item: &CompletionCandidate,
    context: &CompletionContext,
    recent: &HashMap<String, u32>,
) -> i32 {
    context_completion_score(item, context)
        + recent_completion_score(item, recent)
        + nearby_completion_score(item, context)
}

fn recent_completion_score(item: &CompletionCandidate, recent: &HashMap<String, u32>) -> i32 {
    let key = item
        .filter_text
        .as_deref()
        .unwrap_or(&item.label)
        .to_ascii_lowercase();
    (recent.get(&key).copied().unwrap_or(0).min(10) as i32) * 25
}

fn context_completion_score(item: &CompletionCandidate, context: &CompletionContext) -> i32 {
    let kind = item.kind.as_deref();
    match context.kind {
        CompletionContextKind::Member => match kind {
            Some("method") | Some("field") | Some("property") => 180,
            Some("function") => 70,
            Some("module") | Some("keyword") | Some("snippet") => -120,
            _ => 0,
        },
        CompletionContextKind::Path | CompletionContextKind::Module => match kind {
            Some("module") => 180,
            Some("type") | Some("struct") | Some("class") | Some("interface") => 100,
            Some("function") | Some("constant") => 50,
            Some("keyword") | Some("snippet") => -80,
            _ => 0,
        },
        CompletionContextKind::Type => match kind {
            Some("type") | Some("struct") | Some("class") | Some("interface") => 170,
            Some("constructor") => 80,
            Some("module") => 40,
            Some("keyword") | Some("snippet") => -50,
            _ => 0,
        },
        CompletionContextKind::Function => match kind {
            Some("function") | Some("method") => 160,
            Some("snippet") => 80,
            Some("keyword") => 40,
            _ => 0,
        },
        CompletionContextKind::StatementStart => match kind {
            Some("keyword") | Some("snippet") => 130,
            Some("function") | Some("module") => 40,
            _ => 0,
        },
        CompletionContextKind::General => 0,
    }
}

fn nearby_completion_score(item: &CompletionCandidate, context: &CompletionContext) -> i32 {
    if context.nearby_text.is_empty() {
        return 0;
    }
    let label = item.label.to_ascii_lowercase();
    if label.len() < 3 {
        return 0;
    }
    if context.nearby_text.contains(&label) {
        45
    } else {
        0
    }
}

fn text_match_score(candidate: &str, prefix: &str) -> Option<CompletionMatchScore> {
    let folded_candidate = candidate.to_ascii_lowercase();
    let folded_prefix = prefix.to_ascii_lowercase();
    let case_bonus = i32::from(candidate.contains(prefix)) * 5;
    if folded_candidate == folded_prefix {
        return Some(CompletionMatchScore {
            quality: 5,
            score: i32::from(candidate == prefix) * 5,
        });
    }
    if folded_candidate.starts_with(&folded_prefix) {
        return Some(CompletionMatchScore {
            quality: 4,
            score: case_bonus
                - i32::try_from(
                    candidate
                        .chars()
                        .count()
                        .saturating_sub(prefix.chars().count())
                        .min(200),
                )
                .unwrap_or(200),
        });
    }
    if let Some(byte_index) = folded_candidate.find(&folded_prefix) {
        let quality = if completion_match_starts_at_boundary(candidate, byte_index) {
            3
        } else {
            2
        };
        return Some(CompletionMatchScore {
            quality,
            score: case_bonus - i32::try_from(byte_index.min(200)).unwrap_or(200),
        });
    }
    fuzzy_subsequence_score(candidate, prefix)
        .map(|score| CompletionMatchScore { quality: 1, score })
}

fn completion_match_starts_at_boundary(candidate: &str, byte_index: usize) -> bool {
    if byte_index == 0 {
        return true;
    }
    let previous = candidate[..byte_index].chars().next_back();
    let current = candidate[byte_index..].chars().next();
    match (previous, current) {
        (Some(previous), Some(current)) => {
            !previous.is_alphanumeric()
                || previous == '_'
                || previous.is_lowercase() && current.is_uppercase()
        }
        _ => false,
    }
}

pub(super) fn completion_label_highlights(label: &str, prefix: &str) -> Vec<Range<usize>> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let folded_label = label.to_ascii_lowercase();
    let folded_prefix = prefix.to_ascii_lowercase();
    if let Some(start) = folded_label.find(&folded_prefix) {
        return std::iter::once(start..start.saturating_add(folded_prefix.len())).collect();
    }

    let mut highlights = Vec::<Range<usize>>::new();
    let mut search_from = 0usize;
    for character in folded_prefix.chars() {
        let offset = folded_label[search_from..].find(character);
        let Some(offset) = offset else {
            return Vec::new();
        };
        let start = search_from.saturating_add(offset);
        let end = start.saturating_add(character.len_utf8());
        if let Some(previous) = highlights.last_mut()
            && previous.end == start
        {
            previous.end = end;
        } else {
            highlights.push(start..end);
        }
        search_from = end;
    }
    highlights
}

fn fuzzy_subsequence_score(candidate: &str, prefix: &str) -> Option<i32> {
    let candidate = candidate.to_ascii_lowercase().chars().collect::<Vec<_>>();
    let prefix = prefix.to_ascii_lowercase();
    let mut score = 300i32;
    let mut search_from = 0usize;
    let mut previous_match = None;
    for needle in prefix.chars() {
        let offset = candidate[search_from..]
            .iter()
            .position(|candidate| *candidate == needle)?;
        let absolute = search_from + offset;
        score -= i32::try_from(offset.min(50)).unwrap_or(50);
        if previous_match.is_some_and(|previous| previous + 1 == absolute) {
            score += 20;
        }
        previous_match = Some(absolute);
        search_from = absolute + 1;
    }
    Some(score)
}

#[derive(Debug, Clone)]
pub(super) struct CompletionEdit {
    pub(super) start: Pos,
    pub(super) end: Pos,
    pub(super) insert: String,
}

pub(super) fn completion_edit_for_buffer(
    item: &CompletionCandidate,
    buffer: &redox_core::TextBuffer,
    requested_at: Pos,
) -> Option<CompletionEdit> {
    if let Some(text_edit) = &item.text_edit {
        let (start, end) = super::buffer_positions_for_range(buffer, &text_edit.range)?;
        if start.line != requested_at.line
            || end.line != requested_at.line
            || start.col > requested_at.col
            || end.col < requested_at.col
        {
            return None;
        }
        return Some(CompletionEdit {
            start,
            end,
            insert: text_edit.new_text.clone(),
        });
    }

    let prefix_start = completion_prefix_start(buffer, requested_at);
    Some(CompletionEdit {
        start: prefix_start,
        end: requested_at,
        insert: item.insert_text.clone(),
    })
}

pub(super) fn completion_prefix_start(buffer: &redox_core::TextBuffer, cursor: Pos) -> Pos {
    let line = cursor.line.min(buffer.len_lines().saturating_sub(1));
    let text = buffer.line_string(line);
    let cursor_col = cursor.col.min(text.chars().count());
    let start_col = text
        .chars()
        .take(cursor_col)
        .collect::<Vec<_>>()
        .into_iter()
        .rposition(|ch| !completion_word_char(ch))
        .map(|idx| idx.saturating_add(1))
        .unwrap_or(0);
    Pos::new(line, start_col)
}

pub(super) fn completion_prefix(buffer: &redox_core::TextBuffer, cursor: Pos) -> String {
    let start = completion_prefix_start(buffer, cursor);
    let line = cursor.line.min(buffer.len_lines().saturating_sub(1));
    let text = buffer.line_string(line);
    text.chars()
        .skip(start.col)
        .take(cursor.col.saturating_sub(start.col))
        .collect()
}

pub(super) fn completion_context(
    buffer: &redox_core::TextBuffer,
    cursor: Pos,
) -> CompletionContext {
    let line = cursor.line.min(buffer.len_lines().saturating_sub(1));
    let text = buffer.line_string(line);
    let before = text.chars().take(cursor.col).collect::<String>();
    let trimmed_before = before.trim_end();
    let last_word = trimmed_before
        .split(|ch: char| !(ch == '_' || ch.is_alphanumeric()))
        .filter(|word| !word.is_empty())
        .next_back()
        .unwrap_or_default();
    let kind = if trimmed_before.ends_with('.') {
        CompletionContextKind::Member
    } else if trimmed_before.ends_with("::") {
        CompletionContextKind::Path
    } else if matches!(last_word, "use" | "mod" | "crate" | "super" | "self") {
        CompletionContextKind::Module
    } else if matches!(
        last_word,
        "struct" | "enum" | "type" | "impl" | "trait" | "as"
    ) {
        CompletionContextKind::Type
    } else if matches!(last_word, "fn" | "macro_rules") {
        CompletionContextKind::Function
    } else if trimmed_before.trim().is_empty() {
        CompletionContextKind::StatementStart
    } else {
        CompletionContextKind::General
    };

    CompletionContext {
        kind,
        nearby_text: nearby_completion_text(buffer, line),
    }
}

pub(super) fn nearby_completion_text(buffer: &redox_core::TextBuffer, line: usize) -> String {
    let start = line.saturating_sub(20);
    let end = line
        .saturating_add(20)
        .min(buffer.len_lines().saturating_sub(1));
    let mut text = String::new();
    for idx in start..=end {
        if idx != line {
            text.push_str(&buffer.line_string(idx).to_ascii_lowercase());
            text.push('\n');
        }
    }
    text
}

pub(super) fn line_after_cursor_completion_preview_suffix(
    buffer: &redox_core::TextBuffer,
    cursor: Pos,
) -> Option<String> {
    let line = cursor.line.min(buffer.len_lines().saturating_sub(1));
    let suffix = buffer
        .line_string(line)
        .chars()
        .skip(cursor.col)
        .collect::<String>();
    suffix
        .chars()
        .all(|ch| {
            ch.is_whitespace() || matches!(ch, ')' | ']' | '}' | '>' | '"' | '\'' | '`' | ';' | ',')
        })
        .then_some(suffix)
}

pub(super) fn completion_word_char(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

pub(super) fn active_snippet_from_expansion(
    buffer_id: redox_core::BufferId,
    start_char: usize,
    expansion: &SnippetExpansion,
) -> Option<ActiveSnippet> {
    let mut placeholders = expansion
        .placeholders
        .iter()
        .filter(|placeholder| placeholder.tabstop != 0)
        .map(|placeholder| ActiveSnippetPlaceholder {
            tabstop: placeholder.tabstop,
            start_char: start_char.saturating_add(placeholder.start),
            end_char: start_char.saturating_add(placeholder.end),
            filled: false,
        })
        .collect::<Vec<_>>();
    placeholders.sort_by_key(|placeholder| (placeholder.tabstop, placeholder.start_char));
    (!placeholders.is_empty()).then_some(ActiveSnippet {
        buffer_id,
        placeholders,
        current: 0,
        selected: true,
        final_char: expansion
            .cursor_offset
            .map(|offset| start_char.saturating_add(offset)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_lsp::InsertTextFormat;

    #[test]
    fn completion_ranking_keeps_stronger_text_matches_first() {
        let exact = completion_candidate("map", "keyword", None);
        let favoured_prefix = completion_candidate("mappingFunction", "method", None);
        let boundary_match = completion_candidate("HashMap", "type", None);
        let inner_match = completion_candidate("bitmap", "variable", None);
        let context = CompletionContext {
            kind: CompletionContextKind::Member,
            nearby_text: "mappingfunction".to_string(),
        };
        let recent = HashMap::from([("mappingfunction".to_string(), 10)]);

        let ranked = filter_and_sort_completion_items(
            vec![favoured_prefix, inner_match, boundary_match, exact],
            "map",
            &context,
            &recent,
        );

        assert_eq!(
            ranked
                .iter()
                .map(|candidate| candidate.label.as_str())
                .collect::<Vec<_>>(),
            vec!["map", "mappingFunction", "HashMap", "bitmap"]
        );
    }

    #[test]
    fn completion_ranking_uses_server_order_to_break_equal_matches() {
        let later = completion_candidate("alphaOne", "variable", Some("02"));
        let earlier = completion_candidate("alphaTwo", "variable", Some("01"));
        let context = CompletionContext {
            kind: CompletionContextKind::General,
            nearby_text: String::new(),
        };

        let ranked = filter_and_sort_completion_items(
            vec![later, earlier],
            "alpha",
            &context,
            &HashMap::new(),
        );

        assert_eq!(ranked[0].label, "alphaTwo");
    }

    fn completion_candidate(
        label: &str,
        kind: &str,
        sort_text: Option<&str>,
    ) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            detail: None,
            label_detail: None,
            label_description: None,
            documentation: None,
            kind: Some(kind.to_string()),
            filter_text: None,
            sort_text: sort_text.map(ToString::to_string),
            insert_text: label.to_string(),
            insert_text_format: InsertTextFormat::PlainText,
            text_edit: None,
            additional_text_edits: Vec::new(),
        }
    }
}
