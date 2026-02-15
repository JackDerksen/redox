//! Input handling for `editor_tui`.
//!
//! This is located in the TUI crate because it interfaces with MinUI's event
//! system, rather than being pure editor logic. Any more complex logic required
//! for future motions will be implemented in the core crate.
//!
//! Over time, this module will grow to a comprehensive list implementing
//! all basic vim motions.

use minui::prelude::*;

/// High-level input intents the TUI understands.
///
/// This enum will stay small and stable, and variants will be added as needed
/// for more complex motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    /// Quit the application (temp).
    /// TODO: map this to a command later, once the command system is implemented.
    Quit,

    /// Scroll the viewport by a delta in cells (x) and visual rows (y).
    ScrollBy { dx: i32, dy: i32 },

    /// No action.
    None,
}

/// Map a MinUI [`Event`] to a TUI [`InputAction`].
///
/// Notes:
/// - For now this only handles `Event::KeyWithModifiers` plus legacy `Event::Character('q')`.
/// - Vim-like motions are intentionally minimal (hjkl + arrow keys).
/// - Ignoring modifiers for now; later this can grow into a real keymap.
pub fn map_event(event: &Event) -> InputAction {
    match event {
        // Prefer modifier-aware key model
        Event::KeyWithModifiers(k) => map_key(k.key),

        // Legacy convenience variant
        Event::Character('q') => InputAction::Quit,

        _ => InputAction::None,
    }
}

fn map_key(key: KeyKind) -> InputAction {
    match key {
        // Quit
        KeyKind::Char('q') => InputAction::Quit,

        // Arrow keys => scroll by one
        KeyKind::Up => InputAction::ScrollBy { dx: 0, dy: -1 },
        KeyKind::Down => InputAction::ScrollBy { dx: 0, dy: 1 },
        KeyKind::Left => InputAction::ScrollBy { dx: -1, dy: 0 },
        KeyKind::Right => InputAction::ScrollBy { dx: 1, dy: 0 },

        // Vim-ish => scroll by one
        KeyKind::Char('k') => InputAction::ScrollBy { dx: 0, dy: -1 },
        KeyKind::Char('j') => InputAction::ScrollBy { dx: 0, dy: 1 },
        KeyKind::Char('h') => InputAction::ScrollBy { dx: -1, dy: 0 },
        KeyKind::Char('l') => InputAction::ScrollBy { dx: 1, dy: 0 },

        _ => InputAction::None,
    }
}
