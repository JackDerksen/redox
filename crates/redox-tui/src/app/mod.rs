//! TUI orchestration between editor state, core mechanisms, and rendering.

pub mod state;

pub use state::{
    AboutPopup, CompletionEntry, CompletionPopup, DiagnosticLine, DiagnosticSeverity, EditorMode,
    EditorState, ExplorerPopup, FinderPopup, FinderPreview, FramePerfSample, FramePerfStats,
    GitDiffSnapshot, GitFileStatusKind, GitGutterKind, LspEntryStatusKind, LspMarketplacePopup,
    PaneRect, PerfPopup, PinSelectorPopup, StatusMessageStyle, UndoTreeSurfaceRole,
};
