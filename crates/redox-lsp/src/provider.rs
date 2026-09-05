use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::Mutex;

use crate::lint::LintRunnerKind;
use crate::transport::ServerCommand;

static CLIPPY_AVAILABLE: Mutex<bool> = Mutex::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    C,
    Cpp,
    Css,
    Go,
    Html,
    JavaScript,
    Json,
    Lua,
    Markdown,
    Python,
    Rust,
    Toml,
    TypeScript,
    Tsx,
    Yaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderId {
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
    #[must_use]
    pub const fn as_str(self) -> &'static str {
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
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        built_in_providers()
            .iter()
            .find(|provider| provider.id.as_str() == value)
            .map(|provider| provider.id)
            .ok_or(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderSpec {
    pub id: ProviderId,
    pub label: &'static str,
    pub language_label: &'static str,
    pub executable: &'static str,
    pub args: &'static [&'static str],
    pub languages: &'static [Language],
    pub install_plans: &'static [InstallPlan],
}

/// Metadata and workspace policy needed to launch a language server.
///
/// Downstream crates can implement this trait for providers outside Redox's
/// built-in catalogue.
pub trait LanguageServerProvider {
    fn id(&self) -> &str;

    fn label(&self) -> &str;

    fn command(&self) -> ServerCommand;

    fn language_id_for(&self, language: Language) -> Option<&str>;

    fn install_plans(&self) -> &[InstallPlan] {
        &[]
    }

    fn workspace_root(&self, path: &Path, launch_dir: &Path) -> PathBuf {
        crate::workspace::default_workspace_root(path, launch_dir)
    }
}

impl ProviderSpec {
    #[must_use]
    pub fn matches_language(self, language: Language) -> bool {
        self.languages.contains(&language)
    }

    #[must_use]
    pub fn language_id_for(self, language: Language) -> Option<&'static str> {
        match (self.id, language) {
            (ProviderId::Clangd, Language::C) => Some("c"),
            (ProviderId::Clangd, Language::Cpp) => Some("cpp"),
            (ProviderId::TypeScriptLanguageServer, Language::JavaScript) => Some("javascript"),
            (ProviderId::TypeScriptLanguageServer, Language::TypeScript) => Some("typescript"),
            (ProviderId::TypeScriptLanguageServer, Language::Tsx) => Some("typescriptreact"),
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

    #[must_use]
    pub fn command(self) -> ServerCommand {
        ServerCommand::new(self.label, self.executable).args(self.args)
    }
}

impl LanguageServerProvider for ProviderSpec {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn label(&self) -> &str {
        self.label
    }

    fn command(&self) -> ServerCommand {
        (*self).command()
    }

    fn language_id_for(&self, language: Language) -> Option<&str> {
        (*self).language_id_for(language)
    }

    fn install_plans(&self) -> &[InstallPlan] {
        self.install_plans
    }

    fn workspace_root(&self, path: &Path, launch_dir: &Path) -> PathBuf {
        crate::workspace::built_in_workspace_root_for(path, self.id, launch_dir)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LinterSpec {
    pub kind: LintRunnerKind,
    pub label: &'static str,
    pub language_label: &'static str,
    pub install_plans: &'static [InstallPlan],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstallMethod {
    Brew,
    Cargo,
    Go,
    Npm,
    Rustup,
}

impl InstallMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Brew => "brew",
            Self::Cargo => "cargo",
            Self::Go => "go",
            Self::Npm => "npm",
            Self::Rustup => "rustup",
        }
    }
}

impl FromStr for InstallMethod {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "brew" => Ok(Self::Brew),
            "cargo" => Ok(Self::Cargo),
            "go" => Ok(Self::Go),
            "npm" => Ok(Self::Npm),
            "rustup" => Ok(Self::Rustup),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InstallPlan {
    pub method: InstallMethod,
    pub install_args: &'static [&'static str],
    pub uninstall: Uninstall,
}

#[derive(Debug, Clone, Copy)]
pub enum Uninstall {
    Command(&'static [&'static str]),
    GoBinary(&'static str),
    DisableOnly,
}

#[derive(Debug, Clone)]
pub struct ToolOperationResult {
    pub install_source: Option<InstallMethod>,
    pub success: bool,
    pub message: String,
}

const JS_TS_LANGUAGES: &[Language] = &[Language::JavaScript, Language::TypeScript, Language::Tsx];
const CSS_LANGUAGES: &[Language] = &[Language::Css];
const HTML_LANGUAGES: &[Language] = &[Language::Html];
const JSON_LANGUAGES: &[Language] = &[Language::Json];
const LUA_LANGUAGES: &[Language] = &[Language::Lua];
const MARKDOWN_LANGUAGES: &[Language] = &[Language::Markdown];
const PYTHON_LANGUAGES: &[Language] = &[Language::Python];
const RUST_LANGUAGES: &[Language] = &[Language::Rust];
const TOML_LANGUAGES: &[Language] = &[Language::Toml];
const YAML_LANGUAGES: &[Language] = &[Language::Yaml];
const GO_LANGUAGES: &[Language] = &[Language::Go];
const C_CPP_LANGUAGES: &[Language] = &[Language::C, Language::Cpp];

const BREW_RUST_ANALYZER: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Brew,
    install_args: &["install", "rust-analyzer"],
    uninstall: Uninstall::Command(&["uninstall", "rust-analyzer"]),
}];
const BREW_LUA_LANGUAGE_SERVER: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Brew,
    install_args: &["install", "lua-language-server"],
    uninstall: Uninstall::Command(&["uninstall", "lua-language-server"]),
}];
const BREW_MARKSMAN: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Brew,
    install_args: &["install", "marksman"],
    uninstall: Uninstall::Command(&["uninstall", "marksman"]),
}];
const CARGO_TAPLO: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Cargo,
    install_args: &["install", "taplo-cli", "--locked"],
    uninstall: Uninstall::Command(&["uninstall", "taplo-cli"]),
}];
const GO_GOPLS: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Go,
    install_args: &["install", "golang.org/x/tools/gopls@latest"],
    uninstall: Uninstall::GoBinary("gopls"),
}];
const NPM_PYRIGHT: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Npm,
    install_args: &["install", "-g", "pyright"],
    uninstall: Uninstall::Command(&["uninstall", "-g", "pyright"]),
}];
const NPM_TYPESCRIPT_LSP: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Npm,
    install_args: &["install", "-g", "typescript", "typescript-language-server"],
    uninstall: Uninstall::Command(&[
        "uninstall",
        "-g",
        "typescript-language-server",
        "typescript",
    ]),
}];
const NPM_YAML_LSP: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Npm,
    install_args: &["install", "-g", "yaml-language-server"],
    uninstall: Uninstall::Command(&["uninstall", "-g", "yaml-language-server"]),
}];
const NPM_VSCODE_JSON: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Npm,
    install_args: &["install", "-g", "vscode-langservers-extracted"],
    uninstall: Uninstall::DisableOnly,
}];
const NPM_VSCODE_HTML: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Npm,
    install_args: &["install", "-g", "vscode-langservers-extracted"],
    uninstall: Uninstall::DisableOnly,
}];
const NPM_VSCODE_CSS: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Npm,
    install_args: &["install", "-g", "vscode-langservers-extracted"],
    uninstall: Uninstall::DisableOnly,
}];
const RUSTUP_CLIPPY: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Rustup,
    install_args: &["component", "add", "clippy"],
    uninstall: Uninstall::Command(&["component", "remove", "clippy"]),
}];
const BREW_GOLANGCI_LINT: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Brew,
    install_args: &["install", "golangci-lint"],
    uninstall: Uninstall::Command(&["uninstall", "golangci-lint"]),
}];
const GO_GOLANGCI_LINT: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Go,
    install_args: &[
        "install",
        "github.com/golangci/golangci-lint/v2/cmd/golangci-lint@latest",
    ],
    uninstall: Uninstall::GoBinary("golangci-lint"),
}];
const BREW_RUFF: &[InstallPlan] = &[InstallPlan {
    method: InstallMethod::Brew,
    install_args: &["install", "ruff"],
    uninstall: Uninstall::Command(&["uninstall", "ruff"]),
}];
const NO_AUTO_INSTALL: &[InstallPlan] = &[];

static PROVIDERS: &[ProviderSpec] = &[
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

static LINTERS: &[LinterSpec] = &[
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

#[must_use]
pub const fn built_in_providers() -> &'static [ProviderSpec] {
    PROVIDERS
}

#[must_use]
pub const fn built_in_linters() -> &'static [LinterSpec] {
    LINTERS
}

#[must_use]
pub fn provider_spec(provider_id: ProviderId) -> Option<ProviderSpec> {
    PROVIDERS
        .iter()
        .copied()
        .find(|provider| provider.id == provider_id)
}

#[must_use]
pub fn linter_spec(kind: LintRunnerKind) -> Option<LinterSpec> {
    LINTERS.iter().copied().find(|linter| linter.kind == kind)
}

#[must_use]
pub fn executable_on_path(executable: &str) -> bool {
    if executable.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(executable).exists();
    }

    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|directory| directory.join(executable).exists())
}

#[must_use]
pub fn tool_available(executable: &str) -> bool {
    if executable == LintRunnerKind::Clippy.executable() {
        clippy_available(false)
    } else {
        executable_on_path(executable)
    }
}

#[must_use]
pub fn install_method_available(method: InstallMethod) -> bool {
    executable_on_path(method.as_str())
}

pub fn install_tool(label: &str, executable: &str, plan: InstallPlan) -> ToolOperationResult {
    let output = Command::new(plan.method.as_str())
        .args(plan.install_args)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let success = if executable == LintRunnerKind::Clippy.executable() {
                clippy_available(true)
            } else {
                tool_available(executable)
            };
            ToolOperationResult {
                install_source: Some(plan.method),
                success,
                message: if success {
                    format!("installed {label}")
                } else {
                    format!(
                        "{} finished, but {executable} is still not on PATH",
                        plan.method.as_str()
                    )
                },
            }
        }
        Ok(output) => ToolOperationResult {
            install_source: Some(plan.method),
            success: false,
            message: format!(
                "failed to install {label} via {}: {}",
                plan.method.as_str(),
                first_stderr_line(&output.stderr)
            ),
        },
        Err(error) => ToolOperationResult {
            install_source: Some(plan.method),
            success: false,
            message: format!("failed to start {label} installer: {error}"),
        },
    }
}

pub fn uninstall_tool(label: &str, plan: InstallPlan) -> ToolOperationResult {
    let install_source = Some(plan.method);
    match plan.uninstall {
        Uninstall::Command(args) => {
            let method = plan.method;
            let output = Command::new(method.as_str()).args(args).output();
            match output {
                Ok(output) if output.status.success() => {
                    if method == InstallMethod::Rustup {
                        *CLIPPY_AVAILABLE
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = false;
                    }
                    ToolOperationResult {
                        install_source,
                        success: true,
                        message: format!("removed {label}"),
                    }
                }
                Ok(output) => ToolOperationResult {
                    install_source,
                    success: false,
                    message: format!(
                        "failed to uninstall {label} via {}: {}",
                        method.as_str(),
                        first_stderr_line(&output.stderr)
                    ),
                },
                Err(error) => ToolOperationResult {
                    install_source,
                    success: false,
                    message: format!("failed to start {label} uninstall: {error}"),
                },
            }
        }
        Uninstall::GoBinary(binary) => {
            let result = remove_go_binary(binary);
            ToolOperationResult {
                install_source,
                success: result.is_ok(),
                message: result
                    .map(|()| format!("removed {label}"))
                    .unwrap_or_else(|error| format!("failed to remove {label}: {error}")),
            }
        }
        Uninstall::DisableOnly => ToolOperationResult {
            install_source,
            success: true,
            message: format!("removed {label} from Redox"),
        },
    }
}

fn clippy_available(refresh: bool) -> bool {
    let mut available = CLIPPY_AVAILABLE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Cache successful probes only, so missing tools can be discovered on a later check.
    if refresh || !*available {
        *available = Command::new("cargo")
            .args(["clippy", "--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    }
    *available
}

fn remove_go_binary(binary: &str) -> io::Result<()> {
    let gobin_output = Command::new("go").args(["env", "GOBIN"]).output()?;
    if !gobin_output.status.success() {
        return Err(io::Error::other(first_stderr_line(&gobin_output.stderr)));
    }
    let gobin = String::from_utf8_lossy(&gobin_output.stdout)
        .trim()
        .to_string();
    let target = if gobin.is_empty() {
        let gopath_output = Command::new("go").args(["env", "GOPATH"]).output()?;
        if !gopath_output.status.success() {
            return Err(io::Error::other(first_stderr_line(&gopath_output.stderr)));
        }
        PathBuf::from(String::from_utf8_lossy(&gopath_output.stdout).trim())
            .join("bin")
            .join(binary)
    } else {
        PathBuf::from(gobin).join(binary)
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
