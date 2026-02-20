//! Application-layer module for `editor_tui`.
//!
//! This module owns editor-TUI specific orchestration state (mode, command line,
//! dirty flag, pending actions) while keeping terminal-agnostic editing logic in
//! `editor_core` and rendering in `editor_tui::ui`.

pub mod state;

pub use state::{EditorMode, EditorState};
