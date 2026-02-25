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

pub mod render;
pub mod style;
pub mod widgets;

// Re-export the render helpers/types so call sites can keep using `ui::...`.
pub use render::{snapshot_lines_wrapped_cached, GraphemeCache, TextViewport};
pub use style::{UiStyle, STATUS_BAR_HEIGHT_CELLS, STATUS_BAR_HEIGHT_ROWS};

// Re-export common widgets for convenience.
pub use widgets::{build_editor_status_bar, draw_explorer_popup_view, explorer_popup_inner_size};
