use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use redox_core::{BufferId, BufferKind, Pos};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::{EditorMode, EditorState, StatusMessageStyle};
use crate::ui::language_for_path;
use crate::ui::syntax::SyntaxLanguage;

const INSTALLED_LSPS_FILE: &str = "installed_lsps.json";
const INITIALIZE_REQUEST_ID: i64 = 1;
const FIRST_DYNAMIC_REQUEST_ID: i64 = 2;
const LSP_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const DIAGNOSTICS_POPUP_VISIBLE_ROWS: usize = 12;
const LSP_CHANGE_DEBOUNCE: Duration = Duration::from_millis(175);
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
    message_count: usize,
}

#[derive(Debug, Clone)]
pub struct LspMarketplacePopup {
    pub entries: Vec<LspMarketplaceEntry>,
    pub selected: usize,
    pub scroll: usize,
}

#[derive(Debug, Clone)]
pub struct LspMarketplaceEntry {
    item_id: MarketplaceItemId,
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
struct DefinitionTarget {
    uri: String,
    range: IncomingRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkspaceKey {
    provider_id: ProviderId,
    root: PathBuf,
}

#[derive(Debug, Clone)]
struct StoredDiagnostic {
    severity: DiagnosticSeverity,
    message: String,
    start_line: usize,
    end_line: usize,
    start_utf16: u32, // Positions are UTF-16 based!!
    end_utf16: u32,
}

impl StoredDiagnostic {
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
            message: self.message.clone(),
            line,
            start_col,
            end_col: end_col.max(start_col.saturating_add(1)),
        }
    }
}

#[derive(Debug, Clone)]
struct StoredDiagnostics {
    source: DiagnosticSource,
    items: Vec<StoredDiagnostic>,
}

#[derive(Debug, Clone)]
struct DeferredDiagnostics {
    uri: String,
    version: Option<i32>,
    source: DiagnosticSource,
    items: Vec<StoredDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum DiagnosticSource {
    Lsp(WorkspaceKey),
    Lint(LintSource),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LintSource {
    kind: LintRunnerKind,
    root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LintRunnerKind {
    Clippy,
    GolangciLint,
    Ruff,
}

impl LintRunnerKind {
    fn executable(self) -> &'static str {
        match self {
            Self::Clippy => "cargo-clippy",
            Self::GolangciLint => "golangci-lint",
            Self::Ruff => "ruff",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRequest {
    GotoDefinition,
}

#[derive(Debug, Clone, Copy)]
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
                    "textDocument": {
                        "publishDiagnostics": {
                            "relatedInformation": true,
                            "versionSupport": true
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

    fn try_recv(&self) -> Option<SessionEvent> {
        self.events.try_recv().ok()
    }
}

impl EditorState {
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
    }

    pub(super) fn open_lsp_marketplace(&mut self) {
        if self.explorer_is_active() || self.about_popup().is_some() {
            return;
        }
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
        let entries = self.current_diagnostic_popup_entries();
        if entries.is_empty() {
            self.set_status("no diagnostics in current file");
            return;
        }
        self.lsp.diagnostics_popup = Some(DiagnosticsPopupState { selected: 0 });
        self.mode = EditorMode::DiagnosticsList;
    }

    pub(super) fn close_diagnostics_popup(&mut self) {
        if self.mode == EditorMode::DiagnosticsList {
            self.mode = EditorMode::Normal;
        }
        self.lsp.diagnostics_popup = None;
    }

    pub(super) fn diagnostics_popup_move(&mut self, delta: isize) {
        let entries = self.current_diagnostic_popup_entries();
        if entries.is_empty() {
            return;
        }
        let Some(state) = self.lsp.diagnostics_popup.as_mut() else {
            return;
        };
        let max_index = entries.len().saturating_sub(1) as isize;
        state.selected = (state.selected as isize + delta).clamp(0, max_index) as usize;
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
                summary: diagnostic_summary_line(&diagnostic.message),
                message: diagnostic.message,
            })
            .collect()
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
                (key.workspace == *workspace && request.kind == kind).then_some(key.clone())
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
                    .then_some((key.clone(), request.kind))
            })
            .collect::<Vec<_>>();
        for (key, kind) in timed_out {
            if self.lsp.pending_requests.remove(&key).is_none() {
                continue;
            }
            self.send_lsp_cancel_request(&key);
            match kind {
                PendingRequest::GotoDefinition => self.set_status("definition lookup timed out"),
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
        let request = self.lsp.pending_requests.remove(&RequestKey {
            workspace: workspace.clone(),
            id,
        })?;
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
        }
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

        let Some(client) = self.lsp.clients.get_mut(workspace) else {
            return true;
        };
        let result = match method {
            "client/registerCapability" | "client/unregisterCapability" => Some(Value::Null),
            "workspace/configuration" => Some(configuration_response(message)),
            "workspace/workspaceFolders" => Some(workspace_folders_response(workspace)),
            _ => {
                let _ = client.session.send_method_not_found(id, method);
                return true;
            }
        };

        if let Some(result) = result {
            let _ = client.session.send_response(id, result);
        }
        true
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

#[derive(Debug, Deserialize)]
struct PublishDiagnosticsParams {
    uri: String,
    version: Option<i32>,
    diagnostics: Vec<IncomingDiagnostic>,
}

#[derive(Debug, Deserialize)]
struct IncomingDiagnostic {
    range: IncomingRange,
    severity: Option<u64>,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct IncomingRange {
    start: IncomingPosition,
    end: IncomingPosition,
}

#[derive(Debug, Clone, Deserialize)]
struct IncomingPosition {
    line: u64,
    character: u64,
}

#[derive(Debug, Deserialize)]
struct IncomingLocation {
    uri: String,
    range: IncomingRange,
}

#[derive(Debug, Deserialize)]
struct IncomingLocationLink {
    #[serde(rename = "targetUri")]
    target_uri: String,
    #[serde(rename = "targetSelectionRange")]
    target_selection_range: IncomingRange,
}

fn configuration_response(message: &Value) -> Value {
    let item_count = message
        .get("params")
        .and_then(|params| params.get("items"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Value::Array(vec![Value::Null; item_count])
}

fn workspace_folders_response(workspace: &WorkspaceKey) -> Value {
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

fn parse_publish_diagnostics(
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
    Some(StoredDiagnostic {
        severity,
        message: diagnostic.message,
        start_line: usize::try_from(diagnostic.range.start.line).ok()?,
        end_line: usize::try_from(diagnostic.range.end.line).ok()?,
        start_utf16: u32::try_from(diagnostic.range.start.character).ok()?,
        end_utf16: u32::try_from(diagnostic.range.end.character).ok()?,
    })
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

fn should_suppress_lint_diagnostics<'a>(
    entries: impl IntoIterator<Item = (&'a DiagnosticSource, &'a StoredDiagnostic)>,
) -> bool {
    entries.into_iter().any(|(source, diagnostic)| {
        !matches!(source, DiagnosticSource::Lint(_))
            && diagnostic.severity == DiagnosticSeverity::Error
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

fn clip_diagnostic_message(message: &str) -> String {
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

fn diagnostic_summary_line(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| message.trim())
        .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
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
