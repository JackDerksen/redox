use std::path::PathBuf;

use super::{DiagnosticSeverity, IncomingRange, MarketplaceItemId, ProviderId};
use crate::ui::syntax::{LineSyntaxSpan, SyntaxLanguage};

#[derive(Debug, Clone)]
pub struct LspMarketplacePopup {
    pub entries: Vec<LspMarketplaceEntry>,
    pub selected: usize,
    pub scroll: usize,
}

#[derive(Debug, Clone)]
pub struct LspMarketplaceEntry {
    pub(super) item_id: MarketplaceItemId,
    pub language_label: String,
    pub tool_label: String,
    pub installed: bool,
    pub action_label: String,
    pub status_label: String,
    pub status_kind: LspEntryStatusKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspEntryStatusKind {
    Ready,
    Pending,
    Missing,
    Informational,
}

#[derive(Debug, Clone)]
pub struct DiagnosticsPopup {
    pub entries: Vec<DiagnosticsPopupEntry>,
    pub selected: usize,
    pub scroll: usize,
    pub focus: DiagnosticsPopupFocus,
    pub code_actions: Option<DiagnosticsCodeActionsPane>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsPopupEntry {
    pub severity: DiagnosticSeverity,
    pub line: usize,
    pub col: usize,
    pub end_col: usize,
    pub summary: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticsPopupFocus {
    Diagnostics,
    CodeActions,
}

#[derive(Debug, Clone)]
pub struct DiagnosticsCodeActionsPane {
    pub title: String,
    pub entries: Vec<CodeActionPopupEntry>,
    pub selected: usize,
    pub scroll: usize,
    pub loading: bool,
}

#[derive(Debug, Clone)]
pub struct CodeActionPopup {
    pub title: String,
    pub entries: Vec<CodeActionPopupEntry>,
    pub selected: usize,
    pub scroll: usize,
}

#[derive(Debug, Clone)]
pub struct CodeActionPopupEntry {
    pub title: String,
    pub kind: Option<String>,
    pub preferred: bool,
}

#[derive(Debug, Clone)]
pub struct CompletionPopup {
    pub entries: Vec<CompletionEntry>,
    pub selected: usize,
    pub scroll: usize,
}

#[derive(Debug, Clone)]
pub struct CompletionEntry {
    pub kind: Option<String>,
    pub keyword: String,
    pub type_label: Option<String>,
    pub extra: Option<String>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CompletionPreview {
    pub text: String,
    pub suffix: String,
}

#[derive(Debug, Clone, Copy)]
pub struct SymbolInfoPopup<'a> {
    pub title: &'a str,
    pub display_lines: &'a [SymbolInfoDisplayLine],
    pub scroll: usize,
}

#[derive(Debug, Clone)]
pub struct SymbolInfoDisplayLine {
    pub text: String,
    pub kind: SymbolInfoDisplayKind,
    pub spans: Vec<LineSyntaxSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolInfoDisplayKind {
    PlainText,
    Markdown,
    Code { language: Option<SyntaxLanguage> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolInfoBlock {
    pub kind: SymbolInfoKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolInfoKind {
    PlainText,
    Markdown,
    Code { language: Option<String> },
}

#[derive(Debug, Clone)]
pub(super) struct DefinitionTarget {
    pub(super) uri: String,
    pub(super) range: IncomingRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct WorkspaceKey {
    pub(super) provider_id: ProviderId,
    pub(super) root: PathBuf,
}
