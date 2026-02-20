//! UI module for `editor_tui`.
//!
//! This module is intentionally UI-only and should not leak into `editor_core`.
//!
//! Refactor note:
//! - Rendering helpers previously lived directly in `ui/mod.rs`.
//! - They have been moved to `ui/render.rs`.
//! - Editor-specific widgets live under `ui/widgets/`.
//!
//! Keep this module as the stable public surface for UI utilities.

pub mod render;
pub mod widgets;

// Re-export the render helpers/types so call sites can keep using `ui::...`.
pub use render::{GraphemeCache, TextViewport, draw_snapshot, snapshot_lines_wrapped_cached};

// Re-export common widgets for convenience.
pub use widgets::{Align, EditorStatusBar, Segment};
