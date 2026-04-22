//! Editor-specific UI widgets for `redox-tui`.
//!
//! These widgets are built directly on top of MinUI's `Window` trait and are
//! intended to stay frontend-only (no `redox-core` dependencies).

pub mod about;
pub mod command_line;
pub mod explorer;
pub mod perf;
pub mod popup;
pub mod status_bar;
pub mod toast;

pub use about::{about_popup_inner_size, draw_about_popup_view};
pub use command_line::draw_command_line_popup;
pub use explorer::{draw_explorer_popup_view, explorer_popup_inner_size};
pub use perf::{draw_perf_popup_view, perf_popup_layout, perf_popup_occludes_cursor};
pub use status_bar::build_editor_status_bar;
pub use toast::{draw_status_toast, status_toast_occludes_cursor};
