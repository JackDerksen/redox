//! Frontend-only editor widgets built on MinUI.

pub mod about;
pub mod command_line;
pub mod completion;
pub(crate) mod dashboard;
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
pub mod which_key;

pub(crate) use about::{about_popup_inner_size, draw_about_popup_view};
pub(crate) use command_line::{draw_command_line_popup, draw_command_line_popup_below};
pub(crate) use completion::{draw_completion_popup, draw_completion_preview};
pub(crate) use explorer::{draw_explorer_popup_view, explorer_popup_inner_size};
pub(crate) use finder::{draw_finder_popup, draw_pin_selector_popup};
pub(crate) use lsp::{
    build_symbol_info_source_lines, draw_code_actions_popup, draw_diagnostics_popup,
    draw_symbol_info_popup, symbol_info_content_width_limit, wrap_symbol_info_lines,
};
pub(crate) use marketplace::{draw_lsp_marketplace_popup, lsp_marketplace_popup_inner_size};
pub(crate) use pane::draw_pane_split_lines;
pub(crate) use perf::{draw_perf_popup_view, perf_popup_layout};
pub(crate) use status_bar::build_editor_status_bar;
pub(crate) use toast::draw_status_toast;
pub(crate) use undo_tree::{
    UNDO_TREE_HEADER_ROWS, draw_undo_tree_lines, draw_undo_tree_preview_lines,
};
pub(crate) use which_key::draw_which_key_popup;
