use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use serde::Deserialize;

use crate::diagnostics::{Diagnostic, DiagnosticSeverity};
use crate::protocol::char_col_to_utf16;
use crate::provider::executable_on_path;
use crate::workspace::file_uri;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LintRunnerKind {
    Clippy,
    GolangciLint,
    Ruff,
}

impl LintRunnerKind {
    #[must_use]
    pub const fn executable(self) -> &'static str {
        match self {
            Self::Clippy => "cargo-clippy",
            Self::GolangciLint => "golangci-lint",
            Self::Ruff => "ruff",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clippy => "Clippy",
            Self::GolangciLint => "golangci-lint",
            Self::Ruff => "Ruff",
        }
    }
}

impl FromStr for LintRunnerKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "cargo" | "clippy" => Ok(Self::Clippy),
            "golangci-lint" => Ok(Self::GolangciLint),
            "ruff" => Ok(Self::Ruff),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LintSource {
    pub kind: LintRunnerKind,
    pub root: PathBuf,
}

#[derive(Debug)]
pub struct LintRunResult {
    pub source: LintSource,
    pub diagnostics_by_uri: HashMap<String, Vec<Diagnostic>>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CargoCompilerMessage {
    reason: String,
    message: Option<RustcDiagnosticMessage>,
}

#[derive(Debug, Deserialize)]
struct RustcDiagnosticMessage {
    level: String,
    message: String,
    spans: Vec<RustcDiagnosticSpan>,
}

#[derive(Debug, Deserialize)]
struct RustcDiagnosticSpan {
    file_name: String,
    line_start: usize,
    line_end: usize,
    column_start: usize,
    column_end: usize,
    is_primary: bool,
}

#[derive(Debug, Deserialize)]
struct GolangciLintReport {
    #[serde(default, rename = "Issues")]
    issues: Vec<GolangciLintIssue>,
}

#[derive(Debug, Deserialize)]
struct GolangciLintIssue {
    #[serde(default, rename = "FromLinter")]
    from_linter: String,
    #[serde(rename = "Text")]
    text: String,
    #[serde(default, rename = "Severity")]
    severity: Option<String>,
    #[serde(rename = "Pos")]
    pos: GolangciLintPosition,
}

#[derive(Debug, Deserialize)]
struct GolangciLintPosition {
    #[serde(rename = "Filename")]
    filename: String,
    #[serde(rename = "Line")]
    line: usize,
    #[serde(rename = "Column")]
    column: usize,
}

#[derive(Debug, Deserialize)]
struct RuffDiagnostic {
    filename: String,
    message: String,
    code: Option<String>,
    location: RuffPosition,
    end_location: RuffPosition,
}

#[derive(Debug, Deserialize)]
struct RuffPosition {
    row: usize,
    column: usize,
}

#[must_use]
pub fn lint_runner_available(source: &LintSource, path: &Path) -> bool {
    match source.kind {
        LintRunnerKind::Clippy => {
            crate::provider::tool_available(source.kind.executable())
                && source.root.join("Cargo.toml").exists()
        }
        LintRunnerKind::GolangciLint => {
            executable_on_path(source.kind.executable()) && path.starts_with(&source.root)
        }
        LintRunnerKind::Ruff => executable_on_path(source.kind.executable()),
    }
}

/// Runs one configured linter and parses its output by file URI.
#[must_use]
pub fn run_linter(source: &LintSource, path: &Path) -> LintRunResult {
    let output = match source.kind {
        LintRunnerKind::Clippy => Command::new("cargo")
            .args([
                "clippy",
                "--message-format=json",
                "--all-targets",
                "--all-features",
            ])
            .current_dir(&source.root)
            .output(),
        LintRunnerKind::GolangciLint => Command::new("golangci-lint")
            .args([
                "run",
                "--output.json.path",
                "stdout",
                "--output.text.path",
                "stderr",
                "./...",
            ])
            .current_dir(&source.root)
            .output(),
        LintRunnerKind::Ruff => Command::new("ruff")
            .args(["check", "--output-format", "json"])
            .arg(path)
            .current_dir(&source.root)
            .output(),
    };

    match output {
        Ok(output) => {
            let diagnostics_by_uri = match source.kind {
                LintRunnerKind::Clippy => parse_clippy_output(&output.stdout, &source.root),
                LintRunnerKind::GolangciLint => {
                    let mut diagnostics = parse_golangci_lint_output(&output.stdout, &source.root);
                    if diagnostics.is_empty() {
                        diagnostics = parse_golangci_lint_text_output(&output.stderr, &source.root);
                    }
                    diagnostics
                }
                LintRunnerKind::Ruff => parse_ruff_output(&output.stdout, &source.root),
            };
            let parsed_any = diagnostics_by_uri.values().any(|items| !items.is_empty());
            let error = if output.status.success() || parsed_any {
                None
            } else {
                Some(format!(
                    "{} failed: {}",
                    source.kind.label(),
                    first_non_empty_output_line(&output.stderr, &output.stdout)
                ))
            };
            LintRunResult {
                source: source.clone(),
                diagnostics_by_uri,
                error,
            }
        }
        Err(error) => LintRunResult {
            source: source.clone(),
            diagnostics_by_uri: HashMap::new(),
            error: Some(format!("failed to start {}: {error}", source.kind.label())),
        },
    }
}

#[must_use]
pub fn parse_clippy_output(stdout: &[u8], root: &Path) -> HashMap<String, Vec<Diagnostic>> {
    let mut diagnostics_by_uri = HashMap::<String, Vec<Diagnostic>>::new();
    let mut line_cache = HashMap::<PathBuf, Option<Vec<String>>>::new();

    for line in String::from_utf8_lossy(stdout).lines() {
        if !line.trim_start().starts_with('{') {
            continue;
        }
        let Ok(message) = serde_json::from_str::<CargoCompilerMessage>(line) else {
            continue;
        };
        if message.reason != "compiler-message" {
            continue;
        }
        let Some(message) = message.message else {
            continue;
        };
        let Some(span) = message
            .spans
            .iter()
            .find(|span| span.is_primary)
            .or_else(|| message.spans.first())
        else {
            continue;
        };

        let file_path = resolve_lint_path(root, Path::new(&span.file_name));
        let Ok(uri) = file_uri(&file_path) else {
            continue;
        };
        let Some(diagnostic) = diagnostic_from_char_span(
            &file_path,
            severity_from_text(&message.level),
            message.message,
            span.line_start,
            span.line_end,
            span.column_start,
            span.column_end,
            &mut line_cache,
        ) else {
            continue;
        };
        diagnostics_by_uri.entry(uri).or_default().push(diagnostic);
    }
    diagnostics_by_uri
}

#[must_use]
pub fn parse_golangci_lint_output(stdout: &[u8], root: &Path) -> HashMap<String, Vec<Diagnostic>> {
    let Ok(report) = serde_json::from_slice::<GolangciLintReport>(stdout) else {
        return HashMap::new();
    };
    let mut diagnostics_by_uri = HashMap::<String, Vec<Diagnostic>>::new();
    let mut line_cache = HashMap::<PathBuf, Option<Vec<String>>>::new();

    for issue in report.issues {
        let file_path = resolve_lint_path(root, Path::new(&issue.pos.filename));
        let Ok(uri) = file_uri(&file_path) else {
            continue;
        };
        let severity = issue
            .severity
            .as_deref()
            .map(severity_from_text)
            .unwrap_or(DiagnosticSeverity::Warning);
        let message = if issue.from_linter.trim().is_empty() {
            issue.text
        } else {
            format!("{}: {}", issue.from_linter, issue.text)
        };
        let Some(diagnostic) = diagnostic_from_char_span(
            &file_path,
            severity,
            message,
            issue.pos.line,
            issue.pos.line,
            issue.pos.column,
            issue.pos.column.saturating_add(1),
            &mut line_cache,
        ) else {
            continue;
        };
        diagnostics_by_uri.entry(uri).or_default().push(diagnostic);
    }
    diagnostics_by_uri
}

#[must_use]
pub fn parse_golangci_lint_text_output(
    stderr: &[u8],
    root: &Path,
) -> HashMap<String, Vec<Diagnostic>> {
    let mut diagnostics_by_uri = HashMap::<String, Vec<Diagnostic>>::new();
    let mut line_cache = HashMap::<PathBuf, Option<Vec<String>>>::new();

    for line in String::from_utf8_lossy(stderr).lines() {
        let Some((path_part, line_number, column_number, message)) =
            parse_colon_diagnostic_line(line)
        else {
            continue;
        };
        let file_path = resolve_lint_path(root, Path::new(path_part));
        let Ok(uri) = file_uri(&file_path) else {
            continue;
        };
        let Some(diagnostic) = diagnostic_from_char_span(
            &file_path,
            DiagnosticSeverity::Warning,
            message.to_string(),
            line_number,
            line_number,
            column_number,
            column_number.saturating_add(1),
            &mut line_cache,
        ) else {
            continue;
        };
        diagnostics_by_uri.entry(uri).or_default().push(diagnostic);
    }
    diagnostics_by_uri
}

#[must_use]
pub fn parse_ruff_output(stdout: &[u8], root: &Path) -> HashMap<String, Vec<Diagnostic>> {
    let Ok(diagnostics) = serde_json::from_slice::<Vec<RuffDiagnostic>>(stdout) else {
        return HashMap::new();
    };
    let mut diagnostics_by_uri = HashMap::<String, Vec<Diagnostic>>::new();
    let mut line_cache = HashMap::<PathBuf, Option<Vec<String>>>::new();

    for diagnostic in diagnostics {
        let file_path = resolve_lint_path(root, Path::new(&diagnostic.filename));
        let Ok(uri) = file_uri(&file_path) else {
            continue;
        };
        let message = diagnostic
            .code
            .as_deref()
            .map(|code| format!("{code}: {}", diagnostic.message))
            .unwrap_or(diagnostic.message);
        let Some(parsed) = diagnostic_from_char_span(
            &file_path,
            DiagnosticSeverity::Warning,
            message,
            diagnostic.location.row,
            diagnostic.end_location.row,
            diagnostic.location.column,
            diagnostic.end_location.column,
            &mut line_cache,
        ) else {
            continue;
        };
        diagnostics_by_uri.entry(uri).or_default().push(parsed);
    }
    diagnostics_by_uri
}

fn parse_colon_diagnostic_line(line: &str) -> Option<(&str, usize, usize, &str)> {
    let (path_part, rest) = line.split_once(':')?;
    let (line_part, rest) = rest.split_once(':')?;
    let (column_part, message) = rest.split_once(':')?;
    let line_number = line_part.trim().parse::<usize>().ok()?;
    let column_number = column_part.trim().parse::<usize>().ok()?;
    let message = message.trim();
    if path_part.trim().is_empty() || message.is_empty() {
        return None;
    }
    Some((path_part.trim(), line_number, column_number, message))
}

fn severity_from_text(level: &str) -> DiagnosticSeverity {
    match level {
        "error" => DiagnosticSeverity::Error,
        "warning" | "warn" => DiagnosticSeverity::Warning,
        "note" | "info" | "information" => DiagnosticSeverity::Information,
        "help" | "hint" => DiagnosticSeverity::Hint,
        _ => DiagnosticSeverity::Warning,
    }
}

fn resolve_lint_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[allow(clippy::too_many_arguments)]
fn diagnostic_from_char_span(
    path: &Path,
    severity: DiagnosticSeverity,
    message: String,
    start_line_one_based: usize,
    end_line_one_based: usize,
    start_col_one_based: usize,
    end_col_one_based: usize,
    line_cache: &mut HashMap<PathBuf, Option<Vec<String>>>,
) -> Option<Diagnostic> {
    let start_line = start_line_one_based.checked_sub(1)?;
    let end_line = end_line_one_based.checked_sub(1)?;
    let start_col = start_col_one_based.saturating_sub(1);
    let mut end_col = end_col_one_based.saturating_sub(1);
    if start_line == end_line {
        end_col = end_col.max(start_col.saturating_add(1));
    }

    let start_utf16 = char_col_to_utf16_in_file(path, start_line, start_col, line_cache)?;
    let mut end_utf16 = char_col_to_utf16_in_file(path, end_line, end_col, line_cache)?;
    if start_line == end_line {
        end_utf16 = end_utf16.max(start_utf16.saturating_add(1));
    }
    Some(Diagnostic {
        severity,
        message,
        start_line,
        end_line,
        start_utf16,
        end_utf16,
        related_information: Vec::new(),
    })
}

fn char_col_to_utf16_in_file(
    path: &Path,
    line_index: usize,
    char_col: usize,
    line_cache: &mut HashMap<PathBuf, Option<Vec<String>>>,
) -> Option<u32> {
    let lines = cached_file_lines(path, line_cache)?;
    let line = lines.get(line_index)?;
    Some(char_col_to_utf16(line, char_col.min(line.chars().count())))
}

fn cached_file_lines<'a>(
    path: &Path,
    line_cache: &'a mut HashMap<PathBuf, Option<Vec<String>>>,
) -> Option<&'a [String]> {
    let entry = line_cache.entry(path.to_path_buf()).or_insert_with(|| {
        fs::read_to_string(path)
            .ok()
            .map(|text| text.split('\n').map(str::to_string).collect())
    });
    entry.as_deref()
}

fn first_non_empty_output_line(primary: &[u8], secondary: &[u8]) -> String {
    let primary_text = String::from_utf8_lossy(primary);
    if let Some(line) = primary_text.lines().find(|line| !line.trim().is_empty()) {
        return line.to_string();
    }
    let secondary_text = String::from_utf8_lossy(secondary);
    secondary_text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("unknown error")
        .to_string()
}
