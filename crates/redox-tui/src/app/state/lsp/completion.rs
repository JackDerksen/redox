use std::collections::HashMap;
use std::time::{Duration, Instant};

use redox_core::Pos;
use serde_json::Value;

use super::{
    COMPLETION_AUTO_TRIGGER_DEBOUNCE, COMPLETION_TRIGGER_CHARACTER_DEBOUNCE, IncomingRange,
    utf16_code_unit_to_char_col,
};

#[derive(Debug, Clone)]
pub(super) struct CompletionCandidate {
    pub(super) label: String,
    pub(super) detail: Option<String>,
    pub(super) label_detail: Option<String>,
    pub(super) label_description: Option<String>,
    pub(super) documentation: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) filter_text: Option<String>,
    pub(super) sort_text: Option<String>,
    pub(super) insert_text: String,
    pub(super) insert_text_format: InsertTextFormat,
    pub(super) text_edit: Option<CompletionTextEdit>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CompletionDefaults {
    pub(super) edit_range: Option<IncomingRange>,
    pub(super) insert_text_format: Option<InsertTextFormat>,
}

#[derive(Debug, Clone)]
pub(super) struct CompletionTextEdit {
    pub(super) range: IncomingRange,
    pub(super) new_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InsertTextFormat {
    PlainText,
    Snippet,
}

#[derive(Debug, Clone)]
pub(super) struct CompletionState {
    pub(super) selected: usize,
    pub(super) requested_at: Pos,
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

pub(super) fn parse_completion_response(message: &Value) -> Vec<CompletionCandidate> {
    let Some(result) = message.get("result") else {
        return Vec::new();
    };
    if result.is_null() {
        return Vec::new();
    }
    let defaults = result
        .get("itemDefaults")
        .map(parse_completion_defaults)
        .unwrap_or_default();
    let values = if let Some(items) = result.get("items").and_then(Value::as_array) {
        items
    } else if let Some(items) = result.as_array() {
        items
    } else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|item| parse_completion_item(item, &defaults))
        .take(100)
        .collect()
}

pub(super) fn parse_completion_defaults(value: &Value) -> CompletionDefaults {
    CompletionDefaults {
        edit_range: value.get("editRange").and_then(parse_completion_edit_range),
        insert_text_format: value
            .get("insertTextFormat")
            .and_then(Value::as_u64)
            .map(insert_text_format_from_lsp),
    }
}

pub(super) fn parse_completion_item(
    value: &Value,
    defaults: &CompletionDefaults,
) -> Option<CompletionCandidate> {
    let label = value.get("label")?.as_str()?.to_string();
    let detail = value
        .get("detail")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let label_detail = value
        .get("labelDetails")
        .and_then(|details| details.get("detail"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    let label_description = value
        .get("labelDetails")
        .and_then(|details| details.get("description"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    let documentation = value
        .get("documentation")
        .and_then(parse_completion_documentation);
    let kind = value
        .get("kind")
        .and_then(Value::as_u64)
        .map(completion_kind_label)
        .map(ToString::to_string);
    let filter_text = value
        .get("filterText")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let sort_text = value
        .get("sortText")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let insert_text = value
        .get("insertText")
        .and_then(Value::as_str)
        .unwrap_or(&label)
        .to_string();
    let insert_text_format = value
        .get("insertTextFormat")
        .and_then(Value::as_u64)
        .map(insert_text_format_from_lsp)
        .or(defaults.insert_text_format)
        .unwrap_or(InsertTextFormat::PlainText);
    let text_edit = value
        .get("textEdit")
        .and_then(parse_completion_text_edit)
        .or_else(|| {
            defaults.edit_range.clone().map(|range| CompletionTextEdit {
                range,
                new_text: insert_text.clone(),
            })
        });
    Some(CompletionCandidate {
        label,
        detail,
        label_detail,
        label_description,
        documentation,
        kind,
        filter_text,
        sort_text,
        insert_text,
        insert_text_format,
        text_edit,
    })
}

pub(super) fn insert_text_format_from_lsp(value: u64) -> InsertTextFormat {
    match value {
        2 => InsertTextFormat::Snippet,
        _ => InsertTextFormat::PlainText,
    }
}

pub(super) fn parse_completion_text_edit(value: &Value) -> Option<CompletionTextEdit> {
    let range = parse_completion_edit_range(value)?;
    let new_text = value
        .get("newText")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(CompletionTextEdit { range, new_text })
}

pub(super) fn parse_completion_documentation(value: &Value) -> Option<String> {
    let text = value
        .as_str()
        .or_else(|| value.get("value").and_then(Value::as_str))?;
    let text = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .trim_matches('`')
        .to_string();
    (!text.is_empty()).then_some(text)
}

pub(super) fn parse_completion_edit_range(value: &Value) -> Option<IncomingRange> {
    value
        .get("range")
        .cloned()
        .or_else(|| value.get("replace").cloned())
        .or_else(|| value.get("insert").cloned())
        .or_else(|| {
            (value.get("start").is_some() && value.get("end").is_some()).then(|| value.clone())
        })
        .and_then(|range| serde_json::from_value::<IncomingRange>(range).ok())
}

pub(super) fn completion_kind_label(kind: u64) -> &'static str {
    match kind {
        1 => "text",
        2 => "method",
        3 => "function",
        4 => "constructor",
        5 => "field",
        6 => "variable",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        14 => "keyword",
        15 => "snippet",
        21 => "constant",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "type",
        _ => "item",
    }
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

pub(super) fn completion_type_label(item: &CompletionCandidate) -> Option<String> {
    item.label_detail
        .clone()
        .or_else(|| item.detail.clone())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty() && text != &item.label)
}

pub(super) fn completion_extra_label(item: &CompletionCandidate) -> Option<String> {
    item.label_description
        .clone()
        .or_else(|| item.kind.clone())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
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

pub(super) fn compare_completion_candidates(
    left: &CompletionCandidate,
    right: &CompletionCandidate,
) -> std::cmp::Ordering {
    left.sort_text
        .as_deref()
        .unwrap_or(&left.label)
        .cmp(right.sort_text.as_deref().unwrap_or(&right.label))
        .then_with(|| left.label.cmp(&right.label))
}

pub(super) fn completion_match_score(
    item: &CompletionCandidate,
    prefix: &str,
    context: &CompletionContext,
    recent: &HashMap<String, u32>,
) -> Option<i32> {
    let prefix = prefix.to_ascii_lowercase();
    let label = item.label.to_ascii_lowercase();
    let filter_text = item
        .filter_text
        .as_deref()
        .unwrap_or(&item.label)
        .to_ascii_lowercase();
    let insert_text = item.insert_text.to_ascii_lowercase();

    let mut best = [label.as_str(), filter_text.as_str(), insert_text.as_str()]
        .into_iter()
        .filter_map(|candidate| text_match_score(candidate, &prefix))
        .max()?;
    if matches!(item.kind.as_deref(), Some("snippet") | Some("keyword")) {
        best += 5;
    }
    best += completion_rank_score(item, context, recent);
    Some(best)
}

pub(super) fn completion_rank_score(
    item: &CompletionCandidate,
    context: &CompletionContext,
    recent: &HashMap<String, u32>,
) -> i32 {
    context_completion_score(item, context)
        + recent_completion_score(item, recent)
        + nearby_completion_score(item, context)
}

pub(super) fn recent_completion_score(
    item: &CompletionCandidate,
    recent: &HashMap<String, u32>,
) -> i32 {
    let key = item
        .filter_text
        .as_deref()
        .unwrap_or(&item.label)
        .to_ascii_lowercase();
    (recent.get(&key).copied().unwrap_or(0).min(10) as i32) * 25
}

pub(super) fn context_completion_score(
    item: &CompletionCandidate,
    context: &CompletionContext,
) -> i32 {
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

pub(super) fn nearby_completion_score(
    item: &CompletionCandidate,
    context: &CompletionContext,
) -> i32 {
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

pub(super) fn text_match_score(candidate: &str, prefix: &str) -> Option<i32> {
    if candidate == prefix {
        return Some(1200);
    }
    if candidate.starts_with(prefix) {
        return Some(1000 - candidate.len().saturating_sub(prefix.len()).min(200) as i32);
    }
    if let Some(idx) = candidate.find(prefix) {
        let boundary_bonus = match candidate.chars().nth(idx.saturating_sub(1)) {
            Some(ch) if idx > 0 => !completion_word_char(ch),
            _ => true,
        };
        return Some(700 - idx.min(200) as i32 + if boundary_bonus { 80 } else { 0 });
    }
    fuzzy_subsequence_score(candidate, prefix)
}

pub(super) fn fuzzy_subsequence_score(candidate: &str, prefix: &str) -> Option<i32> {
    let mut score = 300i32;
    let mut search_from = 0usize;
    let mut previous_match = None;
    for needle in prefix.chars() {
        let haystack = &candidate[search_from..];
        let offset = haystack.find(needle)?;
        let absolute = search_from + offset;
        score -= offset.min(50) as i32;
        if previous_match.is_some_and(|prev| prev + needle.len_utf8() == absolute) {
            score += 20;
        }
        previous_match = Some(absolute);
        search_from = absolute + needle.len_utf8();
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
) -> CompletionEdit {
    if let Some(text_edit) = &item.text_edit {
        let start_line = usize::try_from(text_edit.range.start.line)
            .unwrap_or(usize::MAX)
            .min(buffer.len_lines().saturating_sub(1));
        let end_line = usize::try_from(text_edit.range.end.line)
            .unwrap_or(usize::MAX)
            .min(buffer.len_lines().saturating_sub(1));
        let start = Pos::new(
            start_line,
            utf16_code_unit_to_char_col(
                &buffer.line_string(start_line),
                u32::try_from(text_edit.range.start.character).unwrap_or(u32::MAX),
            ),
        );
        let end = Pos::new(
            end_line,
            utf16_code_unit_to_char_col(
                &buffer.line_string(end_line),
                u32::try_from(text_edit.range.end.character).unwrap_or(u32::MAX),
            ),
        );
        return CompletionEdit {
            start,
            end,
            insert: text_edit.new_text.clone(),
        };
    }

    let prefix_start = completion_prefix_start(buffer, requested_at);
    CompletionEdit {
        start: prefix_start,
        end: requested_at,
        insert: item.insert_text.clone(),
    }
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

pub(super) fn completion_snippet_expansion(
    item: &CompletionCandidate,
    insert: &str,
) -> Option<SnippetExpansion> {
    let mut expansion = match item.insert_text_format {
        InsertTextFormat::PlainText => None,
        InsertTextFormat::Snippet => Some(expand_lsp_snippet(insert)),
    };
    let params = completion_parameter_placeholders(item)?;
    if params.is_empty() {
        return expansion;
    }
    let should_synthesize =
        expansion.as_ref().is_none_or(|expansion| {
            expansion.placeholders.is_empty() || snippet_placeholders_are_empty(expansion)
        }) && (matches!(item.kind.as_deref(), Some("function") | Some("method"))
            || insert_looks_like_call_target(insert));
    let should_replace_existing = expansion.as_ref().is_some_and(|expansion| {
        expansion.cursor_offset.is_some() || snippet_placeholders_are_empty(expansion)
    });
    if !should_synthesize {
        return expansion;
    }
    let call_text = expansion
        .as_ref()
        .map(|expansion| expansion.text.as_str())
        .unwrap_or(insert);
    let synthesized = synthesize_call_snippet(call_text, &params)?;
    if should_replace_existing && !synthesized.text.is_empty() {
        expansion = Some(synthesized);
    } else if expansion.is_none() {
        expansion = Some(synthesized);
    }
    expansion
}

#[derive(Debug, Clone)]
pub(super) struct SnippetExpansion {
    pub(super) text: String,
    pub(super) placeholders: Vec<SnippetPlaceholder>,
    pub(super) cursor_offset: Option<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct SnippetPlaceholder {
    pub(super) tabstop: usize,
    pub(super) start: usize,
    pub(super) end: usize,
}

pub(super) fn expand_lsp_snippet(snippet: &str) -> SnippetExpansion {
    let mut output = String::new();
    let mut placeholders = Vec::new();
    let mut final_cursor = None;
    let mut chars = snippet.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                output.push(next);
            }
            continue;
        }
        if ch != '$' {
            output.push(ch);
            continue;
        }

        match chars.peek().copied() {
            Some('{') => {
                let _ = chars.next();
                let mut body = String::new();
                let mut depth = 1usize;
                for next in chars.by_ref() {
                    match next {
                        '{' => {
                            depth = depth.saturating_add(1);
                            body.push(next);
                        }
                        '}' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                            body.push(next);
                        }
                        _ => body.push(next),
                    }
                }
                let expansion = expand_snippet_placeholder(&body);
                let start = output.chars().count();
                let end = start.saturating_add(expansion.text.chars().count());
                for mut placeholder in expansion.placeholders {
                    placeholder.start = placeholder.start.saturating_add(start);
                    placeholder.end = placeholder.end.saturating_add(start);
                    placeholders.push(placeholder);
                }
                if final_cursor.is_none() {
                    final_cursor = expansion
                        .cursor_offset
                        .map(|offset| start.saturating_add(offset));
                }
                output.push_str(&expansion.text);
                if let Some(tabstop) = snippet_placeholder_tabstop(&body)
                    && tabstop != 0
                {
                    placeholders.push(SnippetPlaceholder {
                        tabstop,
                        start,
                        end,
                    });
                }
            }
            Some(next) if next.is_ascii_digit() => {
                let mut digits = String::new();
                while let Some(digit) = chars.peek().copied().filter(|ch| ch.is_ascii_digit()) {
                    digits.push(digit);
                    let _ = chars.next();
                }
                if let Ok(tabstop) = digits.parse::<usize>() {
                    let at = output.chars().count();
                    if tabstop == 0 {
                        final_cursor.get_or_insert(at);
                    } else {
                        placeholders.push(SnippetPlaceholder {
                            tabstop,
                            start: at,
                            end: at,
                        });
                    }
                }
            }
            _ => output.push(ch),
        }
    }
    placeholders.sort_by_key(|placeholder| (placeholder.tabstop, placeholder.start));
    placeholders
        .dedup_by_key(|placeholder| (placeholder.tabstop, placeholder.start, placeholder.end));
    SnippetExpansion {
        text: output,
        placeholders,
        cursor_offset: final_cursor,
    }
}

pub(super) fn expand_snippet_placeholder(body: &str) -> SnippetExpansion {
    let Some((tabstop, default)) = body.split_once(':') else {
        let tabstop = body.parse::<usize>().ok();
        return SnippetExpansion {
            text: String::new(),
            placeholders: tabstop
                .filter(|tabstop| *tabstop != 0)
                .map(|tabstop| {
                    vec![SnippetPlaceholder {
                        tabstop,
                        start: 0,
                        end: 0,
                    }]
                })
                .unwrap_or_default(),
            cursor_offset: (tabstop == Some(0)).then_some(0),
        };
    };
    if let Ok(tabstop) = tabstop.parse::<usize>() {
        let mut expansion = expand_lsp_snippet(default);
        if tabstop == 0 {
            expansion.cursor_offset = Some(expansion.text.chars().count());
        }
        return expansion;
    }
    SnippetExpansion {
        text: body.to_string(),
        placeholders: Vec::new(),
        cursor_offset: None,
    }
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

fn snippet_placeholder_tabstop(body: &str) -> Option<usize> {
    body.split_once(':')
        .map(|(tabstop, _)| tabstop)
        .unwrap_or(body)
        .parse()
        .ok()
}

fn completion_parameter_placeholders(item: &CompletionCandidate) -> Option<Vec<String>> {
    [
        Some(item.label.as_str()),
        item.label_detail.as_deref(),
        item.detail.as_deref(),
        item.documentation.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find_map(parameters_from_signature_text)
}

fn insert_looks_like_call_target(insert: &str) -> bool {
    empty_call_parens(insert).is_some()
        || insert
            .chars()
            .all(|ch| ch == '_' || ch.is_alphanumeric() || ch == '.')
}

fn parameters_from_signature_text(text: &str) -> Option<Vec<String>> {
    let open = text.find('(')?;
    let close = matching_signature_paren(text, open)?;
    let params = &text[open.saturating_add(1)..close];
    let params = split_top_level_commas(params)
        .into_iter()
        .map(parameter_placeholder_text)
        .filter(|param| !param.is_empty())
        .collect::<Vec<_>>();
    Some(params)
}

fn matching_signature_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, ch) in text.char_indices().skip_while(|(idx, _)| *idx < open) {
        match ch {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' => paren_depth = paren_depth.saturating_add(1),
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth = bracket_depth.saturating_add(1),
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth = brace_depth.saturating_add(1),
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                parts.push(text[start..idx].trim());
                start = idx.saturating_add(ch.len_utf8());
            }
            _ => {}
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn parameter_placeholder_text(param: &str) -> String {
    let param = param.trim();
    if param.is_empty() || param == "..." {
        return String::new();
    }
    let first = param
        .split_whitespace()
        .next()
        .unwrap_or(param)
        .trim_start_matches("...")
        .trim_start_matches('*')
        .trim_start_matches('&');
    if is_parameter_name(first) {
        first.to_string()
    } else {
        param.to_string()
    }
}

fn is_parameter_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
        && !matches!(
            text,
            "func" | "map" | "chan" | "interface" | "struct" | "..." | "string" | "bool" | "int"
        )
}

fn synthesize_call_snippet(insert: &str, params: &[String]) -> Option<SnippetExpansion> {
    let param_snippet = params
        .iter()
        .enumerate()
        .map(|(idx, param)| {
            format!(
                "${{{}:{}}}",
                idx.saturating_add(1),
                escape_snippet_text(param)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    if let Some((open, close)) = replaceable_call_parens(insert) {
        let mut snippet = String::new();
        snippet.push_str(&insert[..open.saturating_add(1)]);
        snippet.push_str(&param_snippet);
        snippet.push_str(&insert[close..]);
        if !snippet.contains("$0") {
            snippet.push_str("$0");
        }
        return Some(expand_lsp_snippet(&snippet));
    }

    if insert
        .chars()
        .all(|ch| ch == '_' || ch.is_alphanumeric() || ch == '.')
    {
        let snippet = format!("{insert}({param_snippet})$0");
        return Some(expand_lsp_snippet(&snippet));
    }
    None
}

fn empty_call_parens(text: &str) -> Option<(usize, usize)> {
    let open = text.find('(')?;
    let close = matching_signature_paren(text, open)?;
    text[open.saturating_add(1)..close]
        .trim()
        .is_empty()
        .then_some((open, close))
}

fn replaceable_call_parens(text: &str) -> Option<(usize, usize)> {
    let open = text.find('(')?;
    let close = matching_signature_paren(text, open)?;
    let inner = text[open.saturating_add(1)..close].trim();
    (inner.is_empty() || inner.chars().all(|ch| ch == ',' || ch.is_whitespace()))
        .then_some((open, close))
}

fn snippet_placeholders_are_empty(expansion: &SnippetExpansion) -> bool {
    !expansion.placeholders.is_empty()
        && expansion
            .placeholders
            .iter()
            .all(|placeholder| placeholder.start == placeholder.end)
}

fn escape_snippet_text(text: &str) -> String {
    text.chars()
        .flat_map(|ch| match ch {
            '\\' | '$' | '}' => ['\\', ch],
            _ => ['\0', ch],
        })
        .filter(|ch| *ch != '\0')
        .collect()
}
