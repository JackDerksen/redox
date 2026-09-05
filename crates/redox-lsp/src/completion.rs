use serde_json::Value;

use crate::code_action::TextEdit;
use crate::hover::{SymbolInfoBlock, SymbolInfoKind};
use crate::protocol::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub label: String,
    pub detail: Option<String>,
    pub label_detail: Option<String>,
    pub label_description: Option<String>,
    pub documentation: Option<SymbolInfoBlock>,
    pub kind: Option<String>,
    pub filter_text: Option<String>,
    pub sort_text: Option<String>,
    pub insert_text: String,
    pub insert_text_format: InsertTextFormat,
    pub text_edit: Option<CompletionTextEdit>,
    pub additional_text_edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionDefaults {
    pub edit_range: Option<Range>,
    pub insert_text_format: Option<InsertTextFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionTextEdit {
    pub range: Range,
    pub new_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertTextFormat {
    PlainText,
    Snippet,
}

/// Parses both `CompletionList` and completion-item array responses.
#[must_use]
pub fn parse_completion_response(message: &Value) -> Vec<CompletionCandidate> {
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
        .collect()
}

fn parse_completion_defaults(value: &Value) -> CompletionDefaults {
    CompletionDefaults {
        edit_range: value.get("editRange").and_then(parse_completion_edit_range),
        insert_text_format: value
            .get("insertTextFormat")
            .and_then(Value::as_u64)
            .map(insert_text_format_from_lsp),
    }
}

fn parse_completion_item(
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
    let text_edit_text = value
        .get("textEditText")
        .and_then(Value::as_str)
        .unwrap_or(&insert_text);
    let insert_text_format = value
        .get("insertTextFormat")
        .and_then(Value::as_u64)
        .map(insert_text_format_from_lsp)
        .or(defaults.insert_text_format)
        .unwrap_or(InsertTextFormat::PlainText);
    let text_edit = match value.get("textEdit") {
        Some(edit) => Some(parse_completion_text_edit(edit)?),
        None => defaults.edit_range.clone().map(|range| CompletionTextEdit {
            range,
            new_text: text_edit_text.to_string(),
        }),
    };
    let additional_text_edits = match value.get("additionalTextEdits") {
        Some(edits) => edits
            .as_array()?
            .iter()
            .map(crate::code_action::parse_text_edit)
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
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
        additional_text_edits,
    })
}

fn insert_text_format_from_lsp(value: u64) -> InsertTextFormat {
    match value {
        2 => InsertTextFormat::Snippet,
        _ => InsertTextFormat::PlainText,
    }
}

fn parse_completion_text_edit(value: &Value) -> Option<CompletionTextEdit> {
    let range = parse_completion_edit_range(value)?;
    let new_text = value.get("newText").and_then(Value::as_str)?.to_string();
    Some(CompletionTextEdit { range, new_text })
}

fn parse_completion_documentation(value: &Value) -> Option<SymbolInfoBlock> {
    let text = value
        .as_str()
        .or_else(|| value.get("value").and_then(Value::as_str))?;
    let kind = if value.get("kind").and_then(Value::as_str) == Some("markdown") {
        SymbolInfoKind::Markdown
    } else {
        SymbolInfoKind::PlainText
    };
    (!text.trim().is_empty()).then(|| SymbolInfoBlock {
        kind,
        text: text.to_string(),
    })
}

fn parse_completion_edit_range(value: &Value) -> Option<Range> {
    value
        .get("range")
        .cloned()
        .or_else(|| value.get("replace").cloned())
        .or_else(|| value.get("insert").cloned())
        .or_else(|| {
            (value.get("start").is_some() && value.get("end").is_some()).then(|| value.clone())
        })
        .and_then(|range| serde_json::from_value::<Range>(range).ok())
}

fn completion_kind_label(kind: u64) -> &'static str {
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
