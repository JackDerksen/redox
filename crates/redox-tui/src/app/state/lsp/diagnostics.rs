use redox_lsp::LintSource;
use redox_lsp::protocol::utf16_code_unit_to_char_col;

use super::WorkspaceKey;

pub(super) use redox_lsp::Diagnostic as StoredDiagnostic;
pub use redox_lsp::DiagnosticSeverity;

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

pub(super) trait StoredDiagnosticExt {
    fn to_display(&self, buffer: &redox_core::TextBuffer) -> Diagnostic;
}

impl StoredDiagnosticExt for StoredDiagnostic {
    fn to_display(&self, buffer: &redox_core::TextBuffer) -> Diagnostic {
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
            message: diagnostic_message_with_details(&self.message, &self.related_information),
            line,
            start_col,
            end_col: end_col.max(start_col.saturating_add(1)),
        }
    }
}

fn diagnostic_message_with_details(
    message: &str,
    related_information: &[redox_lsp::DiagnosticRelatedInformation],
) -> String {
    let details = related_information
        .iter()
        .map(|information| information.message.trim())
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
        output.push_str(detail);
    }
    output
}

fn strip_trailing_see_details_marker(message: &str) -> &str {
    const MARKER: &str = "(see details)";
    let trimmed = message.trim_end();
    if trimmed.to_ascii_lowercase().ends_with(MARKER) {
        trimmed[..trimmed.len().saturating_sub(MARKER.len())].trim_end()
    } else {
        trimmed
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
        let mut output = clipped
            .chars()
            .take(MAX_INLINE_CHARS.saturating_sub(1))
            .collect::<String>();
        output.push('…');
        output
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
