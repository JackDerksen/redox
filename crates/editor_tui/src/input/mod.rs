//! Input handling for `editor_tui`.
//!
//! This module is intentionally TUI-specific: it translates MinUI events into
//! higher-level editor intents (now mode-aware for normal, insert, and command modes).
//!
//! Supported (currently):
//! - Normal mode:
//!   - Arrow keys / `hjkl`: basic cursor motions (count partially working, see below)
//!   - `w`: word forward
//!   - `e`: word end
//!   - `b`: word backward
//!   - `gg`: file start
//!   - `G`: file end
//!   - `i` / `a`: enter Insert mode (insert/append)
//!     - As well as `I` / `A` for line start/end
//!   - `:`: enter Command mode
//!   - `q`: quit (temporary, will become `:q` later)
//! - Insert mode:
//!   - Arrow keys: cursor motion (no `hjkl` because those should type, obviously)
//!   - `Esc`: return to Normal mode
//!   - text input is forwarded as `InsertChar` / `InsertText`
//!   - Backspace / Enter are forwarded as actions
//! - Command mode:
//!   - `Esc`: cancel and return to Normal mode
//!   - characters build a command line buffer (execution is handled elsewhere)
//!
//! Notes:
//! - This file maintains a tiny key-sequence state machine for `gg`.
//! - Counts (e.g. `3w`) are scaffolded but only partially wired.

use editor_core::motion::Motion;
use minui::prelude::*;

pub mod cursor;

/// Editor input mode (Vim-like).
/// I'll be adding visual mode later!
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Insert,
    Command,
}

/// How to enter insert mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertKind {
    /// `i`: insert at cursor
    Insert,

    /// `a`: append after cursor
    Append,

    /// `I`: insert at beginning of line
    InsertLineStart,

    /// `A`: append at end of line
    AppendLineEnd,
}

/// High-level input intents the TUI understands.
///
/// These are *mode-aware*; the main editor loop decides how to apply them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    /// Quit the application.
    Quit,

    /// Apply a document motion (UI-agnostic) with a Vim-style count.
    ///
    /// `count` is always >= 1.
    Motion {
        motion: Motion,
        count: usize,
    },

    /// Switch editor mode.
    SetMode(InputMode),

    /// Enter insert mode (with Vim-like `i` / `a` semantics).
    EnterInsert(InsertKind),

    /// Enter command mode (like Vim's `:`).
    EnterCommand,

    /// Command-line editing actions (buffer is owned by editor state).
    CommandChar(char),
    CommandBackspace,
    CommandEnter,
    CommandCancel,

    /// Insert/editing actions.
    InsertChar(char),
    InsertText(String),
    Backspace,
    Enter,

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
/// Callers must pass the current `mode` so mapping can be mode-aware.
pub fn map_event_with_state(state: &mut InputState, mode: InputMode, event: &Event) -> InputAction {
    match event {
        // Prefer the specific key events for these special keys.
        Event::Escape => match mode {
            InputMode::Insert => InputAction::SetMode(InputMode::Normal),
            InputMode::Command => InputAction::CommandCancel,
            InputMode::Normal => InputAction::None,
        },

        Event::Backspace => match mode {
            InputMode::Insert => InputAction::Backspace,
            InputMode::Command => InputAction::CommandBackspace,
            InputMode::Normal => InputAction::None,
        },

        Event::Enter => match mode {
            InputMode::Insert => InputAction::Enter,
            InputMode::Command => InputAction::CommandEnter,
            InputMode::Normal => InputAction::None,
        },

        // Key events (arrows, etc).
        Event::KeyWithModifiers(k) => map_key_with_state(state, mode, *k),

        // Text input.
        Event::Character(c) => match mode {
            InputMode::Insert => InputAction::InsertChar(*c),
            InputMode::Command => InputAction::CommandChar(*c),
            InputMode::Normal => {
                if *c == 'q' {
                    InputAction::Quit
                } else if *c == ':' {
                    InputAction::EnterCommand
                } else if *c == 'i' {
                    InputAction::EnterInsert(InsertKind::Insert)
                } else if *c == 'a' {
                    InputAction::EnterInsert(InsertKind::Append)
                } else {
                    // Most normal-mode character commands are handled via KeyWithModifiers -> KeyKind::Char.
                    // Fall back to None so we don't accidentally consume input twice.
                    InputAction::None
                }
            }
        },

        _ => InputAction::None,
    }
}

fn map_key_with_state(
    state: &mut InputState,
    mode: InputMode,
    key: KeyWithModifiers,
) -> InputAction {
    // Inspect `key.mods` here later for special keys like `Ctrl` and `Alt`.
    let mods = key.mods;
    let key = key.key;

    match mode {
        InputMode::Insert => {
            // Insert mode: arrow keys move; everything else should generally be typed via `Event::Character`.
            // (We still handle Escape/Backspace/Enter via both Event variants and KeyKind variants.)
            return match key {
                KeyKind::Escape => InputAction::SetMode(InputMode::Normal),
                KeyKind::Backspace => InputAction::Backspace,
                KeyKind::Enter => InputAction::Enter,

                KeyKind::Up => InputAction::Motion {
                    motion: Motion::Up,
                    count: 1,
                },
                KeyKind::Down => InputAction::Motion {
                    motion: Motion::Down,
                    count: 1,
                },
                KeyKind::Left => InputAction::Motion {
                    motion: Motion::Left,
                    count: 1,
                },
                KeyKind::Right => InputAction::Motion {
                    motion: Motion::Right,
                    count: 1,
                },

                _ => InputAction::None,
            };
        }

        InputMode::Command => {
            return match key {
                KeyKind::Escape => InputAction::CommandCancel,
                KeyKind::Backspace => InputAction::CommandBackspace,
                KeyKind::Enter => InputAction::CommandEnter,
                _ => InputAction::None,
            };
        }

        InputMode::Normal => {}
    }

    // Normal mode below.

    // Shift-modified insert commands:
    // - `I`: insert at beginning of line (first non-whitespace in Vim, but for now just BOL)
    // - `A`: append at end of line
    //
    // NOTE: These are detected via modifiers so we don't rely on `Event::Character` emitting uppercase.
    if mods.shift {
        if let KeyKind::Char('I') = key {
            state.reset_prefixes();
            return InputAction::EnterInsert(InsertKind::InsertLineStart);
        }
        if let KeyKind::Char('A') = key {
            state.reset_prefixes();
            return InputAction::EnterInsert(InsertKind::AppendLineEnd);
        }
    }

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
        // Quit (temporary; will become `:q` later).
        KeyKind::Char('q') => {
            state.reset_prefixes();
            InputAction::Quit
        }

        // Enter modes
        KeyKind::Char('i') => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::Insert)
        }
        KeyKind::Char('a') => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::Append)
        }
        KeyKind::Char(':') => {
            state.reset_prefixes();
            InputAction::EnterCommand
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
