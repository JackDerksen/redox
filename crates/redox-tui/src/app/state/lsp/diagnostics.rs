use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{WorkspaceKey, file_uri, utf16_code_unit_to_char_col};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

impl DiagnosticSeverity {
    fn from_lsp(value: u64) -> Self {
        match value {
            1 => Self::Error,
            2 => Self::Warning,
            3 => Self::Information,
            _ => Self::Hint,
        }
    }

    pub fn sort_rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Information => 2,
            Self::Hint => 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticSummary {
    pub errors: usize,
    pub warnings: usize,
    pub information: usize,
    pub hints: usize,
}

impl DiagnosticSummary {
    pub fn is_empty(self) -> bool {
        self.errors == 0 && self.warnings == 0 && self.information == 0 && self.hints == 0
    }
}

#[derive(Debug, Clone)]
pub struct DiagnosticLine {
    pub severity: DiagnosticSeverity,
    pub start_col: usize,
    pub end_col: usize,
    pub inline_text: String,
    pub(super) message_count: usize,
}

#[derive(Debug, Clone)]
pub(super) struct StoredDiagnostic {
    pub(super) severity: DiagnosticSeverity,
    pub(super) message: String,
    pub(super) start_line: usize,
    pub(super) end_line: usize,
    pub(super) start_utf16: u32, // Positions are UTF-16 based!!
    pub(super) end_utf16: u32,
}

impl StoredDiagnostic {
    pub(super) fn to_display(&self, buffer: &redox_core::TextBuffer) -> Diagnostic {
        let line = self.start_line.min(buffer.len_lines().saturating_sub(1));
        let end_line = self.end_line.min(buffer.len_lines().saturating_sub(1));
        let start_col = utf16_code_unit_to_char_col(&buffer.line_string(line), self.start_utf16);
        let end_col = if line == end_line {
            utf16_code_unit_to_char_col(&buffer.line_string(line), self.end_utf16)
        } else {
            buffer.line_len_chars(line)
        };

        Diagnostic {
            severity: self.severity,
            message: self.message.clone(),
            line,
            start_col,
            end_col: end_col.max(start_col.saturating_add(1)),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct StoredDiagnostics {
    pub(super) source: DiagnosticSource,
    pub(super) items: Vec<StoredDiagnostic>,
}

#[derive(Debug, Clone)]
pub(super) struct DeferredDiagnostics {
    pub(super) uri: String,
    pub(super) version: Option<i32>,
    pub(super) source: DiagnosticSource,
    pub(super) items: Vec<StoredDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum DiagnosticSource {
    Lsp(WorkspaceKey),
    Lint(LintSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct LintSource {
    pub(super) kind: LintRunnerKind,
    pub(super) root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LintRunnerKind {
    Clippy,
    GolangciLint,
    Ruff,
}

impl LintRunnerKind {
    pub(super) fn executable(self) -> &'static str {
        match self {
            Self::Clippy => "cargo-clippy",
            Self::GolangciLint => "golangci-lint",
            Self::Ruff => "ruff",
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct PublishDiagnosticsParams {
    pub(super) uri: String,
    pub(super) version: Option<i32>,
    pub(super) diagnostics: Vec<IncomingDiagnostic>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IncomingDiagnostic {
    pub(super) range: IncomingRange,
    pub(super) severity: Option<u64>,
    pub(super) message: String,
    #[serde(default, rename = "relatedInformation")]
    pub(super) related_information: Vec<IncomingDiagnosticRelatedInformation>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IncomingDiagnosticRelatedInformation {
    pub(super) message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct IncomingRange {
    pub(super) start: IncomingPosition,
    pub(super) end: IncomingPosition,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct IncomingPosition {
    pub(super) line: u64,
    pub(super) character: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct IncomingLocation {
    pub(super) uri: String,
    pub(super) range: IncomingRange,
}

#[derive(Debug, Deserialize)]
pub(super) struct IncomingLocationLink {
    #[serde(rename = "targetUri")]
    pub(super) target_uri: String,
    #[serde(rename = "targetSelectionRange")]
    pub(super) target_selection_range: IncomingRange,
}

pub(super) fn configuration_response(message: &Value) -> Value {
    let item_count = message
        .get("params")
        .and_then(|params| params.get("items"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Value::Array(vec![Value::Null; item_count])
}

pub(super) fn workspace_folders_response(workspace: &WorkspaceKey) -> Value {
    let Ok(uri) = file_uri(&workspace.root) else {
        return Value::Null;
    };
    let name = workspace
        .root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    json!([
        {
            "uri": uri,
            "name": name
        }
    ])
}

#[derive(Debug, Deserialize)]
pub(super) struct CargoCompilerMessage {
    pub(super) reason: String,
    pub(super) message: Option<RustcDiagnosticMessage>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RustcDiagnosticMessage {
    pub(super) level: String,
    pub(super) message: String,
    pub(super) spans: Vec<RustcDiagnosticSpan>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RustcDiagnosticSpan {
    pub(super) file_name: String,
    pub(super) line_start: usize,
    pub(super) line_end: usize,
    pub(super) column_start: usize,
    pub(super) column_end: usize,
    pub(super) is_primary: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct GolangciLintReport {
    #[serde(default, rename = "Issues")]
    pub(super) issues: Vec<GolangciLintIssue>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GolangciLintIssue {
    #[serde(default, rename = "FromLinter")]
    pub(super) from_linter: String,
    #[serde(rename = "Text")]
    pub(super) text: String,
    #[serde(default, rename = "Severity")]
    pub(super) severity: Option<String>,
    #[serde(rename = "Pos")]
    pub(super) pos: GolangciLintPosition,
}

#[derive(Debug, Deserialize)]
pub(super) struct GolangciLintPosition {
    #[serde(rename = "Filename")]
    pub(super) filename: String,
    #[serde(rename = "Line")]
    pub(super) line: usize,
    #[serde(rename = "Column")]
    pub(super) column: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct RuffDiagnostic {
    pub(super) filename: String,
    pub(super) message: String,
    pub(super) code: Option<String>,
    pub(super) location: RuffPosition,
    pub(super) end_location: RuffPosition,
}

#[derive(Debug, Deserialize)]
pub(super) struct RuffPosition {
    pub(super) row: usize,
    pub(super) column: usize,
}

pub(super) fn parse_publish_diagnostics(
    message: &Value,
) -> Option<(String, Option<i32>, Vec<StoredDiagnostic>)> {
    let method = message.get("method")?.as_str()?;
    if method != "textDocument/publishDiagnostics" {
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

fn parse_diagnostic(diagnostic: IncomingDiagnostic) -> Option<StoredDiagnostic> {
    let severity = diagnostic
        .severity
        .map(DiagnosticSeverity::from_lsp)
        .unwrap_or(DiagnosticSeverity::Warning);
    let message =
        diagnostic_message_with_details(diagnostic.message, diagnostic.related_information);
    Some(StoredDiagnostic {
        severity,
        message,
        start_line: usize::try_from(diagnostic.range.start.line).ok()?,
        end_line: usize::try_from(diagnostic.range.end.line).ok()?,
        start_utf16: u32::try_from(diagnostic.range.start.character).ok()?,
        end_utf16: u32::try_from(diagnostic.range.end.character).ok()?,
    })
}

fn diagnostic_message_with_details(
    message: String,
    related_information: Vec<IncomingDiagnosticRelatedInformation>,
) -> String {
    let details = related_information
        .into_iter()
        .map(|detail| detail.message.trim().to_string())
        .filter(|detail| !detail.is_empty())
        .collect::<Vec<_>>();

    let base = strip_trailing_see_details_marker(message.trim());
    if details.is_empty() {
        return base.to_string();
    }

    let mut output = base.to_string();
    output.push_str("\n\nDetails:");
    for detail in details {
        output.push_str("\n- ");
        output.push_str(&detail);
    }
    output
}

fn strip_trailing_see_details_marker(message: &str) -> &str {
    let marker = "(see details)";
    let trimmed = message.trim_end();
    if trimmed.to_ascii_lowercase().ends_with(marker) {
        trimmed[..trimmed.len().saturating_sub(marker.len())].trim_end()
    } else {
        trimmed
    }
}

pub(super) fn should_suppress_lint_diagnostics<'a>(
    entries: impl IntoIterator<Item = (&'a DiagnosticSource, &'a StoredDiagnostic)>,
) -> bool {
    entries.into_iter().any(|(source, diagnostic)| {
        !matches!(source, DiagnosticSource::Lint(_))
            && diagnostic.severity == DiagnosticSeverity::Error
    })
}

pub(super) fn clip_diagnostic_message(message: &str) -> String {
    const MAX_INLINE_CHARS: usize = 64;
    let clipped = diagnostic_summary_line(message);
    if clipped.chars().count() <= MAX_INLINE_CHARS {
        clipped
    } else {
        let mut out = clipped
            .chars()
            .take(MAX_INLINE_CHARS.saturating_sub(1))
            .collect::<String>();
        out.push('…');
        out
    }
}

pub(super) fn diagnostic_summary_line(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| message.trim())
        .to_string()
}
