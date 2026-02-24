//! Editor-specific UI widgets for `editor_tui`.
//!
//! These widgets are built directly on top of MinUI's `Window` trait and are
//! intended to stay frontend-only (no `editor_core` dependencies).

pub mod explorer;
pub mod status_bar;

pub use explorer::draw_explorer_popup_view;
pub use status_bar::build_editor_status_bar;
