//! Input mapping for `editor_tui`.
//!
//! This module translates raw MinUI events into mode-aware editor actions.
//! It also tracks normal-mode key prefixes used by `gg` and count motions.

use editor_core::motion::Motion;
use minui::prelude::*;

pub mod cursor;

/// Editor input mode.
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
    /// Bracketed paste payload.
    ///
    /// Treated as bulk text insertion in editable modes.
    Paste(String),

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

/// Map a MinUI `Event` to an `InputAction`, updating key-prefix state.
pub fn map_event_with_state(state: &mut InputState, mode: InputMode, event: &Event) -> InputAction {
    match event {
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

        Event::KeyWithModifiers(k) => map_key_with_state(state, mode, *k),

        Event::Character(c) => match mode {
            InputMode::Insert => InputAction::InsertChar(*c),
            InputMode::Command => InputAction::CommandChar(*c),
            InputMode::Normal => normal_char_action(*c),
        },

        _ => InputAction::None,
    }
}

fn normal_char_action(c: char) -> InputAction {
    match c {
        ':' => InputAction::EnterCommand,
        'i' => InputAction::EnterInsert(InsertKind::Insert),
        'a' => InputAction::EnterInsert(InsertKind::Append),
        _ => InputAction::None,
    }
}

fn map_key_with_state(
    state: &mut InputState,
    mode: InputMode,
    key: KeyWithModifiers,
) -> InputAction {
    let mods = key.mods;
    let key = key.key;

    match mode {
        InputMode::Insert => {
            return match key {
                KeyKind::Escape => InputAction::SetMode(InputMode::Normal),
                KeyKind::Backspace => InputAction::Backspace,
                KeyKind::Enter => InputAction::Enter,
                KeyKind::Tab => InputAction::InsertChar('\t'),

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

                KeyKind::Char(c) => InputAction::InsertChar(c),

                _ => InputAction::None,
            };
        }

        InputMode::Command => {
            return match key {
                KeyKind::Escape => InputAction::CommandCancel,
                KeyKind::Backspace => InputAction::CommandBackspace,
                KeyKind::Enter => InputAction::CommandEnter,
                KeyKind::Char(c) => InputAction::CommandChar(c),
                _ => InputAction::None,
            };
        }

        InputMode::Normal => {}
    }

    // Normal mode below.

    // Detect `I`/`A` via key modifiers so terminal character event shape does not matter.
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

    // Count prefix: leading zero has Vim-specific semantics; keep this simple for now.
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
            // TODO: map `{count}gg` to line `{count}` when that motion exists.
            return InputAction::Motion {
                motion: Motion::FileStart,
                count,
            };
        }
    }

    match key {
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

        _ => {
            state.reset_prefixes();
            InputAction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_mode_character_q_is_not_quit() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('q'));
        assert_eq!(action, InputAction::None);
    }

    #[test]
    fn normal_mode_character_colon_enters_command() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character(':'));
        assert_eq!(action, InputAction::EnterCommand);
    }

    #[test]
    fn normal_mode_shift_i_enters_insert_line_start() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('I'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(
            action,
            InputAction::EnterInsert(InsertKind::InsertLineStart)
        );
    }

    #[test]
    fn normal_mode_count_prefix_applies_to_motion() {
        let mut state = InputState::new();
        let _ = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('3'),
                mods: KeyModifiers::none(),
            }),
        );

        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('w'),
                mods: KeyModifiers::none(),
            }),
        );

        assert_eq!(
            action,
            InputAction::Motion {
                motion: Motion::WordStartAfter,
                count: 3
            }
        );
    }

    #[test]
    fn insert_mode_tab_key_inserts_tab_char() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Insert,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Tab,
                mods: KeyModifiers::none(),
            }),
        );
        assert_eq!(action, InputAction::InsertChar('\t'));
    }
}
