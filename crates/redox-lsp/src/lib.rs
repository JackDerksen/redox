//! Reusable Language Server Protocol support for Redox frontends.
//!
//! This crate owns JSON-RPC transport, protocol decoding, URI and workspace
//! conversion, snippet expansion, built-in provider metadata, and linter
//! processes. Frontends retain interaction policy and presentation state.

pub mod code_action;
pub mod completion;
pub mod diagnostics;
pub mod hover;
pub mod lint;
pub mod protocol;
pub mod provider;
pub mod snippet;
pub mod transport;
pub mod workspace;

pub use code_action::{
    AvailableCodeAction, DocumentEdit, LspCommand, TextEdit, WorkspaceEdit,
    parse_code_action_response, parse_workspace_edit,
};
pub use completion::{
    CompletionCandidate, CompletionDefaults, CompletionTextEdit, InsertTextFormat,
    parse_completion_response,
};
pub use diagnostics::{
    Diagnostic, DiagnosticLocation, DiagnosticRelatedInformation, DiagnosticSeverity,
    parse_publish_diagnostics,
};
pub use hover::{SymbolInfoBlock, SymbolInfoKind, parse_hover_response};
pub use lint::{LintRunResult, LintRunnerKind, LintSource, lint_runner_available, run_linter};
pub use protocol::{DefinitionTarget, Position, Range, parse_definition_response};
pub use provider::{
    InstallMethod, InstallPlan, Language, LanguageServerProvider, LinterSpec, ProviderId,
    ProviderSpec, ToolOperationResult, Uninstall, built_in_linters, built_in_providers,
    install_method_available, install_tool, linter_spec, provider_spec, tool_available,
    uninstall_tool,
};
pub use snippet::{SnippetExpansion, SnippetPlaceholder, completion_snippet_expansion, expand};
pub use transport::{
    Client, ClientEvent, ClientInfo, ServerCommand, TransportError, TransportOptions,
    default_initialize_params,
};
pub use workspace::{default_workspace_root, file_path_from_uri, file_uri, workspace_root_for};
