use serde_json::Value;

use crate::protocol::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct AvailableCodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub preferred: bool,
    pub edit: Option<WorkspaceEdit>,
    pub command: Option<LspCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEdit {
    pub document_edits: Vec<DocumentEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentEdit {
    pub uri: String,
    pub version: Option<i32>,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LspCommand {
    pub command: String,
    pub arguments: Vec<Value>,
}

/// Parses enabled command and code-action response entries.
#[must_use]
pub fn parse_code_action_response(message: &Value) -> Vec<AvailableCodeAction> {
    message
        .get("result")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_code_action_value)
        .collect()
}

fn parse_code_action_value(value: &Value) -> Option<AvailableCodeAction> {
    if value
        .get("disabled")
        .and_then(|disabled| disabled.get("reason"))
        .is_some()
    {
        return None;
    }

    if value.get("title").is_some() && value.get("command").and_then(Value::as_str).is_some() {
        let title = value.get("title")?.as_str()?.to_string();
        let command = Some(LspCommand {
            command: value.get("command")?.as_str()?.to_string(),
            arguments: value
                .get("arguments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        });
        return Some(AvailableCodeAction {
            title,
            kind: Some("quickfix".to_string()),
            preferred: false,
            edit: None,
            command,
        });
    }

    Some(AvailableCodeAction {
        title: value.get("title")?.as_str()?.to_string(),
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        preferred: value
            .get("isPreferred")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        edit: match value.get("edit") {
            Some(edit) => Some(parse_workspace_edit(edit)?),
            None => None,
        },
        command: value.get("command").and_then(parse_lsp_command),
    })
}

#[must_use]
pub fn parse_workspace_edit(value: &Value) -> Option<WorkspaceEdit> {
    let mut document_edits = Vec::new();

    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            let edits = edits
                .as_array()?
                .iter()
                .map(parse_text_edit)
                .collect::<Option<Vec<_>>>()?;
            if !edits.is_empty() {
                document_edits.push(DocumentEdit {
                    uri: uri.clone(),
                    version: None,
                    edits,
                });
            }
        }
    }

    if let Some(changes) = value.get("documentChanges").and_then(Value::as_array) {
        for entry in changes {
            // Applying only the text-edit portion of a mixed workspace edit can
            // leave the workspace inconsistent. Reject resource operations until
            // the caller can apply them atomically.
            let text_document = entry.get("textDocument")?;
            let uri = text_document.get("uri")?.as_str()?.to_string();
            let version = parse_optional_document_version(text_document.get("version"))?;
            let edits = entry
                .get("edits")
                .and_then(Value::as_array)?
                .iter()
                .map(parse_text_edit)
                .collect::<Option<Vec<_>>>()?;
            if !edits.is_empty() {
                document_edits.push(DocumentEdit {
                    uri,
                    version,
                    edits,
                });
            }
        }
    }

    (!document_edits.is_empty()).then_some(WorkspaceEdit { document_edits })
}

fn parse_optional_document_version(value: Option<&Value>) -> Option<Option<i32>> {
    match value {
        None | Some(Value::Null) => Some(None),
        Some(value) => i32::try_from(value.as_i64()?).ok().map(Some),
    }
}

pub(crate) fn parse_text_edit(value: &Value) -> Option<TextEdit> {
    Some(TextEdit {
        range: serde_json::from_value::<Range>(value.get("range")?.clone()).ok()?,
        new_text: value.get("newText")?.as_str()?.to_string(),
    })
}

fn parse_lsp_command(value: &Value) -> Option<LspCommand> {
    Some(LspCommand {
        command: value.get("command")?.as_str()?.to_string(),
        arguments: value
            .get("arguments")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    })
}
