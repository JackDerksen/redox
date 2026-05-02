//! UI module for `redox-tui`.
//!
//! This module is intentionally UI-only and should not leak into `redox-core`.
//!
//! Refactor note:
//! - Rendering helpers previously lived directly in `ui/mod.rs`.
//! - They have been moved to `ui/render.rs`.
//! - Editor-specific widgets live under `ui/widgets/`.
//!
//! Keep this module as the stable public surface for UI utilities.

pub mod helpers;
pub mod overlays;
pub mod rain_animation;
pub mod render;
pub mod style;
pub mod syntax;
pub mod widgets;

// Re-export the render helpers/types so call sites can keep using `ui::...`.
pub use rain_animation::RainAnimation;
pub use render::{GraphemeCache, TextViewport, snapshot_lines_wrapped_cached};
pub use style::{STATUS_BAR_HEIGHT_CELLS, STATUS_BAR_HEIGHT_ROWS, UiStyle};
pub use syntax::{SyntaxHighlighter, language_for_path};

// Re-export common widgets for convenience.
pub use widgets::{
    about_popup_inner_size, build_editor_status_bar, draw_about_popup_view,
    draw_command_line_popup, draw_diagnostics_popup, draw_explorer_popup_view, draw_finder_popup,
    draw_lsp_marketplace_popup, draw_perf_popup_view, draw_pin_selector_popup, draw_status_toast,
    explorer_popup_inner_size, perf_popup_layout, perf_popup_occludes_cursor,
    status_toast_occludes_cursor,
};
