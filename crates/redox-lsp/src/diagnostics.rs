use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::protocol::Range;
use crate::workspace::file_uri;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

impl DiagnosticSeverity {
    #[must_use]
    pub const fn from_lsp(value: u64) -> Self {
        match value {
            1 => Self::Error,
            2 => Self::Warning,
            3 => Self::Information,
            _ => Self::Hint,
        }
    }

    #[must_use]
    pub const fn sort_rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Information => 2,
            Self::Hint => 3,
        }
    }
}

/// A diagnostic kept in LSP coordinates until a frontend applies it to a
/// particular buffer version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_utf16: u32,
    pub end_utf16: u32,
    pub related_information: Vec<DiagnosticRelatedInformation>,
}

/// A secondary diagnostic message with its original source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRelatedInformation {
    pub location: DiagnosticLocation,
    pub message: String,
}

/// A URI and range referenced by a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLocation {
    pub uri: String,
    pub range: Range,
}

#[derive(Debug, Deserialize)]
struct PublishDiagnosticsParams {
    uri: String,
    version: Option<i32>,
    diagnostics: Vec<IncomingDiagnostic>,
}

#[derive(Debug, Deserialize)]
struct IncomingDiagnostic {
    range: Range,
    severity: Option<u64>,
    message: String,
    #[serde(default, rename = "relatedInformation")]
    related_information: Option<Vec<RelatedInformation>>,
}

#[derive(Debug, Deserialize)]
struct RelatedInformation {
    location: IncomingLocation,
    message: String,
}

#[derive(Debug, Deserialize)]
struct IncomingLocation {
    uri: String,
    range: Range,
}

/// Parses a `textDocument/publishDiagnostics` notification.
#[must_use]
pub fn parse_publish_diagnostics(
    message: &Value,
) -> Option<(String, Option<i32>, Vec<Diagnostic>)> {
    if message.get("method")?.as_str()? != "textDocument/publishDiagnostics" {
        return None;
    }

    let params =
        serde_json::from_value::<PublishDiagnosticsParams>(message.get("params")?.clone()).ok()?;
    let diagnostics = params
        .diagnostics
        .into_iter()
        .filter_map(parse_diagnostic)
        .collect();
    Some((params.uri, params.version, diagnostics))
}

#[must_use]
pub fn configuration_response(message: &Value) -> Value {
    let item_count = message
        .get("params")
        .and_then(|params| params.get("items"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Value::Array(vec![Value::Null; item_count])
}

#[must_use]
pub fn workspace_folders_response(root: &Path) -> Value {
    let Ok(uri) = file_uri(root) else {
        return Value::Null;
    };
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    json!([{ "uri": uri, "name": name }])
}

fn parse_diagnostic(diagnostic: IncomingDiagnostic) -> Option<Diagnostic> {
    let severity = diagnostic
        .severity
        .map(DiagnosticSeverity::from_lsp)
        .unwrap_or(DiagnosticSeverity::Warning);
    Some(Diagnostic {
        severity,
        message: diagnostic.message,
        start_line: usize::try_from(diagnostic.range.start.line).ok()?,
        end_line: usize::try_from(diagnostic.range.end.line).ok()?,
        start_utf16: u32::try_from(diagnostic.range.start.character).ok()?,
        end_utf16: u32::try_from(diagnostic.range.end.character).ok()?,
        related_information: diagnostic
            .related_information
            .unwrap_or_default()
            .into_iter()
            .map(|information| DiagnosticRelatedInformation {
                location: DiagnosticLocation {
                    uri: information.location.uri,
                    range: information.location.range,
                },
                message: information.message,
            })
            .collect(),
    })
}
