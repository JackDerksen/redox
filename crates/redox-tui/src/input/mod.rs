//! Input mapping for `redox-tui`.
//!
//! This module translates raw MinUI events into mode-aware editor actions.
//! It also tracks normal-mode key prefixes used by `gg` and count motions.

use minui::prelude::*;
use redox_core::motion::Motion;

pub mod cursor;

/// Editor input mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Insert,
    Command,
    Visual,
    VisualLine,
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
    /// Create a new line below current line and enter insert mode (`o`).
    OpenLineBelow,
    /// Create a new line above current line and enter insert mode (`O`).
    OpenLineAbove,

    /// Enter command mode (like Vim's `:`).
    EnterCommand,
    /// Open explorer surface (`<leader>e`).
    OpenExplorer,
    /// Open item under cursor in active surface (`Enter` in normal mode).
    SurfaceOpenSelected,
    /// Navigate to parent in active surface (`-` in normal mode).
    SurfaceGoParent,
    /// Yank active visual selection into Redox's private (local) register.
    YankSelectionPrivate,
    /// Delete (cut) active visual selection into Redox's private register.
    DeleteSelectionPrivate,
    /// Delete active visual selection without yanking.
    DeleteSelectionNoYank,
    /// Delete (cut) current line(s) into Redox's private register.
    DeleteCurrentLinePrivate {
        count: usize,
    },
    /// Yank active visual selection into system clipboard.
    YankSelectionSystem,
    /// Paste from Redox's private register.
    PastePrivateRegister,
    /// Paste from Redox's private register before cursor / above line.
    PastePrivateRegisterBefore,
    /// Delete character under cursor without yanking.
    DeleteCharNoYank,
    /// Move visual selection up by line(s).
    MoveVisualSelectionUp {
        count: usize,
    },
    /// Move visual selection down by line(s).
    MoveVisualSelectionDown {
        count: usize,
    },
    /// Indent all lines touched by active visual selection.
    IndentVisualSelection {
        count: usize,
    },
    /// Un-indent all lines touched by active visual selection.
    OutdentVisualSelection {
        count: usize,
    },

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
    pending_d: bool,
    pending_count: Option<usize>,
    pending_leader: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_prefixes(&mut self) {
        self.pending_g = false;
        self.pending_d = false;
        self.pending_count = None;
        self.pending_leader = false;
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
            InputMode::Visual | InputMode::VisualLine => InputAction::SetMode(InputMode::Normal),
            InputMode::Normal => InputAction::None,
        },

        Event::Backspace => match mode {
            InputMode::Insert => InputAction::Backspace,
            InputMode::Command => InputAction::CommandBackspace,
            InputMode::Visual | InputMode::VisualLine => InputAction::None,
            InputMode::Normal => InputAction::None,
        },

        Event::Enter => match mode {
            InputMode::Insert => InputAction::Enter,
            InputMode::Command => InputAction::CommandEnter,
            InputMode::Visual | InputMode::VisualLine => InputAction::None,
            InputMode::Normal => InputAction::SurfaceOpenSelected,
        },

        Event::KeyWithModifiers(k) => map_key_with_state(state, mode, *k),

        Event::Character(c) => match mode {
            InputMode::Insert => InputAction::InsertChar(*c),
            InputMode::Command => InputAction::CommandChar(*c),
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine => {
                modal_char_action(state, mode, *c)
            }
        },

        _ => InputAction::None,
    }
}

fn modal_char_action(state: &mut InputState, mode: InputMode, c: char) -> InputAction {
    if state.pending_leader {
        state.pending_leader = false;
        return match c {
            'e' => InputAction::OpenExplorer,
            'y' if matches!(mode, InputMode::Visual | InputMode::VisualLine) => {
                InputAction::YankSelectionSystem
            }
            _ => InputAction::None,
        };
    }

    if state.pending_d && mode == InputMode::Normal {
        state.pending_d = false;
        if c == 'd' {
            return InputAction::DeleteCurrentLinePrivate {
                count: state.take_count_or_1(),
            };
        }
    }

    match c {
        ' ' => {
            state.pending_leader = true;
            InputAction::None
        }
        'v' => {
            state.reset_prefixes();
            match mode {
                InputMode::Normal => InputAction::SetMode(InputMode::Visual),
                InputMode::Visual => InputAction::SetMode(InputMode::Normal),
                InputMode::VisualLine => InputAction::SetMode(InputMode::Visual),
                InputMode::Insert | InputMode::Command => InputAction::None,
            }
        }
        'V' => {
            state.reset_prefixes();
            match mode {
                InputMode::Normal => InputAction::SetMode(InputMode::VisualLine),
                InputMode::Visual => InputAction::SetMode(InputMode::VisualLine),
                InputMode::VisualLine => InputAction::SetMode(InputMode::Normal),
                InputMode::Insert | InputMode::Command => InputAction::None,
            }
        }
        'y' if matches!(mode, InputMode::Visual | InputMode::VisualLine) => {
            state.reset_prefixes();
            InputAction::YankSelectionPrivate
        }
        'd' if matches!(mode, InputMode::Visual | InputMode::VisualLine) => {
            state.reset_prefixes();
            InputAction::DeleteSelectionPrivate
        }
        'x' if matches!(mode, InputMode::Visual | InputMode::VisualLine) => {
            state.reset_prefixes();
            InputAction::DeleteSelectionNoYank
        }
        'd' if mode == InputMode::Normal => {
            state.pending_d = true;
            InputAction::None
        }
        'x' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::DeleteCharNoYank
        }
        'p' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::PastePrivateRegister
        }
        'P' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::PastePrivateRegisterBefore
        }
        'J' if matches!(mode, InputMode::Visual | InputMode::VisualLine) => {
            InputAction::MoveVisualSelectionDown {
                count: state.take_count_or_1(),
            }
        }
        'K' if matches!(mode, InputMode::Visual | InputMode::VisualLine) => {
            InputAction::MoveVisualSelectionUp {
                count: state.take_count_or_1(),
            }
        }
        '\t' if matches!(mode, InputMode::Visual | InputMode::VisualLine) => {
            InputAction::IndentVisualSelection {
                count: state.take_count_or_1(),
            }
        }
        ':' => InputAction::EnterCommand,
        'i' if mode == InputMode::Normal => InputAction::EnterInsert(InsertKind::Insert),
        'a' if mode == InputMode::Normal => InputAction::EnterInsert(InsertKind::Append),
        'o' if mode == InputMode::Normal => InputAction::OpenLineBelow,
        'O' if mode == InputMode::Normal => InputAction::OpenLineAbove,
        '-' if mode == InputMode::Normal => InputAction::SurfaceGoParent,
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

        InputMode::Normal | InputMode::Visual | InputMode::VisualLine => {}
    }

    // Modal (normal/visual) handling below.

    if state.pending_leader {
        state.pending_leader = false;
        match key {
            KeyKind::Char('e') => return InputAction::OpenExplorer,
            KeyKind::Char('y') if matches!(mode, InputMode::Visual | InputMode::VisualLine) => {
                return InputAction::YankSelectionSystem;
            }
            _ => {}
        }
        return InputAction::None;
    }

    // Detect `I`/`A` via key modifiers so terminal character event shape does not matter.
    if mods.shift {
        if let KeyKind::Char('I') = key {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::EnterInsert(InsertKind::InsertLineStart);
        }
        if let KeyKind::Char('A') = key {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::EnterInsert(InsertKind::AppendLineEnd);
        }
        if let KeyKind::Char('O') = key {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::OpenLineAbove;
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

    if state.pending_d && mode == InputMode::Normal {
        state.pending_d = false;
        if matches!(key, KeyKind::Char('d')) {
            return InputAction::DeleteCurrentLinePrivate {
                count: state.take_count_or_1(),
            };
        }
    }

    match key {
        KeyKind::Escape => {
            state.reset_prefixes();
            if matches!(mode, InputMode::Visual | InputMode::VisualLine) {
                InputAction::SetMode(InputMode::Normal)
            } else {
                InputAction::None
            }
        }
        KeyKind::Enter => {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::SurfaceOpenSelected
        }

        KeyKind::Char(' ') => {
            state.pending_leader = true;
            InputAction::None
        }
        KeyKind::Char('v') => {
            state.reset_prefixes();
            match mode {
                InputMode::Normal => InputAction::SetMode(InputMode::Visual),
                InputMode::Visual => InputAction::SetMode(InputMode::Normal),
                InputMode::VisualLine => InputAction::SetMode(InputMode::Visual),
                InputMode::Insert | InputMode::Command => InputAction::None,
            }
        }
        KeyKind::Char('V') => {
            state.reset_prefixes();
            match mode {
                InputMode::Normal => InputAction::SetMode(InputMode::VisualLine),
                InputMode::Visual => InputAction::SetMode(InputMode::VisualLine),
                InputMode::VisualLine => InputAction::SetMode(InputMode::Normal),
                InputMode::Insert | InputMode::Command => InputAction::None,
            }
        }
        KeyKind::Char('y') => {
            if !matches!(mode, InputMode::Visual | InputMode::VisualLine) {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::YankSelectionPrivate
        }
        KeyKind::Char('d') => {
            if matches!(mode, InputMode::Visual | InputMode::VisualLine) {
                state.reset_prefixes();
                return InputAction::DeleteSelectionPrivate;
            }
            if mode == InputMode::Normal {
                state.pending_d = true;
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::None
        }
        KeyKind::Char('x') => {
            if matches!(mode, InputMode::Visual | InputMode::VisualLine) {
                state.reset_prefixes();
                return InputAction::DeleteSelectionNoYank;
            }
            if mode == InputMode::Normal {
                state.reset_prefixes();
                return InputAction::DeleteCharNoYank;
            }
            state.reset_prefixes();
            InputAction::None
        }
        KeyKind::Char('p') => {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::PastePrivateRegister
        }
        KeyKind::Char('P') => {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::PastePrivateRegisterBefore
        }
        KeyKind::Char('J') => {
            if !matches!(mode, InputMode::Visual | InputMode::VisualLine) {
                state.reset_prefixes();
                return InputAction::None;
            }
            InputAction::MoveVisualSelectionDown {
                count: state.take_count_or_1(),
            }
        }
        KeyKind::Char('K') => {
            if !matches!(mode, InputMode::Visual | InputMode::VisualLine) {
                state.reset_prefixes();
                return InputAction::None;
            }
            InputAction::MoveVisualSelectionUp {
                count: state.take_count_or_1(),
            }
        }
        KeyKind::Tab => {
            if !matches!(mode, InputMode::Visual | InputMode::VisualLine) {
                state.reset_prefixes();
                return InputAction::None;
            }
            if mods.shift {
                InputAction::OutdentVisualSelection {
                    count: state.take_count_or_1(),
                }
            } else {
                InputAction::IndentVisualSelection {
                    count: state.take_count_or_1(),
                }
            }
        }

        // Enter modes
        KeyKind::Char('i') => {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::Insert)
        }
        KeyKind::Char('a') => {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::Append)
        }
        KeyKind::Char('o') => {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::OpenLineBelow
        }
        KeyKind::Char('O') => {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::OpenLineAbove
        }
        KeyKind::Char(':') => {
            state.reset_prefixes();
            InputAction::EnterCommand
        }
        KeyKind::Char('-') => {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::SurfaceGoParent
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

    #[test]
    fn normal_mode_leader_e_opens_explorer() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character(' '));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('e'));
        assert_eq!(action, InputAction::OpenExplorer);
    }

    #[test]
    fn normal_mode_enter_opens_surface_selected() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Enter);
        assert_eq!(action, InputAction::SurfaceOpenSelected);
    }

    #[test]
    fn normal_mode_dash_goes_parent_in_surface() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('-'));
        assert_eq!(action, InputAction::SurfaceGoParent);
    }

    #[test]
    fn normal_mode_key_with_modifiers_enter_opens_surface_selected() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Enter,
                mods: KeyModifiers::none(),
            }),
        );
        assert_eq!(action, InputAction::SurfaceOpenSelected);
    }

    #[test]
    fn normal_mode_o_opens_line_below() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('o'));
        assert_eq!(action, InputAction::OpenLineBelow);
    }

    #[test]
    fn normal_mode_shift_o_opens_line_above() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('O'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(action, InputAction::OpenLineAbove);
    }

    #[test]
    fn normal_mode_v_enters_visual_mode() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('v'));
        assert_eq!(action, InputAction::SetMode(InputMode::Visual));
    }

    #[test]
    fn normal_mode_shift_v_enters_visual_line_mode() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('V'));
        assert_eq!(action, InputAction::SetMode(InputMode::VisualLine));
    }

    #[test]
    fn normal_mode_shift_v_key_event_enters_visual_line_mode() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('V'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::VisualLine));
    }

    #[test]
    fn visual_escape_returns_to_normal() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Escape);
        assert_eq!(action, InputAction::SetMode(InputMode::Normal));
    }

    #[test]
    fn visual_escape_key_with_modifiers_returns_to_normal() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Visual,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Escape,
                mods: KeyModifiers::none(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::Normal));
    }

    #[test]
    fn visual_mode_y_yanks_to_private_register() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('y'));
        assert_eq!(action, InputAction::YankSelectionPrivate);
    }

    #[test]
    fn visual_mode_d_deletes_to_private_register() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('d'));
        assert_eq!(action, InputAction::DeleteSelectionPrivate);
    }

    #[test]
    fn normal_mode_x_deletes_char_without_yank() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(action, InputAction::DeleteCharNoYank);
    }

    #[test]
    fn visual_mode_x_deletes_selection_without_yank() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('x'));
        assert_eq!(action, InputAction::DeleteSelectionNoYank);
    }

    #[test]
    fn normal_mode_dd_deletes_current_line_to_private_register() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        assert_eq!(action, InputAction::DeleteCurrentLinePrivate { count: 1 });
    }

    #[test]
    fn normal_mode_shift_p_pastes_private_register_before() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('P'));
        assert_eq!(action, InputAction::PastePrivateRegisterBefore);
    }

    #[test]
    fn visual_mode_shift_j_and_shift_k_move_selection() {
        let mut state = InputState::new();
        let down = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('J'));
        assert_eq!(down, InputAction::MoveVisualSelectionDown { count: 1 });
        let up = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('K'));
        assert_eq!(up, InputAction::MoveVisualSelectionUp { count: 1 });
    }

    #[test]
    fn visual_mode_tab_and_shift_tab_indent_and_outdent() {
        let mut state = InputState::new();
        let indent = map_event_with_state(
            &mut state,
            InputMode::Visual,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Tab,
                mods: KeyModifiers::none(),
            }),
        );
        assert_eq!(indent, InputAction::IndentVisualSelection { count: 1 });

        let outdent = map_event_with_state(
            &mut state,
            InputMode::Visual,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Tab,
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(outdent, InputAction::OutdentVisualSelection { count: 1 });
    }

    #[test]
    fn visual_mode_leader_y_yanks_to_system_clipboard() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Visual, &Event::Character(' '));
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('y'));
        assert_eq!(action, InputAction::YankSelectionSystem);
    }

    #[test]
    fn normal_mode_p_pastes_private_register() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('p'));
        assert_eq!(action, InputAction::PastePrivateRegister);
    }
}
