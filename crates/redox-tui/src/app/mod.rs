//! Application-layer module for `redox-tui`.
//!
//! This module owns editor-TUI specific orchestration state (mode, command line,
//! dirty flag, pending actions) while keeping terminal-agnostic editing logic in
//! `redox-core` and rendering in `redox-tui::ui`.

pub mod state;

pub use state::{
    AboutPopup, CompletionEntry, CompletionPopup, DiagnosticLine, DiagnosticSeverity, EditorMode,
    EditorState, ExplorerPopup, FinderPopup, FinderPreview, FramePerfSample, FramePerfStats,
    GitDiffSnapshot, GitFileStatusKind, GitGutterKind, LspEntryStatusKind, LspMarketplacePopup,
    PerfPopup, PinSelectorPopup, StatusMessageStyle,
};
