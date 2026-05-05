use std::path::PathBuf;

use super::{DiagnosticSeverity, IncomingRange, MarketplaceItemId, ProviderId};

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
}

#[derive(Debug, Clone)]
pub struct DiagnosticsPopupEntry {
    pub severity: DiagnosticSeverity,
    pub line: usize,
    pub col: usize,
    pub summary: String,
    pub message: String,
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
