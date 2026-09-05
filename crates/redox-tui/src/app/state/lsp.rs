use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use redox_core::{BufferId, BufferKind, Pos};
#[cfg(test)]
use redox_lsp::InsertTextFormat;
use redox_lsp::diagnostics::{configuration_response, workspace_folders_response};
use redox_lsp::protocol::{
    Position as IncomingPosition, Range as IncomingRange, char_col_to_utf16,
    utf16_code_unit_to_char_col,
};
use redox_lsp::{
    AvailableCodeAction, Client as LspSession, ClientEvent as SessionEvent, ClientInfo,
    CompletionCandidate, DefinitionTarget, InstallMethod, InstallPlan, Language as LspLanguage,
    LintRunResult, LintRunnerKind, LintSource, LinterSpec, ProviderId, ProviderSpec, Uninstall,
    WorkspaceEdit, completion_snippet_expansion, file_path_from_uri, file_uri,
    install_method_available, install_tool, lint_runner_available, linter_spec,
    parse_code_action_response, parse_completion_response, parse_definition_response,
    parse_hover_response, parse_publish_diagnostics, parse_workspace_edit, provider_spec,
    run_linter as run_lint_source, tool_available, uninstall_tool, workspace_root_for,
};
use serde_json::{Value, json};

use super::{EditorMode, EditorState, StatusMessageStyle};
use crate::ui::build_symbol_info_display_lines;
use crate::ui::language_for_path;
use crate::ui::style::SyntaxRole;
use crate::ui::symbol_info_content_width_limit;
use crate::ui::syntax::{SyntaxLanguage, lexical_fallback_line_spans};

mod completion;
use completion::*;
mod code_actions;
use code_actions::*;
mod diagnostics;
use diagnostics::*;
pub use diagnostics::{DiagnosticLine, DiagnosticSeverity, DiagnosticSummary};
mod types;
use self::types::{LspMarketplaceEntry, WorkspaceKey};
pub use redox_lsp::{SymbolInfoBlock, SymbolInfoKind};
pub use types::{
    CodeActionPopup, CodeActionPopupEntry, CompletionEntry, CompletionPopup, CompletionPreview,
    DiagnosticsCodeActionsPane, DiagnosticsPopup, DiagnosticsPopupEntry, DiagnosticsPopupFocus,
    LspEntryStatusKind, LspMarketplacePopup, SymbolInfoDisplayKind, SymbolInfoDisplayLine,
    SymbolInfoPopup,
};

const LSP_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MAX_LSP_EVENTS_PER_WORKSPACE_POLL: usize = 256;
const DIAGNOSTICS_POPUP_VISIBLE_ROWS: usize = 12;
const COMPLETION_POPUP_VISIBLE_ROWS: usize = 8;
const SYMBOL_INFO_MAX_HEIGHT: usize = 12;
const LSP_CHANGE_DEBOUNCE: Duration = Duration::from_millis(175);
const COMPLETION_AUTO_TRIGGER_DEBOUNCE: Duration = Duration::from_millis(35);
const COMPLETION_TRIGGER_CHARACTER_DEBOUNCE: Duration = Duration::ZERO;
const LSP_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const LSP_INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
const LSP_RETRY_DELAY: Duration = Duration::from_secs(5);

const PROVIDERS: &[ProviderSpec] = redox_lsp::provider::built_in_providers();
const LINTERS: &[LinterSpec] = redox_lsp::provider::built_in_linters();

type InstallMethodId = InstallMethod;
type ProviderInstallPlan = InstallPlan;

fn lsp_language(language: SyntaxLanguage) -> LspLanguage {
    match language {
        SyntaxLanguage::C => LspLanguage::C,
        SyntaxLanguage::Cpp => LspLanguage::Cpp,
        SyntaxLanguage::Css => LspLanguage::Css,
        SyntaxLanguage::Go => LspLanguage::Go,
        SyntaxLanguage::Html => LspLanguage::Html,
        SyntaxLanguage::JavaScript => LspLanguage::JavaScript,
        SyntaxLanguage::Json => LspLanguage::Json,
        SyntaxLanguage::Lua => LspLanguage::Lua,
        SyntaxLanguage::Markdown => LspLanguage::Markdown,
        SyntaxLanguage::Python => LspLanguage::Python,
        SyntaxLanguage::Rust => LspLanguage::Rust,
        SyntaxLanguage::Toml => LspLanguage::Toml,
        SyntaxLanguage::TypeScript => LspLanguage::TypeScript,
        SyntaxLanguage::Tsx => LspLanguage::Tsx,
        SyntaxLanguage::Yaml => LspLanguage::Yaml,
    }
}

#[derive(Debug, Clone, Copy)]
enum MarketplaceSpec {
    Provider(ProviderSpec),
    Linter(LinterSpec),
}

impl MarketplaceSpec {
    fn id(self) -> MarketplaceItemId {
        match self {
            Self::Provider(provider) => MarketplaceItemId::Provider(provider.id),
            Self::Linter(linter) => MarketplaceItemId::Linter(linter.kind),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Provider(provider) => provider.label,
            Self::Linter(linter) => linter.label,
        }
    }

    fn language_label(self) -> &'static str {
        match self {
            Self::Provider(provider) => provider.language_label,
            Self::Linter(linter) => linter.language_label,
        }
    }

    fn install_plans(self) -> &'static [ProviderInstallPlan] {
        match self {
            Self::Provider(provider) => provider.install_plans,
            Self::Linter(linter) => linter.install_plans,
        }
    }

    fn executable(self) -> &'static str {
        match self {
            Self::Provider(provider) => provider.executable,
            Self::Linter(linter) => linter.kind.executable(),
        }
    }
}

#[derive(Debug)]
struct PendingLintRun {
    request: QueuedLintRun,
    receiver: Receiver<LintRunResult>,
}

#[derive(Debug, Clone)]
struct QueuedLintRun {
    source: LintSource,
    path: PathBuf,
    uri: String,
    buffer_id: BufferId,
    analysis_version: u64,
}

#[derive(Default)]
pub(super) struct LspState {
    installed: HashMap<MarketplaceItemId, InstalledToolRecord>,
    tool_availability: HashMap<MarketplaceItemId, bool>,
    marketplace: Option<LspMarketplaceState>,
    diagnostics_popup: Option<DiagnosticsPopupState>,
    code_actions_popup: Option<CodeActionsPopupState>,
    completion: Option<CompletionState>,
    auto_completion: Option<AutoCompletionRequest>,
    active_snippet: Option<ActiveSnippet>,
    symbol_info: Option<SymbolInfoState>,
    recent_completions: HashMap<String, u32>,
    clients: HashMap<WorkspaceKey, ManagedClient>,
    retry_after: HashMap<WorkspaceKey, Instant>,
    documents: HashMap<BufferId, ManagedDocument>,
    diagnostics: HashMap<String, Vec<StoredDiagnostics>>,
    deferred_diagnostics: Vec<DeferredDiagnostics>,
    lint_runs: Vec<PendingLintRun>,
    queued_lint_runs: Vec<QueuedLintRun>,
    pending_requests: HashMap<RequestKey, PendingClientRequest>,
    provider_operations: HashMap<MarketplaceItemId, ProviderOperation>,
}

impl std::fmt::Debug for LspState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspState")
            .field("installed", &self.installed)
            .field("marketplace", &self.marketplace)
            .field("client_count", &self.clients.len())
            .field("document_count", &self.documents.len())
            .field("diagnostic_documents", &self.diagnostics.len())
            .field("pending_lint_runs", &self.lint_runs.len())
            .field("queued_lint_runs", &self.queued_lint_runs.len())
            .field("provider_operations", &self.provider_operations.len())
            .finish()
    }
}

#[derive(Debug, Clone)]
struct LspMarketplaceState {
    selected: usize,
    scroll: usize,
}

#[derive(Debug, Clone)]
struct DiagnosticsPopupState {
    code_action_origin: Option<CodeActionOrigin>,
    selected: usize,
    code_actions: Option<CodeActionsPaneState>,
    focus: DiagnosticsPopupFocus,
    cached_code_actions: Option<CachedCodeActions>,
    pending_code_actions: Option<PendingDiagnosticsCodeActions>,
}

#[derive(Debug, Clone)]
struct SymbolInfoState {
    requested_at: Pos,
    blocks: Vec<SymbolInfoBlock>,
    cached_width: Option<usize>,
    display_lines: Vec<SymbolInfoDisplayLine>,
    scroll: usize,
    return_mode: EditorMode,
}

#[derive(Debug, Clone)]
struct InstalledToolRecord {
    install_source: Option<InstallMethodId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MarketplaceItemId {
    Provider(ProviderId),
    Linter(LintRunnerKind),
}

impl MarketplaceItemId {
    fn kind_label(self) -> &'static str {
        match self {
            Self::Provider(_) => "LSP",
            Self::Linter(_) => "Linter",
        }
    }

    fn id_str(self) -> &'static str {
        match self {
            Self::Provider(provider_id) => provider_id.as_str(),
            Self::Linter(LintRunnerKind::Clippy) => "clippy",
            Self::Linter(kind) => kind.executable(),
        }
    }

    fn persistent_kind(self) -> &'static str {
        match self {
            Self::Provider(_) => "lsp",
            Self::Linter(_) => "linter",
        }
    }
}

#[derive(Debug, Clone)]
struct ManagedDocument {
    workspace: WorkspaceKey,
    path: PathBuf,
    uri: String,
    language_id: &'static str,
    document_version: i32,
    last_sent_analysis_version: Option<u64>,
    last_sent_text: Option<String>,
    pending_sync_since: Option<Instant>,
    pending_sync_analysis_version: Option<u64>,
    opened: bool,
}

#[derive(Debug, Clone, Copy)]
enum SyncPolicy {
    Immediate,
    Debounced { now: Instant },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RequestKey {
    workspace: WorkspaceKey,
    id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingRequest {
    GotoDefinition,
    CodeActions {
        origin: CodeActionOrigin,
        requested_at: Pos,
        return_mode: EditorMode,
        title: String,
        trigger: CodeActionRequestTrigger,
    },
    ExecuteCommand {
        title: String,
        edit_applied: bool,
    },
    SymbolInfo {
        requested_at: Pos,
        return_mode: EditorMode,
    },
    Completion {
        requested_at: Pos,
        manual: bool,
    },
}

fn pending_request_same_family(left: &PendingRequest, right: &PendingRequest) -> bool {
    matches!(
        (left, right),
        (
            PendingRequest::GotoDefinition,
            PendingRequest::GotoDefinition
        ) | (
            PendingRequest::CodeActions { .. },
            PendingRequest::CodeActions { .. }
        ) | (
            PendingRequest::ExecuteCommand { .. },
            PendingRequest::ExecuteCommand { .. }
        ) | (
            PendingRequest::SymbolInfo { .. },
            PendingRequest::SymbolInfo { .. }
        ) | (
            PendingRequest::Completion { .. },
            PendingRequest::Completion { .. }
        )
    )
}

#[derive(Debug, Clone)]
struct PendingClientRequest {
    kind: PendingRequest,
    started_at: Instant,
    context: RequestContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequestContext {
    buffer_id: BufferId,
    analysis_version: u64,
    cursor: Pos,
    mode: EditorMode,
}

struct ProviderOperation {
    kind: ProviderOperationKind,
    started_at: Instant,
    receiver: Receiver<ProviderOperationResult>,
}

impl std::fmt::Debug for ProviderOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderOperation")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderOperationKind {
    Installing,
    Uninstalling,
}

#[derive(Debug)]
struct ProviderOperationResult {
    item_id: MarketplaceItemId,
    kind: ProviderOperationKind,
    install_source: Option<InstallMethodId>,
    success: bool,
    message: String,
}

struct ManagedClient {
    provider: ProviderSpec,
    session: LspSession,
    loading_since: Instant,
}

impl std::fmt::Debug for ManagedClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedClient")
            .field("provider", &self.provider.label)
            .finish()
    }
}

impl EditorState {
    pub(in crate::app::state) fn lsp_statusline_return_mode(&self) -> Option<EditorMode> {
        match self.mode {
            EditorMode::CodeActions => self
                .lsp
                .code_actions_popup
                .as_ref()
                .map(|popup| popup.return_mode),
            EditorMode::SymbolInfo => self.lsp.symbol_info.as_ref().map(|popup| popup.return_mode),
            _ => None,
        }
    }

    pub fn symbol_info_popup_is_visible(&self) -> bool {
        self.mode == EditorMode::SymbolInfo && self.lsp.symbol_info.is_some()
    }

    pub fn completion_popup(&self) -> Option<CompletionPopup> {
        let state = self.visible_completion_state()?;
        let buffer = self.session.buffer(self.session.active_id())?;
        let prefix = completion_prefix(buffer, self.active_cursor_pos());
        let max_selected = state.items.len().saturating_sub(1);
        let selected = state.selected.min(max_selected);
        let scroll = selected.saturating_sub(COMPLETION_POPUP_VISIBLE_ROWS.saturating_sub(1));
        Some(CompletionPopup {
            entries: state
                .items
                .iter()
                .map(|item| CompletionEntry {
                    kind: item.kind.clone(),
                    keyword: item.label.clone(),
                    highlights: completion_label_highlights(&item.label, &prefix),
                })
                .collect(),
            selected,
            scroll,
        })
    }

    pub(super) fn has_visible_completion_popup(&self) -> bool {
        self.visible_completion_state().is_some()
    }

    pub(crate) fn lsp_needs_fast_poll(&self) -> bool {
        self.lsp.auto_completion.is_some()
            || self.lsp.pending_requests.values().any(|request| {
                matches!(
                    request.kind,
                    PendingRequest::Completion { .. } | PendingRequest::SymbolInfo { .. }
                )
            })
    }

    pub fn completion_preview(&self) -> Option<CompletionPreview> {
        let state = self.visible_completion_state()?;
        let item = state.items.get(state.selected)?;
        let buffer = self.session.buffer(self.session.active_id())?;
        let cursor = self.active_cursor_pos();
        let suffix = line_after_cursor_completion_preview_suffix(buffer, cursor)?;
        let edit = completion_edit_for_buffer(item, buffer, state.requested_at)?;
        let insert = completion_snippet_expansion(item, &edit.insert)
            .map(|expansion| expansion.text)
            .unwrap_or_else(|| edit.insert.clone());
        let prefix = completion_prefix(buffer, cursor);
        let preview = insert
            .strip_prefix(&prefix)
            .unwrap_or(&insert)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        (!preview.is_empty()).then_some(CompletionPreview {
            text: preview,
            suffix,
        })
    }

    pub fn symbol_info_popup(&mut self, term_w: u16) -> Option<SymbolInfoPopup<'_>> {
        let width = symbol_info_content_width_limit(term_w);
        self.ensure_symbol_info_layout(width);
        let state = self.lsp.symbol_info.as_ref()?;
        if self.active_cursor_pos() != state.requested_at || state.blocks.is_empty() {
            return None;
        }
        let inner_h = state
            .display_lines
            .len()
            .clamp(1, SYMBOL_INFO_MAX_HEIGHT as usize);
        let max_scroll = state.display_lines.len().saturating_sub(inner_h);
        Some(SymbolInfoPopup {
            title: "Symbol info",
            display_lines: &state.display_lines,
            scroll: state.scroll.min(max_scroll),
        })
    }

    pub fn diagnostics_popup(&self) -> Option<DiagnosticsPopup> {
        let state = self.lsp.diagnostics_popup.as_ref()?;
        let entries = self.current_diagnostic_popup_entries();
        if entries.is_empty() {
            return None;
        }
        let max_selected = entries.len().saturating_sub(1);
        let selected = state.selected.min(max_selected);
        let scroll = selected.saturating_sub(DIAGNOSTICS_POPUP_VISIBLE_ROWS.saturating_sub(1));
        Some(DiagnosticsPopup {
            entries,
            selected,
            scroll,
            focus: state.focus,
            code_actions: self.diagnostics_code_actions_pane(state),
        })
    }

    fn diagnostics_code_actions_pane(
        &self,
        state: &DiagnosticsPopupState,
    ) -> Option<DiagnosticsCodeActionsPane> {
        let pane = state.code_actions.as_ref()?;
        if pane.actions.is_empty() {
            return None;
        }
        let max_selected = pane.actions.len().saturating_sub(1);
        let selected = pane.selected.min(max_selected);
        let scroll = selected.saturating_sub(DIAGNOSTICS_POPUP_VISIBLE_ROWS.saturating_sub(1));
        Some(DiagnosticsCodeActionsPane {
            title: pane.title.clone(),
            entries: pane
                .actions
                .iter()
                .map(|action| CodeActionPopupEntry {
                    title: action.title.clone(),
                    kind: action.kind.clone(),
                    preferred: action.preferred,
                })
                .collect(),
            selected,
            scroll,
            loading: pane.loading,
        })
    }

    pub fn lsp_marketplace_popup(&self) -> Option<LspMarketplacePopup> {
        let popup = self.lsp.marketplace.as_ref()?;
        let mut entries = PROVIDERS
            .iter()
            .copied()
            .map(MarketplaceSpec::Provider)
            .chain(LINTERS.iter().copied().map(MarketplaceSpec::Linter))
            .map(|spec| self.marketplace_entry(spec))
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| {
            (
                !entry.installed,
                entry.language_label.clone(),
                entry.tool_label.clone(),
            )
        });
        let max_selected = entries.len().saturating_sub(1);
        let selected = popup.selected.min(max_selected);
        Some(LspMarketplacePopup {
            entries,
            selected,
            scroll: popup.scroll,
        })
    }

    pub fn diagnostic_summary_for_buffer(&self, buffer_id: BufferId) -> DiagnosticSummary {
        let mut summary = DiagnosticSummary::default();
        for diagnostic in self.display_diagnostics_for_buffer(buffer_id) {
            match diagnostic.severity {
                DiagnosticSeverity::Error => summary.errors += 1,
                DiagnosticSeverity::Warning => summary.warnings += 1,
                DiagnosticSeverity::Information => summary.information += 1,
                DiagnosticSeverity::Hint => summary.hints += 1,
            }
        }
        summary
    }

    pub fn lsp_provider_installed_for_buffer(&self, buffer_id: BufferId) -> bool {
        let Some(meta) = self.session.meta(buffer_id) else {
            return false;
        };
        if meta.kind != BufferKind::File {
            return false;
        }
        let Some(language) = language_for_path(meta.path.as_deref()) else {
            return false;
        };

        PROVIDERS.iter().copied().any(|provider| {
            provider.matches_language(lsp_language(language))
                && self
                    .lsp
                    .installed
                    .contains_key(&MarketplaceItemId::Provider(provider.id))
        })
    }

    fn replace_diagnostics_for_source(
        &mut self,
        uri: String,
        source: DiagnosticSource,
        items: Vec<StoredDiagnostic>,
    ) {
        if items.is_empty() {
            self.remove_diagnostics_for_source_uri(&uri, &source);
            return;
        }

        let entries = self.lsp.diagnostics.entry(uri).or_default();
        if let Some(existing) = entries.iter_mut().find(|entry| entry.source == source) {
            existing.items = items;
        } else {
            entries.push(StoredDiagnostics { source, items });
        }
    }

    fn replace_or_defer_diagnostics_for_source(
        &mut self,
        uri: String,
        version: Option<i32>,
        source: DiagnosticSource,
        items: Vec<StoredDiagnostic>,
    ) {
        if self.diagnostics_are_stale(&uri, version) {
            return;
        }
        if self.mode == EditorMode::Insert {
            self.lsp
                .deferred_diagnostics
                .retain(|pending| !(pending.uri == uri && pending.source == source));
            self.lsp.deferred_diagnostics.push(DeferredDiagnostics {
                uri,
                version,
                source,
                items,
            });
            return;
        }
        self.replace_diagnostics_for_source(uri, source, items);
    }

    fn flush_deferred_diagnostics(&mut self) {
        if self.mode == EditorMode::Insert || self.lsp.deferred_diagnostics.is_empty() {
            return;
        }

        let pending = std::mem::take(&mut self.lsp.deferred_diagnostics);
        for diagnostics in pending {
            if self.diagnostics_are_stale(&diagnostics.uri, diagnostics.version) {
                continue;
            }
            self.replace_diagnostics_for_source(
                diagnostics.uri,
                diagnostics.source,
                diagnostics.items,
            );
        }
    }

    fn remove_diagnostics_for_source_uri(&mut self, uri: &str, source: &DiagnosticSource) {
        let mut should_remove_uri = false;
        if let Some(entries) = self.lsp.diagnostics.get_mut(uri) {
            entries.retain(|entry| &entry.source != source);
            should_remove_uri = entries.is_empty();
        }
        if should_remove_uri {
            self.lsp.diagnostics.remove(uri);
        }
    }

    fn remove_diagnostics_for_source_everywhere(&mut self, source: &DiagnosticSource) {
        self.lsp.diagnostics.retain(|_, entries| {
            entries.retain(|entry| &entry.source != source);
            !entries.is_empty()
        });
    }

    pub fn active_diagnostic_lines(
        &self,
        first_line: usize,
        line_count: usize,
    ) -> BTreeMap<usize, DiagnosticLine> {
        let last_line = first_line.saturating_add(line_count);
        let mut by_line: BTreeMap<usize, DiagnosticLine> = BTreeMap::new();
        for diagnostic in self.active_display_diagnostics() {
            if diagnostic.line < first_line || diagnostic.line >= last_line {
                continue;
            }

            by_line
                .entry(diagnostic.line)
                .and_modify(|entry: &mut DiagnosticLine| {
                    entry.message_count += 1;
                    if diagnostic.severity.sort_rank() < entry.severity.sort_rank() {
                        entry.severity = diagnostic.severity;
                        entry.start_col = diagnostic.start_col;
                        entry.end_col = diagnostic.end_col;
                        entry.inline_text = clip_diagnostic_message(&diagnostic.message);
                    }
                })
                .or_insert_with(|| DiagnosticLine {
                    severity: diagnostic.severity,
                    start_col: diagnostic.start_col,
                    end_col: diagnostic
                        .end_col
                        .max(diagnostic.start_col.saturating_add(1)),
                    inline_text: clip_diagnostic_message(&diagnostic.message),
                    message_count: 1,
                });
        }

        for entry in by_line.values_mut() {
            if entry.message_count > 1 {
                entry.inline_text =
                    format!("▌ {} (+{})", entry.inline_text, entry.message_count - 1);
            } else {
                entry.inline_text = format!("▌ {}", entry.inline_text);
            }
        }

        by_line
    }

    pub fn active_lsp_loading_toast(&self, now: Instant) -> Option<String> {
        if let Some(message) = self.active_provider_operation_toast(now) {
            return Some(message);
        }
        let active_id = self.session.active_id();
        let document = self.lsp.documents.get(&active_id)?;
        let client = self.lsp.clients.get(&document.workspace)?;
        if client.session.is_initialized() {
            return None;
        }
        let elapsed = now.saturating_duration_since(client.loading_since);
        let idx = ((elapsed.as_millis() / 100) as usize) % LSP_SPINNER_FRAMES.len();
        Some(format!(
            "{} loading {} diagnostics",
            LSP_SPINNER_FRAMES[idx], client.provider.label
        ))
    }

    pub fn poll_lsp(&mut self) {
        let now = Instant::now();
        self.cancel_obsolete_lsp_requests();
        self.lsp.retry_after.retain(|_, retry_at| *retry_at > now);
        self.poll_provider_operations();
        self.poll_lint_runs();
        self.ensure_active_lsp_client();
        let _ = self.sync_active_lsp_document_debounced(now);
        self.cancel_timed_out_lsp_requests(now);
        self.trigger_due_auto_completion(now);
        self.flush_deferred_diagnostics();

        let mut terminated = Vec::new();
        let workspaces = self.lsp.clients.keys().cloned().collect::<Vec<_>>();
        for workspace in workspaces {
            if self.lsp.clients.get(&workspace).is_some_and(|client| {
                !client.session.is_initialized()
                    && now.saturating_duration_since(client.loading_since) >= LSP_INITIALIZE_TIMEOUT
            }) {
                terminated.push((
                    workspace,
                    Some("language-server initialization timed out".to_string()),
                ));
                continue;
            }
            for _ in 0..MAX_LSP_EVENTS_PER_WORKSPACE_POLL {
                let event = self
                    .lsp
                    .clients
                    .get_mut(&workspace)
                    .and_then(|client| client.session.try_recv());
                let Some(event) = event else {
                    break;
                };

                match event {
                    SessionEvent::Initialized { .. } => {
                        let document_ids = self
                            .lsp
                            .documents
                            .iter()
                            .filter_map(|(buffer_id, document)| {
                                (document.workspace == workspace).then_some(*buffer_id)
                            })
                            .collect::<Vec<_>>();
                        for buffer_id in document_ids {
                            let _ = self.sync_lsp_document(buffer_id, SyncPolicy::Immediate);
                        }
                    }
                    SessionEvent::InitializationFailed { message } => {
                        let label = self
                            .lsp
                            .clients
                            .get(&workspace)
                            .map(|client| client.provider.label)
                            .unwrap_or("language server");
                        terminated.push((
                            workspace.clone(),
                            Some(format!("failed to initialize {label}: {message}")),
                        ));
                        break;
                    }
                    SessionEvent::Message(message) => {
                        if let Some((uri, version, diagnostics)) =
                            parse_publish_diagnostics(&message)
                        {
                            self.replace_or_defer_diagnostics_for_source(
                                uri,
                                version,
                                DiagnosticSource::Lsp(workspace.clone()),
                                diagnostics,
                            );
                            continue;
                        }

                        if self.respond_to_lsp_server_request(&workspace, &message) {
                            continue;
                        }

                        if let Some(target) = self.take_definition_response(&workspace, &message) {
                            self.jump_to_definition_target(target);
                            continue;
                        }

                        if self.take_code_action_response(&workspace, &message) {
                            continue;
                        }

                        if self.take_execute_command_response(&workspace, &message) {
                            continue;
                        }

                        if self.take_symbol_info_response(&workspace, &message) {
                            continue;
                        }

                        if self.take_completion_response(&workspace, &message) {
                            continue;
                        }
                    }
                    SessionEvent::Terminated { error } => {
                        terminated.push((
                            workspace.clone(),
                            error.map(|error| format!("language server stopped: {error}")),
                        ));
                        break;
                    }
                }
            }
        }

        for (workspace, error) in terminated {
            self.lsp.clients.remove(&workspace);
            self.lsp
                .retry_after
                .insert(workspace.clone(), now + LSP_RETRY_DELAY);
            self.reset_documents_for_workspace(&workspace);
            self.remove_diagnostics_for_source_everywhere(&DiagnosticSource::Lsp(
                workspace.clone(),
            ));
            self.lsp
                .pending_requests
                .retain(|key, _| key.workspace != workspace);
            self.lsp
                .deferred_diagnostics
                .retain(|pending| pending.source != DiagnosticSource::Lsp(workspace.clone()));
            if self.lsp.completion.as_ref().is_some_and(|completion| {
                self.lsp
                    .documents
                    .get(&completion.context.buffer_id)
                    .is_some_and(|document| document.workspace == workspace)
            }) {
                self.close_completion();
            }
            if let Some(error) = error {
                self.set_status(error);
            }
        }

        self.cleanup_orphaned_lsp_state();
        if self.mode == EditorMode::DiagnosticsList
            && self.current_diagnostic_popup_entries().is_empty()
        {
            self.close_diagnostics_popup();
        }
        if self.mode == EditorMode::CodeActions
            && self
                .lsp
                .code_actions_popup
                .as_ref()
                .is_some_and(|popup| popup.actions.is_empty())
        {
            self.close_code_actions_popup();
        }
    }

    pub(super) fn open_lsp_marketplace(&mut self) {
        if self.explorer_is_active() || self.about_popup().is_some() {
            return;
        }
        self.close_completion();
        self.refresh_lsp_tool_availability();
        self.lsp.marketplace = Some(LspMarketplaceState {
            selected: 0,
            scroll: 0,
        });
        self.mode = EditorMode::LspMarketplace;
    }

    pub(super) fn close_lsp_marketplace(&mut self) {
        if self.mode == EditorMode::LspMarketplace {
            self.mode = EditorMode::Normal;
        }
        self.lsp.marketplace = None;
    }

    pub(super) fn toggle_diagnostics_popup(&mut self) {
        if self.mode == EditorMode::DiagnosticsList {
            self.close_diagnostics_popup();
            return;
        }
        if self.explorer_is_active() || self.about_popup().is_some() {
            return;
        }
        self.close_completion();
        let entries = self.current_diagnostic_popup_entries();
        if entries.is_empty() {
            self.set_status("no diagnostics in current file");
            return;
        }
        self.lsp.diagnostics_popup = Some(DiagnosticsPopupState {
            code_action_origin: None,
            selected: 0,
            code_actions: None,
            focus: DiagnosticsPopupFocus::Diagnostics,
            cached_code_actions: None,
            pending_code_actions: None,
        });
        self.mode = EditorMode::DiagnosticsList;
        self.prefetch_selected_diagnostic_code_actions();
    }

    pub(super) fn close_diagnostics_popup(&mut self) {
        if self.mode == EditorMode::DiagnosticsList {
            self.mode = EditorMode::Normal;
        }
        self.lsp.diagnostics_popup = None;
    }

    pub(super) fn close_symbol_info_popup(&mut self) {
        let _ = self.close_symbol_info();
    }

    pub(super) fn symbol_info_popup_move(&mut self, delta: isize) {
        let _ = self.symbol_info_move(delta);
    }

    pub(super) fn diagnostics_popup_move(&mut self, delta: isize) {
        if self.diagnostics_code_actions_are_focused() {
            self.move_diagnostics_code_actions(delta);
            return;
        }
        let entries = self.current_diagnostic_popup_entries();
        if entries.is_empty() {
            return;
        }
        let Some(state) = self.lsp.diagnostics_popup.as_mut() else {
            return;
        };
        let max_index = entries.len().saturating_sub(1) as isize;
        state.selected = (state.selected as isize + delta).clamp(0, max_index) as usize;
        self.prefetch_selected_diagnostic_code_actions();
    }

    pub(super) fn jump_to_selected_diagnostic(&mut self) {
        let entries = self.current_diagnostic_popup_entries();
        let Some(state) = self.lsp.diagnostics_popup.as_ref() else {
            return;
        };
        let Some(entry) = entries.get(state.selected) else {
            return;
        };
        let target = Pos::new(entry.line, entry.col);
        let (width, height) = self.viewport_size();
        let text_vh = height.saturating_sub(crate::ui::STATUS_BAR_HEIGHT_ROWS);
        self.with_active_buffer_view_mut(|buffer, view| {
            view.cursor.cursor = buffer.clamp_pos(target);
            view.cursor.reconcile_after_edit(buffer, width, text_vh);
        });
        self.close_diagnostics_popup();
    }

    pub(super) fn diagnostics_popup_open_selected(&mut self) {
        if self.diagnostics_code_actions_are_focused() {
            self.apply_selected_code_action();
        } else {
            self.jump_to_selected_diagnostic();
        }
    }

    pub(super) fn diagnostics_popup_cancel(&mut self) {
        if !self.close_diagnostics_code_actions_pane() {
            self.close_diagnostics_popup();
        }
    }

    pub(super) fn lsp_marketplace_move(&mut self, delta: isize) {
        let Some(popup) = self.lsp_marketplace_popup() else {
            return;
        };
        if popup.entries.is_empty() {
            return;
        }
        let viewport_height_rows = self.viewport_size().1;
        let Some(state) = self.lsp.marketplace.as_mut() else {
            return;
        };
        let max_index = popup.entries.len().saturating_sub(1) as isize;
        state.selected = (state.selected as isize + delta).clamp(0, max_index) as usize;
        reconcile_marketplace_scroll(&popup.entries, state, viewport_height_rows);
    }

    pub(super) fn install_selected_lsp(&mut self) {
        let Some(item) = self.selected_marketplace_item() else {
            return;
        };
        if self.lsp.provider_operations.contains_key(&item.id()) {
            return;
        }
        if !self.marketplace_tool_available(item) {
            if !self.start_provider_install(item) {
                self.set_status(format!("no supported installer found for {}", item.label()));
                return;
            }
            self.set_status(format!("installing {}", item.label()));
            return;
        }
        let std::collections::hash_map::Entry::Vacant(entry) = self.lsp.installed.entry(item.id())
        else {
            return;
        };
        entry.insert(InstalledToolRecord {
            install_source: None,
        });
        if let Err(error) = save_installed_tools(&self.lsp.installed) {
            self.set_status(format!("failed to save installed tools: {error}"));
            self.lsp.installed.remove(&item.id());
            return;
        }
        self.set_status(format!("installed {}", item.label()));
        self.ensure_active_lsp_client();
    }

    pub(super) fn uninstall_selected_lsp(&mut self) {
        let Some(item) = self.selected_marketplace_item() else {
            return;
        };
        if self.lsp.provider_operations.contains_key(&item.id()) {
            return;
        }
        let Some(record) = self.lsp.installed.get(&item.id()).cloned() else {
            self.set_status(format!("{} is not installed", item.label()));
            return;
        };
        if self.start_provider_uninstall(item, &record) {
            return;
        }
        self.lsp.installed.remove(&item.id());
        if let Err(error) = save_installed_tools(&self.lsp.installed) {
            self.set_status(format!("failed to save installed tools: {error}"));
            self.lsp.installed.insert(item.id(), record);
            return;
        }
        if let MarketplaceSpec::Provider(provider) = item {
            self.remove_provider_runtime_state(provider.id);
        }
        self.set_status(format!("removed {}", item.label()));
    }

    pub(super) fn initialise_lsp_state(&mut self) {
        self.lsp.installed = load_installed_tools();
        self.refresh_lsp_tool_availability();
        self.ensure_active_lsp_client();
    }

    pub(super) fn command_lsp_status(&mut self) {
        self.ensure_active_lsp_client();

        let active_id = self.session.active_id();
        let Some(meta) = self.session.meta(active_id) else {
            self.set_status("no active buffer");
            return;
        };
        let Some(path) = meta.path.as_deref() else {
            self.set_status("current buffer is not file-backed");
            return;
        };
        let Some(language) = language_for_path(Some(path)) else {
            self.set_status("no LSP language detected for current file");
            return;
        };

        let provider = PROVIDERS.iter().copied().find(|provider| {
            self.lsp
                .installed
                .contains_key(&MarketplaceItemId::Provider(provider.id))
                && provider.matches_language(lsp_language(language))
        });
        let linter = self
            .lint_source_for_path(path, language)
            .filter(|source| {
                self.lsp
                    .installed
                    .contains_key(&MarketplaceItemId::Linter(source.kind))
            })
            .and_then(|source| linter_spec(source.kind));

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(meta.display_name.as_str());
        let language_label = syntax_language_label(language);
        let lsp_label = provider.map(|provider| provider.label).unwrap_or("none");
        let linter_label = linter.map(|linter| linter.label).unwrap_or("none");

        self.set_status_lines(vec![
            (
                format!("LSP status for: {file_name}"),
                StatusMessageStyle::Normal,
            ),
            (
                format!("Language: {language_label}"),
                StatusMessageStyle::Normal,
            ),
            (format!("LSP: {lsp_label}"), StatusMessageStyle::Normal),
            (
                format!("Linter: {linter_label}"),
                StatusMessageStyle::Normal,
            ),
        ]);
    }

    fn selected_marketplace_item(&self) -> Option<MarketplaceSpec> {
        let popup = self.lsp_marketplace_popup()?;
        let selected = popup.entries.get(popup.selected)?;
        marketplace_spec(selected.item_id)
    }

    fn ensure_active_lsp_client(&mut self) {
        let active_id = self.session.active_id();
        let Some(meta) = self.session.meta(active_id) else {
            return;
        };
        if meta.kind != BufferKind::File {
            return;
        }
        if !self.session.active_buffer_is_fully_loaded() {
            return;
        }

        let Some(path) = meta.path.as_deref() else {
            return;
        };
        let Some(language) = language_for_path(Some(path)) else {
            return;
        };
        let Some(provider) = PROVIDERS.iter().copied().find(|provider| {
            self.lsp
                .installed
                .contains_key(&MarketplaceItemId::Provider(provider.id))
                && provider.matches_language(lsp_language(language))
        }) else {
            return;
        };
        let Some(language_id) = provider.language_id_for(lsp_language(language)) else {
            return;
        };

        if self.lsp.documents.get(&active_id).is_some_and(|document| {
            document.path == path
                && document.language_id == language_id
                && document.workspace.provider_id == provider.id
                && self.lsp.clients.contains_key(&document.workspace)
        }) {
            return;
        }

        let root = workspace_root_for(path, &provider, self.session.launch_dir());
        let workspace = WorkspaceKey {
            provider_id: provider.id,
            root: root.clone(),
        };
        if !self.lsp.clients.contains_key(&workspace) {
            if self
                .lsp
                .retry_after
                .get(&workspace)
                .is_some_and(|retry_at| Instant::now() < *retry_at)
            {
                return;
            }
            match LspSession::spawn(
                &provider.command(),
                &root,
                ClientInfo {
                    name: "redox",
                    version: env!("CARGO_PKG_VERSION"),
                },
            ) {
                Ok(session) => {
                    self.lsp.clients.insert(
                        workspace.clone(),
                        ManagedClient {
                            provider,
                            session,
                            loading_since: Instant::now(),
                        },
                    );
                }
                Err(error) => {
                    self.lsp
                        .retry_after
                        .insert(workspace.clone(), Instant::now() + LSP_RETRY_DELAY);
                    self.set_status(format!("failed to start {}: {error}", provider.label));
                    return;
                }
            }
        }

        let uri = match file_uri(path) {
            Ok(uri) => uri,
            Err(error) => {
                self.set_status(format!("failed to build file URI: {error}"));
                return;
            }
        };

        let matches_existing = self.lsp.documents.get(&active_id).is_some_and(|document| {
            document.workspace == workspace
                && document.path == path
                && document.uri == uri
                && document.language_id == language_id
        });
        if matches_existing {
            return;
        }

        let path = path.to_path_buf();
        self.close_lsp_document(active_id);
        self.lsp.documents.insert(
            active_id,
            ManagedDocument {
                workspace,
                path,
                uri,
                language_id,
                document_version: 0,
                last_sent_analysis_version: None,
                last_sent_text: None,
                pending_sync_since: None,
                pending_sync_analysis_version: None,
                opened: false,
            },
        );
    }

    fn sync_active_lsp_document(&mut self) -> io::Result<()> {
        self.sync_lsp_document(self.session.active_id(), SyncPolicy::Immediate)
    }

    fn sync_active_lsp_document_debounced(&mut self, now: Instant) -> io::Result<()> {
        let policy = if self.mode == EditorMode::Insert {
            SyncPolicy::Debounced { now }
        } else {
            SyncPolicy::Immediate
        };
        self.sync_lsp_document(self.session.active_id(), policy)
    }

    fn sync_lsp_document(&mut self, buffer_id: BufferId, policy: SyncPolicy) -> io::Result<()> {
        let Some(document) = self.lsp.documents.get(&buffer_id).cloned() else {
            return Ok(());
        };
        let Some(client) = self.lsp.clients.get_mut(&document.workspace) else {
            return Ok(());
        };
        if !client.session.is_initialized() {
            return Ok(());
        }
        if !self
            .session
            .buffer_is_fully_loaded(buffer_id)
            .unwrap_or(true)
        {
            return Ok(());
        }
        if self.session.buffer(buffer_id).is_none() {
            return Ok(());
        }
        let analysis_version = self
            .views
            .get(&buffer_id)
            .map(|view| view.analysis_version())
            .unwrap_or(0);

        let Some(document) = self.lsp.documents.get_mut(&buffer_id) else {
            return Ok(());
        };
        if !document.opened {
            let Some(text) = self
                .session
                .buffer(buffer_id)
                .map(|buffer| buffer.to_string())
            else {
                return Ok(());
            };
            document.document_version = 1;
            client.session.send_did_open(
                &document.path,
                document.language_id,
                document.document_version,
                &text,
            )?;
            document.opened = true;
            document.last_sent_analysis_version = Some(analysis_version);
            document.last_sent_text = Some(text);
            document.pending_sync_since = None;
            document.pending_sync_analysis_version = None;
            return Ok(());
        }

        if document.last_sent_analysis_version == Some(analysis_version) {
            document.pending_sync_since = None;
            document.pending_sync_analysis_version = None;
            return Ok(());
        }

        let Some(text) = self
            .session
            .buffer(buffer_id)
            .map(|buffer| buffer.to_string())
        else {
            return Ok(());
        };

        if document.last_sent_text.as_deref() == Some(text.as_str()) {
            document.last_sent_analysis_version = Some(analysis_version);
            document.pending_sync_since = None;
            document.pending_sync_analysis_version = None;
            return Ok(());
        }

        if let SyncPolicy::Debounced { now } = policy {
            if document.pending_sync_analysis_version != Some(analysis_version) {
                document.pending_sync_since = Some(now);
                document.pending_sync_analysis_version = Some(analysis_version);
                return Ok(());
            }
            let pending_since = document.pending_sync_since.unwrap_or(now);
            if now.saturating_duration_since(pending_since) < LSP_CHANGE_DEBOUNCE {
                return Ok(());
            }
        }

        document.document_version = document.document_version.saturating_add(1);
        client
            .session
            .send_did_change(&document.path, document.document_version, &text)?;
        document.last_sent_analysis_version = Some(analysis_version);
        document.last_sent_text = Some(text);
        document.pending_sync_since = None;
        document.pending_sync_analysis_version = None;
        Ok(())
    }

    fn cleanup_orphaned_lsp_state(&mut self) {
        let valid_ids = self
            .session
            .summaries()
            .into_iter()
            .map(|summary| summary.id)
            .collect::<HashSet<_>>();
        let closed = self
            .lsp
            .documents
            .iter()
            .filter_map(|(buffer_id, document)| {
                (self
                    .session
                    .meta(*buffer_id)
                    .and_then(|meta| meta.path.as_deref())
                    != Some(document.path.as_path()))
                .then_some(*buffer_id)
            })
            .collect::<Vec<_>>();
        for buffer_id in closed {
            self.close_lsp_document(buffer_id);
        }
        let live_workspaces = self
            .lsp
            .documents
            .values()
            .map(|document| document.workspace.clone())
            .collect::<HashSet<_>>();
        let live_lint_sources = valid_ids
            .iter()
            .filter_map(|buffer_id| {
                self.saved_buffer_lint_context(*buffer_id)
                    .map(|request| request.source)
            })
            .collect::<HashSet<_>>();
        self.lsp
            .clients
            .retain(|workspace, _| live_workspaces.contains(workspace));
        self.lsp.diagnostics.retain(|_, entries| {
            entries.retain(|entry| match &entry.source {
                DiagnosticSource::Lsp(workspace) => live_workspaces.contains(workspace),
                DiagnosticSource::Lint(source) => live_lint_sources.contains(source),
            });
            !entries.is_empty()
        });
        self.lsp
            .pending_requests
            .retain(|key, _| live_workspaces.contains(&key.workspace));
        self.lsp
            .deferred_diagnostics
            .retain(|pending| match &pending.source {
                DiagnosticSource::Lsp(workspace) => live_workspaces.contains(workspace),
                DiagnosticSource::Lint(source) => live_lint_sources.contains(source),
            });
        self.lsp.queued_lint_runs = std::mem::take(&mut self.lsp.queued_lint_runs)
            .into_iter()
            .filter(|request| self.lint_request_is_current(request))
            .collect();
    }

    fn close_lsp_document(&mut self, buffer_id: BufferId) {
        let Some(document) = self.lsp.documents.remove(&buffer_id) else {
            return;
        };
        if document.opened
            && let Some(client) = self.lsp.clients.get_mut(&document.workspace)
        {
            let _ = client.session.send_did_close(&document.path);
        }
        let source = DiagnosticSource::Lsp(document.workspace.clone());
        self.remove_diagnostics_for_source_uri(&document.uri, &source);
        self.lsp
            .deferred_diagnostics
            .retain(|pending| pending.uri != document.uri || pending.source != source);
        let requests = self
            .lsp
            .pending_requests
            .iter()
            .filter_map(|(key, request)| {
                (request.context.buffer_id == buffer_id).then_some(key.clone())
            })
            .collect::<Vec<_>>();
        for key in requests {
            self.lsp.pending_requests.remove(&key);
            self.send_lsp_cancel_request(&key);
        }
    }

    fn current_diagnostic_popup_entries(&self) -> Vec<DiagnosticsPopupEntry> {
        let mut diagnostics = self.active_display_diagnostics();
        diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.severity.sort_rank(),
                diagnostic.line,
                diagnostic.start_col,
            )
        });
        diagnostics
            .into_iter()
            .map(|diagnostic| DiagnosticsPopupEntry {
                severity: diagnostic.severity,
                line: diagnostic.line,
                col: diagnostic.start_col,
                end_col: diagnostic.end_col,
                summary: diagnostic_summary_line(&diagnostic.message),
                message: diagnostic.message,
            })
            .collect()
    }

    fn diagnostic_entry_under_cursor(&self) -> Option<DiagnosticsPopupEntry> {
        let cursor = self.active_cursor_pos();
        self.current_diagnostic_popup_entries()
            .into_iter()
            .find(|entry| {
                entry.line == cursor.line && cursor.col >= entry.col && cursor.col <= entry.end_col
            })
    }

    fn stored_diagnostics_for_buffer(
        &self,
        buffer_id: BufferId,
    ) -> Vec<(&DiagnosticSource, &StoredDiagnostic)> {
        let Some(uri) = self.document_uri_for_buffer(buffer_id) else {
            return Vec::new();
        };
        self.lsp
            .diagnostics
            .get(&uri)
            .into_iter()
            .flat_map(|entries| entries.iter())
            .flat_map(|entry| entry.items.iter().map(move |item| (&entry.source, item)))
            .collect()
    }

    fn active_display_diagnostics(&self) -> Vec<Diagnostic> {
        self.display_diagnostics_for_buffer(self.session.active_id())
    }

    fn display_diagnostics_for_buffer(&self, buffer_id: BufferId) -> Vec<Diagnostic> {
        let Some(buffer) = self.session.buffer(buffer_id) else {
            return Vec::new();
        };
        let stored = self.stored_diagnostics_for_buffer(buffer_id);
        let suppress_lint = should_suppress_lint_diagnostics(stored.iter().copied());
        let mut deduped = Vec::<Diagnostic>::new();
        let mut seen = HashMap::<(DiagnosticSeverity, usize, usize, String), usize>::new();

        for diagnostic in stored
            .into_iter()
            .filter(|(source, _)| !(suppress_lint && matches!(source, DiagnosticSource::Lint(_))))
            .map(|(_, diagnostic)| diagnostic.to_display(buffer))
        {
            let summary = diagnostic_summary_line(&diagnostic.message);
            let key = (
                diagnostic.severity,
                diagnostic.line,
                diagnostic.start_col,
                summary,
            );
            if let Some(existing_idx) = seen.get(&key).copied() {
                let existing = &mut deduped[existing_idx];
                existing.end_col = existing.end_col.max(diagnostic.end_col);
                if diagnostic.message.len() > existing.message.len() {
                    existing.message = diagnostic.message;
                }
            } else {
                seen.insert(key, deduped.len());
                deduped.push(diagnostic);
            }
        }

        deduped
    }

    fn document_uri_for_buffer(&self, buffer_id: BufferId) -> Option<String> {
        self.lsp
            .documents
            .get(&buffer_id)
            .map(|document| document.uri.clone())
            .or_else(|| {
                let path = self.session.meta(buffer_id)?.path.as_deref()?;
                file_uri(path).ok()
            })
    }

    fn reset_documents_for_workspace(&mut self, workspace: &WorkspaceKey) {
        for document in self.lsp.documents.values_mut() {
            if &document.workspace == workspace {
                document.document_version = 0;
                document.opened = false;
                document.last_sent_analysis_version = None;
                document.last_sent_text = None;
                document.pending_sync_since = None;
                document.pending_sync_analysis_version = None;
            }
        }
    }

    fn remove_provider_runtime_state(&mut self, provider_id: ProviderId) {
        self.lsp
            .retry_after
            .retain(|workspace, _| workspace.provider_id != provider_id);
        let mut doomed_workspaces = self
            .lsp
            .clients
            .keys()
            .filter(|workspace| workspace.provider_id == provider_id)
            .cloned()
            .collect::<Vec<_>>();
        doomed_workspaces.extend(
            self.lsp
                .documents
                .values()
                .filter(|document| document.workspace.provider_id == provider_id)
                .map(|document| document.workspace.clone()),
        );
        let doomed_workspace_set = doomed_workspaces.iter().cloned().collect::<HashSet<_>>();
        self.lsp
            .clients
            .retain(|workspace, _| !doomed_workspace_set.contains(workspace));
        self.lsp
            .documents
            .retain(|_, document| !doomed_workspace_set.contains(&document.workspace));
        for workspace in &doomed_workspace_set {
            self.remove_diagnostics_for_source_everywhere(&DiagnosticSource::Lsp(
                workspace.clone(),
            ));
        }
        self.lsp
            .pending_requests
            .retain(|key, _| !doomed_workspace_set.contains(&key.workspace));
        self.lsp
            .deferred_diagnostics
            .retain(|pending| match &pending.source {
                DiagnosticSource::Lsp(workspace) => !doomed_workspace_set.contains(workspace),
                DiagnosticSource::Lint(_) => true,
            });
    }

    fn diagnostics_are_stale(&self, uri: &str, version: Option<i32>) -> bool {
        let Some(version) = version else {
            return false;
        };
        self.lsp.documents.values().any(|document| {
            document.uri == uri && document.opened && version < document.document_version
        })
    }

    fn cancel_pending_lsp_requests(&mut self, workspace: &WorkspaceKey, kind: PendingRequest) {
        let doomed = self
            .lsp
            .pending_requests
            .iter()
            .filter_map(|(key, request)| {
                (key.workspace == *workspace && pending_request_same_family(&request.kind, &kind))
                    .then_some(key.clone())
            })
            .collect::<Vec<_>>();
        for key in doomed {
            if self.lsp.pending_requests.remove(&key).is_some() {
                self.send_lsp_cancel_request(&key);
            }
        }
    }

    fn lsp_request_context(&self) -> RequestContext {
        let buffer_id = self.session.active_id();
        RequestContext {
            buffer_id,
            analysis_version: self
                .views
                .get(&buffer_id)
                .map_or(0, |view| view.analysis_version()),
            cursor: self.active_cursor_pos(),
            mode: self.mode,
        }
    }

    pub(super) fn cancel_obsolete_lsp_requests(&mut self) {
        let context = self.lsp_request_context();
        let obsolete = self
            .lsp
            .pending_requests
            .iter()
            .filter_map(|(key, request)| {
                (!matches!(request.kind, PendingRequest::ExecuteCommand { .. })
                    && request.context != context)
                    .then_some(key.clone())
            })
            .collect::<Vec<_>>();
        for key in obsolete {
            self.lsp.pending_requests.remove(&key);
            self.send_lsp_cancel_request(&key);
        }
    }

    fn cancel_lsp_request_family(&mut self, kind: PendingRequest) {
        let workspaces = self
            .lsp
            .pending_requests
            .keys()
            .map(|key| key.workspace.clone())
            .collect::<HashSet<_>>();
        for workspace in workspaces {
            self.cancel_pending_lsp_requests(&workspace, kind.clone());
        }
    }

    fn cancel_timed_out_lsp_requests(&mut self, now: Instant) {
        let timed_out = self
            .lsp
            .pending_requests
            .iter()
            .filter_map(|(key, request)| {
                (now.saturating_duration_since(request.started_at) >= LSP_REQUEST_TIMEOUT)
                    .then_some((key.clone(), request.kind.clone()))
            })
            .collect::<Vec<_>>();
        for (key, kind) in timed_out {
            if self.lsp.pending_requests.remove(&key).is_none() {
                continue;
            }
            self.send_lsp_cancel_request(&key);
            match kind {
                PendingRequest::GotoDefinition => self.set_status("definition lookup timed out"),
                PendingRequest::CodeActions {
                    origin, trigger, ..
                } => {
                    if let Some(state) = self.lsp.diagnostics_popup.as_mut()
                        && state.code_action_origin.as_ref() == Some(&origin)
                    {
                        state.pending_code_actions = None;
                        if matches!(trigger, CodeActionRequestTrigger::Manual) {
                            state.code_actions = None;
                            state.focus = DiagnosticsPopupFocus::Diagnostics;
                        }
                    }
                    if matches!(trigger, CodeActionRequestTrigger::Manual) {
                        self.close_code_actions_popup();
                        self.set_status("code action request timed out");
                    }
                }
                PendingRequest::ExecuteCommand { .. } => {
                    self.set_status("quick fix command timed out");
                }
                PendingRequest::SymbolInfo { .. } => {
                    self.clear_symbol_info();
                    self.set_status("symbol info request timed out");
                }
                PendingRequest::Completion { .. } => {
                    self.close_completion();
                    self.set_status("completion request timed out");
                }
            }
        }
    }

    fn send_lsp_cancel_request(&mut self, key: &RequestKey) {
        if let Some(client) = self.lsp.clients.get_mut(&key.workspace) {
            let _ = client.session.send_cancel_request(key.id);
        }
    }

    pub(super) fn sync_active_lsp_before_save(&mut self) {
        let _ = self.sync_active_lsp_document();
    }

    pub(super) fn goto_definition(&mut self) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let _ = self.sync_active_lsp_document();

        let active_id = self.session.active_id();
        let Some(document) = self.lsp.documents.get(&active_id).cloned() else {
            self.set_status("no LSP document for current buffer");
            return;
        };
        let cursor = self.active_cursor_pos();
        let Some(buffer) = self.session.buffer(active_id) else {
            self.set_status("active buffer unavailable");
            return;
        };
        let line = cursor.line.min(buffer.len_lines().saturating_sub(1));
        let character = char_col_to_utf16(&buffer.line_string(line), cursor.col);
        self.cancel_pending_lsp_requests(&document.workspace, PendingRequest::GotoDefinition);
        let Some(client) = self.lsp.clients.get_mut(&document.workspace) else {
            self.set_status("no LSP client for current buffer");
            return;
        };
        if !client.session.is_initialized() {
            self.set_status("LSP still loading");
            return;
        }
        match client
            .session
            .send_goto_definition(&document.path, line, character)
        {
            Ok(id) => {
                self.lsp.pending_requests.insert(
                    RequestKey {
                        workspace: document.workspace,
                        id,
                    },
                    PendingClientRequest {
                        context: self.lsp_request_context(),
                        kind: PendingRequest::GotoDefinition,
                        started_at: Instant::now(),
                    },
                );
                self.set_status("looking up definition…");
            }
            Err(error) => {
                self.set_status(format!("definition request failed: {error}"));
            }
        }
    }

    pub(super) fn trigger_symbol_info(&mut self) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let completion_blocks = self.selected_completion_symbol_info_blocks();
        let return_mode = self.mode;
        let active_id = self.session.active_id();
        let cursor = self.active_cursor_pos();
        if let Some(workspace) = self
            .lsp
            .documents
            .get(&active_id)
            .map(|document| document.workspace.clone())
        {
            self.cancel_pending_lsp_requests(
                &workspace,
                PendingRequest::SymbolInfo {
                    requested_at: cursor,
                    return_mode,
                },
            );
        }
        self.close_completion();
        self.clear_symbol_info();
        if !completion_blocks.is_empty() {
            self.show_symbol_info(completion_blocks, cursor, return_mode);
            return;
        }
        let _ = self.sync_active_lsp_document();

        let Some(document) = self.lsp.documents.get(&active_id).cloned() else {
            self.set_status("no LSP document for current buffer");
            return;
        };
        let Some(buffer) = self.session.buffer(active_id) else {
            self.set_status("active buffer unavailable");
            return;
        };
        let line = cursor.line.min(buffer.len_lines().saturating_sub(1));
        let character = char_col_to_utf16(&buffer.line_string(line), cursor.col);
        let Some(client) = self.lsp.clients.get_mut(&document.workspace) else {
            self.set_status("no LSP client for current buffer");
            return;
        };
        if !client.session.is_initialized() {
            self.set_status("LSP still loading");
            return;
        }
        match client.session.send_hover(&document.path, line, character) {
            Ok(id) => {
                self.lsp.pending_requests.insert(
                    RequestKey {
                        workspace: document.workspace,
                        id,
                    },
                    PendingClientRequest {
                        context: self.lsp_request_context(),
                        kind: PendingRequest::SymbolInfo {
                            requested_at: cursor,
                            return_mode,
                        },
                        started_at: Instant::now(),
                    },
                );
                self.set_status("loading symbol info...");
            }
            Err(error) => {
                self.set_status(format!("symbol info request failed: {error}"));
            }
        }
    }

    pub(super) fn trigger_completion(&mut self) {
        self.request_completion(true);
    }

    pub(super) fn queue_auto_completion_after_insert(&mut self, inserted: char) {
        if !should_auto_trigger_completion(inserted) || self.cursor_is_in_comment_for_completion() {
            self.lsp.auto_completion = None;
            if self.cursor_is_in_comment_for_completion() {
                self.close_completion();
            }
            return;
        }
        self.refilter_completion_for_active_cursor(inserted);
        self.lsp.auto_completion = Some(AutoCompletionRequest {
            requested_at: self.active_cursor_pos(),
            due_at: Instant::now() + completion_auto_trigger_delay(inserted),
        });
    }

    pub(super) fn completion_survives_insert(&self, inserted: char) -> bool {
        should_auto_trigger_completion(inserted)
    }

    pub(super) fn snippet_jump_next(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> bool {
        let active_id = self.session.active_id();
        let cursor_char = {
            let cursor = self
                .views
                .get(&active_id)
                .map(|view| view.cursor.cursor)
                .unwrap_or(Pos::new(0, 0));
            let buffer = self.session.active_buffer();
            buffer.pos_to_char(cursor)
        };
        let Some(snippet) = self.lsp.active_snippet.as_mut() else {
            return false;
        };
        if snippet.buffer_id != active_id {
            self.lsp.active_snippet = None;
            return false;
        }
        let Some(placeholder) = snippet.placeholders.get(snippet.current).cloned() else {
            self.lsp.active_snippet = None;
            return false;
        };
        if cursor_char < placeholder.start_char || cursor_char > placeholder.end_char {
            self.lsp.active_snippet = None;
            self.invalidate_active_render_caches();
            return false;
        }
        let next_tabstop = snippet
            .placeholders
            .iter()
            .filter_map(|next| (next.tabstop > placeholder.tabstop).then_some(next.tabstop))
            .min();
        let Some(next_tabstop) = next_tabstop else {
            let end_char = snippet.final_char.unwrap_or(placeholder.end_char);
            self.lsp.active_snippet = None;
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            view.cursor.cursor = buffer.char_to_pos(end_char.min(buffer.len_chars()));
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
            self.invalidate_active_render_caches();
            return true;
        };
        let next_placeholder = snippet
            .placeholders
            .iter()
            .position(|next| next.tabstop == next_tabstop)
            .expect("next tabstop should have a placeholder");

        snippet.current = next_placeholder;
        snippet.selected = true;
        let start_char = snippet.placeholders[snippet.current].start_char;
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        view.cursor.cursor = buffer.char_to_pos(start_char.min(buffer.len_chars()));
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        self.invalidate_active_render_caches();
        true
    }

    pub(super) fn replace_active_snippet_selection_text(
        &mut self,
        text: &str,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> bool {
        self.replace_active_snippet_selection_text_with_cursor_offset(
            text,
            text.chars().count(),
            viewport_width_cells,
            text_vh,
        )
    }

    pub(super) fn replace_active_snippet_selection_text_with_cursor_offset(
        &mut self,
        text: &str,
        cursor_offset: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> bool {
        let active_id = self.session.active_id();
        let Some(snippet) = self.lsp.active_snippet.as_ref() else {
            return false;
        };
        if snippet.buffer_id != active_id {
            self.lsp.active_snippet = None;
            return false;
        }
        if !snippet.selected {
            return false;
        }
        let Some(placeholder) = snippet.placeholders.get(snippet.current).cloned() else {
            self.lsp.active_snippet = None;
            return false;
        };
        let tabstop = placeholder.tabstop;

        let edits = snippet
            .placeholders
            .iter()
            .filter(|other| other.tabstop == tabstop)
            .map(|other| (other.start_char, other.end_char, text.to_string()))
            .collect::<Vec<_>>();
        let before = self.capture_active_insert_coalesced_checkpoint();
        {
            let buffer = self.session.active_buffer_mut();
            apply_character_edits(buffer, &edits);
        }
        self.update_active_snippet_after_edits(&edits);
        self.mark_active_snippet_tabstop_filled(tabstop);
        if let Some(snippet) = self.lsp.active_snippet.as_mut() {
            snippet.selected = false;
        }
        {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let cursor_char = self
                .lsp
                .active_snippet
                .as_ref()
                .and_then(|snippet| snippet.placeholders.get(snippet.current))
                .map(|placeholder| {
                    placeholder
                        .start_char
                        .saturating_add(cursor_offset.min(text.chars().count()))
                })
                .unwrap_or(placeholder.start_char.saturating_add(text.chars().count()));
            view.cursor.cursor = buffer.char_to_pos(cursor_char.min(buffer.len_chars()));
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
        true
    }

    pub(super) fn mirror_active_snippet_insert_after_cursor_insert(
        &mut self,
        at_char: usize,
        text: &str,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> bool {
        let active_id = self.session.active_id();
        let Some(snippet) = self.lsp.active_snippet.as_ref() else {
            return false;
        };
        if snippet.buffer_id != active_id {
            self.lsp.active_snippet = None;
            return false;
        }
        let Some(placeholder) = snippet.placeholders.get(snippet.current).cloned() else {
            self.lsp.active_snippet = None;
            return false;
        };
        if at_char < placeholder.start_char || at_char > placeholder.end_char {
            self.lsp.active_snippet = None;
            return false;
        }
        let relative_offset = at_char.saturating_sub(placeholder.start_char);
        let tabstop = placeholder.tabstop;

        let current_edit = (at_char, at_char, text.to_string());
        self.update_active_snippet_after_edits(std::slice::from_ref(&current_edit));
        self.mark_active_snippet_tabstop_filled(tabstop);

        let Some(snippet) = self.lsp.active_snippet.as_ref() else {
            return false;
        };
        let edits = snippet
            .placeholders
            .iter()
            .enumerate()
            .filter(|(idx, other)| *idx != snippet.current && other.tabstop == tabstop)
            .map(|(_, other)| {
                let at = other.start_char.saturating_add(
                    relative_offset.min(other.end_char.saturating_sub(other.start_char)),
                );
                (at, at, text.to_string())
            })
            .collect::<Vec<_>>();
        let mirrored = !edits.is_empty();
        if !edits.is_empty() {
            let buffer = self.session.active_buffer_mut();
            apply_character_edits(buffer, &edits);
        }
        self.update_active_snippet_after_edits(&edits);
        self.mark_active_snippet_tabstop_filled(tabstop);
        if let Some(snippet) = self.lsp.active_snippet.as_mut() {
            snippet.selected = false;
        }
        {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let cursor_char =
                transform_snippet_char(at_char.saturating_add(text.chars().count()), &edits);
            view.cursor.cursor = buffer.char_to_pos(cursor_char.min(buffer.len_chars()));
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
        if mirrored {
            self.invalidate_active_render_caches();
        }
        mirrored
    }

    pub(super) fn mirror_active_snippet_delete_after_cursor_delete(
        &mut self,
        deleted_start_char: usize,
        deleted_end_char: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> bool {
        if deleted_start_char >= deleted_end_char {
            return false;
        }
        let active_id = self.session.active_id();
        let Some(snippet) = self.lsp.active_snippet.as_ref() else {
            return false;
        };
        if snippet.buffer_id != active_id {
            self.lsp.active_snippet = None;
            return false;
        }
        let Some(placeholder) = snippet.placeholders.get(snippet.current).cloned() else {
            self.lsp.active_snippet = None;
            return false;
        };
        if deleted_start_char < placeholder.start_char || deleted_end_char > placeholder.end_char {
            self.lsp.active_snippet = None;
            return false;
        }

        let deleted_chars = deleted_end_char.saturating_sub(deleted_start_char);
        let relative_offset = deleted_start_char.saturating_sub(placeholder.start_char);
        let tabstop = placeholder.tabstop;
        let current_edit = (deleted_start_char, deleted_end_char, String::new());
        self.update_active_snippet_after_edits(std::slice::from_ref(&current_edit));
        self.mark_active_snippet_tabstop_filled(tabstop);

        let Some(snippet) = self.lsp.active_snippet.as_ref() else {
            return false;
        };
        let edits = snippet
            .placeholders
            .iter()
            .enumerate()
            .filter(|(idx, other)| *idx != snippet.current && other.tabstop == tabstop)
            .filter_map(|(_, other)| {
                let start = other.start_char.saturating_add(
                    relative_offset.min(other.end_char.saturating_sub(other.start_char)),
                );
                let end = start.saturating_add(deleted_chars).min(other.end_char);
                (start < end).then_some((start, end, String::new()))
            })
            .collect::<Vec<_>>();
        let mirrored = !edits.is_empty();
        if mirrored {
            let buffer = self.session.active_buffer_mut();
            apply_character_edits(buffer, &edits);
        }
        self.update_active_snippet_after_edits(&edits);
        self.mark_active_snippet_tabstop_filled(tabstop);
        if let Some(snippet) = self.lsp.active_snippet.as_mut() {
            snippet.selected = false;
        }
        {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let cursor_char = buffer.pos_to_char(view.cursor.cursor);
            view.cursor.cursor = buffer.char_to_pos(transform_snippet_char(cursor_char, &edits));
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
        if mirrored {
            self.invalidate_active_render_caches();
        }
        mirrored
    }

    pub fn active_snippet_placeholder_ranges(
        &self,
        first_line: usize,
        line_count: usize,
    ) -> BTreeMap<usize, Vec<std::ops::Range<usize>>> {
        let mut ranges = BTreeMap::new();
        let Some(snippet) = self.lsp.active_snippet.as_ref() else {
            return ranges;
        };
        if snippet.buffer_id != self.session.active_id() {
            return ranges;
        }
        let Some(buffer) = self.session.buffer(snippet.buffer_id) else {
            return ranges;
        };
        let last_line = first_line.saturating_add(line_count);
        for placeholder in &snippet.placeholders {
            if placeholder.filled || placeholder.start_char == placeholder.end_char {
                continue;
            }
            let start = buffer.char_to_pos(placeholder.start_char.min(buffer.len_chars()));
            let end = buffer.char_to_pos(placeholder.end_char.min(buffer.len_chars()));
            if start.line != end.line || start.line < first_line || start.line >= last_line {
                continue;
            }
            ranges
                .entry(start.line)
                .or_insert_with(Vec::new)
                .push(start.col..end.col);
        }
        ranges
    }

    fn mark_active_snippet_tabstop_filled(&mut self, tabstop: usize) {
        let Some(snippet) = self.lsp.active_snippet.as_mut() else {
            return;
        };
        for placeholder in snippet
            .placeholders
            .iter_mut()
            .filter(|placeholder| placeholder.tabstop == tabstop)
        {
            placeholder.filled = true;
        }
    }

    fn update_active_snippet_after_edits(&mut self, edits: &[(usize, usize, String)]) {
        let Some(snippet) = self.lsp.active_snippet.as_mut() else {
            return;
        };
        let current_tabstop = snippet
            .placeholders
            .get(snippet.current)
            .map(|placeholder| placeholder.tabstop);
        for placeholder in &mut snippet.placeholders {
            if let Some((edit_idx, (_, _, text))) =
                edits.iter().enumerate().find(|(_, (start, end, _))| {
                    placeholder.start_char == *start && placeholder.end_char == *end
                })
            {
                let start_char =
                    transform_snippet_char_left_skipping(placeholder.start_char, edits, edit_idx);
                placeholder.start_char = start_char;
                placeholder.end_char = start_char.saturating_add(text.chars().count());
                continue;
            }
            placeholder.start_char = transform_snippet_char_left(placeholder.start_char, edits);
            placeholder.end_char = transform_snippet_char_right(placeholder.end_char, edits);
        }
        if let Some(final_char) = snippet.final_char.as_mut() {
            *final_char = transform_snippet_char_right(*final_char, edits);
        }
        if let Some(tabstop) = current_tabstop
            && snippet
                .placeholders
                .get(snippet.current)
                .is_none_or(|placeholder| placeholder.tabstop != tabstop)
        {
            if let Some(idx) = snippet
                .placeholders
                .iter()
                .position(|placeholder| placeholder.tabstop == tabstop)
            {
                snippet.current = idx;
            }
        }
    }

    fn trigger_due_auto_completion(&mut self, now: Instant) {
        let Some(request) = self.lsp.auto_completion else {
            return;
        };
        if now < request.due_at {
            return;
        }
        self.lsp.auto_completion = None;
        if self.mode != EditorMode::Insert || self.active_cursor_pos() != request.requested_at {
            return;
        }
        self.request_completion(false);
    }

    fn request_completion(&mut self, manual: bool) {
        if !manual && self.cursor_is_in_comment_for_completion() {
            self.close_completion();
            self.lsp.auto_completion = None;
            return;
        }
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        self.ensure_active_lsp_client();
        let _ = self.sync_active_lsp_document();

        let active_id = self.session.active_id();
        let Some(document) = self.lsp.documents.get(&active_id).cloned() else {
            if manual {
                self.set_status("no LSP document for current buffer");
            }
            return;
        };
        let cursor = self.active_cursor_pos();
        let Some(buffer) = self.session.buffer(active_id) else {
            if manual {
                self.set_status("active buffer unavailable");
            }
            return;
        };
        let line = cursor.line.min(buffer.len_lines().saturating_sub(1));
        let character = char_col_to_utf16(&buffer.line_string(line), cursor.col);

        let pending = self
            .lsp
            .pending_requests
            .iter()
            .filter_map(|(key, request)| {
                (key.workspace == document.workspace
                    && matches!(request.kind, PendingRequest::Completion { .. }))
                .then_some(key.clone())
            })
            .collect::<Vec<_>>();
        for key in pending {
            self.lsp.pending_requests.remove(&key);
            self.send_lsp_cancel_request(&key);
        }

        let Some(client) = self.lsp.clients.get_mut(&document.workspace) else {
            if manual {
                self.set_status("no LSP client for current buffer");
            }
            return;
        };
        if !client.session.is_initialized() {
            if manual {
                self.set_status("LSP still loading");
            }
            return;
        }
        match client
            .session
            .send_completion(&document.path, line, character)
        {
            Ok(id) => {
                self.lsp.pending_requests.insert(
                    RequestKey {
                        workspace: document.workspace,
                        id,
                    },
                    PendingClientRequest {
                        context: self.lsp_request_context(),
                        kind: PendingRequest::Completion {
                            requested_at: cursor,
                            manual,
                        },
                        started_at: Instant::now(),
                    },
                );
                if manual {
                    self.set_status("loading completions...");
                }
            }
            Err(error) => {
                if manual {
                    self.set_status(format!("completion request failed: {error}"));
                }
            }
        }
    }

    pub(super) fn completion_move(&mut self, delta: isize) -> bool {
        let Some(completion) = self.lsp.completion.as_mut() else {
            return false;
        };
        if completion.items.is_empty() {
            return false;
        }
        let max_index = completion.items.len().saturating_sub(1) as isize;
        completion.selected = (completion.selected as isize + delta).clamp(0, max_index) as usize;
        true
    }

    pub(super) fn symbol_info_move(&mut self, delta: isize) -> bool {
        let Some(symbol_info) = self.lsp.symbol_info.as_mut() else {
            return false;
        };
        symbol_info.scroll = symbol_info.scroll.saturating_add_signed(delta);
        true
    }

    fn ensure_symbol_info_layout(&mut self, width: usize) {
        let Some(symbol_info) = self.lsp.symbol_info.as_mut() else {
            return;
        };
        if symbol_info.cached_width == Some(width) {
            return;
        }
        symbol_info.display_lines = build_symbol_info_display_lines(&symbol_info.blocks, width);
        symbol_info.cached_width = Some(width);
    }

    pub(crate) fn clamp_symbol_info_scroll(&mut self, term_w: u16) {
        let width = symbol_info_content_width_limit(term_w);
        self.ensure_symbol_info_layout(width);
        if let Some(symbol_info) = self.lsp.symbol_info.as_mut() {
            let inner_h = symbol_info
                .display_lines
                .len()
                .clamp(1, SYMBOL_INFO_MAX_HEIGHT);
            let max_scroll = symbol_info.display_lines.len().saturating_sub(inner_h);
            symbol_info.scroll = symbol_info.scroll.min(max_scroll);
        }
    }

    pub(super) fn close_completion(&mut self) -> bool {
        self.cancel_lsp_request_family(PendingRequest::Completion {
            requested_at: self.active_cursor_pos(),
            manual: false,
        });
        self.lsp.auto_completion = None;
        self.lsp.completion.take().is_some()
    }

    pub(super) fn close_symbol_info(&mut self) -> bool {
        self.cancel_lsp_request_family(PendingRequest::SymbolInfo {
            requested_at: self.active_cursor_pos(),
            return_mode: self.mode,
        });
        let Some(symbol_info) = self.lsp.symbol_info.take() else {
            return false;
        };
        if self.mode == EditorMode::SymbolInfo {
            self.mode = symbol_info.return_mode;
        }
        true
    }

    pub(super) fn clear_symbol_info(&mut self) -> bool {
        self.cancel_lsp_request_family(PendingRequest::SymbolInfo {
            requested_at: self.active_cursor_pos(),
            return_mode: self.mode,
        });
        self.lsp.symbol_info.take().is_some()
    }

    pub(super) fn close_active_snippet(&mut self) -> bool {
        self.lsp.active_snippet.take().is_some()
    }

    pub(super) fn accept_completion(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> bool {
        if self.visible_completion_state().is_none() {
            self.close_completion();
            return false;
        }
        let state = self.lsp.completion.take().expect("visible completion");
        self.close_completion();
        let Some(item) = state.items.get(state.selected).cloned() else {
            return false;
        };
        let active_id = self.session.active_id();
        let buffer = self.session.active_buffer();
        let Some(edit) = completion_edit_for_buffer(&item, buffer, state.requested_at) else {
            self.set_status("completion has an invalid edit range");
            return true;
        };
        let additional_edits = match prepare_lsp_edits(buffer, &item.additional_text_edits) {
            Ok(edits) => edits,
            Err(error) => {
                self.set_status(format!("completion failed: {error}"));
                return true;
            }
        };
        let start_char = buffer.pos_to_char(edit.start);
        let end_char = buffer.pos_to_char(edit.end);
        if additional_edits
            .iter()
            .any(|(start, end, _)| *start == start_char || *end > start_char && *start < end_char)
        {
            self.set_status("completion has overlapping edits");
            return true;
        }
        let snippet_expansion = completion_snippet_expansion(&item, &edit.insert);
        let preserve_active_snippet_edit = snippet_expansion
            .is_none()
            .then(|| self.active_snippet_completion_edit(buffer, active_id, &edit))
            .flatten();
        let insert = snippet_expansion
            .as_ref()
            .map(|expansion| expansion.text.clone())
            .unwrap_or(edit.insert);
        let inserted_start = transform_snippet_char_left(start_char, &additional_edits);
        let inserted_end = inserted_start.saturating_add(insert.chars().count());
        let mut edits = additional_edits;
        edits.push((start_char, end_char, insert));
        self.remember_completion_accept(&item);
        let before = self.capture_active_insert_coalesced_checkpoint();
        apply_character_edits(self.session.active_buffer_mut(), &edits);
        let cursor_char = if let Some(expansion) = snippet_expansion {
            self.lsp.active_snippet =
                active_snippet_from_expansion(active_id, inserted_start, &expansion);
            self.lsp
                .active_snippet
                .as_ref()
                .and_then(|snippet| snippet.placeholders.first())
                .map(|placeholder| placeholder.start_char)
                .or_else(|| {
                    expansion
                        .cursor_offset
                        .map(|offset| inserted_start.saturating_add(offset))
                })
                .unwrap_or(inserted_end)
        } else {
            if let Some((tabstop, _, _)) = preserve_active_snippet_edit {
                self.update_active_snippet_after_edits(&edits);
                self.mark_active_snippet_tabstop_filled(tabstop);
                if let Some(snippet) = self.lsp.active_snippet.as_mut() {
                    snippet.selected = false;
                }
            } else {
                self.lsp.active_snippet = None;
            }
            inserted_end
        };
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        view.cursor.cursor = buffer.char_to_pos(cursor_char.min(buffer.len_chars()));
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
        true
    }

    fn active_snippet_completion_edit(
        &self,
        buffer: &redox_core::TextBuffer,
        active_id: BufferId,
        edit: &CompletionEdit,
    ) -> Option<(usize, usize, usize)> {
        let snippet = self.lsp.active_snippet.as_ref()?;
        if snippet.buffer_id != active_id {
            return None;
        }
        let placeholder = snippet.placeholders.get(snippet.current)?;
        let start_char = buffer.pos_to_char(edit.start);
        let end_char = buffer.pos_to_char(edit.end);
        (start_char >= placeholder.start_char && end_char <= placeholder.end_char).then_some((
            placeholder.tabstop,
            start_char,
            end_char,
        ))
    }

    fn remember_completion_accept(&mut self, item: &CompletionCandidate) {
        let key = item
            .filter_text
            .as_deref()
            .unwrap_or(&item.label)
            .to_ascii_lowercase();
        let next = self
            .lsp
            .recent_completions
            .get(&key)
            .copied()
            .unwrap_or(0)
            .saturating_add(1)
            .min(25);
        self.lsp.recent_completions.insert(key, next);
    }

    pub(super) fn notify_active_lsp_did_save(&mut self) -> io::Result<()> {
        self.notify_lsp_did_save(self.session.active_id())
    }

    fn notify_lsp_did_save(&mut self, buffer_id: BufferId) -> io::Result<()> {
        let result = (|| {
            self.sync_lsp_document(buffer_id, SyncPolicy::Immediate)?;
            if let Some(document) = self.lsp.documents.get(&buffer_id)
                && document.opened
                && let Some(client) = self.lsp.clients.get_mut(&document.workspace)
            {
                client.session.send_did_save(&document.path)?;
            }
            Ok(())
        })();
        if let Some(request) = self.saved_buffer_lint_context(buffer_id) {
            self.start_lint_run(request);
        }
        result
    }

    fn poll_lint_runs(&mut self) {
        let mut pending = Vec::new();
        let mut completed = Vec::new();
        for run in self.lsp.lint_runs.drain(..) {
            match run.receiver.try_recv() {
                Ok(result) => completed.push((run.request, Some(result))),
                Err(TryRecvError::Empty) => pending.push(run),
                Err(TryRecvError::Disconnected) => completed.push((run.request, None)),
            }
        }
        self.lsp.lint_runs = pending;
        for (request, result) in completed {
            if self.lint_request_is_current(&request)
                && let Some(result) = result
            {
                let source = DiagnosticSource::Lint(result.source.clone());
                if result.source.kind == LintRunnerKind::Ruff {
                    self.remove_diagnostics_for_source_uri(&request.uri, &source);
                } else {
                    self.remove_diagnostics_for_source_everywhere(&source);
                }
                for (uri, diagnostics) in result.diagnostics_by_uri {
                    self.replace_diagnostics_for_source(uri, source.clone(), diagnostics);
                }
                if let Some(error) = result.error {
                    self.set_status(error);
                }
            }
            if let Some(index) = self.lsp.queued_lint_runs.iter().position(|queued| {
                queued.source == request.source && queued.buffer_id == request.buffer_id
            }) {
                let queued = self.lsp.queued_lint_runs.swap_remove(index);
                if self.lint_request_is_current(&queued) {
                    self.start_lint_run(queued);
                }
            }
        }
    }

    fn lint_source_for_path(&self, path: &Path, language: SyntaxLanguage) -> Option<LintSource> {
        let launch_dir = self.session.launch_dir();
        let (kind, provider_id) = match language {
            SyntaxLanguage::Rust => (LintRunnerKind::Clippy, ProviderId::RustAnalyzer),
            SyntaxLanguage::Go => (LintRunnerKind::GolangciLint, ProviderId::Gopls),
            SyntaxLanguage::Python => (LintRunnerKind::Ruff, ProviderId::Pyright),
            _ => return None,
        };

        let provider = provider_spec(provider_id)?;
        let root = workspace_root_for(path, &provider, launch_dir);
        Some(LintSource { kind, root })
    }

    fn saved_buffer_lint_context(&self, buffer_id: BufferId) -> Option<QueuedLintRun> {
        let meta = self.session.meta(buffer_id)?;
        let path = meta.path.as_deref()?;
        let source = self.lint_source_for_path(path, language_for_path(Some(path))?)?;
        if !self
            .lsp
            .installed
            .contains_key(&MarketplaceItemId::Linter(source.kind))
        {
            return None;
        }
        Some(QueuedLintRun {
            source,
            path: path.to_path_buf(),
            uri: file_uri(path).ok()?,
            buffer_id,
            analysis_version: self
                .views
                .get(&buffer_id)
                .map_or(0, |view| view.analysis_version()),
        })
    }

    fn lint_request_is_current(&self, request: &QueuedLintRun) -> bool {
        self.saved_buffer_lint_context(request.buffer_id)
            .is_some_and(|current| {
                current.source == request.source
                    && current.path == request.path
                    && current.analysis_version == request.analysis_version
            })
    }

    fn start_lint_run(&mut self, request: QueuedLintRun) {
        if self.lsp.lint_runs.iter().any(|run| {
            run.request.source == request.source && run.request.buffer_id == request.buffer_id
        }) {
            self.lsp.queued_lint_runs.retain(|queued| {
                queued.source != request.source || queued.buffer_id != request.buffer_id
            });
            self.lsp.queued_lint_runs.push(request);
            return;
        }
        let source = request.source.clone();
        let path = request.path.clone();
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name(format!("redox-lint-{}", source.kind.executable()))
            .spawn(move || {
                if lint_runner_available(&source, &path) {
                    let _ = sender.send(run_lint_source(&source, &path));
                }
            }) {
            Ok(_) => self
                .lsp
                .lint_runs
                .push(PendingLintRun { request, receiver }),
            Err(error) => self.set_status(format!("failed to start linter: {error}")),
        }
    }

    fn take_definition_response(
        &mut self,
        workspace: &WorkspaceKey,
        message: &Value,
    ) -> Option<DefinitionTarget> {
        let id = message.get("id")?.as_i64()?;
        let key = RequestKey {
            workspace: workspace.clone(),
            id,
        };
        if !self
            .lsp
            .pending_requests
            .get(&key)
            .is_some_and(|request| request.kind == PendingRequest::GotoDefinition)
        {
            return None;
        }
        let request = self.lsp.pending_requests.remove(&key)?;
        if request.context != self.lsp_request_context() {
            return None;
        }
        match request.kind {
            PendingRequest::GotoDefinition => {
                if let Some(error) = message.get("error") {
                    let detail = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown LSP error");
                    self.set_status(format!("definition lookup failed: {detail}"));
                    return None;
                }
                let target = parse_definition_response(message);
                if target.is_none() {
                    self.set_status("definition not found");
                }
                target
            }
            PendingRequest::CodeActions { .. }
            | PendingRequest::ExecuteCommand { .. }
            | PendingRequest::SymbolInfo { .. }
            | PendingRequest::Completion { .. } => None,
        }
    }

    fn take_symbol_info_response(&mut self, workspace: &WorkspaceKey, message: &Value) -> bool {
        let Some(id) = message.get("id").and_then(Value::as_i64) else {
            return false;
        };
        let key = RequestKey {
            workspace: workspace.clone(),
            id,
        };
        let Some(request) = self.lsp.pending_requests.get(&key).cloned() else {
            return false;
        };
        let PendingRequest::SymbolInfo {
            requested_at,
            return_mode,
        } = request.kind
        else {
            return false;
        };
        self.lsp.pending_requests.remove(&key);

        if request.context != self.lsp_request_context() {
            return true;
        }

        if let Some(error) = message.get("error") {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown LSP error");
            self.set_status(format!("symbol info failed: {detail}"));
            return true;
        }

        let blocks = parse_hover_response(message);
        if blocks.is_empty() {
            self.clear_symbol_info();
            self.set_status("no symbol info");
            return true;
        }

        self.show_symbol_info(blocks, requested_at, return_mode);
        true
    }

    fn take_completion_response(&mut self, workspace: &WorkspaceKey, message: &Value) -> bool {
        let Some(id) = message.get("id").and_then(Value::as_i64) else {
            return false;
        };
        let key = RequestKey {
            workspace: workspace.clone(),
            id,
        };
        let Some(request) = self.lsp.pending_requests.get(&key).cloned() else {
            return false;
        };
        let PendingRequest::Completion {
            requested_at,
            manual,
        } = request.kind
        else {
            return false;
        };
        self.lsp.pending_requests.remove(&key);
        if request.context != self.lsp_request_context() || self.mode != EditorMode::Insert {
            return true;
        }

        if let Some(error) = message.get("error") {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown LSP error");
            self.set_status(format!("completion failed: {detail}"));
            return true;
        }

        let mut items = parse_completion_response(message);
        if let Some(buffer) = self.session.buffer(self.session.active_id()) {
            let prefix = completion_prefix(buffer, requested_at);
            let context = completion_context(buffer, requested_at);
            items = filter_and_sort_completion_items(
                items,
                &prefix,
                &context,
                &self.lsp.recent_completions,
            );
        }
        items.truncate(100);
        if self.active_cursor_pos() != requested_at
            || (!manual && self.cursor_is_in_comment_for_completion())
        {
            self.clear_status();
            return true;
        }
        self.lsp.completion = Some(CompletionState {
            context: self.lsp_request_context(),
            selected: 0,
            requested_at,
            items,
        });
        self.clear_status();
        true
    }

    fn cursor_is_in_comment_for_completion(&self) -> bool {
        matches!(
            self.completion_context_syntax_role(),
            Some(SyntaxRole::Comment)
        )
    }

    fn visible_completion_state(&self) -> Option<&CompletionState> {
        let state = self.lsp.completion.as_ref()?;
        (!state.items.is_empty()
            && state.context == self.lsp_request_context()
            && self.mode == EditorMode::Insert)
            .then_some(state)
    }

    fn refilter_completion_for_active_cursor(&mut self, inserted: char) {
        let context = self.lsp_request_context();
        let Some(buffer) = self.session.buffer(context.buffer_id) else {
            return;
        };
        let Some(mut completion) = self.lsp.completion.take() else {
            return;
        };
        let previous = completion.context;
        if previous.buffer_id != context.buffer_id
            || previous.mode != EditorMode::Insert
            || previous.analysis_version.checked_add(1) != Some(context.analysis_version)
            || context.cursor
                != Pos::new(previous.cursor.line, previous.cursor.col.saturating_add(1))
            || completion_prefix_start(buffer, previous.cursor)
                != completion_prefix_start(buffer, context.cursor)
        {
            return;
        }
        let at = IncomingPosition {
            line: previous.cursor.line as u64,
            character: char_col_to_utf16(
                &buffer.line_string(previous.cursor.line),
                previous.cursor.col,
            ) as u64,
        };
        let shift = |position: &mut IncomingPosition, include_boundary: bool| {
            if position.line == at.line
                && (position.character > at.character
                    || include_boundary && position.character == at.character)
            {
                position.character = position
                    .character
                    .saturating_add(inserted.len_utf16() as u64);
            }
        };
        for item in &mut completion.items {
            if let Some(edit) = item.text_edit.as_mut() {
                shift(&mut edit.range.start, false);
                shift(&mut edit.range.end, true);
            }
            for edit in &mut item.additional_text_edits {
                shift(&mut edit.range.start, true);
                shift(&mut edit.range.end, true);
            }
        }
        completion.items = filter_and_sort_completion_items(
            completion.items,
            &completion_prefix(buffer, context.cursor),
            &completion_context(buffer, context.cursor),
            &self.lsp.recent_completions,
        );
        completion.selected = completion
            .selected
            .min(completion.items.len().saturating_sub(1));
        completion.requested_at = context.cursor;
        completion.context = context;
        self.lsp.completion = Some(completion);
    }

    fn selected_completion_symbol_info_blocks(&self) -> Vec<SymbolInfoBlock> {
        let Some(completion) = self.visible_completion_state() else {
            return Vec::new();
        };
        let Some(item) = completion.items.get(completion.selected) else {
            return Vec::new();
        };
        let mut blocks = Vec::new();
        for text in [
            item.detail.as_deref(),
            item.label_detail.as_deref(),
            item.label_description.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        {
            if !blocks
                .iter()
                .any(|block: &SymbolInfoBlock| block.text == text)
            {
                blocks.push(SymbolInfoBlock {
                    kind: SymbolInfoKind::PlainText,
                    text: text.to_string(),
                });
            }
        }
        if let Some(documentation) = &item.documentation {
            blocks.push(documentation.clone());
        }
        blocks
    }

    fn show_symbol_info(
        &mut self,
        blocks: Vec<SymbolInfoBlock>,
        requested_at: Pos,
        return_mode: EditorMode,
    ) {
        self.lsp.symbol_info = Some(SymbolInfoState {
            requested_at,
            blocks,
            cached_width: None,
            display_lines: Vec::new(),
            scroll: 0,
            return_mode,
        });
        self.mode = EditorMode::SymbolInfo;
        self.clear_status();
    }

    fn completion_context_syntax_role(&self) -> Option<SyntaxRole> {
        let active_id = self.session.active_id();
        let buffer = self.session.buffer(active_id)?;
        let view = self.views.get(&active_id)?;
        let cursor = buffer.clamp_pos(view.cursor.cursor);
        let line = cursor.line.min(buffer.len_lines().saturating_sub(1));
        let line_text = buffer.line_string(line);
        if line_text.is_empty() {
            return None;
        }

        let probe_col = cursor.col.min(line_text.chars().count());
        let probe_byte = if probe_col == 0 {
            0
        } else {
            line_text
                .char_indices()
                .nth(probe_col.saturating_sub(1))
                .map(|(idx, _)| idx)?
        };
        let language = language_for_path(
            self.session
                .meta(active_id)
                .and_then(|meta| meta.path.as_deref()),
        );

        if let Some(spans) = view
            .syntax_highlighter
            .visible_line_spans_cached(language, line, 1)
            .and_then(|visible| visible.get(0))
        {
            if let Some(role) = syntax_role_covering_byte(spans, probe_byte) {
                return Some(role);
            }
        }

        let fallback = lexical_fallback_line_spans(&line_text);
        syntax_role_covering_byte(&fallback, probe_byte)
    }

    fn jump_to_definition_target(&mut self, target: DefinitionTarget) {
        let Some(path) = file_path_from_uri(&target.uri) else {
            self.set_status("definition target is not a local file");
            return;
        };
        self.transient_origin_buffer_id = None;
        self.transient_origin_dir = None;
        match self.session.open_file(&path) {
            Ok(buffer_id) => {
                let _ = self.views.entry(buffer_id).or_default();
                self.ensure_buffer_analysis(buffer_id);
                self.ensure_active_lsp_client();

                if let Err(error) = self.session.ensure_buffer_fully_loaded(buffer_id) {
                    self.set_status(format!("definition load failed: {error}"));
                    return;
                }

                let (viewport_width_cells, viewport_height_rows) = self.viewport_size();
                let text_vh =
                    viewport_height_rows.saturating_sub(crate::ui::STATUS_BAR_HEIGHT_ROWS);
                let _ = self.with_buffer_view_mut(buffer_id, |buffer, view| {
                    let line = target
                        .range
                        .start
                        .line
                        .min(buffer.len_lines().saturating_sub(1) as u64)
                        as usize;
                    let col = utf16_code_unit_to_char_col(
                        &buffer.line_string(line),
                        u32::try_from(target.range.start.character).unwrap_or(u32::MAX),
                    );
                    view.cursor.cursor = buffer.clamp_pos(Pos::new(line, col));
                    view.cursor
                        .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                });
                self.clear_status();
            }
            Err(error) => {
                self.set_status(format!("definition open failed: {error}"));
            }
        }
    }

    fn marketplace_entry(&self, item: MarketplaceSpec) -> LspMarketplaceEntry {
        let installed = self.lsp.installed.contains_key(&item.id());
        let executable_found = self.marketplace_tool_available(item);
        let pending = self
            .lsp
            .provider_operations
            .get(&item.id())
            .map(|op| op.kind);
        let (action_label, status_label, status_kind) = if let Some(kind) = pending {
            match kind {
                ProviderOperationKind::Installing => (
                    "…".to_string(),
                    "installing…".to_string(),
                    LspEntryStatusKind::Pending,
                ),
                ProviderOperationKind::Uninstalling => (
                    "…".to_string(),
                    "uninstalling…".to_string(),
                    LspEntryStatusKind::Pending,
                ),
            }
        } else if installed {
            (
                "u".to_string(),
                if executable_found {
                    "ready".to_string()
                } else {
                    "enabled only".to_string()
                },
                if executable_found {
                    LspEntryStatusKind::Ready
                } else {
                    LspEntryStatusKind::Informational
                },
            )
        } else if executable_found {
            (
                "i".to_string(),
                "found on PATH".to_string(),
                LspEntryStatusKind::Ready,
            )
        } else if let Some(plan) = item
            .install_plans()
            .iter()
            .copied()
            .find(|plan| install_method_available(plan.method))
        {
            (
                "i".to_string(),
                format!("installs via {}", install_method_label(plan.method)),
                LspEntryStatusKind::Informational,
            )
        } else if item.install_plans().is_empty() {
            (
                "i".to_string(),
                "manual install".to_string(),
                LspEntryStatusKind::Missing,
            )
        } else {
            (
                "i".to_string(),
                "installer unavailable".to_string(),
                LspEntryStatusKind::Missing,
            )
        };

        LspMarketplaceEntry {
            item_id: item.id(),
            tool_label: format!("{} ({})", item.label(), item.id().kind_label()),
            language_label: item.language_label().to_string(),
            installed,
            action_label,
            status_label,
            status_kind,
        }
    }

    fn active_provider_operation_toast(&self, now: Instant) -> Option<String> {
        let (&item_id, operation) = self.lsp.provider_operations.iter().next()?;
        let item = marketplace_spec(item_id)?;
        let elapsed = now.saturating_duration_since(operation.started_at);
        let idx = ((elapsed.as_millis() / 100) as usize) % LSP_SPINNER_FRAMES.len();
        let verb = match operation.kind {
            ProviderOperationKind::Installing => "installing",
            ProviderOperationKind::Uninstalling => "uninstalling",
        };
        Some(format!(
            "{} {} {}",
            LSP_SPINNER_FRAMES[idx],
            verb,
            item.label()
        ))
    }

    fn poll_provider_operations(&mut self) {
        let provider_ids = self
            .lsp
            .provider_operations
            .keys()
            .copied()
            .collect::<Vec<_>>();
        let mut completed = Vec::new();
        for provider_id in provider_ids {
            let Some(result) = self
                .lsp
                .provider_operations
                .get(&provider_id)
                .and_then(|operation| operation.receiver.try_recv().ok())
            else {
                continue;
            };
            completed.push(result);
        }

        for result in completed {
            self.lsp.provider_operations.remove(&result.item_id);
            if result.success {
                match result.kind {
                    ProviderOperationKind::Installing => {
                        self.lsp.installed.insert(
                            result.item_id,
                            InstalledToolRecord {
                                install_source: result.install_source,
                            },
                        );
                        if let Err(error) = save_installed_tools(&self.lsp.installed) {
                            self.set_status(format!("failed to save installed tools: {error}"));
                        } else {
                            self.set_status(result.message);
                        }
                        self.refresh_lsp_tool_availability();
                        self.ensure_active_lsp_client();
                    }
                    ProviderOperationKind::Uninstalling => {
                        self.lsp.installed.remove(&result.item_id);
                        if let Err(error) = save_installed_tools(&self.lsp.installed) {
                            self.set_status(format!("failed to save installed tools: {error}"));
                        } else {
                            self.set_status(result.message);
                        }
                        self.refresh_lsp_tool_availability();
                        if let MarketplaceItemId::Provider(provider_id) = result.item_id {
                            self.remove_provider_runtime_state(provider_id);
                        }
                    }
                }
            } else {
                self.set_status(result.message);
            }
        }
    }

    fn start_provider_install(&mut self, item: MarketplaceSpec) -> bool {
        let Some(plan) = item
            .install_plans()
            .iter()
            .copied()
            .find(|plan| install_method_available(plan.method))
        else {
            return false;
        };
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name(format!("redox-install-{}", item.label()))
            .spawn(move || {
                let result = run_provider_install(item, plan);
                let _ = tx.send(result);
            })
            .expect("failed to start provider install");
        self.lsp.provider_operations.insert(
            item.id(),
            ProviderOperation {
                kind: ProviderOperationKind::Installing,
                started_at: Instant::now(),
                receiver: rx,
            },
        );
        true
    }

    fn start_provider_uninstall(
        &mut self,
        item: MarketplaceSpec,
        record: &InstalledToolRecord,
    ) -> bool {
        let Some(plan) = record.install_source.and_then(|method| {
            item.install_plans()
                .iter()
                .copied()
                .find(|plan| plan.method == method)
        }) else {
            return false;
        };
        if matches!(plan.uninstall, Uninstall::DisableOnly) {
            return false;
        }
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name(format!("redox-uninstall-{}", item.label()))
            .spawn(move || {
                let result = run_provider_uninstall(item, plan);
                let _ = tx.send(result);
            })
            .expect("failed to start provider uninstall");
        self.lsp.provider_operations.insert(
            item.id(),
            ProviderOperation {
                kind: ProviderOperationKind::Uninstalling,
                started_at: Instant::now(),
                receiver: rx,
            },
        );
        true
    }

    fn respond_to_lsp_server_request(&mut self, workspace: &WorkspaceKey, message: &Value) -> bool {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return false;
        };
        let Some(id) = message.get("id").cloned() else {
            return false;
        };

        let result = match method {
            "client/registerCapability"
            | "client/unregisterCapability"
            | "window/showMessageRequest" => Some(Value::Null),
            "workspace/configuration" => Some(configuration_response(message)),
            "workspace/workspaceFolders" => Some(workspace_folders_response(&workspace.root)),
            "workspace/applyEdit" => Some(self.workspace_apply_edit_response(message)),
            _ => None,
        };

        let Some(client) = self.lsp.clients.get_mut(workspace) else {
            return true;
        };
        if let Some(result) = result {
            let _ = client.session.send_response(id, result);
        } else {
            let _ = client.session.send_method_not_found(id, method);
        }
        true
    }

    fn workspace_apply_edit_response(&mut self, message: &Value) -> Value {
        let Some(edit_value) = message.get("params").and_then(|params| params.get("edit")) else {
            return json!({
                "applied": false,
                "failureReason": "missing edit payload",
            });
        };
        let Some(edit) = parse_workspace_edit(edit_value) else {
            return json!({
                "applied": false,
                "failureReason": "unsupported workspace edit",
            });
        };

        match self.apply_workspace_edit(&edit) {
            Ok(()) => json!({ "applied": true }),
            Err(error) => json!({
                "applied": false,
                "failureReason": error.to_string(),
            }),
        }
    }

    fn marketplace_tool_available(&self, item: MarketplaceSpec) -> bool {
        self.lsp
            .tool_availability
            .get(&item.id())
            .copied()
            .unwrap_or_else(|| marketplace_tool_available(item))
    }

    fn refresh_lsp_tool_availability(&mut self) {
        self.lsp.tool_availability = PROVIDERS
            .iter()
            .copied()
            .map(MarketplaceSpec::Provider)
            .chain(LINTERS.iter().copied().map(MarketplaceSpec::Linter))
            .map(|item| (item.id(), marketplace_tool_available(item)))
            .collect();
    }
}

fn prepare_lsp_edits(
    buffer: &redox_core::TextBuffer,
    edits: &[redox_lsp::TextEdit],
) -> io::Result<Vec<(usize, usize, String)>> {
    let mut prepared = edits
        .iter()
        .map(|edit| {
            let (start, end) =
                buffer_positions_for_range(buffer, &edit.range).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid LSP edit range")
                })?;
            Ok((
                buffer.pos_to_char(start),
                buffer.pos_to_char(end),
                edit.new_text.clone(),
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    // Apply right to left, including reversing equal-position insertions so
    // their final text keeps the server's order.
    prepared.reverse();
    prepared.sort_by_key(|(start, end, _)| std::cmp::Reverse((*start, *end)));
    if prepared.windows(2).any(|pair| pair[1].1 > pair[0].0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "overlapping LSP edits",
        ));
    }
    Ok(prepared)
}

fn buffer_positions_for_range(
    buffer: &redox_core::TextBuffer,
    range: &IncomingRange,
) -> Option<(Pos, Pos)> {
    let start_line = usize::try_from(range.start.line).ok()?;
    let end_line = usize::try_from(range.end.line).ok()?;
    if start_line >= buffer.len_lines() || end_line >= buffer.len_lines() {
        return None;
    }
    let column = |line, character| {
        let text = buffer.line_string(line);
        let text = text.trim_end_matches(['\r', '\n']);
        let character = u32::try_from(character).ok()?;
        let column = utf16_code_unit_to_char_col(text, character);
        (char_col_to_utf16(text, column) == character).then_some(column)
    };
    let start = Pos::new(start_line, column(start_line, range.start.character)?);
    let end = Pos::new(end_line, column(end_line, range.end.character)?);
    ((start.line, start.col) <= (end.line, end.col)).then_some((start, end))
}

fn syntax_role_covering_byte(
    spans: &[crate::ui::syntax::LineSyntaxSpan],
    byte: usize,
) -> Option<SyntaxRole> {
    spans
        .iter()
        .rev()
        .find(|span| span.start_byte <= byte && byte < span.end_byte)
        .map(|span| span.role)
}

fn marketplace_spec(item_id: MarketplaceItemId) -> Option<MarketplaceSpec> {
    match item_id {
        MarketplaceItemId::Provider(provider_id) => {
            provider_spec(provider_id).map(MarketplaceSpec::Provider)
        }
        MarketplaceItemId::Linter(kind) => linter_spec(kind).map(MarketplaceSpec::Linter),
    }
}

fn marketplace_tool_available(item: MarketplaceSpec) -> bool {
    tool_available(item.executable())
}

fn install_method_label(method: InstallMethodId) -> &'static str {
    method.as_str()
}

fn run_provider_install(
    item: MarketplaceSpec,
    plan: ProviderInstallPlan,
) -> ProviderOperationResult {
    let result = install_tool(item.label(), item.executable(), plan);
    ProviderOperationResult {
        item_id: item.id(),
        kind: ProviderOperationKind::Installing,
        install_source: result.install_source,
        success: result.success,
        message: result.message,
    }
}

fn run_provider_uninstall(
    item: MarketplaceSpec,
    plan: ProviderInstallPlan,
) -> ProviderOperationResult {
    let result = uninstall_tool(item.label(), plan);
    ProviderOperationResult {
        item_id: item.id(),
        kind: ProviderOperationKind::Uninstalling,
        install_source: result.install_source,
        success: result.success,
        message: result.message,
    }
}

fn load_installed_tools() -> HashMap<MarketplaceItemId, InstalledToolRecord> {
    let path = installed_tools_storage_path();
    let Ok(contents) = fs::read_to_string(path) else {
        return HashMap::new();
    };

    if let Ok(entries) = serde_json::from_str::<Vec<String>>(&contents) {
        return entries
            .into_iter()
            .filter_map(|entry| {
                entry.parse::<ProviderId>().ok().map(|id| {
                    (
                        MarketplaceItemId::Provider(id),
                        InstalledToolRecord {
                            install_source: None,
                        },
                    )
                })
            })
            .collect();
    }

    let Ok(entries) = serde_json::from_str::<Vec<Value>>(&contents) else {
        return HashMap::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("lsp");
            let id = match kind {
                "lsp" => MarketplaceItemId::Provider(entry.get("id")?.as_str()?.parse().ok()?),
                "linter" => MarketplaceItemId::Linter(entry.get("id")?.as_str()?.parse().ok()?),
                _ => return None,
            };
            let install_source = entry
                .get("install_source")
                .and_then(Value::as_str)
                .and_then(|value| value.parse().ok());
            Some((id, InstalledToolRecord { install_source }))
        })
        .collect()
}

fn save_installed_tools(
    installed: &HashMap<MarketplaceItemId, InstalledToolRecord>,
) -> io::Result<()> {
    let path = installed_tools_storage_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut entries = installed
        .iter()
        .map(|(item_id, record)| {
            json!({
                "kind": item_id.persistent_kind(),
                "id": item_id.id_str(),
                "install_source": record.install_source.map(InstallMethod::as_str),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    });
    let payload = serde_json::to_vec(&entries)?;
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, payload)?;
    fs::rename(temp_path, path)
}

fn syntax_language_label(language: SyntaxLanguage) -> &'static str {
    match language {
        SyntaxLanguage::C => "C",
        SyntaxLanguage::Cpp => "C++",
        SyntaxLanguage::Css => "CSS",
        SyntaxLanguage::Go => "Go",
        SyntaxLanguage::Html => "HTML",
        SyntaxLanguage::JavaScript => "JavaScript",
        SyntaxLanguage::Json => "JSON",
        SyntaxLanguage::Lua => "Lua",
        SyntaxLanguage::Markdown => "Markdown",
        SyntaxLanguage::Python => "Python",
        SyntaxLanguage::Rust => "Rust",
        SyntaxLanguage::Toml => "TOML",
        SyntaxLanguage::TypeScript => "TypeScript",
        SyntaxLanguage::Tsx => "TSX",
        SyntaxLanguage::Yaml => "YAML",
    }
}

fn reconcile_marketplace_scroll(
    entries: &[LspMarketplaceEntry],
    state: &mut LspMarketplaceState,
    viewport_height_rows: usize,
) {
    let visible_rows = viewport_height_rows
        .saturating_sub(crate::ui::STATUS_BAR_HEIGHT_ROWS)
        .saturating_sub(2);
    if visible_rows == 0 || entries.is_empty() {
        state.scroll = 0;
        return;
    }

    let installed_count = entries.iter().filter(|entry| entry.installed).count();
    let has_separator = installed_count > 0 && installed_count < entries.len();
    let selected_virtual = state
        .selected
        .saturating_add((has_separator && state.selected >= installed_count) as usize);
    let total_virtual_rows = entries.len().saturating_add(has_separator as usize);
    let max_scroll = total_virtual_rows.saturating_sub(visible_rows);

    if selected_virtual < state.scroll {
        state.scroll = selected_virtual;
    } else if selected_virtual >= state.scroll.saturating_add(visible_rows) {
        state.scroll = selected_virtual
            .saturating_add(1)
            .saturating_sub(visible_rows);
    }
    state.scroll = state.scroll.min(max_scroll);
}

fn installed_tools_storage_path() -> PathBuf {
    crate::storage::installed_tools_path()
}

fn apply_character_edits(buffer: &mut redox_core::TextBuffer, edits: &[(usize, usize, String)]) {
    let mut ordered = edits.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start_char, end_char, text) in ordered {
        let start = buffer.char_to_pos((*start_char).min(buffer.len_chars()));
        let end = buffer.char_to_pos((*end_char).min(buffer.len_chars()));
        let _ = buffer.delete_range(start, end);
        let _ = buffer.insert(start, text);
    }
}

fn transform_snippet_char(pos: usize, edits: &[(usize, usize, String)]) -> usize {
    let mut transformed = pos;
    let mut ordered = edits.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, text) in ordered {
        let replacement_len = text.chars().count();
        if transformed <= *start {
            continue;
        }
        if transformed >= *end {
            if replacement_len >= end.saturating_sub(*start) {
                transformed =
                    transformed.saturating_add(replacement_len - end.saturating_sub(*start));
            } else {
                transformed =
                    transformed.saturating_sub(end.saturating_sub(*start) - replacement_len);
            }
        } else {
            transformed = start.saturating_add(replacement_len);
        }
    }
    transformed
}

fn transform_snippet_char_left(pos: usize, edits: &[(usize, usize, String)]) -> usize {
    transform_snippet_char_left_with_skip(pos, edits, None)
}

fn transform_snippet_char_left_skipping(
    pos: usize,
    edits: &[(usize, usize, String)],
    skip_idx: usize,
) -> usize {
    transform_snippet_char_left_with_skip(pos, edits, Some(skip_idx))
}

fn transform_snippet_char_left_with_skip(
    pos: usize,
    edits: &[(usize, usize, String)],
    skip_idx: Option<usize>,
) -> usize {
    let mut transformed = pos;
    let mut ordered = edits.iter().enumerate().collect::<Vec<_>>();
    ordered.sort_by_key(|(_, (start, _, _))| std::cmp::Reverse(*start));
    for (idx, (start, end, text)) in ordered {
        if skip_idx == Some(idx) {
            continue;
        }
        let replacement_len = text.chars().count();
        let replaced_len = end.saturating_sub(*start);
        if transformed <= *start {
            continue;
        }
        if transformed >= *end {
            transformed = shift_snippet_char(transformed, replacement_len, replaced_len);
        } else {
            transformed = start.saturating_add(replacement_len);
        }
    }
    transformed
}

fn transform_snippet_char_right(pos: usize, edits: &[(usize, usize, String)]) -> usize {
    let mut transformed = pos;
    let mut ordered = edits.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, text) in ordered {
        let replacement_len = text.chars().count();
        let replaced_len = end.saturating_sub(*start);
        if transformed < *start {
            continue;
        }
        if *start == *end && transformed == *start {
            transformed = transformed.saturating_add(replacement_len);
        } else if transformed >= *end {
            transformed = shift_snippet_char(transformed, replacement_len, replaced_len);
        } else {
            transformed = start.saturating_add(replacement_len);
        }
    }
    transformed
}

fn shift_snippet_char(pos: usize, replacement_len: usize, replaced_len: usize) -> usize {
    if replacement_len >= replaced_len {
        pos.saturating_add(replacement_len - replaced_len)
    } else {
        pos.saturating_sub(replaced_len - replacement_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_core::EditorSession;

    fn completion_candidate(
        label: &str,
        insert_text: &str,
        kind: &str,
        insert_text_format: InsertTextFormat,
    ) -> CompletionCandidate {
        CompletionCandidate {
            label: label.to_string(),
            detail: None,
            label_detail: None,
            label_description: None,
            documentation: None,
            kind: Some(kind.to_string()),
            filter_text: None,
            sort_text: None,
            insert_text: insert_text.to_string(),
            insert_text_format,
            text_edit: None,
            additional_text_edits: Vec::new(),
        }
    }

    fn snippet_placeholder(
        tabstop: usize,
        start_char: usize,
        end_char: usize,
    ) -> ActiveSnippetPlaceholder {
        ActiveSnippetPlaceholder {
            tabstop,
            start_char,
            end_char,
            filled: false,
        }
    }

    fn state_with_active_snippet(
        text: &str,
        placeholders: Vec<ActiveSnippetPlaceholder>,
        final_char: Option<usize>,
    ) -> (EditorState, BufferId) {
        let session = EditorSession::open_initial_unnamed().expect("session should open");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_text(text);
        state.lsp.active_snippet = Some(ActiveSnippet {
            buffer_id,
            placeholders,
            current: 0,
            selected: true,
            final_char,
        });
        (state, buffer_id)
    }

    fn state_with_symbol_info(text: &str, scroll: usize, return_mode: EditorMode) -> EditorState {
        let session = EditorSession::open_initial_unnamed().expect("session should open");
        let mut state = EditorState::new(session);
        state.lsp.symbol_info = Some(SymbolInfoState {
            requested_at: Pos::new(0, 0),
            blocks: vec![SymbolInfoBlock {
                kind: SymbolInfoKind::PlainText,
                text: text.to_string(),
            }],
            cached_width: None,
            display_lines: Vec::new(),
            scroll,
            return_mode,
        });
        state
    }

    #[test]
    fn snippet_tab_skips_mirrored_placeholders() {
        let (mut state, buffer_id) = state_with_active_snippet(
            "foo, foo, bar",
            vec![
                snippet_placeholder(1, 0, 3),
                snippet_placeholder(1, 5, 8),
                snippet_placeholder(2, 10, 13),
            ],
            Some(13),
        );

        assert!(state.snippet_jump_next(80, 24));

        let snippet = state
            .lsp
            .active_snippet
            .as_ref()
            .expect("snippet should remain active");
        assert_eq!(snippet.current, 2);
        assert_eq!(
            state
                .views
                .get(&buffer_id)
                .expect("active view should exist")
                .cursor
                .cursor,
            Pos::new(0, 10)
        );
    }
    #[test]
    fn snippet_tab_repeats_after_placeholder_edits() {
        let (mut state, _) = state_with_active_snippet(
            "one, two, three",
            vec![
                snippet_placeholder(1, 0, 3),
                snippet_placeholder(2, 5, 8),
                snippet_placeholder(3, 10, 15),
            ],
            Some(15),
        );

        assert!(state.replace_active_snippet_selection_text("alpha", 80, 24));
        assert!(state.snippet_jump_next(80, 24));
        assert!(state.replace_active_snippet_selection_text("beta", 80, 24));
        assert!(state.snippet_jump_next(80, 24));

        let snippet = state
            .lsp
            .active_snippet
            .as_ref()
            .expect("snippet should remain active");
        assert_eq!(snippet.placeholders[snippet.current].tabstop, 3);
        assert_eq!(
            state.session.active_buffer().to_string(),
            "alpha, beta, three"
        );
        assert!(
            !state
                .active_snippet_placeholder_ranges(0, 1)
                .get(&0)
                .expect("placeholder ranges should remain visible")
                .is_empty()
        );
    }
    #[test]
    fn snippet_edits_update_mirrored_placeholders() {
        let (mut state, buffer_id) = state_with_active_snippet(
            "call(arg, arg)",
            vec![
                snippet_placeholder(1, 5, 8),
                snippet_placeholder(1, 10, 13),
                snippet_placeholder(2, 14, 14),
            ],
            Some(14),
        );

        assert!(state.replace_active_snippet_selection_text("f", 80, 24));
        assert!(
            !state
                .lsp
                .active_snippet
                .as_ref()
                .expect("snippet should remain active")
                .selected
        );

        for character in ['o', 'o'] {
            let text = character.to_string();
            let cursor = state
                .views
                .get(&buffer_id)
                .expect("active view should exist")
                .cursor
                .cursor;
            let insert_at_char = state.session.active_buffer().pos_to_char(cursor);
            let new_cursor = state.session.active_buffer_mut().insert(cursor, &text);
            state
                .views
                .get_mut(&buffer_id)
                .expect("active view should exist")
                .cursor
                .cursor = new_cursor;
            assert!(state.mirror_active_snippet_insert_after_cursor_insert(
                insert_at_char,
                &text,
                80,
                24,
            ));
        }
        assert_eq!(state.session.active_buffer().to_string(), "call(foo, foo)");

        let cursor = state
            .views
            .get(&buffer_id)
            .expect("active view should exist")
            .cursor
            .cursor;
        let deleted_end = state.session.active_buffer().pos_to_char(cursor);
        let deleted_start = deleted_end.saturating_sub(1);
        let new_cursor = state
            .session
            .active_buffer_mut()
            .backspace(redox_core::Selection::empty(cursor))
            .cursor;
        state
            .views
            .get_mut(&buffer_id)
            .expect("active view should exist")
            .cursor
            .cursor = new_cursor;

        assert!(state.mirror_active_snippet_delete_after_cursor_delete(
            deleted_start,
            deleted_end,
            80,
            24,
        ));
        assert_eq!(state.session.active_buffer().to_string(), "call(fo, fo)");

        assert!(state.snippet_jump_next(80, 24));
        let snippet = state
            .lsp
            .active_snippet
            .as_ref()
            .expect("snippet should remain active");
        assert_eq!(snippet.placeholders[snippet.current].tabstop, 2);
    }
    #[test]
    fn leaving_insert_mode_closes_active_snippet() {
        let (mut state, _) =
            state_with_active_snippet("", vec![snippet_placeholder(1, 0, 0)], Some(0));
        state.mode = EditorMode::Insert;

        state.apply_input(
            crate::input::InputAction::SetMode(crate::input::InputMode::Normal),
            80,
            24,
        );

        assert!(state.lsp.active_snippet.is_none());
    }
    #[test]
    fn completion_cancel_only_stays_in_insert_for_visible_popup() {
        let session = EditorSession::open_initial_unnamed().expect("session should open");
        let mut state = EditorState::new(session);
        state.mode = EditorMode::Insert;
        state.lsp.completion = Some(CompletionState {
            context: state.lsp_request_context(),
            selected: 0,
            requested_at: Pos::new(0, 0),
            items: vec![completion_candidate(
                "print",
                "print",
                "function",
                InsertTextFormat::PlainText,
            )],
        });

        state.apply_input(crate::input::InputAction::CompletionCancel, 80, 24);

        assert_eq!(state.mode, EditorMode::Insert);
        assert!(state.lsp.completion.is_none());

        state.lsp.completion = Some(CompletionState {
            context: state.lsp_request_context(),
            selected: 0,
            requested_at: Pos::new(0, 0),
            items: Vec::new(),
        });

        state.apply_input(crate::input::InputAction::CompletionCancel, 80, 24);

        assert_eq!(state.mode, EditorMode::Normal);
        assert!(state.lsp.completion.is_none());
    }
    #[test]
    fn completion_popup_visibility_tracks_current_prefix() {
        let session = EditorSession::open_initial_unnamed().expect("session should open");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_text("pri");
        state
            .views
            .get_mut(&buffer_id)
            .expect("active view should exist")
            .cursor
            .cursor = Pos::new(0, 3);
        state.mode = EditorMode::Insert;
        state.lsp.completion = Some(CompletionState {
            context: state.lsp_request_context(),
            selected: 0,
            requested_at: Pos::new(0, 3),
            items: vec![completion_candidate(
                "println",
                "println",
                "function",
                InsertTextFormat::PlainText,
            )],
        });

        state.apply_input(crate::input::InputAction::InsertChar('n'), 80, 24);
        assert!(state.completion_popup().is_some());

        state
            .views
            .get_mut(&buffer_id)
            .expect("active view should exist")
            .cursor
            .cursor = Pos::new(0, 0);
        assert!(state.completion_popup().is_none());
    }
    #[test]
    fn snippet_tab_clears_snippet_after_cursor_moves_away() {
        let (mut state, buffer_id) =
            state_with_active_snippet("foo bar", vec![snippet_placeholder(1, 0, 3)], Some(3));
        state
            .lsp
            .active_snippet
            .as_mut()
            .expect("snippet should be active")
            .selected = false;
        state
            .views
            .get_mut(&buffer_id)
            .expect("active view should exist")
            .cursor
            .cursor = Pos::new(0, 7);

        assert!(!state.snippet_jump_next(80, 24));
        assert!(state.lsp.active_snippet.is_none());
    }
    #[test]
    fn typing_in_comment_disables_auto_completion_and_closes_popup() {
        let session = EditorSession::open_initial_unnamed().expect("session should open");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_text("// comment");
        state
            .views
            .get_mut(&buffer_id)
            .expect("active view should exist")
            .cursor
            .cursor = Pos::new(0, 10);
        state.mode = EditorMode::Insert;
        state.lsp.completion = Some(CompletionState {
            context: state.lsp_request_context(),
            selected: 0,
            requested_at: Pos::new(0, 10),
            items: vec![completion_candidate(
                "comment",
                "comment",
                "text",
                InsertTextFormat::PlainText,
            )],
        });

        state.queue_auto_completion_after_insert('a');

        assert!(state.lsp.auto_completion.is_none());
        assert!(state.lsp.completion.is_none());
    }
    #[test]
    fn close_symbol_info_restores_previous_mode() {
        let mut state = state_with_symbol_info("hello", 2, EditorMode::Insert);
        state.mode = EditorMode::SymbolInfo;

        assert!(state.close_symbol_info());
        assert_eq!(state.mode, EditorMode::Insert);
        assert!(state.lsp.symbol_info.is_none());
    }
    #[test]
    fn clamp_symbol_info_scroll_trims_overscroll_to_visible_bottom() {
        let text = (0..20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = state_with_symbol_info(&text, 99, EditorMode::Normal);
        state.mode = EditorMode::SymbolInfo;

        state.clamp_symbol_info_scroll(80);

        assert_eq!(
            state.lsp.symbol_info.as_ref().map(|info| info.scroll),
            Some(8)
        );
    }
    #[test]
    fn lsp_errors_suppress_lint_diagnostics_for_active_file() {
        let lsp_source = DiagnosticSource::Lsp(WorkspaceKey {
            provider_id: ProviderId::RustAnalyzer,
            root: PathBuf::from("/tmp/project"),
        });
        let lint_source = DiagnosticSource::Lint(LintSource {
            kind: LintRunnerKind::Clippy,
            root: PathBuf::from("/tmp/project"),
        });
        let lsp_error = StoredDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: "this file contains an unclosed delimiter".to_string(),
            start_line: 4,
            end_line: 4,
            start_utf16: 0,
            end_utf16: 1,
            related_information: Vec::new(),
        };
        let lint_warning = StoredDiagnostic {
            severity: DiagnosticSeverity::Warning,
            message: "unused variable".to_string(),
            start_line: 1,
            end_line: 1,
            start_utf16: 0,
            end_utf16: 1,
            related_information: Vec::new(),
        };

        assert!(should_suppress_lint_diagnostics([
            (&lsp_source, &lsp_error),
            (&lint_source, &lint_warning),
        ]));
        assert!(!should_suppress_lint_diagnostics([(
            &lint_source,
            &lint_warning
        )]));
    }
}

#[cfg(test)]
mod regressions {
    use super::*;
    use redox_core::{EditorSession, TextBuffer};

    fn state(text: &str, cursor: Pos) -> EditorState {
        let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        *state.session.active_buffer_mut() = TextBuffer::from_text(text);
        let active = state.session.active_id();
        state.views.entry(active).or_default().cursor.cursor = cursor;
        state.mode = EditorMode::Insert;
        state
    }

    #[test]
    fn completion_replace_range_tracks_typing_inside_word() {
        for (text, inserted) in [("foobar", 'o'), ("fo𐐀bar", '𐐀')] {
            let mut state = state(text, Pos::new(0, 2));
            let items = parse_completion_response(&json!({"result":[{
                "label":text, "textEdit": {"newText":text, "range":{
                    "start":{"line":0,"character":0},
                    "end":{"line":0,"character":text.encode_utf16().count()}
                }}
            }]}));
            state.lsp.completion = Some(CompletionState {
                context: state.lsp_request_context(),
                selected: 0,
                requested_at: Pos::new(0, 2),
                items,
            });
            state.apply_input(crate::input::InputAction::InsertChar(inserted), 80, 24);
            assert!(state.has_visible_completion_popup());
            assert!(state.accept_completion(80, 24));
            assert_eq!(state.session.active_buffer().to_string(), text);
        }
    }

    #[test]
    fn completion_symbol_info_preserves_all_documentation() {
        let mut state = state("demo", Pos::new(0, 4));
        let items = parse_completion_response(&json!({"result":[{
            "label":"demo", "detail":"fn demo()", "documentation":{
                "kind":"markdown", "value":"Summary.\n\n# Examples\n\n```rust\ndemo();\n```"
            }
        }]}));
        state.lsp.completion = Some(CompletionState {
            context: state.lsp_request_context(),
            selected: 0,
            requested_at: Pos::new(0, 4),
            items,
        });
        state.trigger_symbol_info();
        let info = state.lsp.symbol_info.unwrap();
        assert!(
            info.blocks
                .iter()
                .any(|block| block.kind == SymbolInfoKind::Markdown
                    && block.text.contains("demo();")),
            "{:#?}",
            info.blocks
        );
    }

    #[test]
    fn late_hover_does_not_change_mode_after_cursor_moves() {
        let mut state = state("abc", Pos::new(0, 0));
        let workspace = WorkspaceKey {
            provider_id: ProviderId::RustAnalyzer,
            root: PathBuf::from("/tmp"),
        };
        state.lsp.pending_requests.insert(
            RequestKey {
                workspace: workspace.clone(),
                id: 2,
            },
            PendingClientRequest {
                context: state.lsp_request_context(),
                started_at: Instant::now(),
                kind: PendingRequest::SymbolInfo {
                    requested_at: Pos::new(0, 0),
                    return_mode: EditorMode::Insert,
                },
            },
        );
        for motion in [
            redox_core::motion::Motion::Right,
            redox_core::motion::Motion::Left,
        ] {
            state.apply_input(
                crate::input::InputAction::Motion { motion, count: 1 },
                80,
                24,
            );
        }
        state.take_symbol_info_response(
            &workspace,
            &json!({"id":2,"result":{"contents":"old symbol"}}),
        );
        assert!(state.lsp.symbol_info.is_none());
        assert_eq!(state.mode, EditorMode::Insert);
    }

    #[test]
    fn cancelled_completion_does_not_reopen_on_late_response() {
        let mut state = state("abc", Pos::new(0, 3));
        let workspace = WorkspaceKey {
            provider_id: ProviderId::RustAnalyzer,
            root: PathBuf::from("/tmp"),
        };
        state.lsp.pending_requests.insert(
            RequestKey {
                workspace: workspace.clone(),
                id: 2,
            },
            PendingClientRequest {
                context: state.lsp_request_context(),
                started_at: Instant::now(),
                kind: PendingRequest::Completion {
                    requested_at: Pos::new(0, 3),
                    manual: true,
                },
            },
        );
        state.close_completion();
        state.mode = EditorMode::Normal;
        state.take_completion_response(&workspace, &json!({"id":2,"result":[{"label":"abcdef"}]}));
        assert!(!state.has_visible_completion_popup());
    }

    #[test]
    fn workspace_edit_can_be_undone() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("example.txt");
        fs::write(&path, "before").unwrap();
        let mut session = EditorSession::open_initial_unnamed().unwrap();
        session.open_file(&path).unwrap();
        let mut state = EditorState::new(session);
        state.mode = EditorMode::Normal;
        let edit = parse_workspace_edit(&json!({"documentChanges":[{
            "textDocument":{"uri":file_uri(&path).unwrap(),"version":null},"edits":[{
            "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":6}}, "newText":"after"
        }]}]})).unwrap();
        state.apply_workspace_edit(&edit).unwrap();
        assert_eq!(state.session.active_buffer().to_string(), "after");
        state.undo_active(80, 24);
        assert_eq!(state.session.active_buffer().to_string(), "before");
    }

    #[test]
    fn linter_without_lsp_is_retained() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("example.py");
        fs::write(&path, "import os\n").unwrap();
        let mut session = EditorSession::open_initial_unnamed().unwrap();
        session.open_file(&path).unwrap();
        let mut state = EditorState::new(session);
        state.lsp.installed.clear();
        state.lsp.installed.insert(
            MarketplaceItemId::Linter(LintRunnerKind::Ruff),
            InstalledToolRecord {
                install_source: None,
            },
        );
        let request = state
            .saved_buffer_lint_context(state.session.active_id())
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let uri = request.uri.clone();
        let diagnostics = parse_publish_diagnostics(&json!({
            "method":"textDocument/publishDiagnostics", "params": {"uri":uri, "diagnostics":[{
                "range":{"start":{"line":0,"character":7},"end":{"line":0,"character":9}},
                "severity":2,"message":"unused import"
            }]}
        }))
        .unwrap()
        .2;
        let result = LintRunResult {
            source: request.source.clone(),
            diagnostics_by_uri: HashMap::from([(uri.clone(), diagnostics)]),
            error: None,
        };
        state
            .lsp
            .lint_runs
            .push(PendingLintRun { request, receiver });
        state.cleanup_orphaned_lsp_state();
        assert_eq!(state.lsp.lint_runs.len(), 1);
        sender.send(result).unwrap();
        state.poll_lint_runs();
        state.cleanup_orphaned_lsp_state();
        assert!(state.lsp.diagnostics.contains_key(&uri));
    }

    #[test]
    fn enabling_an_enabled_tool_preserves_uninstall_source() {
        let mut state = state("", Pos::new(0, 0));
        state.lsp.installed.clear();
        let selected = MarketplaceItemId::Provider(ProviderId::RustAnalyzer);
        state.lsp.installed.insert(
            selected,
            InstalledToolRecord {
                install_source: Some(InstallMethod::Brew),
            },
        );
        state.lsp.tool_availability = PROVIDERS
            .iter()
            .map(|provider| (MarketplaceItemId::Provider(provider.id), true))
            .chain(
                LINTERS
                    .iter()
                    .map(|linter| (MarketplaceItemId::Linter(linter.kind), true)),
            )
            .collect();
        state.lsp.marketplace = Some(LspMarketplaceState {
            selected: 0,
            scroll: 0,
        });
        assert_eq!(state.selected_marketplace_item().unwrap().id(), selected);
        state.install_selected_lsp();
        assert_eq!(
            state.lsp.installed[&selected].install_source,
            Some(InstallMethod::Brew)
        );
    }

    #[test]
    fn completion_applies_import_edit() {
        let mut state = state(
            "package main\n\nfunc main() {\n    fmt\n}\n",
            Pos::new(3, 7),
        );
        let items = parse_completion_response(&json!({"result":[{
            "label":"fmt", "insertText":"fmt", "additionalTextEdits":[{
                "range":{"start":{"line":1,"character":0},"end":{"line":1,"character":0}},
                "newText":"import \"fmt\"\n"
            }]
        }]}));
        state.lsp.completion = Some(CompletionState {
            context: state.lsp_request_context(),
            selected: 0,
            requested_at: Pos::new(3, 7),
            items,
        });
        assert!(state.accept_completion(80, 24));
        assert!(
            state
                .session
                .active_buffer()
                .to_string()
                .contains("import \"fmt\"")
        );
    }
    #[test]
    fn completion_imports_and_snippet_positions_are_one_undo_step() {
        let original = "head\n\nfoo\nend\n";
        let mut state = state(original, Pos::new(2, 3));
        let items = parse_completion_response(&json!({"result":[{
            "label":"foo", "insertText":"foo(${1:value})$0", "insertTextFormat":2,
            "additionalTextEdits":[
                {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"first import\n"},
                {"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":0}},"newText":"second import\n"},
                {"range":{"start":{"line":3,"character":0},"end":{"line":3,"character":3}},"newText":"tail"}
            ]
        }]}));
        state.lsp.completion = Some(CompletionState {
            context: state.lsp_request_context(),
            selected: 0,
            requested_at: Pos::new(2, 3),
            items,
        });
        assert!(state.accept_completion(80, 24));
        assert_eq!(
            state.session.active_buffer().to_string(),
            "first import\nhead\nsecond import\n\nfoo(value)\ntail\n"
        );
        assert_eq!(state.active_cursor_pos(), Pos::new(4, 4));
        let placeholder = &state.lsp.active_snippet.as_ref().unwrap().placeholders[0];
        assert_eq!(placeholder.end_char - placeholder.start_char, 5);
        state.apply_input(
            crate::input::InputAction::SetMode(crate::input::InputMode::Normal),
            80,
            24,
        );
        state.undo_active(80, 24);
        assert_eq!(state.session.active_buffer().to_string(), original);
    }

    #[test]
    fn workspace_edits_validate_all_files_before_mutation_and_record_inactive_undo() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        fs::write(&first, "one").unwrap();
        fs::write(&second, "😀two").unwrap();
        let mut session = EditorSession::open_initial_unnamed().unwrap();
        let first_id = session.open_file(&first).unwrap();
        let second_id = session.open_file(&second).unwrap();
        assert!(session.activate(first_id));
        let mut state = EditorState::new(session);
        state.lsp.installed.clear();
        let payload = json!({"documentChanges":[
            {"textDocument":{"uri":file_uri(&first).unwrap(),"version":null},"edits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"changed"}]},
            {"textDocument":{"uri":file_uri(&second).unwrap(),"version":null},"edits":[{"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":5}},"newText":"other"}]}
        ]});
        let mut edit = parse_workspace_edit(&payload).unwrap();
        assert!(state.apply_workspace_edit(&edit).is_err());
        assert_eq!(state.session.buffer(first_id).unwrap().to_string(), "one");
        assert_eq!(
            state.session.buffer(second_id).unwrap().to_string(),
            "😀two"
        );
        let second_edit = edit
            .document_edits
            .iter_mut()
            .find(|edit| edit.uri == file_uri(&second).unwrap())
            .unwrap();
        second_edit.edits[0].range.start.character = 0;
        state.apply_workspace_edit(&edit).unwrap();
        assert_eq!(state.session.active_id(), first_id);
        state.undo_active(80, 24);
        assert_eq!(state.session.active_buffer().to_string(), "one");
        assert!(state.session.activate(second_id));
        state.undo_active(80, 24);
        assert_eq!(state.session.active_buffer().to_string(), "😀two");
    }

    #[test]
    fn stale_linter_results_are_ignored_without_abandoning_the_worker() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("example.py");
        fs::write(&path, "import os\n").unwrap();
        let mut session = EditorSession::open_initial_unnamed().unwrap();
        session.open_file(&path).unwrap();
        let mut state = EditorState::new(session);
        state.lsp.installed.clear();
        state.lsp.installed.insert(
            MarketplaceItemId::Linter(LintRunnerKind::Ruff),
            InstalledToolRecord {
                install_source: None,
            },
        );
        let request = state
            .saved_buffer_lint_context(state.session.active_id())
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let result = LintRunResult {
            source: request.source.clone(),
            diagnostics_by_uri: HashMap::new(),
            error: Some("stale lint error".into()),
        };
        state
            .lsp
            .lint_runs
            .push(PendingLintRun { request, receiver });
        state.mode = EditorMode::Insert;
        state.apply_input(crate::input::InputAction::InsertChar('x'), 80, 24);
        state.cleanup_orphaned_lsp_state();
        assert_eq!(state.lsp.lint_runs.len(), 1);
        sender.send(result).unwrap();
        state.poll_lint_runs();
        assert!(state.lsp.lint_runs.is_empty());
        assert!(state.lsp.diagnostics.is_empty());
        assert!(
            !state
                .status_msg
                .as_ref()
                .is_some_and(|status| status.contains("stale lint error"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn initialization_timeout_backs_off_and_document_cleanup_removes_closed_buffers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("example.rs");
        fs::write(&path, "fn main() {}\n").unwrap();
        let mut session = EditorSession::open_initial_unnamed().unwrap();
        let buffer_id = session.open_file(&path).unwrap();
        let mut state = EditorState::new(session);
        let provider = provider_spec(ProviderId::RustAnalyzer).unwrap();
        let root = workspace_root_for(&path, &provider, state.session.launch_dir());
        let workspace = WorkspaceKey {
            provider_id: provider.id,
            root: root.clone(),
        };
        state.lsp.installed.clear();
        state.lsp.installed.insert(
            MarketplaceItemId::Provider(provider.id),
            InstalledToolRecord {
                install_source: None,
            },
        );
        let client = LspSession::spawn(
            &redox_lsp::ServerCommand::new("mock", "sh").args(&["-c", "exec sleep 10"]),
            &root,
            ClientInfo {
                name: "test",
                version: "1",
            },
        )
        .unwrap();
        state.lsp.clients.insert(
            workspace.clone(),
            ManagedClient {
                provider,
                session: client,
                loading_since: Instant::now() - LSP_INITIALIZE_TIMEOUT,
            },
        );
        state.ensure_active_lsp_client();
        assert!(state.lsp.documents.contains_key(&buffer_id));
        state.poll_lsp();
        assert!(!state.lsp.clients.contains_key(&workspace));
        assert!(state.lsp.retry_after[&workspace] > Instant::now());
        state.poll_lsp();
        assert!(!state.lsp.clients.contains_key(&workspace));
        assert!(!state.lsp.documents[&buffer_id].opened);
        state.session.close_buffer(buffer_id);
        state.cleanup_orphaned_lsp_state();
        assert!(!state.lsp.documents.contains_key(&buffer_id));
    }
}
