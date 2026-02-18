//! Input handling for `editor_tui`.
//!
//! This module is intentionally TUI-specific: it translates MinUI events into
//! higher-level editor intents, expressed as `editor_core::motion::Motion` plus
//! a Vim-style count.
//!
//! Supported (currently):
//! - Arrow keys / `hjkl`: basic cursor motions (count partially working, see below)
//! - `w`: word forward
//! - `e`: word end
//! - `b`: word backward
//! - `gg`: file start
//! - `G`: file end
//! - `q`: quit
//!
//! Notes:
//! - This file maintains a tiny key-sequence state machine for `gg`.
//! - Counts (e.g. `3w`) are scaffolded but only partially wired. It's easy to
//!   extend by accumulating digits and applying them to the next motion.

use editor_core::motion::Motion;
use minui::prelude::*;

pub mod cursor;

/// High-level input intents the TUI understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    /// Quit the application.
    Quit,

    /// Apply a document motion (UI-agnostic) with a Vim-style count.
    ///
    /// `count` is always >= 1.
    Motion { motion: Motion, count: usize },

    /// No action.
    None,
}

/// Small state machine for multi-key sequences (eg. `gg`) and counts.
#[derive(Debug, Default, Clone)]
pub struct InputState {
    pending_g: bool,
    pending_count: Option<usize>,
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_prefixes(&mut self) {
        self.pending_g = false;
        self.pending_count = None;
    }

    fn push_count_digit(&mut self, d: u8) {
        debug_assert!(d <= 9);
        let current = self.pending_count.unwrap_or(0);
        let next = current.saturating_mul(10).saturating_add(d as usize);
        self.pending_count = Some(next);
    }

    fn take_count_or_1(&mut self) -> usize {
        match self.pending_count.take() {
            Some(0) | None => 1,
            Some(n) => n,
        }
    }
}

/// Map a MinUI `Event` to a TUI `InputAction`, updating the key-sequence state.
///
/// This is the preferred app update loop entry point.
pub fn map_event_with_state(state: &mut InputState, event: &Event) -> InputAction {
    match event {
        Event::KeyWithModifiers(k) => map_key_with_state(state, *k),
        Event::Character('q') => InputAction::Quit,
        _ => InputAction::None,
    }
}

fn map_key_with_state(state: &mut InputState, key: KeyWithModifiers) -> InputAction {
    // Inspect `key.mods` here later for special keys like `Ctrl` and `Alt`.
    let key = key.key;

    // Count prefix: accumulate digits (Vim: leading 0 is special, but for now treat any digit as count)
    if let KeyKind::Char(c) = key {
        if let Some(d) = c.to_digit(10) {
            state.push_count_digit(d as u8);
            return InputAction::None;
        }
    }

    // Handle `gg` sequence.
    if state.pending_g {
        state.pending_g = false;

        if matches!(key, KeyKind::Char('g')) {
            let count = state.take_count_or_1();
            // NOTE: In real Vim, `{count}gg` goes to line {count}. I don't have that motion yet,
            // so `count` currently just repeats FileStart (no effect beyond 1).
            return InputAction::Motion {
                motion: Motion::FileStart,
                count,
            };
        }

        // If it wasn't a second `g`, fall through and treat the new key normally.
        // Intentionally keep the count prefix for the next key.
    }

    match key {
        // Quit (will instead be set as a command later, for obvious reasons)
        KeyKind::Char('q') => {
            state.reset_prefixes();
            InputAction::Quit
        }

        // Start `g` sequence
        KeyKind::Char('g') => {
            state.pending_g = true;
            InputAction::None
        }

        // Motions with optional counts
        KeyKind::Up | KeyKind::Char('k') => InputAction::Motion {
            motion: Motion::Up,
            count: state.take_count_or_1(),
        },
        KeyKind::Down | KeyKind::Char('j') => InputAction::Motion {
            motion: Motion::Down,
            count: state.take_count_or_1(),
        },
        KeyKind::Left | KeyKind::Char('h') => InputAction::Motion {
            motion: Motion::Left,
            count: state.take_count_or_1(),
        },
        KeyKind::Right | KeyKind::Char('l') => InputAction::Motion {
            motion: Motion::Right,
            count: state.take_count_or_1(),
        },

        KeyKind::Char('w') => InputAction::Motion {
            motion: Motion::WordStartAfter,
            count: state.take_count_or_1(),
        },

        KeyKind::Char('b') => InputAction::Motion {
            motion: Motion::WordStartBefore,
            count: state.take_count_or_1(),
        },

        KeyKind::Char('e') => InputAction::Motion {
            motion: Motion::WordEndAfter,
            count: state.take_count_or_1(),
        },

        KeyKind::Char('G') => InputAction::Motion {
            motion: Motion::FileEnd,
            count: state.take_count_or_1(),
        },

        // Unknown key: clear pending `g` but keep count? In Vim, an unused count is consumed by
        // the next command; clear it here to avoid surprising behaviour.
        _ => {
            state.reset_prefixes();
            InputAction::None
        }
    }
}
