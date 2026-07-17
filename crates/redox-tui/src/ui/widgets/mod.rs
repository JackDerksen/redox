//! Editor-specific UI widgets for `redox-tui`.
//!
//! These widgets are built directly on top of MinUI's `Window` trait and are
//! intended to stay frontend-only (no `redox-core` dependencies).

pub mod about;
pub mod command_line;
pub mod completion;
pub mod explorer;
pub mod finder;
pub mod lsp;
pub mod marketplace;
pub mod pane;
pub mod perf;
pub mod popup;
pub mod status_bar;
pub mod toast;
pub mod undo_tree;

pub use about::{about_popup_inner_size, draw_about_popup_view};
pub use command_line::{draw_command_line_popup, draw_command_line_popup_below};
pub use completion::{draw_completion_popup, draw_completion_preview};
pub use explorer::{draw_explorer_popup_view, explorer_popup_inner_size};
pub use finder::{draw_finder_popup, draw_pin_selector_popup};
pub use lsp::{
    build_symbol_info_display_lines, draw_code_actions_popup, draw_diagnostics_popup,
    draw_symbol_info_popup, symbol_info_content_width_limit,
};
pub use marketplace::{draw_lsp_marketplace_popup, lsp_marketplace_popup_inner_size};
pub use pane::draw_pane_split_lines;
pub use perf::{draw_perf_popup_view, perf_popup_layout};
pub use status_bar::build_editor_status_bar;
pub use toast::draw_status_toast;
pub use undo_tree::{draw_undo_tree_lines, draw_undo_tree_preview_lines};
