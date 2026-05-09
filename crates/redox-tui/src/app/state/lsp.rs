use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use redox_core::{BufferId, BufferKind, Pos};
use serde_json::{Value, json};
use url::Url;

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
use self::types::{DefinitionTarget, LspMarketplaceEntry, WorkspaceKey};
pub use types::{
    CodeActionPopup, CodeActionPopupEntry, CompletionEntry, CompletionPopup, CompletionPreview,
    DiagnosticsCodeActionsPane, DiagnosticsPopup, DiagnosticsPopupEntry, DiagnosticsPopupFocus,
    LspEntryStatusKind, LspMarketplacePopup, SymbolInfoBlock, SymbolInfoDisplayKind,
    SymbolInfoDisplayLine, SymbolInfoKind, SymbolInfoPopup,
};

const INSTALLED_LSPS_FILE: &str = "installed_lsps.json";
const INITIALIZE_REQUEST_ID: i64 = 1;
const FIRST_DYNAMIC_REQUEST_ID: i64 = 2;
const LSP_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const DIAGNOSTICS_POPUP_VISIBLE_ROWS: usize = 12;
const COMPLETION_POPUP_VISIBLE_ROWS: usize = 8;
const SYMBOL_INFO_MAX_HEIGHT: usize = 12;
const LSP_CHANGE_DEBOUNCE: Duration = Duration::from_millis(175);
const COMPLETION_AUTO_TRIGGER_DEBOUNCE: Duration = Duration::from_millis(90);
const COMPLETION_TRIGGER_CHARACTER_DEBOUNCE: Duration = Duration::from_millis(15);
const LSP_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

// Provider grouping should be fine
const JS_TS_LANGUAGES: &[SyntaxLanguage] = &[
    SyntaxLanguage::JavaScript,
    SyntaxLanguage::TypeScript,
    SyntaxLanguage::Tsx,
];
const CSS_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Css];
const HTML_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Html];
const JSON_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Json];
const LUA_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Lua];
const MARKDOWN_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Markdown];
const PYTHON_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Python];
const RUST_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Rust];
const TOML_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Toml];
const YAML_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Yaml];
const GO_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::Go];
const C_CPP_LANGUAGES: &[SyntaxLanguage] = &[SyntaxLanguage::C, SyntaxLanguage::Cpp];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ProviderId {
    RustAnalyzer,
    Clangd,
    Gopls,
    Pyright,
    TypeScriptLanguageServer,
    LuaLanguageServer,
    Taplo,
    Marksman,
    YamlLanguageServer,
    JsonLanguageServer,
    HtmlLanguageServer,
    CssLanguageServer,
}

impl ProviderId {
    fn as_str(self) -> &'static str {
        match self {
            Self::RustAnalyzer => "rust-analyzer",
            Self::Clangd => "clangd",
            Self::Gopls => "gopls",
            Self::Pyright => "pyright-langserver",
            Self::TypeScriptLanguageServer => "typescript-language-server",
            Self::LuaLanguageServer => "lua-language-server",
            Self::Taplo => "taplo",
            Self::Marksman => "marksman",
            Self::YamlLanguageServer => "yaml-language-server",
            Self::JsonLanguageServer => "vscode-json-language-server",
            Self::HtmlLanguageServer => "vscode-html-language-server",
            Self::CssLanguageServer => "vscode-css-language-server",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        PROVIDERS
            .iter()
            .find(|provider| provider.id.as_str() == value)
            .map(|provider| provider.id)
    }
}

#[derive(Debug, Clone, Copy)]
struct ProviderSpec {
    id: ProviderId,
    label: &'static str,
    language_label: &'static str,
    executable: &'static str,
    args: &'static [&'static str],
    languages: &'static [SyntaxLanguage],
    install_plans: &'static [ProviderInstallPlan],
}

impl ProviderSpec {
    fn matches_language(self, language: SyntaxLanguage) -> bool {
        self.languages.contains(&language)
    }

    fn language_id_for(self, language: SyntaxLanguage) -> Option<&'static str> {
        match (self.id, language) {
            (ProviderId::Clangd, SyntaxLanguage::C) => Some("c"),
            (ProviderId::Clangd, SyntaxLanguage::Cpp) => Some("cpp"),
            (ProviderId::TypeScriptLanguageServer, SyntaxLanguage::JavaScript) => {
                Some("javascript")
            }
            (ProviderId::TypeScriptLanguageServer, SyntaxLanguage::TypeScript) => {
                Some("typescript")
            }
            (ProviderId::TypeScriptLanguageServer, SyntaxLanguage::Tsx) => Some("typescriptreact"),
            (_, language) if self.matches_language(language) => Some(match self.id {
                ProviderId::RustAnalyzer => "rust",
                ProviderId::Gopls => "go",
                ProviderId::Pyright => "python",
                ProviderId::LuaLanguageServer => "lua",
                ProviderId::Taplo => "toml",
                ProviderId::Marksman => "markdown",
                ProviderId::YamlLanguageServer => "yaml",
                ProviderId::JsonLanguageServer => "json",
                ProviderId::HtmlLanguageServer => "html",
                ProviderId::CssLanguageServer => "css",
                ProviderId::Clangd | ProviderId::TypeScriptLanguageServer => return None,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LinterSpec {
    kind: LintRunnerKind,
    label: &'static str,
    language_label: &'static str,
    install_plans: &'static [ProviderInstallPlan],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum InstallMethodId {
    Brew,
    Cargo,
    Go,
    Npm,
    Rustup,
}

#[derive(Debug, Clone, Copy)]
struct ProviderInstallPlan {
    method: InstallMethodId,
    install_args: &'static [&'static str],
    uninstall: ProviderUninstall,
}

#[derive(Debug, Clone, Copy)]
enum ProviderUninstall {
    Command(&'static [&'static str]),
    GoBinary(&'static str),
    DisableOnly,
}

const BREW_RUST_ANALYZER: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Brew,
    install_args: &["install", "rust-analyzer"],
    uninstall: ProviderUninstall::Command(&["uninstall", "rust-analyzer"]),
}];
const BREW_LUA_LANGUAGE_SERVER: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Brew,
    install_args: &["install", "lua-language-server"],
    uninstall: ProviderUninstall::Command(&["uninstall", "lua-language-server"]),
}];
const BREW_MARKSMAN: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Brew,
    install_args: &["install", "marksman"],
    uninstall: ProviderUninstall::Command(&["uninstall", "marksman"]),
}];
const CARGO_TAPLO: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Cargo,
    install_args: &["install", "taplo-cli", "--locked"],
    uninstall: ProviderUninstall::Command(&["uninstall", "taplo-cli"]),
}];
const GO_GOPLS: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Go,
    install_args: &["install", "golang.org/x/tools/gopls@latest"],
    uninstall: ProviderUninstall::GoBinary("gopls"),
}];
const NPM_PYRIGHT: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Npm,
    install_args: &["install", "-g", "pyright"],
    uninstall: ProviderUninstall::Command(&["uninstall", "-g", "pyright"]),
}];
const NPM_TYPESCRIPT_LSP: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Npm,
    install_args: &["install", "-g", "typescript", "typescript-language-server"],
    uninstall: ProviderUninstall::Command(&[
        "uninstall",
        "-g",
        "typescript-language-server",
        "typescript",
    ]),
}];
const NPM_YAML_LSP: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Npm,
    install_args: &["install", "-g", "yaml-language-server"],
    uninstall: ProviderUninstall::Command(&["uninstall", "-g", "yaml-language-server"]),
}];
const NPM_VSCODE_JSON: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Npm,
    install_args: &["install", "-g", "vscode-langservers-extracted"],
    uninstall: ProviderUninstall::DisableOnly,
}];
const NPM_VSCODE_HTML: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Npm,
    install_args: &["install", "-g", "vscode-langservers-extracted"],
    uninstall: ProviderUninstall::DisableOnly,
}];
const NPM_VSCODE_CSS: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Npm,
    install_args: &["install", "-g", "vscode-langservers-extracted"],
    uninstall: ProviderUninstall::DisableOnly,
}];
const RUSTUP_CLIPPY: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Rustup,
    install_args: &["component", "add", "clippy"],
    uninstall: ProviderUninstall::Command(&["component", "remove", "clippy"]),
}];
const BREW_GOLANGCI_LINT: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Brew,
    install_args: &["install", "golangci-lint"],
    uninstall: ProviderUninstall::Command(&["uninstall", "golangci-lint"]),
}];
const GO_GOLANGCI_LINT: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Go,
    install_args: &[
        "install",
        "github.com/golangci/golangci-lint/cmd/golangci-lint@latest",
    ],
    uninstall: ProviderUninstall::GoBinary("golangci-lint"),
}];
const BREW_RUFF: &[ProviderInstallPlan] = &[ProviderInstallPlan {
    method: InstallMethodId::Brew,
    install_args: &["install", "ruff"],
    uninstall: ProviderUninstall::Command(&["uninstall", "ruff"]),
}];
const NO_AUTO_INSTALL: &[ProviderInstallPlan] = &[];

const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: ProviderId::RustAnalyzer,
        label: "rust-analyzer",
        language_label: "Rust",
        executable: "rust-analyzer",
        args: &[],
        languages: RUST_LANGUAGES,
        install_plans: BREW_RUST_ANALYZER,
    },
    ProviderSpec {
        id: ProviderId::Clangd,
        label: "clangd",
        language_label: "C / C++",
        executable: "clangd",
        args: &[],
        languages: C_CPP_LANGUAGES,
        install_plans: NO_AUTO_INSTALL,
    },
    ProviderSpec {
        id: ProviderId::Gopls,
        label: "gopls",
        language_label: "Go",
        executable: "gopls",
        args: &[],
        languages: GO_LANGUAGES,
        install_plans: GO_GOPLS,
    },
    ProviderSpec {
        id: ProviderId::Pyright,
        label: "pyright",
        language_label: "Python",
        executable: "pyright-langserver",
        args: &["--stdio"],
        languages: PYTHON_LANGUAGES,
        install_plans: NPM_PYRIGHT,
    },
    ProviderSpec {
        id: ProviderId::TypeScriptLanguageServer,
        label: "typescript-language-server",
        language_label: "JS/TS",
        executable: "typescript-language-server",
        args: &["--stdio"],
        languages: JS_TS_LANGUAGES,
        install_plans: NPM_TYPESCRIPT_LSP,
    },
    ProviderSpec {
        id: ProviderId::LuaLanguageServer,
        label: "lua-language-server",
        language_label: "Lua",
        executable: "lua-language-server",
        args: &[],
        languages: LUA_LANGUAGES,
        install_plans: BREW_LUA_LANGUAGE_SERVER,
    },
    ProviderSpec {
        id: ProviderId::Taplo,
        label: "taplo",
        language_label: "TOML",
        executable: "taplo",
        args: &["lsp", "stdio"],
        languages: TOML_LANGUAGES,
        install_plans: CARGO_TAPLO,
    },
    ProviderSpec {
        id: ProviderId::Marksman,
        label: "marksman",
        language_label: "Markdown",
        executable: "marksman",
        args: &["server"],
        languages: MARKDOWN_LANGUAGES,
        install_plans: BREW_MARKSMAN,
    },
    ProviderSpec {
        id: ProviderId::YamlLanguageServer,
        label: "yaml-language-server",
        language_label: "YAML",
        executable: "yaml-language-server",
        args: &["--stdio"],
        languages: YAML_LANGUAGES,
        install_plans: NPM_YAML_LSP,
    },
    ProviderSpec {
        id: ProviderId::JsonLanguageServer,
        label: "vscode-json-language-server",
        language_label: "JSON",
        executable: "vscode-json-language-server",
        args: &["--stdio"],
        languages: JSON_LANGUAGES,
        install_plans: NPM_VSCODE_JSON,
    },
    ProviderSpec {
        id: ProviderId::HtmlLanguageServer,
        label: "vscode-html-language-server",
        language_label: "HTML",
        executable: "vscode-html-language-server",
        args: &["--stdio"],
        languages: HTML_LANGUAGES,
        install_plans: NPM_VSCODE_HTML,
    },
    ProviderSpec {
        id: ProviderId::CssLanguageServer,
        label: "vscode-css-language-server",
        language_label: "CSS",
        executable: "vscode-css-language-server",
        args: &["--stdio"],
        languages: CSS_LANGUAGES,
        install_plans: NPM_VSCODE_CSS,
    },
];

const LINTERS: &[LinterSpec] = &[
    LinterSpec {
        kind: LintRunnerKind::Clippy,
        label: "clippy",
        language_label: "Rust",
        install_plans: RUSTUP_CLIPPY,
    },
    LinterSpec {
        kind: LintRunnerKind::GolangciLint,
        label: "golangci-lint",
        language_label: "Go",
        install_plans: &[BREW_GOLANGCI_LINT[0], GO_GOLANGCI_LINT[0]],
    },
    LinterSpec {
        kind: LintRunnerKind::Ruff,
        label: "ruff",
        language_label: "Python",
        install_plans: BREW_RUFF,
    },
];

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

struct PendingLintRun {
    source: LintSource,
    uri: String,
    document_version: i32,
    receiver: Receiver<LintRunResult>,
}

impl std::fmt::Debug for PendingLintRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingLintRun")
            .field("source", &self.source)
            .field("uri", &self.uri)
            .field("document_version", &self.document_version)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
struct QueuedLintRun {
    source: LintSource,
    path: PathBuf,
    uri: String,
    document_version: i32,
}

#[derive(Debug)]
struct LintRunResult {
    source: LintSource,
    diagnostics_by_uri: HashMap<String, Vec<StoredDiagnostic>>,
    error: Option<String>,
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

enum SessionEvent {
    Message(Value),
    Terminated,
}

struct LspSession {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<SessionEvent>,
    initialized: bool,
    next_request_id: i64,
}

impl std::fmt::Debug for LspSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspSession")
            .field("initialized", &self.initialized)
            .finish_non_exhaustive()
    }
}

impl Drop for LspSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl LspSession {
    fn spawn(provider: ProviderSpec, root: &Path) -> io::Result<Self> {
        let mut child = Command::new(provider.executable)
            .args(provider.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(root)
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("failed to open LSP stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("failed to open LSP stdout"))?;
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name(format!("redox-lsp-{}", provider.label))
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                while let Some(message) = read_lsp_message(&mut reader) {
                    if tx.send(SessionEvent::Message(message)).is_err() {
                        return;
                    }
                }
                let _ = tx.send(SessionEvent::Terminated);
            })
            .expect("failed to start LSP reader");

        let mut session = Self {
            child,
            stdin,
            events: rx,
            initialized: false,
            next_request_id: FIRST_DYNAMIC_REQUEST_ID,
        };
        session.send_initialize(root)?;
        Ok(session)
    }

    fn send_initialize(&mut self, root: &Path) -> io::Result<()> {
        let root_uri = file_uri(root)?;
        let root_path = root.to_string_lossy().to_string();
        let workspace_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace")
            .to_string();
        let message = json!({
            "jsonrpc": "2.0",
            "id": INITIALIZE_REQUEST_ID,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootPath": root_path,
                "rootUri": root_uri.clone(),
                "workspaceFolders": [
                    {
                        "uri": root_uri,
                        "name": workspace_name
                    }
                ],
                "capabilities": {
                    "workspace": {
                        "applyEdit": true
                    },
                    "textDocument": {
                        "publishDiagnostics": {
                            "relatedInformation": true,
                            "versionSupport": true
                        },
                        "hover": {
                            "contentFormat": ["markdown", "plaintext"]
                        },
                        "completion": {
                            "completionItem": {
                                "snippetSupport": true
                            }
                        },
                        "codeAction": {
                            "codeActionLiteralSupport": {
                                "codeActionKind": {
                                    "valueSet": [
                                        "quickfix",
                                        "refactor",
                                        "refactor.extract",
                                        "refactor.inline",
                                        "refactor.rewrite",
                                        "source",
                                        "source.organizeImports"
                                    ]
                                }
                            }
                        }
                    }
                },
                "clientInfo": {
                    "name": "redox",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });
        write_lsp_message(&mut self.stdin, &message)
    }

    fn send_initialized(&mut self) -> io::Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        });
        write_lsp_message(&mut self.stdin, &message)
    }

    fn send_response(&mut self, id: Value, result: Value) -> io::Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        });
        write_lsp_message(&mut self.stdin, &message)
    }

    fn send_method_not_found(&mut self, id: Value, method: &str) -> io::Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("unsupported request: {method}")
            }
        });
        write_lsp_message(&mut self.stdin, &message)
    }

    fn send_did_open(
        &mut self,
        path: &Path,
        language_id: &str,
        version: i32,
        text: &str,
    ) -> io::Result<()> {
        let uri = file_uri(path)?;
        let message = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": version,
                    "text": text,
                }
            }
        });
        write_lsp_message(&mut self.stdin, &message)
    }

    fn send_did_change(&mut self, path: &Path, version: i32, text: &str) -> io::Result<()> {
        let uri = file_uri(path)?;
        let message = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "version": version,
                },
                "contentChanges": [
                    {
                        "text": text,
                    }
                ]
            }
        });
        write_lsp_message(&mut self.stdin, &message)
    }

    fn send_did_save(&mut self, path: &Path) -> io::Result<()> {
        let uri = file_uri(path)?;
        let message = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": {
                "textDocument": {
                    "uri": uri
                }
            }
        });
        write_lsp_message(&mut self.stdin, &message)
    }

    fn send_cancel_request(&mut self, id: i64) -> io::Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "$/cancelRequest",
            "params": {
                "id": id
            }
        });
        write_lsp_message(&mut self.stdin, &message)
    }

    fn send_goto_definition(
        &mut self,
        path: &Path,
        line: usize,
        character: u32,
    ) -> io::Result<i64> {
        let uri = file_uri(path)?;
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let message = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "textDocument/definition",
            "params": {
                "textDocument": {
                    "uri": uri
                },
                "position": {
                    "line": line,
                    "character": character
                }
            }
        });
        write_lsp_message(&mut self.stdin, &message)?;
        Ok(request_id)
    }

    fn send_hover(&mut self, path: &Path, line: usize, character: u32) -> io::Result<i64> {
        let uri = file_uri(path)?;
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let message = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "textDocument/hover",
            "params": {
                "textDocument": {
                    "uri": uri
                },
                "position": {
                    "line": line,
                    "character": character
                }
            }
        });
        write_lsp_message(&mut self.stdin, &message)?;
        Ok(request_id)
    }

    fn send_completion(&mut self, path: &Path, line: usize, character: u32) -> io::Result<i64> {
        let uri = file_uri(path)?;
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let message = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "textDocument/completion",
            "params": {
                "textDocument": {
                    "uri": uri
                },
                "position": {
                    "line": line,
                    "character": character
                },
                "context": {
                    "triggerKind": 1
                }
            }
        });
        write_lsp_message(&mut self.stdin, &message)?;
        Ok(request_id)
    }

    fn send_code_actions(
        &mut self,
        path: &Path,
        range: &IncomingRange,
        diagnostics: &[Value],
    ) -> io::Result<i64> {
        let uri = file_uri(path)?;
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let message = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "textDocument/codeAction",
            "params": {
                "textDocument": {
                    "uri": uri
                },
                "range": range,
                "context": {
                    "diagnostics": diagnostics,
                    "only": ["quickfix"]
                }
            }
        });
        write_lsp_message(&mut self.stdin, &message)?;
        Ok(request_id)
    }

    fn send_execute_command(&mut self, command: &str, arguments: &[Value]) -> io::Result<i64> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let message = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "workspace/executeCommand",
            "params": {
                "command": command,
                "arguments": arguments,
            }
        });
        write_lsp_message(&mut self.stdin, &message)?;
        Ok(request_id)
    }

    fn try_recv(&self) -> Option<SessionEvent> {
        self.events.try_recv().ok()
    }
}

impl EditorState {
    pub fn completion_popup(&self) -> Option<CompletionPopup> {
        let state = self.visible_completion_state()?;
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
                    type_label: completion_type_label(item),
                    extra: completion_extra_label(item),
                    documentation: item.documentation.clone(),
                })
                .collect(),
            selected,
            scroll,
        })
    }

    pub(super) fn has_visible_completion_popup(&self) -> bool {
        self.visible_completion_state().is_some()
    }

    pub fn completion_preview(&self) -> Option<CompletionPreview> {
        let state = self.visible_completion_state()?;
        let item = state.items.get(state.selected)?;
        let buffer = self.session.buffer(self.session.active_id())?;
        let cursor = self.active_cursor_pos();
        let suffix = line_after_cursor_completion_preview_suffix(buffer, cursor)?;
        let edit = completion_edit_for_buffer(item, buffer, state.requested_at);
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

    pub fn active_diagnostic_summary(&self) -> DiagnosticSummary {
        let mut summary = DiagnosticSummary::default();
        for diagnostic in self.active_display_diagnostics() {
            match diagnostic.severity {
                DiagnosticSeverity::Error => summary.errors += 1,
                DiagnosticSeverity::Warning => summary.warnings += 1,
                DiagnosticSeverity::Information => summary.information += 1,
                DiagnosticSeverity::Hint => summary.hints += 1,
            }
        }
        summary
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
                    format!("⦿ {} (+{})", entry.inline_text, entry.message_count - 1);
            } else {
                entry.inline_text = format!("⦿ {}", entry.inline_text);
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
        if client.session.initialized {
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
            loop {
                let event = self
                    .lsp
                    .clients
                    .get(&workspace)
                    .and_then(|client| client.session.try_recv());
                let Some(event) = event else {
                    break;
                };

                match event {
                    SessionEvent::Message(message) => {
                        if is_initialize_response(&message) {
                            if let Some(client) = self.lsp.clients.get_mut(&workspace) {
                                client.session.initialized = true;
                                let _ = client.session.send_initialized();
                            }
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
                            continue;
                        }

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
                    SessionEvent::Terminated => {
                        terminated.push(workspace.clone());
                        break;
                    }
                }
            }
        }

        for workspace in terminated {
            self.lsp.clients.remove(&workspace);
            self.reset_documents_for_workspace(&workspace);
            self.remove_diagnostics_for_source_everywhere(&DiagnosticSource::Lsp(
                workspace.clone(),
            ));
            self.lsp
                .pending_requests
                .retain(|key, _| key.workspace != workspace);
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
        } else if self
            .lsp
            .installed
            .insert(
                item.id(),
                InstalledToolRecord {
                    install_source: None,
                },
            )
            .is_none()
            && let Err(error) = save_installed_tools(&self.lsp.installed)
        {
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
                && provider.matches_language(language)
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
                && provider.matches_language(language)
        }) else {
            return;
        };
        let Some(language_id) = provider.language_id_for(language) else {
            return;
        };

        let root = workspace_root_for(path, provider.id, self.session.launch_dir());
        let workspace = WorkspaceKey {
            provider_id: provider.id,
            root: root.clone(),
        };
        if !self.lsp.clients.contains_key(&workspace) {
            match LspSession::spawn(provider, &root) {
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

        self.lsp.documents.insert(
            active_id,
            ManagedDocument {
                workspace,
                path: path.to_path_buf(),
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
        if !client.session.initialized {
            return Ok(());
        }
        if !self
            .session
            .buffer_is_fully_loaded(buffer_id)
            .unwrap_or(true)
        {
            return Ok(());
        }
        let Some(buffer) = self.session.buffer(buffer_id) else {
            return Ok(());
        };
        let analysis_version = self
            .views
            .get(&buffer_id)
            .map(|view| view.analysis_version())
            .unwrap_or(0);
        let text = buffer.to_string();

        let Some(document) = self.lsp.documents.get_mut(&buffer_id) else {
            return Ok(());
        };
        if !document.opened {
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

        if document.last_sent_analysis_version == Some(analysis_version)
            && document.last_sent_text.as_deref() == Some(text.as_str())
        {
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
        self.lsp
            .documents
            .retain(|buffer_id, _| valid_ids.contains(buffer_id));
        let live_workspaces = self
            .lsp
            .documents
            .values()
            .map(|document| document.workspace.clone())
            .collect::<HashSet<_>>();
        let live_lint_sources = self
            .lsp
            .documents
            .values()
            .filter_map(|document| self.lint_source_for_document(document))
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
        let live_lint_runs = self
            .lsp
            .documents
            .values()
            .filter_map(|document| {
                self.lint_source_for_document(document)
                    .map(|source| (document.uri.clone(), document.document_version, source))
            })
            .collect::<HashSet<_>>();
        self.lsp.lint_runs.retain(|run| {
            live_lint_runs.contains(&(run.uri.clone(), run.document_version, run.source.clone()))
        });
        self.lsp.queued_lint_runs.retain(|run| {
            live_lint_runs.contains(&(run.uri.clone(), run.document_version, run.source.clone()))
        });
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

    fn active_stored_diagnostics(&self) -> Vec<(&DiagnosticSource, &StoredDiagnostic)> {
        let Some(uri) = self.active_document_uri() else {
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
        let active_id = self.session.active_id();
        let Some(buffer) = self.session.buffer(active_id) else {
            return Vec::new();
        };
        let stored = self.active_stored_diagnostics();
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

    fn active_document_uri(&self) -> Option<String> {
        let active_id = self.session.active_id();
        self.lsp
            .documents
            .get(&active_id)
            .map(|document| document.uri.clone())
            .or_else(|| {
                let path = self.session.meta(active_id)?.path.as_deref()?;
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
                PendingRequest::CodeActions { trigger, .. } => {
                    if let Some(state) = self.lsp.diagnostics_popup.as_mut() {
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
        if !client.session.initialized {
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
        self.close_completion();
        self.clear_symbol_info();
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
        let return_mode = self.mode;
        self.cancel_pending_lsp_requests(
            &document.workspace,
            PendingRequest::SymbolInfo {
                requested_at: cursor,
                return_mode,
            },
        );
        let Some(client) = self.lsp.clients.get_mut(&document.workspace) else {
            self.set_status("no LSP client for current buffer");
            return;
        };
        if !client.session.initialized {
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
        let before = self.capture_active_insert_coalesced_snapshot();
        {
            let buffer = self.session.active_buffer_mut();
            apply_snippet_edits(buffer, &edits);
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
            apply_snippet_edits(buffer, &edits);
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
            apply_snippet_edits(buffer, &edits);
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
        if !client.session.initialized {
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
        self.lsp.auto_completion = None;
        let had_completion = self.lsp.completion.take().is_some();
        had_completion
    }

    pub(super) fn close_symbol_info(&mut self) -> bool {
        let Some(symbol_info) = self.lsp.symbol_info.take() else {
            return false;
        };
        if self.mode == EditorMode::SymbolInfo {
            self.mode = symbol_info.return_mode;
        }
        true
    }

    pub(super) fn clear_symbol_info(&mut self) -> bool {
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
        let Some(state) = self.lsp.completion.take() else {
            return false;
        };
        let Some(item) = state.items.get(state.selected).cloned() else {
            return false;
        };
        self.remember_completion_accept(&item);
        let active_id = self.session.active_id();
        let Some(buffer) = self.session.buffer(active_id) else {
            return false;
        };
        let edit = completion_edit_for_buffer(&item, buffer, state.requested_at);
        let snippet_expansion = completion_snippet_expansion(&item, &edit.insert);
        let preserve_active_snippet_edit = snippet_expansion
            .is_none()
            .then(|| self.active_snippet_completion_edit(buffer, active_id, &edit))
            .flatten();
        let insert = snippet_expansion
            .as_ref()
            .map(|expansion| expansion.text.clone())
            .unwrap_or_else(|| edit.insert.clone());

        let before = self.capture_active_insert_coalesced_snapshot();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let start = buffer.clamp_pos(edit.start);
            let end = buffer.clamp_pos(edit.end);
            let _ = buffer.delete_range(start, end);
            let inserted_end = buffer.insert(start, &insert);
            let start_char = buffer.pos_to_char(start);
            if let Some(expansion) = snippet_expansion {
                self.lsp.active_snippet =
                    active_snippet_from_expansion(active_id, start_char, &expansion);
                view.cursor.cursor = self
                    .lsp
                    .active_snippet
                    .as_ref()
                    .and_then(|snippet| snippet.placeholders.first())
                    .map(|placeholder| buffer.char_to_pos(placeholder.start_char))
                    .or_else(|| {
                        expansion
                            .cursor_offset
                            .map(|offset| buffer.char_to_pos(start_char.saturating_add(offset)))
                    })
                    .unwrap_or(inserted_end);
            } else {
                if preserve_active_snippet_edit.is_none() {
                    self.lsp.active_snippet = None;
                }
                view.cursor.cursor = inserted_end;
            }
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
        if let Some((tabstop, start_char, end_char)) = preserve_active_snippet_edit {
            self.update_active_snippet_after_edits(&[(start_char, end_char, insert)]);
            self.mark_active_snippet_tabstop_filled(tabstop);
            if let Some(snippet) = self.lsp.active_snippet.as_mut() {
                snippet.selected = false;
            }
        }
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
        let lint_context = self.saved_buffer_lint_context(buffer_id);
        let Some(document) = self.lsp.documents.get(&buffer_id).cloned() else {
            if let Some((source, path, uri, document_version)) = lint_context {
                self.start_lint_run(source, path, uri, document_version);
            }
            return Ok(());
        };
        let Some(client) = self.lsp.clients.get_mut(&document.workspace) else {
            if let Some((source, path, uri, document_version)) = lint_context {
                self.start_lint_run(source, path, uri, document_version);
            }
            return Ok(());
        };
        if !client.session.initialized {
            if let Some((source, path, uri, document_version)) = lint_context {
                self.start_lint_run(source, path, uri, document_version);
            }
            return Ok(());
        }
        let Some(text) = self
            .session
            .buffer(buffer_id)
            .map(|buffer| buffer.to_string())
        else {
            return Ok(());
        };
        let Some(document) = self.lsp.documents.get_mut(&buffer_id) else {
            return Ok(());
        };
        if document.last_sent_text.as_deref() != Some(text.as_str()) {
            document.document_version = document.document_version.saturating_add(1);
            client
                .session
                .send_did_change(&document.path, document.document_version, &text)?;
            document.last_sent_analysis_version = self
                .views
                .get(&buffer_id)
                .map(|view| view.analysis_version());
            document.last_sent_text = Some(text);
            document.pending_sync_since = None;
            document.pending_sync_analysis_version = None;
        }
        client.session.send_did_save(&document.path)?;
        let lint_document = document.clone();
        self.start_lint_run_for_document(&lint_document);
        Ok(())
    }

    fn poll_lint_runs(&mut self) {
        let mut pending = Vec::with_capacity(self.lsp.lint_runs.len());
        let mut completed = Vec::new();

        for run in self.lsp.lint_runs.drain(..) {
            match run.receiver.try_recv() {
                Ok(result) => completed.push((run, result)),
                Err(TryRecvError::Empty) => pending.push(run),
                Err(TryRecvError::Disconnected) => {}
            }
        }

        self.lsp.lint_runs = pending;

        for (run, result) in completed {
            if self.diagnostics_are_stale(&run.uri, Some(run.document_version)) {
                self.start_queued_lint_run(&run.source, &run.uri);
                continue;
            }

            let source = DiagnosticSource::Lint(result.source.clone());
            self.remove_diagnostics_for_source_everywhere(&source);
            for (uri, diagnostics) in result.diagnostics_by_uri {
                self.replace_diagnostics_for_source(uri, source.clone(), diagnostics);
            }

            if let Some(error) = result.error {
                self.set_status(error);
            }
            self.start_queued_lint_run(&run.source, &run.uri);
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

        let root = workspace_root_for(path, provider_id, launch_dir);
        Some(LintSource { kind, root })
    }

    fn lint_source_for_document(&self, document: &ManagedDocument) -> Option<LintSource> {
        let language = language_for_path(Some(&document.path))?;
        let source = self.lint_source_for_path(&document.path, language)?;
        self.lsp
            .installed
            .contains_key(&MarketplaceItemId::Linter(source.kind))
            .then_some(source)
    }

    fn saved_buffer_lint_context(
        &self,
        buffer_id: BufferId,
    ) -> Option<(LintSource, PathBuf, String, i32)> {
        if let Some(document) = self.lsp.documents.get(&buffer_id) {
            return Some((
                self.lint_source_for_document(document)?,
                document.path.clone(),
                document.uri.clone(),
                document.document_version,
            ));
        }

        let meta = self.session.meta(buffer_id)?;
        let path = meta.path.as_deref()?;
        let language = language_for_path(Some(path))?;
        let source = self.lint_source_for_path(path, language)?;
        if !self
            .lsp
            .installed
            .contains_key(&MarketplaceItemId::Linter(source.kind))
        {
            return None;
        }
        Some((source, path.to_path_buf(), file_uri(path).ok()?, 0))
    }

    fn start_lint_run_for_document(&mut self, document: &ManagedDocument) {
        let Some(source) = self.lint_source_for_document(document) else {
            return;
        };
        self.start_lint_run(
            source,
            document.path.clone(),
            document.uri.clone(),
            document.document_version,
        );
    }

    fn start_lint_run(
        &mut self,
        source: LintSource,
        path: PathBuf,
        uri: String,
        document_version: i32,
    ) {
        if !lint_runner_available(&source, &path) {
            return;
        }
        if self
            .lsp
            .lint_runs
            .iter()
            .any(|run| run.source == source && run.uri == uri)
        {
            self.queue_lint_run(source, path, uri, document_version);
            return;
        }

        self.spawn_lint_run(source, path, uri, document_version);
    }

    fn queue_lint_run(
        &mut self,
        source: LintSource,
        path: PathBuf,
        uri: String,
        document_version: i32,
    ) {
        if let Some(queued) = self
            .lsp
            .queued_lint_runs
            .iter_mut()
            .find(|queued| queued.source == source && queued.uri == uri)
        {
            queued.path = path;
            queued.document_version = document_version;
            return;
        }
        self.lsp.queued_lint_runs.push(QueuedLintRun {
            source,
            path,
            uri,
            document_version,
        });
    }

    fn start_queued_lint_run(&mut self, source: &LintSource, uri: &str) {
        let Some(index) = self
            .lsp
            .queued_lint_runs
            .iter()
            .position(|queued| &queued.source == source && queued.uri == uri)
        else {
            return;
        };
        let queued = self.lsp.queued_lint_runs.swap_remove(index);
        if self.diagnostics_are_stale(&queued.uri, Some(queued.document_version)) {
            return;
        }
        self.start_lint_run(
            queued.source,
            queued.path,
            queued.uri,
            queued.document_version,
        );
    }

    fn spawn_lint_run(
        &mut self,
        source: LintSource,
        path: PathBuf,
        uri: String,
        document_version: i32,
    ) {
        let source_for_thread = source.clone();
        let path_for_thread = path.clone();
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name(format!("redox-lint-{}", source.kind.executable()))
            .spawn(move || {
                let result = run_lint_source(&source_for_thread, &path_for_thread);
                let _ = tx.send(result);
            })
            .expect("failed to start lint runner");
        self.lsp.lint_runs.push(PendingLintRun {
            source,
            uri,
            document_version,
            receiver: rx,
        });
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
        if self.active_cursor_pos() != requested_at
            || (!manual && self.cursor_is_in_comment_for_completion())
        {
            self.clear_status();
            return true;
        }
        /*
        if items.is_empty() {
            self.lsp.completion = None;
            self.set_status("no completions");
            return true;
        }
        */
        self.lsp.completion = Some(CompletionState {
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
        if state.items.is_empty() || !self.completion_matches_active_cursor(state.requested_at) {
            return None;
        }
        Some(state)
    }

    fn completion_matches_active_cursor(&self, requested_at: Pos) -> bool {
        let cursor = self.active_cursor_pos();
        if cursor == requested_at {
            return true;
        }
        let Some(buffer) = self.session.buffer(self.session.active_id()) else {
            return false;
        };
        if cursor.line != requested_at.line || cursor.col < requested_at.col {
            return false;
        }
        completion_prefix_start(buffer, cursor) == completion_prefix_start(buffer, requested_at)
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
        let uninstall = record
            .install_source
            .and_then(|method| {
                item.install_plans()
                    .iter()
                    .copied()
                    .find(|plan| plan.method == method)
                    .map(|plan| plan.uninstall)
            })
            .unwrap_or(ProviderUninstall::DisableOnly);

        match uninstall {
            ProviderUninstall::DisableOnly => false,
            uninstall => {
                let install_source = record.install_source;
                let (tx, rx) = mpsc::channel();
                thread::Builder::new()
                    .name(format!("redox-uninstall-{}", item.label()))
                    .spawn(move || {
                        let result = run_provider_uninstall(item, uninstall, install_source);
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
        }
    }

    fn respond_to_lsp_server_request(&mut self, workspace: &WorkspaceKey, message: &Value) -> bool {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return false;
        };
        let Some(id) = message.get("id").cloned() else {
            return false;
        };

        let result = match method {
            "client/registerCapability" | "client/unregisterCapability" => Some(Value::Null),
            "workspace/configuration" => Some(configuration_response(message)),
            "workspace/workspaceFolders" => Some(workspace_folders_response(workspace)),
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

fn parse_definition_response(message: &Value) -> Option<DefinitionTarget> {
    let result = message.get("result")?;
    if result.is_null() {
        return None;
    }

    if result.is_array() {
        let entries = result.as_array()?;
        let first = entries.first()?;
        return parse_definition_target_value(first);
    }

    parse_definition_target_value(result)
}

fn parse_code_action_response(message: &Value) -> Vec<AvailableCodeAction> {
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

    let title = value.get("title")?.as_str()?.to_string();
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let preferred = value
        .get("isPreferred")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let edit = value.get("edit").and_then(parse_workspace_edit);
    let command = value.get("command").and_then(parse_lsp_command);
    Some(AvailableCodeAction {
        title,
        kind,
        preferred,
        edit,
        command,
    })
}

fn parse_workspace_edit(value: &Value) -> Option<WorkspaceEdit> {
    let mut document_edits = Vec::new();

    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            let edits = edits
                .as_array()?
                .iter()
                .filter_map(parse_text_edit)
                .collect::<Vec<_>>();
            if !edits.is_empty() {
                document_edits.push(DocumentEdit {
                    uri: uri.clone(),
                    edits,
                });
            }
        }
    }

    if let Some(changes) = value.get("documentChanges").and_then(Value::as_array) {
        for entry in changes {
            let Some(text_document) = entry.get("textDocument") else {
                continue;
            };
            let uri = text_document.get("uri")?.as_str()?.to_string();
            let edits = entry
                .get("edits")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(parse_text_edit)
                .collect::<Vec<_>>();
            if !edits.is_empty() {
                document_edits.push(DocumentEdit { uri, edits });
            }
        }
    }

    (!document_edits.is_empty()).then_some(WorkspaceEdit { document_edits })
}

fn parse_text_edit(value: &Value) -> Option<TextEdit> {
    let range = serde_json::from_value::<IncomingRange>(value.get("range")?.clone()).ok()?;
    let new_text = value.get("newText")?.as_str()?.to_string();
    Some(TextEdit { range, new_text })
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

fn parse_hover_response(message: &Value) -> Vec<SymbolInfoBlock> {
    let Some(result) = message.get("result") else {
        return Vec::new();
    };
    if result.is_null() {
        return Vec::new();
    }
    let Some(contents) = result.get("contents") else {
        return Vec::new();
    };
    normalise_hover_blocks(hover_contents_blocks(contents))
}

fn hover_contents_blocks(value: &Value) -> Vec<SymbolInfoBlock> {
    if let Some(text) = value.as_str() {
        return vec![SymbolInfoBlock {
            kind: SymbolInfoKind::Markdown,
            text: text.to_string(),
        }];
    }
    if let Some(array) = value.as_array() {
        return array.iter().flat_map(hover_contents_blocks).collect();
    }
    if let Some(kind) = value.get("kind").and_then(Value::as_str) {
        let text = value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let kind = match kind {
            "markdown" => SymbolInfoKind::Markdown,
            _ => SymbolInfoKind::PlainText,
        };
        return vec![SymbolInfoBlock { kind, text }];
    }
    if value.get("language").is_some() || value.get("value").is_some() {
        return vec![hover_marked_string_block(value)];
    }
    Vec::new()
}

fn hover_marked_string_block(value: &Value) -> SymbolInfoBlock {
    let language = value
        .get("language")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    if let Some(text) = value.as_str() {
        return SymbolInfoBlock {
            kind: SymbolInfoKind::Markdown,
            text: text.to_string(),
        };
    }
    let text = value
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    SymbolInfoBlock {
        kind: SymbolInfoKind::Code { language },
        text,
    }
}

fn normalise_hover_blocks(blocks: Vec<SymbolInfoBlock>) -> Vec<SymbolInfoBlock> {
    blocks
        .into_iter()
        .filter_map(|block| {
            let SymbolInfoBlock { kind, text } = block;
            let text = match &kind {
                SymbolInfoKind::Code { .. } => {
                    trim_blank_edges(&trim_trailing_whitespace_lines(&text))
                }
                SymbolInfoKind::Markdown | SymbolInfoKind::PlainText => {
                    collapse_blank_lines(&trim_trailing_whitespace_lines(&text))
                }
            };
            (!text.is_empty()).then_some(SymbolInfoBlock { kind, text })
        })
        .collect()
}

fn trim_trailing_whitespace_lines(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

fn trim_blank_edges(text: &str) -> String {
    text.lines()
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

fn collapse_blank_lines(text: &str) -> String {
    let mut lines = Vec::new();
    let mut last_was_blank = true;

    for line in text.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !last_was_blank {
                lines.push(String::new());
            }
        } else {
            lines.push(line.to_string());
        }
        last_was_blank = is_blank;
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

fn parse_definition_target_value(value: &Value) -> Option<DefinitionTarget> {
    if let Ok(location) = serde_json::from_value::<IncomingLocation>(value.clone()) {
        return Some(DefinitionTarget {
            uri: location.uri,
            range: location.range,
        });
    }
    let link = serde_json::from_value::<IncomingLocationLink>(value.clone()).ok()?;
    Some(DefinitionTarget {
        uri: link.target_uri,
        range: link.target_selection_range,
    })
}

fn lint_runner_available(source: &LintSource, path: &Path) -> bool {
    match source.kind {
        LintRunnerKind::Clippy => clippy_available() && source.root.join("Cargo.toml").exists(),
        LintRunnerKind::GolangciLint => {
            executable_on_path(source.kind.executable()) && path.starts_with(&source.root)
        }
        LintRunnerKind::Ruff => executable_on_path(source.kind.executable()),
    }
}

fn run_lint_source(source: &LintSource, path: &Path) -> LintRunResult {
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
                    lint_runner_label(source.kind),
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
            error: Some(format!(
                "failed to start {}: {error}",
                lint_runner_label(source.kind)
            )),
        },
    }
}

fn parse_clippy_output(stdout: &[u8], root: &Path) -> HashMap<String, Vec<StoredDiagnostic>> {
    let mut diagnostics_by_uri = HashMap::<String, Vec<StoredDiagnostic>>::new();
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
        let Some(uri) = file_uri(&file_path).ok() else {
            continue;
        };
        let severity = diagnostic_severity_from_text(&message.level);
        let Some(diagnostic) = stored_diagnostic_from_char_span(
            &file_path,
            severity,
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

fn parse_golangci_lint_output(
    stdout: &[u8],
    root: &Path,
) -> HashMap<String, Vec<StoredDiagnostic>> {
    let Ok(report) = serde_json::from_slice::<GolangciLintReport>(stdout) else {
        return HashMap::new();
    };
    let mut diagnostics_by_uri = HashMap::<String, Vec<StoredDiagnostic>>::new();
    let mut line_cache = HashMap::<PathBuf, Option<Vec<String>>>::new();

    for issue in report.issues {
        let file_path = resolve_lint_path(root, Path::new(&issue.pos.filename));
        let Some(uri) = file_uri(&file_path).ok() else {
            continue;
        };
        let severity = issue
            .severity
            .as_deref()
            .map(diagnostic_severity_from_text)
            .unwrap_or(DiagnosticSeverity::Warning);
        let message = if issue.from_linter.trim().is_empty() {
            issue.text
        } else {
            format!("{}: {}", issue.from_linter, issue.text)
        };
        let Some(diagnostic) = stored_diagnostic_from_char_span(
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

fn parse_golangci_lint_text_output(
    stderr: &[u8],
    root: &Path,
) -> HashMap<String, Vec<StoredDiagnostic>> {
    let mut diagnostics_by_uri = HashMap::<String, Vec<StoredDiagnostic>>::new();
    let mut line_cache = HashMap::<PathBuf, Option<Vec<String>>>::new();

    for line in String::from_utf8_lossy(stderr).lines() {
        let Some((path_part, line_no, col_no, message)) = parse_colon_diagnostic_line(line) else {
            continue;
        };
        let file_path = resolve_lint_path(root, Path::new(path_part));
        let Some(uri) = file_uri(&file_path).ok() else {
            continue;
        };
        let Some(diagnostic) = stored_diagnostic_from_char_span(
            &file_path,
            DiagnosticSeverity::Warning,
            message.to_string(),
            line_no,
            line_no,
            col_no,
            col_no.saturating_add(1),
            &mut line_cache,
        ) else {
            continue;
        };
        diagnostics_by_uri.entry(uri).or_default().push(diagnostic);
    }

    diagnostics_by_uri
}

fn parse_colon_diagnostic_line(line: &str) -> Option<(&str, usize, usize, &str)> {
    let (path_part, rest) = line.split_once(':')?;
    let (line_part, rest) = rest.split_once(':')?;
    let (col_part, message) = rest.split_once(':')?;
    let line_no = line_part.trim().parse::<usize>().ok()?;
    let col_no = col_part.trim().parse::<usize>().ok()?;
    let message = message.trim();
    if path_part.trim().is_empty() || message.is_empty() {
        return None;
    }
    Some((path_part.trim(), line_no, col_no, message))
}

fn parse_ruff_output(stdout: &[u8], root: &Path) -> HashMap<String, Vec<StoredDiagnostic>> {
    let Ok(diagnostics) = serde_json::from_slice::<Vec<RuffDiagnostic>>(stdout) else {
        return HashMap::new();
    };
    let mut diagnostics_by_uri = HashMap::<String, Vec<StoredDiagnostic>>::new();
    let mut line_cache = HashMap::<PathBuf, Option<Vec<String>>>::new();

    for diagnostic in diagnostics {
        let file_path = resolve_lint_path(root, Path::new(&diagnostic.filename));
        let Some(uri) = file_uri(&file_path).ok() else {
            continue;
        };
        let message = diagnostic
            .code
            .as_deref()
            .map(|code| format!("{code}: {}", diagnostic.message))
            .unwrap_or(diagnostic.message);
        let Some(stored) = stored_diagnostic_from_char_span(
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
        diagnostics_by_uri.entry(uri).or_default().push(stored);
    }

    diagnostics_by_uri
}

fn is_initialize_response(message: &Value) -> bool {
    message
        .get("id")
        .and_then(Value::as_i64)
        .is_some_and(|id| id == INITIALIZE_REQUEST_ID)
}

fn utf16_code_unit_to_char_col(line: &str, utf16_col: u32) -> usize {
    let mut consumed_utf16 = 0u32;
    let mut chars = 0usize;
    for ch in line.chars() {
        if consumed_utf16 >= utf16_col {
            break;
        }
        consumed_utf16 = consumed_utf16.saturating_add(ch.len_utf16() as u32);
        chars += 1;
    }
    chars
}

fn char_col_to_utf16(line: &str, char_col: usize) -> u32 {
    line.chars()
        .take(char_col)
        .fold(0u32, |acc, ch| acc.saturating_add(ch.len_utf16() as u32))
}

fn compare_edit_ranges_desc(left: &IncomingRange, right: &IncomingRange) -> std::cmp::Ordering {
    (
        right.start.line,
        right.start.character,
        right.end.line,
        right.end.character,
    )
        .cmp(&(
            left.start.line,
            left.start.character,
            left.end.line,
            left.end.character,
        ))
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
    let start_col = utf16_code_unit_to_char_col(
        &buffer.line_string(start_line),
        range.start.character as u32,
    );
    let end_col =
        utf16_code_unit_to_char_col(&buffer.line_string(end_line), range.end.character as u32);
    Some((
        buffer.clamp_pos(Pos::new(start_line, start_col)),
        buffer.clamp_pos(Pos::new(end_line, end_col)),
    ))
}

fn write_lsp_message(stdin: &mut ChildStdin, message: &Value) -> io::Result<()> {
    let json = message.to_string();
    write!(stdin, "Content-Length: {}\r\n\r\n{json}", json.len())?;
    stdin.flush()
}

fn read_lsp_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut content_length = None;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("Content-Length") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }

    let content_length = content_length?;
    let mut payload = vec![0; content_length];
    reader.read_exact(&mut payload).ok()?;
    serde_json::from_slice(&payload).ok()
}

fn file_uri(path: &Path) -> io::Result<String> {
    Url::from_file_path(path)
        .map(Into::into)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path is not a valid file URI"))
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

fn file_path_from_uri(uri: &str) -> Option<PathBuf> {
    Url::parse(uri).ok()?.to_file_path().ok()
}

fn workspace_root_for(path: &Path, provider_id: ProviderId, launch_dir: &Path) -> PathBuf {
    let Some(start_dir) = path.parent() else {
        return launch_dir.to_path_buf();
    };

    match provider_id {
        ProviderId::RustAnalyzer => {
            find_outermost_ancestor_with_any_marker(start_dir, &["Cargo.toml", "rust-project.json"])
                .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"]))
        }
        ProviderId::Gopls => find_outermost_ancestor_with_any_marker(start_dir, &["go.work"])
            .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &["go.mod"]))
            .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"])),
        ProviderId::TypeScriptLanguageServer => find_nearest_ancestor_with_any_marker(
            start_dir,
            &[
                "tsconfig.json",
                "jsconfig.json",
                "package.json",
                "deno.json",
                "deno.jsonc",
            ],
        )
        .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"])),
        ProviderId::Pyright => find_nearest_ancestor_with_any_marker(
            start_dir,
            &[
                "pyproject.toml",
                "setup.py",
                "setup.cfg",
                "requirements.txt",
            ],
        )
        .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"])),
        ProviderId::Clangd => find_nearest_ancestor_with_any_marker(
            start_dir,
            &["compile_commands.json", "compile_flags.txt", ".clangd"],
        )
        .or_else(|| find_nearest_ancestor_with_any_marker(start_dir, &[".git"])),
        ProviderId::LuaLanguageServer => find_nearest_ancestor_with_any_marker(
            start_dir,
            &[".luarc.json", ".luarc.jsonc", "stylua.toml", ".git"],
        ),
        ProviderId::Taplo => find_nearest_ancestor_with_any_marker(
            start_dir,
            &["taplo.toml", ".taplo.toml", "Cargo.toml", ".git"],
        ),
        ProviderId::Marksman => find_nearest_ancestor_with_any_marker(
            start_dir,
            &[".marksman.toml", "package.json", ".git"],
        ),
        ProviderId::YamlLanguageServer
        | ProviderId::JsonLanguageServer
        | ProviderId::HtmlLanguageServer
        | ProviderId::CssLanguageServer => {
            find_nearest_ancestor_with_any_marker(start_dir, &["package.json", ".git"])
        }
    }
    .unwrap_or_else(|| start_dir.to_path_buf())
}

fn find_nearest_ancestor_with_any_marker(start_dir: &Path, markers: &[&str]) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .find(|dir| markers.iter().any(|marker| dir.join(marker).exists()))
        .map(Path::to_path_buf)
}

fn find_outermost_ancestor_with_any_marker(start_dir: &Path, markers: &[&str]) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .filter(|dir| markers.iter().any(|marker| dir.join(marker).exists()))
        .last()
        .map(Path::to_path_buf)
}

fn provider_spec(provider_id: ProviderId) -> Option<ProviderSpec> {
    PROVIDERS
        .iter()
        .copied()
        .find(|provider| provider.id == provider_id)
}

fn linter_spec(kind: LintRunnerKind) -> Option<LinterSpec> {
    LINTERS.iter().copied().find(|linter| linter.kind == kind)
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
    match item {
        MarketplaceSpec::Linter(linter) if linter.kind == LintRunnerKind::Clippy => {
            clippy_available()
        }
        _ => executable_on_path(item.executable()),
    }
}

fn install_method_label(method: InstallMethodId) -> &'static str {
    match method {
        InstallMethodId::Brew => "brew",
        InstallMethodId::Cargo => "cargo",
        InstallMethodId::Go => "go",
        InstallMethodId::Npm => "npm",
        InstallMethodId::Rustup => "rustup",
    }
}

fn install_method_command(method: InstallMethodId) -> &'static str {
    install_method_label(method)
}

fn install_method_available(method: InstallMethodId) -> bool {
    executable_on_path(install_method_command(method))
}

fn run_provider_install(
    item: MarketplaceSpec,
    plan: ProviderInstallPlan,
) -> ProviderOperationResult {
    let output = Command::new(install_method_command(plan.method))
        .args(plan.install_args)
        .output();
    match output {
        Ok(output) if output.status.success() => ProviderOperationResult {
            item_id: item.id(),
            kind: ProviderOperationKind::Installing,
            install_source: Some(plan.method),
            success: marketplace_tool_available(item),
            message: if marketplace_tool_available(item) {
                format!("installed {}", item.label())
            } else {
                format!(
                    "{} finished, but {} is still not on PATH",
                    install_method_label(plan.method),
                    item.executable()
                )
            },
        },
        Ok(output) => ProviderOperationResult {
            item_id: item.id(),
            kind: ProviderOperationKind::Installing,
            install_source: Some(plan.method),
            success: false,
            message: format!(
                "failed to install {} via {}: {}",
                item.label(),
                install_method_label(plan.method),
                first_stderr_line(&output.stderr)
            ),
        },
        Err(error) => ProviderOperationResult {
            item_id: item.id(),
            kind: ProviderOperationKind::Installing,
            install_source: Some(plan.method),
            success: false,
            message: format!("failed to start {} installer: {error}", item.label()),
        },
    }
}

fn run_provider_uninstall(
    item: MarketplaceSpec,
    uninstall: ProviderUninstall,
    install_source: Option<InstallMethodId>,
) -> ProviderOperationResult {
    match uninstall {
        ProviderUninstall::Command(args) => {
            let method = install_source.expect("command uninstall should have install source");
            let output = Command::new(install_method_command(method))
                .args(args)
                .output();
            match output {
                Ok(output) if output.status.success() => ProviderOperationResult {
                    item_id: item.id(),
                    kind: ProviderOperationKind::Uninstalling,
                    install_source,
                    success: true,
                    message: format!("removed {}", item.label()),
                },
                Ok(output) => ProviderOperationResult {
                    item_id: item.id(),
                    kind: ProviderOperationKind::Uninstalling,
                    install_source,
                    success: false,
                    message: format!(
                        "failed to uninstall {} via {}: {}",
                        item.label(),
                        install_method_label(method),
                        first_stderr_line(&output.stderr)
                    ),
                },
                Err(error) => ProviderOperationResult {
                    item_id: item.id(),
                    kind: ProviderOperationKind::Uninstalling,
                    install_source,
                    success: false,
                    message: format!("failed to start {} uninstall: {error}", item.label()),
                },
            }
        }
        ProviderUninstall::GoBinary(binary) => {
            let result = remove_go_binary(binary);
            ProviderOperationResult {
                item_id: item.id(),
                kind: ProviderOperationKind::Uninstalling,
                install_source,
                success: result.is_ok(),
                message: result
                    .map(|_| format!("removed {}", item.label()))
                    .unwrap_or_else(|error| format!("failed to remove {}: {error}", item.label())),
            }
        }
        ProviderUninstall::DisableOnly => ProviderOperationResult {
            item_id: item.id(),
            kind: ProviderOperationKind::Uninstalling,
            install_source,
            success: true,
            message: format!("removed {} from Redox", item.label()),
        },
    }
}

fn remove_go_binary(binary: &str) -> io::Result<()> {
    let gobin_output = Command::new("go").args(["env", "GOBIN"]).output()?;
    if !gobin_output.status.success() {
        return Err(io::Error::other(first_stderr_line(&gobin_output.stderr)));
    }
    let gobin = String::from_utf8_lossy(&gobin_output.stdout)
        .trim()
        .to_string();
    let target = if !gobin.is_empty() {
        PathBuf::from(gobin).join(binary)
    } else {
        let gopath_output = Command::new("go").args(["env", "GOPATH"]).output()?;
        if !gopath_output.status.success() {
            return Err(io::Error::other(first_stderr_line(&gopath_output.stderr)));
        }
        let gopath = String::from_utf8_lossy(&gopath_output.stdout)
            .trim()
            .to_string();
        PathBuf::from(gopath).join("bin").join(binary)
    };
    fs::remove_file(target)
}

fn first_stderr_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("unknown error")
        .to_string()
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

fn lint_runner_label(kind: LintRunnerKind) -> &'static str {
    match kind {
        LintRunnerKind::Clippy => "Clippy",
        LintRunnerKind::GolangciLint => "golangci-lint",
        LintRunnerKind::Ruff => "Ruff",
    }
}

fn diagnostic_severity_from_text(level: &str) -> DiagnosticSeverity {
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

fn stored_diagnostic_from_char_span(
    path: &Path,
    severity: DiagnosticSeverity,
    message: String,
    start_line_1: usize,
    end_line_1: usize,
    start_col_1: usize,
    end_col_1: usize,
    line_cache: &mut HashMap<PathBuf, Option<Vec<String>>>,
) -> Option<StoredDiagnostic> {
    let start_line = start_line_1.checked_sub(1)?;
    let end_line = end_line_1.checked_sub(1)?;
    let start_col = start_col_1.saturating_sub(1);
    let mut end_col = end_col_1.saturating_sub(1);
    if start_line == end_line {
        end_col = end_col.max(start_col.saturating_add(1));
    }

    let start_utf16 = char_col_to_utf16_in_file(path, start_line, start_col, line_cache)?;
    let mut end_utf16 = char_col_to_utf16_in_file(path, end_line, end_col, line_cache)?;
    if start_line == end_line {
        end_utf16 = end_utf16.max(start_utf16.saturating_add(1));
    }

    Some(StoredDiagnostic {
        severity,
        message,
        start_line,
        end_line,
        start_utf16,
        end_utf16,
    })
}

fn char_col_to_utf16_in_file(
    path: &Path,
    line_idx: usize,
    char_col: usize,
    line_cache: &mut HashMap<PathBuf, Option<Vec<String>>>,
) -> Option<u32> {
    let lines = cached_file_lines(path, line_cache)?;
    let line = lines.get(line_idx)?;
    let clamped_char_col = char_col.min(line.chars().count());
    Some(char_col_to_utf16(line, clamped_char_col))
}

fn cached_file_lines<'a>(
    path: &Path,
    line_cache: &'a mut HashMap<PathBuf, Option<Vec<String>>>,
) -> Option<&'a [String]> {
    let entry = line_cache.entry(path.to_path_buf()).or_insert_with(|| {
        fs::read_to_string(path)
            .ok()
            .map(|text| text.split('\n').map(|line| line.to_string()).collect())
    });
    entry.as_deref()
}

fn load_installed_tools() -> HashMap<MarketplaceItemId, InstalledToolRecord> {
    let path = installed_lsps_storage_path();
    let Ok(contents) = fs::read_to_string(path) else {
        return HashMap::new();
    };

    if let Ok(entries) = serde_json::from_str::<Vec<String>>(&contents) {
        return entries
            .into_iter()
            .filter_map(|entry| {
                ProviderId::from_str(&entry).map(|id| {
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
                "lsp" => {
                    MarketplaceItemId::Provider(ProviderId::from_str(entry.get("id")?.as_str()?)?)
                }
                "linter" => {
                    MarketplaceItemId::Linter(parse_lint_runner_kind(entry.get("id")?.as_str()?)?)
                }
                _ => return None,
            };
            let install_source = entry
                .get("install_source")
                .and_then(Value::as_str)
                .and_then(parse_install_method_id);
            Some((id, InstalledToolRecord { install_source }))
        })
        .collect()
}

fn save_installed_tools(
    installed: &HashMap<MarketplaceItemId, InstalledToolRecord>,
) -> io::Result<()> {
    let path = installed_lsps_storage_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut entries = installed
        .iter()
        .map(|(item_id, record)| {
            json!({
                "kind": item_id.persistent_kind(),
                "id": item_id.id_str(),
                "install_source": record.install_source.map(install_method_id_str),
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

fn install_method_id_str(method: InstallMethodId) -> &'static str {
    match method {
        InstallMethodId::Brew => "brew",
        InstallMethodId::Cargo => "cargo",
        InstallMethodId::Go => "go",
        InstallMethodId::Npm => "npm",
        InstallMethodId::Rustup => "rustup",
    }
}

fn parse_install_method_id(value: &str) -> Option<InstallMethodId> {
    match value {
        "brew" => Some(InstallMethodId::Brew),
        "cargo" => Some(InstallMethodId::Cargo),
        "go" => Some(InstallMethodId::Go),
        "npm" => Some(InstallMethodId::Npm),
        "rustup" => Some(InstallMethodId::Rustup),
        _ => None,
    }
}

fn parse_lint_runner_kind(value: &str) -> Option<LintRunnerKind> {
    match value {
        "cargo" | "clippy" => Some(LintRunnerKind::Clippy),
        "golangci-lint" => Some(LintRunnerKind::GolangciLint),
        "ruff" => Some(LintRunnerKind::Ruff),
        _ => None,
    }
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

fn installed_lsps_storage_path() -> PathBuf {
    if let Some(xdg_config) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config)
            .join("redox")
            .join(INSTALLED_LSPS_FILE);
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("redox")
            .join(INSTALLED_LSPS_FILE);
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".config")
        .join("redox")
        .join(INSTALLED_LSPS_FILE)
}

fn executable_on_path(executable: &str) -> bool {
    if executable.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(executable).exists();
    }

    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&paths).any(|dir| dir.join(executable).exists())
}

fn clippy_available() -> bool {
    Command::new("cargo")
        .args(["clippy", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn apply_snippet_edits(buffer: &mut redox_core::TextBuffer, edits: &[(usize, usize, String)]) {
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
    ordered.sort_by_key(|(start, _, _)| *start);
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
    ordered.sort_by_key(|(_, (start, _, _))| *start);
    for (idx, (start, end, text)) in ordered {
        if skip_idx == Some(idx) {
            continue;
        }
        let replacement_len = text.chars().count();
        let replaced_len = end.saturating_sub(*start);
        if transformed < *start || transformed == *start {
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
    ordered.sort_by_key(|(start, _, _)| *start);
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("redox-{name}-{nonce}"));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    #[test]
    fn file_uri_percent_encodes_special_characters() {
        let uri = file_uri(Path::new("/tmp/redox test #1.rs")).expect("URI should encode");
        assert_eq!(uri, "file:///tmp/redox%20test%20%231.rs");
    }

    #[test]
    fn publish_diagnostics_uses_payload_uri() {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///tmp/example.rs",
                "diagnostics": [
                    {
                        "range": {
                            "start": { "line": 2, "character": 4 },
                            "end": { "line": 2, "character": 9 }
                        },
                        "severity": 1,
                        "message": "something went wrong\n`#[warn(foo)]` on by default"
                    }
                ]
            }
        });

        let (uri, version, diagnostics) =
            parse_publish_diagnostics(&message).expect("diagnostics should parse");
        assert_eq!(uri, "file:///tmp/example.rs");
        assert_eq!(version, None);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostics[0].start_line, 2);
        assert_eq!(diagnostics[0].start_utf16, 4);
        assert_eq!(
            diagnostics[0].message,
            "something went wrong\n`#[warn(foo)]` on by default"
        );
    }

    #[test]
    fn publish_diagnostics_preserves_version() {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///tmp/example.rs",
                "version": 7,
                "diagnostics": []
            }
        });

        let (uri, version, diagnostics) =
            parse_publish_diagnostics(&message).expect("diagnostics should parse");
        assert_eq!(uri, "file:///tmp/example.rs");
        assert_eq!(version, Some(7));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn publish_diagnostics_strips_empty_see_details_marker() {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///tmp/example.rs",
                "diagnostics": [
                    {
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end": { "line": 0, "character": 1 }
                        },
                        "message": "borrowed value does not live long enough (see details)"
                    }
                ]
            }
        });

        let (_, _, diagnostics) =
            parse_publish_diagnostics(&message).expect("diagnostics should parse");
        assert_eq!(
            diagnostics[0].message,
            "borrowed value does not live long enough"
        );
    }

    #[test]
    fn publish_diagnostics_preserves_related_details() {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///tmp/example.rs",
                "diagnostics": [
                    {
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end": { "line": 0, "character": 1 }
                        },
                        "message": "type mismatch (see details)",
                        "relatedInformation": [
                            {
                                "location": {
                                    "uri": "file:///tmp/example.rs",
                                    "range": {
                                        "start": { "line": 2, "character": 4 },
                                        "end": { "line": 2, "character": 8 }
                                    }
                                },
                                "message": "expected `usize` here"
                            }
                        ]
                    }
                ]
            }
        });

        let (_, _, diagnostics) =
            parse_publish_diagnostics(&message).expect("diagnostics should parse");
        assert_eq!(
            diagnostics[0].message,
            "type mismatch\n\nDetails:\n- expected `usize` here"
        );
    }

    #[test]
    fn completion_response_parses_array_and_snippet_items() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "result": [
                {
                    "label": "println!",
                    "kind": 3,
                    "insertText": "println!(\"${1:value}\");$0",
                    "insertTextFormat": 2
                }
            ]
        });

        let completions = parse_completion_response(&message);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].label, "println!");
        assert_eq!(completions[0].kind.as_deref(), Some("function"));
        assert_eq!(completions[0].insert_text_format, InsertTextFormat::Snippet);
    }

    #[test]
    fn completion_response_uses_list_item_defaults() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "result": {
                "isIncomplete": false,
                "itemDefaults": {
                    "insertTextFormat": 2,
                    "editRange": {
                        "start": { "line": 1, "character": 4 },
                        "end": { "line": 1, "character": 7 }
                    }
                },
                "items": [
                    {
                        "label": "Ok",
                        "insertText": "Ok(${1:value})"
                    }
                ]
            }
        });

        let completions = parse_completion_response(&message);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].insert_text_format, InsertTextFormat::Snippet);
        let edit = completions[0]
            .text_edit
            .as_ref()
            .expect("default edit range should be applied");
        assert_eq!(edit.range.start.line, 1);
        assert_eq!(edit.range.end.character, 7);
    }

    #[test]
    fn function_completion_synthesizes_placeholders_from_signature() {
        let item = CompletionCandidate {
            label: "DoThing".to_string(),
            detail: Some("func DoThing(ctx context.Context, name string) error".to_string()),
            label_detail: None,
            label_description: None,
            documentation: None,
            kind: Some("function".to_string()),
            filter_text: None,
            sort_text: None,
            insert_text: "DoThing()".to_string(),
            insert_text_format: InsertTextFormat::PlainText,
            text_edit: None,
        };

        let expansion = completion_snippet_expansion(&item, &item.insert_text)
            .expect("function signature should produce snippet placeholders");

        assert_eq!(expansion.text, "DoThing(ctx, name)");
        assert_eq!(expansion.placeholders.len(), 2);
        assert_eq!(expansion.placeholders[0].start, 8);
        assert_eq!(expansion.placeholders[0].end, 11);
        assert_eq!(expansion.placeholders[1].start, 13);
        assert_eq!(expansion.placeholders[1].end, 17);
    }

    #[test]
    fn function_completion_enriches_cursor_only_snippet_from_signature() {
        let item = CompletionCandidate {
            label: "DoThing".to_string(),
            detail: Some("func DoThing(ctx context.Context) error".to_string()),
            label_detail: None,
            label_description: None,
            documentation: None,
            kind: Some("function".to_string()),
            filter_text: None,
            sort_text: None,
            insert_text: "DoThing($0)".to_string(),
            insert_text_format: InsertTextFormat::Snippet,
            text_edit: None,
        };

        let expansion = completion_snippet_expansion(&item, &item.insert_text)
            .expect("cursor-only call snippet should be enriched");

        assert_eq!(expansion.text, "DoThing(ctx)");
        assert_eq!(expansion.placeholders.len(), 1);
        assert_eq!(expansion.placeholders[0].start, 8);
        assert_eq!(expansion.placeholders[0].end, 11);
    }

    #[test]
    fn function_completion_enriches_empty_snippet_placeholders_from_signature() {
        let item = CompletionCandidate {
            label: "DoThing".to_string(),
            detail: Some("func DoThing(ctx context.Context, name string) error".to_string()),
            label_detail: None,
            label_description: None,
            documentation: None,
            kind: Some("function".to_string()),
            filter_text: None,
            sort_text: None,
            insert_text: "DoThing(${1:}, ${2:})".to_string(),
            insert_text_format: InsertTextFormat::Snippet,
            text_edit: None,
        };

        let expansion = completion_snippet_expansion(&item, &item.insert_text)
            .expect("empty snippet placeholders should be enriched");

        assert_eq!(expansion.text, "DoThing(ctx, name)");
        assert_eq!(expansion.placeholders.len(), 2);
        assert_eq!(expansion.placeholders[0].start, 8);
        assert_eq!(expansion.placeholders[0].end, 11);
        assert_eq!(expansion.placeholders[1].start, 13);
        assert_eq!(expansion.placeholders[1].end, 17);
    }

    #[test]
    fn function_completion_synthesizes_placeholders_from_label_signature() {
        let item = CompletionCandidate {
            label: "DoThing(ctx context.Context, name string)".to_string(),
            detail: None,
            label_detail: None,
            label_description: None,
            documentation: None,
            kind: Some("function".to_string()),
            filter_text: None,
            sort_text: None,
            insert_text: "DoThing".to_string(),
            insert_text_format: InsertTextFormat::PlainText,
            text_edit: None,
        };

        let expansion = completion_snippet_expansion(&item, &item.insert_text)
            .expect("label signature should produce snippet placeholders");

        assert_eq!(expansion.text, "DoThing(ctx, name)");
        assert_eq!(expansion.placeholders.len(), 2);
        assert_eq!(expansion.placeholders[0].start, 8);
        assert_eq!(expansion.placeholders[0].end, 11);
        assert_eq!(expansion.placeholders[1].start, 13);
        assert_eq!(expansion.placeholders[1].end, 17);
    }

    #[test]
    fn snippets_expand_placeholders_and_preserve_first_cursor_target() {
        let expansion = expand_lsp_snippet("fn ${1:name}(${2:arg}) {\n\t$0\n}");

        assert_eq!(expansion.text, "fn name(arg) {\n\t\n}");
        assert_eq!(expansion.cursor_offset, Some(16));
        assert_eq!(expansion.placeholders.len(), 2);
        assert_eq!(expansion.placeholders[0].tabstop, 1);
        assert_eq!(expansion.placeholders[0].start, 3);
        assert_eq!(expansion.placeholders[0].end, 7);
    }

    #[test]
    fn snippet_selection_collapses_after_first_replacement() {
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_str("call(arg, arg)");
        state.lsp.active_snippet = Some(ActiveSnippet {
            buffer_id,
            placeholders: vec![
                ActiveSnippetPlaceholder {
                    tabstop: 1,
                    start_char: 5,
                    end_char: 8,
                    filled: false,
                },
                ActiveSnippetPlaceholder {
                    tabstop: 1,
                    start_char: 10,
                    end_char: 13,
                    filled: false,
                },
            ],
            current: 0,
            selected: true,
            final_char: Some(14),
        });

        assert!(state.replace_active_snippet_selection_text("f", 80, 24));
        assert!(
            !state
                .lsp
                .active_snippet
                .as_ref()
                .expect("snippet should remain active")
                .selected
        );

        for ch in ['o', 'o'] {
            let text = ch.to_string();
            let insert_at_char = state
                .session
                .active_buffer()
                .pos_to_char(state.views.get(&buffer_id).unwrap().cursor.cursor);
            let cursor = state.views.get(&buffer_id).unwrap().cursor.cursor;
            let new_cursor = state.session.active_buffer_mut().insert(cursor, &text);
            state.views.get_mut(&buffer_id).unwrap().cursor.cursor = new_cursor;
            let _ = state.mirror_active_snippet_insert_after_cursor_insert(
                insert_at_char,
                &text,
                80,
                24,
            );
        }

        assert_eq!(state.session.active_buffer().to_string(), "call(foo, foo)");
        assert!(state.active_snippet_placeholder_ranges(0, 1).is_empty());
    }

    #[test]
    fn snippet_tab_skips_mirrored_placeholders() {
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_str("foo, foo, bar");
        state.lsp.active_snippet = Some(ActiveSnippet {
            buffer_id,
            placeholders: vec![
                ActiveSnippetPlaceholder {
                    tabstop: 1,
                    start_char: 0,
                    end_char: 3,
                    filled: false,
                },
                ActiveSnippetPlaceholder {
                    tabstop: 1,
                    start_char: 5,
                    end_char: 8,
                    filled: false,
                },
                ActiveSnippetPlaceholder {
                    tabstop: 2,
                    start_char: 10,
                    end_char: 13,
                    filled: false,
                },
            ],
            current: 0,
            selected: true,
            final_char: Some(13),
        });

        assert!(state.snippet_jump_next(80, 24));

        let snippet = state
            .lsp
            .active_snippet
            .as_ref()
            .expect("snippet should remain active");
        assert_eq!(snippet.current, 2);
        assert_eq!(
            state.views.get(&buffer_id).unwrap().cursor.cursor,
            redox_core::Pos::new(0, 10)
        );
    }

    #[test]
    fn snippet_tab_repeats_after_placeholder_edits() {
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_str("one, two, three");
        state.lsp.active_snippet = Some(ActiveSnippet {
            buffer_id,
            placeholders: vec![
                ActiveSnippetPlaceholder {
                    tabstop: 1,
                    start_char: 0,
                    end_char: 3,
                    filled: false,
                },
                ActiveSnippetPlaceholder {
                    tabstop: 2,
                    start_char: 5,
                    end_char: 8,
                    filled: false,
                },
                ActiveSnippetPlaceholder {
                    tabstop: 3,
                    start_char: 10,
                    end_char: 15,
                    filled: false,
                },
            ],
            current: 0,
            selected: true,
            final_char: Some(15),
        });

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
    fn snippet_backspace_updates_mirrored_placeholders() {
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_str("call(arg, arg)");
        state.lsp.active_snippet = Some(ActiveSnippet {
            buffer_id,
            placeholders: vec![
                ActiveSnippetPlaceholder {
                    tabstop: 1,
                    start_char: 5,
                    end_char: 8,
                    filled: false,
                },
                ActiveSnippetPlaceholder {
                    tabstop: 1,
                    start_char: 10,
                    end_char: 13,
                    filled: false,
                },
                ActiveSnippetPlaceholder {
                    tabstop: 2,
                    start_char: 14,
                    end_char: 14,
                    filled: false,
                },
            ],
            current: 0,
            selected: true,
            final_char: Some(14),
        });

        assert!(state.replace_active_snippet_selection_text("foo", 80, 24));
        let cursor = state.views.get(&buffer_id).unwrap().cursor.cursor;
        let deleted_end = state.session.active_buffer().pos_to_char(cursor);
        let deleted_start = deleted_end.saturating_sub(1);
        let new_cursor = state
            .session
            .active_buffer_mut()
            .backspace(redox_core::Selection::empty(cursor))
            .cursor;
        state.views.get_mut(&buffer_id).unwrap().cursor.cursor = new_cursor;

        assert!(state.mirror_active_snippet_delete_after_cursor_delete(
            deleted_start,
            deleted_end,
            80,
            24
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
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        state.mode = EditorMode::Insert;
        state.lsp.active_snippet = Some(ActiveSnippet {
            buffer_id,
            placeholders: vec![ActiveSnippetPlaceholder {
                tabstop: 1,
                start_char: 0,
                end_char: 0,
                filled: false,
            }],
            current: 0,
            selected: true,
            final_char: Some(0),
        });

        state.apply_input(
            crate::input::InputAction::SetMode(crate::input::InputMode::Normal),
            80,
            24,
        );

        assert!(state.lsp.active_snippet.is_none());
    }

    #[test]
    fn completion_cancel_only_stays_in_insert_for_visible_popup() {
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        state.mode = EditorMode::Insert;
        state.lsp.completion = Some(CompletionState {
            selected: 0,
            requested_at: redox_core::Pos::new(0, 0),
            items: vec![CompletionCandidate {
                label: "print".to_string(),
                detail: None,
                label_detail: None,
                label_description: None,
                documentation: None,
                kind: Some("function".to_string()),
                filter_text: None,
                sort_text: None,
                insert_text: "print".to_string(),
                insert_text_format: InsertTextFormat::PlainText,
                text_edit: None,
            }],
        });

        state.apply_input(crate::input::InputAction::CompletionCancel, 80, 24);

        assert_eq!(state.mode, EditorMode::Insert);
        assert!(state.lsp.completion.is_none());

        state.lsp.completion = Some(CompletionState {
            selected: 0,
            requested_at: redox_core::Pos::new(0, 0),
            items: Vec::new(),
        });

        state.apply_input(crate::input::InputAction::CompletionCancel, 80, 24);

        assert_eq!(state.mode, EditorMode::Normal);
        assert!(state.lsp.completion.is_none());
    }

    #[test]
    fn completion_popup_visibility_tracks_current_prefix() {
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_str("prin");
        state.views.get_mut(&buffer_id).unwrap().cursor.cursor = redox_core::Pos::new(0, 4);
        state.lsp.completion = Some(CompletionState {
            selected: 0,
            requested_at: redox_core::Pos::new(0, 3),
            items: vec![CompletionCandidate {
                label: "println".to_string(),
                detail: None,
                label_detail: None,
                label_description: None,
                documentation: None,
                kind: Some("function".to_string()),
                filter_text: None,
                sort_text: None,
                insert_text: "println".to_string(),
                insert_text_format: InsertTextFormat::PlainText,
                text_edit: None,
            }],
        });

        assert!(state.completion_popup().is_some());

        state.views.get_mut(&buffer_id).unwrap().cursor.cursor = redox_core::Pos::new(0, 0);
        assert!(state.completion_popup().is_none());
    }

    #[test]
    fn snippet_tab_clears_snippet_after_cursor_moves_away() {
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_str("foo bar");
        state.views.get_mut(&buffer_id).unwrap().cursor.cursor = redox_core::Pos::new(0, 7);
        state.lsp.active_snippet = Some(ActiveSnippet {
            buffer_id,
            placeholders: vec![ActiveSnippetPlaceholder {
                tabstop: 1,
                start_char: 0,
                end_char: 3,
                filled: false,
            }],
            current: 0,
            selected: false,
            final_char: Some(3),
        });

        assert!(!state.snippet_jump_next(80, 24));
        assert!(state.lsp.active_snippet.is_none());
    }

    #[test]
    fn typing_in_comment_disables_auto_completion_and_closes_popup() {
        let session = redox_core::EditorSession::open_initial_unnamed()
            .expect("failed to open unnamed session");
        let mut state = EditorState::new(session);
        let buffer_id = state.session.active_id();
        *state.session.active_buffer_mut() = redox_core::TextBuffer::from_str("// comment");
        state.views.get_mut(&buffer_id).unwrap().cursor.cursor = redox_core::Pos::new(0, 10);
        state.mode = EditorMode::Insert;
        state.lsp.completion = Some(CompletionState {
            selected: 0,
            requested_at: redox_core::Pos::new(0, 10),
            items: vec![CompletionCandidate {
                label: "comment".to_string(),
                detail: None,
                label_detail: None,
                label_description: None,
                documentation: None,
                kind: Some("text".to_string()),
                filter_text: None,
                sort_text: None,
                insert_text: "comment".to_string(),
                insert_text_format: InsertTextFormat::PlainText,
                text_edit: None,
            }],
        });

        state.queue_auto_completion_after_insert('a');

        assert!(state.lsp.auto_completion.is_none());
        assert!(state.lsp.completion.is_none());
    }

    #[test]
    fn rust_workspace_root_prefers_outermost_cargo_manifest() {
        let root = temp_test_dir("rust-root");
        let crate_dir = root.join("member");
        let src_dir = crate_dir.join("src");
        fs::create_dir_all(&src_dir).expect("src dir should be created");
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\"]\n",
        )
        .expect("workspace manifest should be written");
        fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
        )
        .expect("crate manifest should be written");

        let file = src_dir.join("lib.rs");
        let detected = workspace_root_for(&file, ProviderId::RustAnalyzer, Path::new("/fallback"));
        assert_eq!(detected, root);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn diagnostic_summary_line_prefers_first_non_empty_line() {
        assert_eq!(
            diagnostic_summary_line(
                "\nunused import: `std::env`\n`#[warn(unused_imports)]` on by default"
            ),
            "unused import: `std::env`"
        );
    }

    #[test]
    fn parse_definition_response_accepts_location_arrays() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 9,
            "result": [
                {
                    "uri": "file:///tmp/example.rs",
                    "range": {
                        "start": { "line": 4, "character": 2 },
                        "end": { "line": 4, "character": 7 }
                    }
                }
            ]
        });

        let target = parse_definition_response(&message).expect("definition target should parse");
        assert_eq!(target.uri, "file:///tmp/example.rs");
        assert_eq!(target.range.start.line, 4);
        assert_eq!(target.range.start.character, 2);
    }

    #[test]
    fn parse_hover_response_preserves_code_blocks_and_markdown() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "contents": [
                    {
                        "language": "rust",
                        "value": "pub fn hover() -> bool"
                    },
                    {
                        "kind": "markdown",
                        "value": "Returns `true` when hover is available.\n\n- Fast"
                    }
                ]
            }
        });

        let blocks = parse_hover_response(&message);
        assert_eq!(
            blocks,
            vec![
                SymbolInfoBlock {
                    kind: SymbolInfoKind::Code {
                        language: Some("rust".to_string()),
                    },
                    text: "pub fn hover() -> bool".to_string(),
                },
                SymbolInfoBlock {
                    kind: SymbolInfoKind::Markdown,
                    text: "Returns `true` when hover is available.\n\n- Fast".to_string(),
                }
            ]
        );
    }

    #[test]
    fn parse_hover_response_keeps_plaintext_paragraphs() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 8,
            "result": {
                "contents": {
                    "kind": "plaintext",
                    "value": "Line one\n\n  Line two  \n"
                }
            }
        });

        let blocks = parse_hover_response(&message);
        assert_eq!(
            blocks,
            vec![SymbolInfoBlock {
                kind: SymbolInfoKind::PlainText,
                text: "Line one\n\n  Line two".to_string(),
            }]
        );
    }

    #[test]
    fn parse_hover_response_collapses_repeated_blank_lines() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 10,
            "result": {
                "contents": {
                    "kind": "markdown",
                    "value": "Title\n\n\n\nBody\n\n\n- item"
                }
            }
        });

        let blocks = parse_hover_response(&message);
        assert_eq!(
            blocks,
            vec![SymbolInfoBlock {
                kind: SymbolInfoKind::Markdown,
                text: "Title\n\nBody\n\n- item".to_string(),
            }]
        );
    }

    #[test]
    fn code_action_response_parses_literals_and_commands() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 12,
            "result": [
                {
                    "title": "Import `std::fmt::Debug`",
                    "kind": "quickfix",
                    "isPreferred": true,
                    "edit": {
                        "changes": {
                            "file:///tmp/example.rs": [
                                {
                                    "range": {
                                        "start": { "line": 0, "character": 0 },
                                        "end": { "line": 0, "character": 0 }
                                    },
                                    "newText": "use std::fmt::Debug;\\n"
                                }
                            ]
                        }
                    }
                },
                {
                    "title": "Run command",
                    "command": "example.command",
                    "arguments": ["value"]
                },
                {
                    "title": "Disabled",
                    "kind": "quickfix",
                    "disabled": { "reason": "nope" }
                }
            ]
        });

        let actions = parse_code_action_response(&message);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Import `std::fmt::Debug`");
        assert!(actions[0].preferred);
        assert!(actions[0].edit.is_some());
        assert_eq!(
            actions[1]
                .command
                .as_ref()
                .map(|command| command.command.as_str()),
            Some("example.command")
        );
    }

    #[test]
    fn workspace_edit_parser_supports_document_changes() {
        let edit = parse_workspace_edit(&json!({
            "documentChanges": [
                {
                    "textDocument": { "uri": "file:///tmp/example.rs", "version": 1 },
                    "edits": [
                        {
                            "range": {
                                "start": { "line": 1, "character": 2 },
                                "end": { "line": 1, "character": 5 }
                            },
                            "newText": "value"
                        }
                    ]
                }
            ]
        }))
        .expect("workspace edit should parse");

        assert_eq!(edit.document_edits.len(), 1);
        assert_eq!(edit.document_edits[0].uri, "file:///tmp/example.rs");
        assert_eq!(edit.document_edits[0].edits.len(), 1);
        assert_eq!(edit.document_edits[0].edits[0].new_text, "value");
    }

    #[test]
    fn close_symbol_info_restores_previous_mode() {
        let session = EditorSession::open_initial_unnamed().expect("session should open");
        let mut state = EditorState::new(session);
        state.mode = EditorMode::SymbolInfo;
        state.lsp.symbol_info = Some(SymbolInfoState {
            requested_at: Pos::new(0, 0),
            blocks: vec![SymbolInfoBlock {
                kind: SymbolInfoKind::PlainText,
                text: "hello".to_string(),
            }],
            cached_width: None,
            display_lines: Vec::new(),
            scroll: 2,
            return_mode: EditorMode::Insert,
        });

        assert!(state.close_symbol_info());
        assert_eq!(state.mode, EditorMode::Insert);
        assert!(state.lsp.symbol_info.is_none());
    }

    #[test]
    fn symbol_info_move_scrolls_by_delta() {
        let session = EditorSession::open_initial_unnamed().expect("session should open");
        let mut state = EditorState::new(session);
        state.lsp.symbol_info = Some(SymbolInfoState {
            requested_at: Pos::new(0, 0),
            blocks: vec![SymbolInfoBlock {
                kind: SymbolInfoKind::PlainText,
                text: "hello".to_string(),
            }],
            cached_width: None,
            display_lines: Vec::new(),
            scroll: 1,
            return_mode: EditorMode::Normal,
        });

        assert!(state.symbol_info_move(3));
        assert_eq!(
            state.lsp.symbol_info.as_ref().map(|info| info.scroll),
            Some(4)
        );

        assert!(state.symbol_info_move(-10));
        assert_eq!(
            state.lsp.symbol_info.as_ref().map(|info| info.scroll),
            Some(0)
        );
    }

    #[test]
    fn clamp_symbol_info_scroll_trims_overscroll_to_visible_bottom() {
        let session = EditorSession::open_initial_unnamed().expect("session should open");
        let mut state = EditorState::new(session);
        state.mode = EditorMode::SymbolInfo;
        state.lsp.symbol_info = Some(SymbolInfoState {
            requested_at: Pos::new(0, 0),
            blocks: vec![SymbolInfoBlock {
                kind: SymbolInfoKind::PlainText,
                text: (0..20)
                    .map(|i| format!("line {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            }],
            cached_width: None,
            display_lines: Vec::new(),
            scroll: 99,
            return_mode: EditorMode::Normal,
        });

        state.clamp_symbol_info_scroll(80);
        assert_eq!(
            state.lsp.symbol_info.as_ref().map(|info| info.scroll),
            Some(8)
        );
    }

    #[test]
    fn clippy_output_parses_workspace_relative_spans() {
        let root = temp_test_dir("clippy-output");
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).expect("src dir should be created");
        fs::write(
            src_dir.join("lib.rs"),
            "pub fn demo() {\n    let unused_value = 42;\n}\n",
        )
        .expect("source file should be written");

        let stdout = br#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `unused_value`","spans":[{"file_name":"src/lib.rs","line_start":2,"line_end":2,"column_start":9,"column_end":21,"is_primary":true}]}}"#;
        let diagnostics = parse_clippy_output(stdout, &root);
        let uri = file_uri(&src_dir.join("lib.rs")).expect("URI should build");
        let items = diagnostics
            .get(&uri)
            .expect("diagnostics should include file");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(items[0].start_line, 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ruff_output_parses_json_diagnostics() {
        let root = temp_test_dir("ruff-output");
        let file = root.join("example.py");
        fs::write(&file, "import os\n").expect("python file should be written");

        let stdout = br#"[{"filename":"example.py","message":"`os` imported but unused","code":"F401","location":{"row":1,"column":8},"end_location":{"row":1,"column":10}}]"#;
        let diagnostics = parse_ruff_output(stdout, &root);
        let uri = file_uri(&file).expect("URI should build");
        let items = diagnostics
            .get(&uri)
            .expect("diagnostics should include file");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].severity, DiagnosticSeverity::Warning);
        assert!(items[0].message.contains("F401"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn golangci_lint_text_output_parses_diagnostics() {
        let root = temp_test_dir("golangci-text-output");
        let dir = root.join("lexer");
        fs::create_dir_all(&dir).expect("lexer dir should be created");
        fs::write(
            dir.join("lexer.go"),
            "package lexer\n\ntype token struct {\n\tfoo string\n}\n",
        )
        .expect("go file should be written");

        let stderr = b"lexer/lexer.go:4:2: field foo is unused (unused)\n";
        let diagnostics = parse_golangci_lint_text_output(stderr, &root);
        let uri = file_uri(&dir.join("lexer.go")).expect("URI should build");
        let items = diagnostics
            .get(&uri)
            .expect("diagnostics should include file");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(items[0].start_line, 3);
        assert!(items[0].message.contains("field foo is unused"));

        let _ = fs::remove_dir_all(&root);
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
        };
        let lint_warning = StoredDiagnostic {
            severity: DiagnosticSeverity::Warning,
            message: "unused variable".to_string(),
            start_line: 1,
            end_line: 1,
            start_utf16: 0,
            end_utf16: 1,
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
