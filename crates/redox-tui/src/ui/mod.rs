//! Frontend-only rendering, styling, and widgets.

pub mod helpers;
pub mod icons;
pub mod overlays;
pub mod rain_animation;
pub mod render;
pub mod style;
pub mod syntax;
pub mod widgets;

pub use rain_animation::RainAnimation;
pub use render::{RenderLineCache, TextViewport};
pub use style::{STATUS_BAR_HEIGHT_CELLS, STATUS_BAR_HEIGHT_ROWS, UiStyle};
pub use syntax::{SyntaxHighlighter, language_for_path};

pub use widgets::{
    UNDO_TREE_HEADER_ROWS, about_popup_inner_size, build_editor_status_bar,
    build_symbol_info_display_lines, draw_about_popup_view, draw_code_actions_popup,
    draw_command_line_popup, draw_command_line_popup_below, draw_completion_popup,
    draw_completion_preview, draw_diagnostics_popup, draw_explorer_popup_view, draw_finder_popup,
    draw_lsp_marketplace_popup, draw_pane_split_lines, draw_perf_popup_view,
    draw_pin_selector_popup, draw_status_toast, draw_symbol_info_popup, draw_undo_tree_lines,
    draw_undo_tree_preview_lines, draw_which_key_popup, explorer_popup_inner_size,
    lsp_marketplace_popup_inner_size, perf_popup_layout, symbol_info_content_width_limit,
};
