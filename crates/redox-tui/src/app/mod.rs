//! Application-layer module for `redox-tui`.
//!
//! This module owns editor-TUI specific orchestration state (mode, command line,
//! dirty flag, pending actions) while keeping terminal-agnostic editing logic in
//! `redox-core` and rendering in `redox-tui::ui`.

pub mod state;

pub use state::{AboutPopup, EditorMode, EditorState, ExplorerPopup};
