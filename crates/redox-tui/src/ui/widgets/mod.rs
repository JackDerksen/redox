//! Editor-specific UI widgets for `redox-tui`.
//!
//! These widgets are built directly on top of MinUI's `Window` trait and are
//! intended to stay frontend-only (no `redox-core` dependencies).

pub mod about;
pub mod explorer;
pub mod popup;
pub mod status_bar;

pub use about::{about_popup_inner_size, draw_about_popup_view};
pub use explorer::{draw_explorer_popup_view, explorer_popup_inner_size};
pub use status_bar::build_editor_status_bar;
