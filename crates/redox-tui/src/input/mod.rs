//! Input mapping for `redox-tui`.
//!
//! This module translates raw MinUI events into mode-aware editor actions.
//! It also tracks count prefixes and a small command tree for multi-key motions.

use minui::prelude::input::{Event, KeyKind, KeyModifiers, KeyWithModifiers};
use redox_core::{motion::Motion, DelimiterKind, TextObjectKind, TextObjectScope, TextObjectSpec};

pub mod cursor;

/// Editor input mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Insert,
    Command,
    Search,
    Visual,
    VisualLine,
    VisualBlock,
}

/// How to enter insert mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertKind {
    /// `i`: insert at cursor
    Insert,

    /// `a`: append after cursor
    Append,

    /// `I`: insert at the first non-whitespace character on the line
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
    /// Enter slash-search mode.
    EnterSearch,
    /// Open explorer surface (`<leader>e`).
    OpenExplorer,
    /// Open item under cursor in active surface (`Enter` in normal mode).
    SurfaceOpenSelected,
    /// Navigate to parent in active surface (`-` in normal mode).
    SurfaceGoParent,
    /// Scroll down by one viewport and keep cursor near viewport middle (`Ctrl+D` in normal mode).
    ViewportDownCenter,
    /// Scroll up by one viewport and keep cursor near viewport middle (`Ctrl+U` in normal mode).
    ViewportUpCenter,
    /// Centre current cursor line in viewport (`zz` in normal mode).
    CenterCursorLine,
    /// Undo most recent edit in active buffer (`u` in normal mode).
    Undo,
    /// Redo most recently undone edit in active buffer (`Ctrl+R` in normal mode).
    Redo,
    /// Confirm a pending explorer deletion prompt (`y` in normal mode).
    ConfirmExplorerDelete,
    /// Yank active visual selection into Redox's private (local) register.
    YankSelectionPrivate,
    /// Delete (cut) active visual selection into Redox's private register.
    DeleteSelectionPrivate,
    /// Change active visual selection and enter insert mode.
    ChangeSelectionPrivate,
    /// Delete active visual selection without yanking.
    DeleteSelectionNoYank,
    /// Apply an operator to a resolved target at the current cursor.
    OperateTarget {
        operator: TextObjectOperator,
        target: OperatorTarget,
    },
    /// Delete (cut) current line(s) into Redox's private register.
    DeleteCurrentLinePrivate {
        count: usize,
    },
    /// Yank current line(s) into Redox's private register.
    YankCurrentLinePrivate {
        count: usize,
    },
    /// Change current line(s) into Redox's private register and enter insert mode.
    ChangeCurrentLinePrivate {
        count: usize,
    },
    /// Yank active visual selection into system clipboard.
    YankSelectionSystem,
    /// Paste from system clipboard.
    PasteSystemClipboard,
    /// Paste concrete text fetched from the system clipboard.
    PasteSystemClipboardText(String),
    /// Paste from Redox's private register.
    PastePrivateRegister,
    /// Paste from Redox's private register before cursor / above line.
    PastePrivateRegisterBefore,
    /// Delete character under cursor without yanking.
    DeleteCharNoYank,
    /// Toggle the case of character(s) under the cursor.
    ToggleCase {
        count: usize,
    },
    /// Replace the character under cursor, or the active visual selection, with a character.
    ReplaceChar(char),
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

    /// Search-line editing actions.
    SearchChar(char),
    SearchBackspace,
    SearchEnter,
    SearchCancel,

    /// Repeat the most recent cached search.
    RepeatSearch {
        forward: bool,
    },
    /// Hide active search highlights while keeping the cached search term.
    ClearSearch,

    /// Insert/editing actions.
    InsertChar(char),
    Backspace,
    Enter,

    /// No action.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorTarget {
    Motion { motion: Motion, count: usize },
    TextObject(TextObjectSpec),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectOperator {
    Delete,
    Change,
    Select,
    Yank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefixFallback {
    Consume,
    RetryCurrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SequenceAction {
    OpenExplorer,
    YankSelectionSystem,
    PasteSystemClipboard,
    FileStart,
    CenterCursorLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SequenceBinding {
    sequence: &'static str,
    fallback: PrefixFallback,
    action: Option<SequenceAction>,
}

const COMMON_SEQUENCE_BINDINGS: &[SequenceBinding] = &[
    SequenceBinding {
        sequence: " ",
        fallback: PrefixFallback::Consume,
        action: None,
    },
    SequenceBinding {
        sequence: " e",
        fallback: PrefixFallback::Consume,
        action: Some(SequenceAction::OpenExplorer),
    },
    SequenceBinding {
        sequence: "g",
        fallback: PrefixFallback::RetryCurrent,
        action: None,
    },
    SequenceBinding {
        sequence: "gg",
        fallback: PrefixFallback::RetryCurrent,
        action: Some(SequenceAction::FileStart),
    },
];

const NORMAL_SEQUENCE_BINDINGS: &[SequenceBinding] = &[
    SequenceBinding {
        sequence: " p",
        fallback: PrefixFallback::Consume,
        action: Some(SequenceAction::PasteSystemClipboard),
    },
    SequenceBinding {
        sequence: "z",
        fallback: PrefixFallback::RetryCurrent,
        action: None,
    },
    SequenceBinding {
        sequence: "zz",
        fallback: PrefixFallback::RetryCurrent,
        action: Some(SequenceAction::CenterCursorLine),
    },
];

const VISUAL_SEQUENCE_BINDINGS: &[SequenceBinding] = &[SequenceBinding {
    sequence: " y",
    fallback: PrefixFallback::Consume,
    action: Some(SequenceAction::YankSelectionSystem),
}];

/// Small state machine for multi-key sequences (eg. `gg`) and counts.
#[derive(Debug, Default, Clone)]
pub struct InputState {
    pending_sequence: String,
    pending_count: Option<usize>,
    pending_operator: Option<PendingOperator>,
    pending_search_motion: Option<PendingSearchMotion>,
    pending_replace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingOperator {
    operator: TextObjectOperator,
    count: usize,
    scope: Option<TextObjectScope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingSearchMotion {
    operator: Option<PendingOperator>,
    count: usize,
    kind: SearchMotionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchMotionKind {
    Find,
    Till,
    FindBefore,
    TillBefore,
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_prefixes(&mut self) {
        self.pending_sequence.clear();
        self.pending_count = None;
        self.pending_operator = None;
        self.pending_search_motion = None;
        self.pending_replace = false;
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

    fn push_sequence_char(&mut self, c: char) {
        self.pending_sequence.push(c);
    }

    fn clear_sequence(&mut self) {
        self.pending_sequence.clear();
    }

    fn begin_replace(&mut self) {
        self.pending_sequence.clear();
        self.pending_count = None;
        self.pending_operator = None;
        self.pending_search_motion = None;
        self.pending_replace = true;
    }

    fn begin_search_motion(
        &mut self,
        operator: Option<PendingOperator>,
        kind: SearchMotionKind,
        count: usize,
    ) {
        self.pending_sequence.clear();
        self.pending_count = None;
        self.pending_operator = None;
        self.pending_replace = false;
        self.pending_search_motion = Some(PendingSearchMotion {
            operator,
            count: count.max(1),
            kind,
        });
    }
}

/// Map a MinUI `Event` to an `InputAction`, updating key-prefix state.
#[cfg_attr(not(test), allow(dead_code))]
pub fn map_event_with_state(state: &mut InputState, mode: InputMode, event: &Event) -> InputAction {
    map_event_with_context(state, mode, false, event)
}

pub fn map_event_with_context(
    state: &mut InputState,
    mode: InputMode,
    confirm_explorer_delete: bool,
    event: &Event,
) -> InputAction {
    match event {
        Event::Escape => {
            state.reset_prefixes();
            match mode {
                InputMode::Insert => InputAction::SetMode(InputMode::Normal),
                InputMode::Command => InputAction::CommandCancel,
                InputMode::Search => InputAction::SearchCancel,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {
                    InputAction::SetMode(InputMode::Normal)
                }
                InputMode::Normal => InputAction::ClearSearch,
            }
        }

        Event::Backspace => {
            state.reset_prefixes();
            match mode {
                InputMode::Insert => InputAction::Backspace,
                InputMode::Command => InputAction::CommandBackspace,
                InputMode::Search => InputAction::SearchBackspace,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {
                    InputAction::None
                }
                InputMode::Normal => InputAction::None,
            }
        }

        Event::Enter => {
            state.reset_prefixes();
            match mode {
                InputMode::Insert => InputAction::Enter,
                InputMode::Command => InputAction::CommandEnter,
                InputMode::Search => InputAction::SearchEnter,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {
                    InputAction::None
                }
                InputMode::Normal => InputAction::SurfaceOpenSelected,
            }
        }

        Event::KeyWithModifiers(k) => map_key_with_state(state, mode, confirm_explorer_delete, *k),

        Event::Character(c) => match mode {
            InputMode::Insert => InputAction::InsertChar(*c),
            InputMode::Command => InputAction::CommandChar(*c),
            InputMode::Search => InputAction::SearchChar(*c),
            InputMode::Normal
            | InputMode::Visual
            | InputMode::VisualLine
            | InputMode::VisualBlock => modal_char_action(state, mode, confirm_explorer_delete, *c),
        },

        _ => InputAction::None,
    }
}

fn modal_char_action(
    state: &mut InputState,
    mode: InputMode,
    confirm_explorer_delete: bool,
    c: char,
) -> InputAction {
    if state.pending_replace {
        state.reset_prefixes();
        return InputAction::ReplaceChar(c);
    }

    if let Some(pending_search_motion) = state.pending_search_motion {
        return resolve_pending_search_motion(state, c, pending_search_motion);
    }

    if let Some(operator) = state.pending_operator {
        if let Some(action) = resolve_pending_operator(state, c, operator) {
            return action;
        }
    }

    if !state.pending_sequence.is_empty() {
        if let Some(action) = resolve_pending_sequence(state, mode, c) {
            return action;
        }
    }

    if let Some(d) = c.to_digit(10) {
        if d == 0 && state.pending_count.is_none() {
            state.reset_prefixes();
            return InputAction::Motion {
                motion: Motion::LineStart,
                count: 1,
            };
        }
        state.push_count_digit(d as u8);
        return InputAction::None;
    }

    match c {
        'v' => {
            state.reset_prefixes();
            match mode {
                InputMode::Normal => InputAction::SetMode(InputMode::Visual),
                InputMode::Visual => InputAction::SetMode(InputMode::Normal),
                InputMode::VisualLine => InputAction::SetMode(InputMode::Visual),
                InputMode::VisualBlock => InputAction::SetMode(InputMode::Visual),
                InputMode::Insert | InputMode::Command | InputMode::Search => InputAction::None,
            }
        }
        'V' => {
            state.reset_prefixes();
            match mode {
                InputMode::Normal => InputAction::SetMode(InputMode::VisualLine),
                InputMode::Visual => InputAction::SetMode(InputMode::VisualLine),
                InputMode::VisualBlock => InputAction::SetMode(InputMode::VisualLine),
                InputMode::VisualLine => InputAction::SetMode(InputMode::Normal),
                InputMode::Insert | InputMode::Command | InputMode::Search => InputAction::None,
            }
        }
        'y' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.reset_prefixes();
            InputAction::YankSelectionPrivate
        }
        'd' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.reset_prefixes();
            InputAction::DeleteSelectionPrivate
        }
        'd' if mode == InputMode::Normal => {
            state.pending_operator = Some(PendingOperator {
                operator: TextObjectOperator::Delete,
                count: state.take_count_or_1(),
                scope: None,
            });
            InputAction::None
        }
        'y' if mode == InputMode::Normal && confirm_explorer_delete => {
            state.reset_prefixes();
            InputAction::ConfirmExplorerDelete
        }
        'y' if mode == InputMode::Normal => {
            state.pending_operator = Some(PendingOperator {
                operator: TextObjectOperator::Yank,
                count: state.take_count_or_1(),
                scope: None,
            });
            InputAction::None
        }
        'i' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.pending_operator = Some(PendingOperator {
                operator: TextObjectOperator::Select,
                count: state.take_count_or_1(),
                scope: Some(TextObjectScope::Inner),
            });
            InputAction::None
        }
        'a' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.pending_operator = Some(PendingOperator {
                operator: TextObjectOperator::Select,
                count: state.take_count_or_1(),
                scope: Some(TextObjectScope::Around),
            });
            InputAction::None
        }
        'c' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.reset_prefixes();
            InputAction::ChangeSelectionPrivate
        }
        'c' if mode == InputMode::Normal => {
            state.pending_operator = Some(PendingOperator {
                operator: TextObjectOperator::Change,
                count: state.take_count_or_1(),
                scope: None,
            });
            InputAction::None
        }
        'x' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.reset_prefixes();
            InputAction::DeleteSelectionNoYank
        }
        'x' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::DeleteCharNoYank
        }
        '~' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            InputAction::ToggleCase {
                count: state.take_count_or_1(),
            }
        }
        'f' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            let count = state.take_count_or_1();
            state.begin_search_motion(None, SearchMotionKind::Find, count);
            InputAction::None
        }
        't' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            let count = state.take_count_or_1();
            state.begin_search_motion(None, SearchMotionKind::Till, count);
            InputAction::None
        }
        'F' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            let count = state.take_count_or_1();
            state.begin_search_motion(None, SearchMotionKind::FindBefore, count);
            InputAction::None
        }
        'T' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            let count = state.take_count_or_1();
            state.begin_search_motion(None, SearchMotionKind::TillBefore, count);
            InputAction::None
        }
        'r' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.begin_replace();
            InputAction::None
        }
        'p' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::PastePrivateRegister
        }
        'P' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::PastePrivateRegisterBefore
        }
        'J' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            InputAction::MoveVisualSelectionDown {
                count: state.take_count_or_1(),
            }
        }
        'K' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            InputAction::MoveVisualSelectionUp {
                count: state.take_count_or_1(),
            }
        }
        '\t' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            InputAction::IndentVisualSelection {
                count: state.take_count_or_1(),
            }
        }
        ':' => {
            state.reset_prefixes();
            InputAction::EnterCommand
        }
        '/' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterSearch
        }
        'i' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::Insert)
        }
        'a' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::Append)
        }
        'I' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::InsertLineStart)
        }
        'A' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::AppendLineEnd)
        }
        'D' if mode == InputMode::Normal => {
            let count = state.take_count_or_1();
            operate_target_action(
                state,
                TextObjectOperator::Delete,
                OperatorTarget::Motion {
                    motion: Motion::LineEnd,
                    count,
                },
            )
        }
        'o' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::OpenLineBelow
        }
        'O' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::OpenLineAbove
        }
        '-' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::SurfaceGoParent
        }
        prefix if starts_sequence(mode, prefix) => {
            state.push_sequence_char(prefix);
            InputAction::None
        }
        'h' => InputAction::Motion {
            motion: Motion::Left,
            count: state.take_count_or_1(),
        },
        'j' => InputAction::Motion {
            motion: Motion::Down,
            count: state.take_count_or_1(),
        },
        'k' => InputAction::Motion {
            motion: Motion::Up,
            count: state.take_count_or_1(),
        },
        'l' => InputAction::Motion {
            motion: Motion::Right,
            count: state.take_count_or_1(),
        },
        'w' => InputAction::Motion {
            motion: Motion::WordStartAfter,
            count: state.take_count_or_1(),
        },
        'b' => InputAction::Motion {
            motion: Motion::WordStartBefore,
            count: state.take_count_or_1(),
        },
        'e' => InputAction::Motion {
            motion: Motion::WordEndAfter,
            count: state.take_count_or_1(),
        },
        '_' => InputAction::Motion {
            motion: Motion::LineFirstNonWhitespace,
            count: state.take_count_or_1(),
        },
        '%' => InputAction::Motion {
            motion: Motion::MatchDelimiter,
            count: state.take_count_or_1(),
        },
        '$' => InputAction::Motion {
            motion: Motion::LineEnd,
            count: state.take_count_or_1(),
        },
        'G' => InputAction::Motion {
            motion: Motion::FileEnd,
            count: state.take_count_or_1(),
        },
        'u' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::Undo
        }
        _ => {
            state.reset_prefixes();
            InputAction::None
        }
    }
}

fn resolve_pending_operator(
    state: &mut InputState,
    c: char,
    pending: PendingOperator,
) -> Option<InputAction> {
    let action = match (pending.operator, pending.scope, c) {
        (TextObjectOperator::Delete, None, 'd') => Some(InputAction::DeleteCurrentLinePrivate {
            count: pending.count,
        }),
        (TextObjectOperator::Yank, None, 'y') => Some(InputAction::YankCurrentLinePrivate {
            count: pending.count,
        }),
        (TextObjectOperator::Change, None, 'c') => Some(InputAction::ChangeCurrentLinePrivate {
            count: pending.count,
        }),
        (operator, None, '0') => Some(InputAction::OperateTarget {
            operator,
            target: OperatorTarget::Motion {
                motion: Motion::LineStart,
                count: pending.count,
            },
        }),
        (operator, None, '_') => Some(InputAction::OperateTarget {
            operator,
            target: OperatorTarget::Motion {
                motion: Motion::LineFirstNonWhitespace,
                count: pending.count,
            },
        }),
        (operator, None, '$') => Some(InputAction::OperateTarget {
            operator,
            target: OperatorTarget::Motion {
                motion: Motion::LineEnd,
                count: pending.count,
            },
        }),
        (_, None, 'f') => {
            state.begin_search_motion(Some(pending), SearchMotionKind::Find, pending.count);
            Some(InputAction::None)
        }
        (_, None, 't') => {
            state.begin_search_motion(Some(pending), SearchMotionKind::Till, pending.count);
            Some(InputAction::None)
        }
        (_, None, 'F') => {
            state.begin_search_motion(Some(pending), SearchMotionKind::FindBefore, pending.count);
            Some(InputAction::None)
        }
        (_, None, 'T') => {
            state.begin_search_motion(Some(pending), SearchMotionKind::TillBefore, pending.count);
            Some(InputAction::None)
        }
        (_, None, 'i') => {
            state.pending_operator = Some(PendingOperator {
                scope: Some(TextObjectScope::Inner),
                ..pending
            });
            Some(InputAction::None)
        }
        (_, None, 'a') => {
            state.pending_operator = Some(PendingOperator {
                scope: Some(TextObjectScope::Around),
                ..pending
            });
            Some(InputAction::None)
        }
        (_, Some(scope), object) => {
            text_object_kind_from_char(object).map(|kind| InputAction::OperateTarget {
                operator: pending.operator,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope,
                    kind,
                    count: pending.count,
                }),
            })
        }
        (_, None, motion_char) => {
            if let Some(motion) = operator_motion_from_input(state, motion_char) {
                Some(InputAction::OperateTarget {
                    operator: pending.operator,
                    target: OperatorTarget::Motion {
                        motion,
                        count: pending.count,
                    },
                })
            } else if operator_motion_sequence_has_children(state, motion_char) {
                state.push_sequence_char(motion_char);
                Some(InputAction::None)
            } else {
                None
            }
        }
    };

    let keep_pending_operator = matches!(action, Some(InputAction::None))
        && (matches!(
            (pending.operator, pending.scope, c),
            (_, None, 'i' | 'a' | 'f' | 't')
        ) || !state.pending_sequence.is_empty());

    if !matches!(action, Some(InputAction::None)) {
        state.pending_operator = None;
    } else if !keep_pending_operator {
        state.pending_operator = None;
    }

    action
}

// TODO: This is kind of a gross temporary fix. In the future, I'll need to
// refactor basically the entire motion handling system into something like
// a shared "key/char to motion-or-sequence" translation layer and then let
// both plain motions and operator-pending motions consume that same result.
fn operator_motion_from_input(state: &mut InputState, c: char) -> Option<Motion> {
    if !state.pending_sequence.is_empty() {
        let mut candidate = state.pending_sequence.clone();
        candidate.push(c);
        if let Some(motion) = motion_from_sequence(&candidate) {
            state.clear_sequence();
            return Some(motion);
        }
    }

    motion_from_char(c)
}

fn operator_motion_sequence_has_children(state: &InputState, c: char) -> bool {
    let mut candidate = state.pending_sequence.clone();
    candidate.push(c);
    motion_sequence_has_children(&candidate)
}

fn motion_from_char(c: char) -> Option<Motion> {
    match c {
        'h' => Some(Motion::Left),
        'j' => Some(Motion::Down),
        'k' => Some(Motion::Up),
        'l' => Some(Motion::Right),
        'w' => Some(Motion::WordStartAfter),
        'b' => Some(Motion::WordStartBefore),
        'e' => Some(Motion::WordEndAfter),
        '_' => Some(Motion::LineFirstNonWhitespace),
        '%' => Some(Motion::MatchDelimiter),
        '$' => Some(Motion::LineEnd),
        'G' => Some(Motion::FileEnd),
        _ => None,
    }
}

fn motion_from_sequence(sequence: &str) -> Option<Motion> {
    match sequence {
        "gg" => Some(Motion::FileStart),
        _ => None,
    }
}

fn motion_sequence_has_children(candidate: &str) -> bool {
    ["gg"]
        .into_iter()
        .any(|sequence| sequence.starts_with(candidate) && sequence.len() > candidate.len())
}

fn resolve_pending_search_motion(
    state: &mut InputState,
    c: char,
    pending: PendingSearchMotion,
) -> InputAction {
    state.pending_search_motion = None;
    let motion = match pending.kind {
        SearchMotionKind::Find => Motion::FindChar(c),
        SearchMotionKind::Till => Motion::TillChar(c),
        SearchMotionKind::FindBefore => Motion::FindCharBefore(c),
        SearchMotionKind::TillBefore => Motion::TillCharBefore(c),
    };

    if let Some(operator) = pending.operator {
        InputAction::OperateTarget {
            operator: operator.operator,
            target: OperatorTarget::Motion {
                motion,
                count: pending.count,
            },
        }
    } else {
        InputAction::Motion {
            motion,
            count: pending.count,
        }
    }
}

fn text_object_kind_from_char(c: char) -> Option<TextObjectKind> {
    match c {
        'w' => Some(TextObjectKind::Word),
        'W' => Some(TextObjectKind::BigWord),
        'p' => Some(TextObjectKind::Paragraph),
        '(' | ')' | 'b' => Some(TextObjectKind::Delimiter(DelimiterKind::Parentheses)),
        '[' | ']' => Some(TextObjectKind::Delimiter(DelimiterKind::Brackets)),
        '{' | '}' | 'B' => Some(TextObjectKind::Delimiter(DelimiterKind::Braces)),
        '\'' => Some(TextObjectKind::Delimiter(DelimiterKind::SingleQuotes)),
        '"' => Some(TextObjectKind::Delimiter(DelimiterKind::DoubleQuotes)),
        '`' => Some(TextObjectKind::Delimiter(DelimiterKind::Backticks)),
        _ => None,
    }
}

fn operate_target_action(
    state: &mut InputState,
    operator: TextObjectOperator,
    target: OperatorTarget,
) -> InputAction {
    state.reset_prefixes();
    InputAction::OperateTarget { operator, target }
}

fn sequence_bindings_for_mode(mode: InputMode) -> impl Iterator<Item = &'static SequenceBinding> {
    COMMON_SEQUENCE_BINDINGS.iter().chain(match mode {
        InputMode::Normal => NORMAL_SEQUENCE_BINDINGS.iter(),
        InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {
            VISUAL_SEQUENCE_BINDINGS.iter()
        }
        InputMode::Insert | InputMode::Command | InputMode::Search => [].iter(),
    })
}

fn starts_sequence(mode: InputMode, c: char) -> bool {
    sequence_bindings_for_mode(mode)
        .any(|binding| binding.sequence.len() > 1 && binding.sequence.starts_with(c))
}

fn resolve_pending_sequence(
    state: &mut InputState,
    mode: InputMode,
    c: char,
) -> Option<InputAction> {
    let mut candidate = state.pending_sequence.clone();
    candidate.push(c);

    let exact = sequence_bindings_for_mode(mode).find(|binding| binding.sequence == candidate);
    let has_children = sequence_bindings_for_mode(mode).any(|binding| {
        binding.sequence.starts_with(&candidate) && binding.sequence.len() > candidate.len()
    });

    if let Some(binding) = exact {
        state.clear_sequence();
        if has_children && binding.action.is_none() {
            state.pending_sequence = candidate;
            return Some(InputAction::None);
        }
        return Some(sequence_binding_action(state, binding));
    }

    if has_children {
        state.pending_sequence = candidate;
        return Some(InputAction::None);
    }

    let fallback = sequence_bindings_for_mode(mode)
        .find(|binding| binding.sequence == state.pending_sequence)
        .map(|binding| binding.fallback)
        .unwrap_or(PrefixFallback::Consume);
    state.clear_sequence();
    if fallback == PrefixFallback::Consume {
        state.pending_count = None;
        Some(InputAction::None)
    } else {
        None
    }
}

fn sequence_binding_action(state: &mut InputState, binding: &SequenceBinding) -> InputAction {
    match binding.action {
        Some(SequenceAction::OpenExplorer) => {
            state.reset_prefixes();
            InputAction::OpenExplorer
        }
        Some(SequenceAction::YankSelectionSystem) => {
            state.reset_prefixes();
            InputAction::YankSelectionSystem
        }
        Some(SequenceAction::PasteSystemClipboard) => {
            state.reset_prefixes();
            InputAction::PasteSystemClipboard
        }
        Some(SequenceAction::FileStart) => {
            let _ = state.take_count_or_1();
            InputAction::Motion {
                motion: Motion::FileStart,
                count: 1,
            }
        }
        Some(SequenceAction::CenterCursorLine) => {
            state.reset_prefixes();
            InputAction::CenterCursorLine
        }
        None => InputAction::None,
    }
}

fn map_key_with_state(
    state: &mut InputState,
    mode: InputMode,
    confirm_explorer_delete: bool,
    key: KeyWithModifiers,
) -> InputAction {
    let mods = key.mods;
    let key = key.key;

    match mode {
        InputMode::Insert => {
            if mods.ctrl && matches!(key, KeyKind::Char('c') | KeyKind::Char('C')) {
                state.reset_prefixes();
                return InputAction::SetMode(InputMode::Normal);
            }

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
            if mods.ctrl && matches!(key, KeyKind::Char('c') | KeyKind::Char('C')) {
                state.reset_prefixes();
                return InputAction::CommandCancel;
            }

            return match key {
                KeyKind::Escape => InputAction::CommandCancel,
                KeyKind::Backspace => InputAction::CommandBackspace,
                KeyKind::Enter => InputAction::CommandEnter,
                KeyKind::Char(c) => InputAction::CommandChar(c),
                _ => InputAction::None,
            };
        }

        InputMode::Search => {
            if mods.ctrl && matches!(key, KeyKind::Char('c') | KeyKind::Char('C')) {
                state.reset_prefixes();
                return InputAction::SearchCancel;
            }

            return match key {
                KeyKind::Escape => InputAction::SearchCancel,
                KeyKind::Backspace => InputAction::SearchBackspace,
                KeyKind::Enter => InputAction::SearchEnter,
                KeyKind::Tab => InputAction::SearchChar('\t'),
                KeyKind::Char(c) => InputAction::SearchChar(c),
                _ => InputAction::None,
            };
        }

        InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {}
    }

    if let Some(pending_search_motion) = state.pending_search_motion {
        match key {
            KeyKind::Escape => {
                state.reset_prefixes();
                return if matches!(
                    mode,
                    InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
                ) {
                    InputAction::SetMode(InputMode::Normal)
                } else {
                    InputAction::None
                };
            }
            KeyKind::Tab => {
                state.pending_search_motion = None;
                return resolve_pending_search_motion(state, '\t', pending_search_motion);
            }
            KeyKind::Char(c) => {
                state.pending_search_motion = None;
                return resolve_pending_search_motion(
                    state,
                    replacement_char_from_key(c, mods),
                    pending_search_motion,
                );
            }
            _ => {
                state.reset_prefixes();
                return InputAction::None;
            }
        }
    }

    if state.pending_replace {
        match key {
            KeyKind::Escape => {
                state.reset_prefixes();
                return if matches!(
                    mode,
                    InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
                ) {
                    InputAction::SetMode(InputMode::Normal)
                } else {
                    InputAction::None
                };
            }
            KeyKind::Tab => {
                state.reset_prefixes();
                return InputAction::ReplaceChar('\t');
            }
            KeyKind::Char(c) => {
                state.reset_prefixes();
                return InputAction::ReplaceChar(replacement_char_from_key(c, mods));
            }
            _ => {
                state.reset_prefixes();
                return InputAction::None;
            }
        }
    }

    if mode == InputMode::Normal
        && mods.ctrl
        && matches!(key, KeyKind::Char('r') | KeyKind::Char('R'))
    {
        state.reset_prefixes();
        return InputAction::Redo;
    }

    if mode == InputMode::Normal
        && mods.ctrl
        && matches!(key, KeyKind::Char('d') | KeyKind::Char('D'))
    {
        state.reset_prefixes();
        return InputAction::ViewportDownCenter;
    }

    if mode == InputMode::Normal
        && mods.ctrl
        && matches!(key, KeyKind::Char('u') | KeyKind::Char('U'))
    {
        state.reset_prefixes();
        return InputAction::ViewportUpCenter;
    }

    if matches!(
        mode,
        InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
    ) && mods.ctrl
        && matches!(key, KeyKind::Char('n') | KeyKind::Char('N'))
    {
        state.reset_prefixes();
        return InputAction::RepeatSearch { forward: true };
    }

    if matches!(
        mode,
        InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
    ) && mods.ctrl
        && matches!(key, KeyKind::Char('p') | KeyKind::Char('P'))
    {
        state.reset_prefixes();
        return InputAction::RepeatSearch { forward: false };
    }

    if matches!(
        mode,
        InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
    ) && mods.ctrl
        && matches!(key, KeyKind::Char('c') | KeyKind::Char('C'))
    {
        state.reset_prefixes();
        return InputAction::SetMode(InputMode::Normal);
    }

    if mods.ctrl && matches!(key, KeyKind::Char('v') | KeyKind::Char('V')) {
        state.reset_prefixes();
        return match mode {
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine => {
                InputAction::SetMode(InputMode::VisualBlock)
            }
            InputMode::VisualBlock => InputAction::SetMode(InputMode::Normal),
            InputMode::Insert | InputMode::Command | InputMode::Search => InputAction::None,
        };
    }

    // Detect `I`, `A`, etc. via key modifiers so terminal character event shape does not matter.
    if mods.shift {
        if matches!(key, KeyKind::Char('I') | KeyKind::Char('i')) {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::EnterInsert(InsertKind::InsertLineStart);
        }
        if matches!(key, KeyKind::Char('A') | KeyKind::Char('a')) {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::EnterInsert(InsertKind::AppendLineEnd);
        }
        if matches!(key, KeyKind::Char('O') | KeyKind::Char('o')) {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::OpenLineAbove;
        }
        if matches!(key, KeyKind::Char('D') | KeyKind::Char('d')) {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            let count = state.take_count_or_1();
            state.reset_prefixes();
            return InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::LineEnd,
                    count,
                },
            };
        }
        if matches!(key, KeyKind::Char('V') | KeyKind::Char('v')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'V');
        }
        if matches!(key, KeyKind::Char('P') | KeyKind::Char('p')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'P');
        }
        if matches!(key, KeyKind::Char('J') | KeyKind::Char('j')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'J');
        }
        if matches!(key, KeyKind::Char('K') | KeyKind::Char('k')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'K');
        }
        if matches!(key, KeyKind::Char('G') | KeyKind::Char('g')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'G');
        }
        if matches!(key, KeyKind::Char('F') | KeyKind::Char('f')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'F');
        }
        if matches!(key, KeyKind::Char('T') | KeyKind::Char('t')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'T');
        }
    }

    match key {
        KeyKind::Escape => {
            state.reset_prefixes();
            if matches!(
                mode,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
            ) {
                InputAction::SetMode(InputMode::Normal)
            } else if mode == InputMode::Normal {
                InputAction::ClearSearch
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
        KeyKind::Tab => {
            if !matches!(
                mode,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
            ) {
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
        KeyKind::Up => InputAction::Motion {
            motion: Motion::Up,
            count: state.take_count_or_1(),
        },
        KeyKind::Down => InputAction::Motion {
            motion: Motion::Down,
            count: state.take_count_or_1(),
        },
        KeyKind::Left => InputAction::Motion {
            motion: Motion::Left,
            count: state.take_count_or_1(),
        },
        KeyKind::Right => InputAction::Motion {
            motion: Motion::Right,
            count: state.take_count_or_1(),
        },
        KeyKind::Char(c) => modal_char_action(state, mode, confirm_explorer_delete, c),
        _ => {
            state.reset_prefixes();
            InputAction::None
        }
    }
}

fn replacement_char_from_key(c: char, mods: KeyModifiers) -> char {
    if mods.shift && c.is_ascii_lowercase() {
        c.to_ascii_uppercase()
    } else {
        c
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
    fn normal_mode_zero_moves_to_real_line_start() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('0'));
        assert_eq!(
            action,
            InputAction::Motion {
                motion: Motion::LineStart,
                count: 1,
            }
        );
    }

    #[test]
    fn normal_mode_underscore_and_dollar_map_to_line_motions() {
        let mut state = InputState::new();
        let underscore =
            map_event_with_state(&mut state, InputMode::Normal, &Event::Character('_'));
        assert_eq!(
            underscore,
            InputAction::Motion {
                motion: Motion::LineFirstNonWhitespace,
                count: 1,
            }
        );

        let dollar = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('$'));
        assert_eq!(
            dollar,
            InputAction::Motion {
                motion: Motion::LineEnd,
                count: 1,
            }
        );
    }

    #[test]
    fn normal_mode_percent_maps_to_match_delimiter_motion() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('%'));
        assert_eq!(
            action,
            InputAction::Motion {
                motion: Motion::MatchDelimiter,
                count: 1,
            }
        );
    }

    #[test]
    fn normal_mode_percent_after_delete_operator_maps_to_motion_operator() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('%'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::MatchDelimiter,
                    count: 1,
                },
            }
        );
    }

    #[test]
    fn normal_mode_dollar_after_delete_operator_maps_to_motion_operator() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('$'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::LineEnd,
                    count: 1,
                },
            }
        );
    }

    #[test]
    fn normal_mode_w_after_delete_operator_maps_to_motion_operator() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('w'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::WordStartAfter,
                    count: 1,
                },
            }
        );
    }

    #[test]
    fn normal_mode_gg_after_delete_operator_maps_to_motion_operator() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let first = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('g'));
        let second = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('g'));
        assert_eq!(first, InputAction::None);
        assert_eq!(
            second,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::FileStart,
                    count: 1,
                },
            }
        );
    }

    #[test]
    fn normal_mode_zero_after_delete_operator_maps_to_motion_operator() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('0'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::LineStart,
                    count: 1,
                },
            }
        );
    }

    #[test]
    fn normal_mode_shift_d_aliases_delete_to_line_end() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('D'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::LineEnd,
                    count: 1,
                },
            }
        );
    }

    #[test]
    fn normal_mode_shift_key_d_aliases_delete_to_line_end() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('d'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::LineEnd,
                    count: 1,
                },
            }
        );
    }

    #[test]
    fn normal_mode_shift_key_d_preserves_count_prefix() {
        let mut state = InputState::new();
        let _ = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('2'),
                mods: KeyModifiers::none(),
            }),
        );
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('d'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::LineEnd,
                    count: 2,
                },
            }
        );
    }

    #[test]
    fn normal_mode_r_consumes_next_character_as_replacement() {
        let mut state = InputState::new();
        let first = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('r'));
        let second = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(first, InputAction::None);
        assert_eq!(second, InputAction::ReplaceChar('x'));
    }

    #[test]
    fn normal_mode_pending_replace_accepts_shifted_letters() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('r'));
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('d'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(action, InputAction::ReplaceChar('D'));
    }

    #[test]
    fn normal_mode_pending_replace_accepts_shifted_symbols() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('r'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('$'));
        assert_eq!(action, InputAction::ReplaceChar('$'));
    }

    #[test]
    fn normal_mode_pending_replace_accepts_tab() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('r'));
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Tab,
                mods: KeyModifiers::none(),
            }),
        );
        assert_eq!(action, InputAction::ReplaceChar('\t'));
    }

    #[test]
    fn normal_mode_raw_non_character_events_clear_pending_replace() {
        let mut escape_state = InputState::new();
        let _ = map_event_with_state(&mut escape_state, InputMode::Normal, &Event::Character('r'));
        let escape = map_event_with_state(&mut escape_state, InputMode::Normal, &Event::Escape);
        let after_escape =
            map_event_with_state(&mut escape_state, InputMode::Normal, &Event::Character('q'));
        assert_eq!(escape, InputAction::ClearSearch);
        assert_eq!(after_escape, InputAction::None);

        let mut backspace_state = InputState::new();
        let _ = map_event_with_state(
            &mut backspace_state,
            InputMode::Normal,
            &Event::Character('r'),
        );
        let backspace =
            map_event_with_state(&mut backspace_state, InputMode::Normal, &Event::Backspace);
        let after_backspace = map_event_with_state(
            &mut backspace_state,
            InputMode::Normal,
            &Event::Character('q'),
        );
        assert_eq!(backspace, InputAction::None);
        assert_eq!(after_backspace, InputAction::None);

        let mut enter_state = InputState::new();
        let _ = map_event_with_state(&mut enter_state, InputMode::Normal, &Event::Character('r'));
        let enter = map_event_with_state(&mut enter_state, InputMode::Normal, &Event::Enter);
        let after_enter =
            map_event_with_state(&mut enter_state, InputMode::Normal, &Event::Character('q'));
        assert_eq!(enter, InputAction::SurfaceOpenSelected);
        assert_eq!(after_enter, InputAction::None);
    }

    #[test]
    fn normal_mode_f_and_t_consume_target_character() {
        let mut state = InputState::new();
        let f = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('f'));
        let f_target = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(f, InputAction::None);
        assert_eq!(
            f_target,
            InputAction::Motion {
                motion: Motion::FindChar('x'),
                count: 1,
            }
        );

        let mut state = InputState::new();
        let t = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('t'));
        let t_target = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(t, InputAction::None);
        assert_eq!(
            t_target,
            InputAction::Motion {
                motion: Motion::TillChar('x'),
                count: 1,
            }
        );

        let mut state = InputState::new();
        let f = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('F'));
        let f_target = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(f, InputAction::None);
        assert_eq!(
            f_target,
            InputAction::Motion {
                motion: Motion::FindCharBefore('x'),
                count: 1,
            }
        );

        let mut state = InputState::new();
        let t = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('T'));
        let t_target = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(t, InputAction::None);
        assert_eq!(
            t_target,
            InputAction::Motion {
                motion: Motion::TillCharBefore('x'),
                count: 1,
            }
        );
    }

    #[test]
    fn normal_mode_operator_find_and_till_map_into_motion_targets() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('t'));
        let dt = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(
            dt,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::TillChar('x'),
                    count: 1,
                },
            }
        );

        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('c'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('f'));
        let cf = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(
            cf,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Change,
                target: OperatorTarget::Motion {
                    motion: Motion::FindChar('x'),
                    count: 1,
                },
            }
        );

        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('T'));
        let dt = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(
            dt,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::TillCharBefore('x'),
                    count: 1,
                },
            }
        );

        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('c'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('F'));
        let cf = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(
            cf,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Change,
                target: OperatorTarget::Motion {
                    motion: Motion::FindCharBefore('x'),
                    count: 1,
                },
            }
        );
    }

    #[test]
    fn normal_mode_slash_enters_search_mode() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('/'));
        assert_eq!(action, InputAction::EnterSearch);
    }

    #[test]
    fn search_mode_characters_map_to_search_actions() {
        let mut state = InputState::new();
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Search, &Event::Character('x')),
            InputAction::SearchChar('x')
        );
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Search, &Event::Backspace),
            InputAction::SearchBackspace
        );
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Search, &Event::Enter),
            InputAction::SearchEnter
        );
    }

    #[test]
    fn ctrl_n_and_ctrl_p_repeat_the_most_recent_search() {
        let mut state = InputState::new();
        let next = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('n'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(next, InputAction::RepeatSearch { forward: true });

        let prev = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('p'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(prev, InputAction::RepeatSearch { forward: false });
    }

    #[test]
    fn normal_mode_escape_clears_active_search_highlights() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Escape);
        assert_eq!(action, InputAction::ClearSearch);
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
    fn normal_mode_count_prefix_is_ignored_by_gg_and_cleared() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('1'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('2'));

        let gg = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('g'));
        assert_eq!(gg, InputAction::None);
        let gg = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('g'));
        assert_eq!(
            gg,
            InputAction::Motion {
                motion: Motion::FileStart,
                count: 1,
            }
        );

        let next = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('j'));
        assert_eq!(
            next,
            InputAction::Motion {
                motion: Motion::Down,
                count: 1,
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
    fn insert_mode_ctrl_c_returns_to_normal() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Insert,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('c'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::Normal));
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
    fn normal_mode_shift_lowercase_v_key_event_enters_visual_line_mode() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('v'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::VisualLine));
    }

    #[test]
    fn normal_mode_ctrl_v_enters_visual_block_mode() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('v'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::VisualBlock));
    }

    #[test]
    fn visual_escape_returns_to_normal() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Escape);
        assert_eq!(action, InputAction::SetMode(InputMode::Normal));
    }

    #[test]
    fn command_mode_ctrl_c_cancels() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Command,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('c'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::CommandCancel);
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
    fn visual_ctrl_c_returns_to_normal() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Visual,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('c'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::Normal));
    }

    #[test]
    fn visual_line_ctrl_c_returns_to_normal() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::VisualLine,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('c'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::Normal));
    }

    #[test]
    fn visual_block_ctrl_c_returns_to_normal() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::VisualBlock,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('c'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::Normal));
    }

    #[test]
    fn visual_mode_ctrl_v_switches_to_visual_block_mode() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Visual,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('v'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::SetMode(InputMode::VisualBlock));
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
    fn visual_mode_c_changes_selection_and_enters_insert() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('c'));
        assert_eq!(action, InputAction::ChangeSelectionPrivate);
    }

    #[test]
    fn normal_mode_x_deletes_char_without_yank() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(action, InputAction::DeleteCharNoYank);
    }

    #[test]
    fn normal_mode_tilde_toggles_case_with_count() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('3'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('~'));
        assert_eq!(action, InputAction::ToggleCase { count: 3 });
    }

    #[test]
    fn visual_mode_tilde_toggles_selection_case() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('~'));
        assert_eq!(action, InputAction::ToggleCase { count: 1 });
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
    fn normal_mode_diw_resolves_to_inner_word_delete() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('d'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('i'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('w'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope: TextObjectScope::Inner,
                    kind: TextObjectKind::Word,
                    count: 1,
                }),
            }
        );
    }

    #[test]
    fn normal_mode_cap_resolves_to_around_paragraph_change() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('c'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('a'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('p'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Change,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope: TextObjectScope::Around,
                    kind: TextObjectKind::Paragraph,
                    count: 1,
                }),
            }
        );
    }

    #[test]
    fn normal_mode_counted_ci_bracket_uses_count() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('2'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('c'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('i'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character(']'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Change,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope: TextObjectScope::Inner,
                    kind: TextObjectKind::Delimiter(DelimiterKind::Brackets),
                    count: 2,
                }),
            }
        );
    }

    #[test]
    fn normal_mode_ci_quote_resolves_to_inner_double_quote_change() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('c'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('i'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('"'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Change,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope: TextObjectScope::Inner,
                    kind: TextObjectKind::Delimiter(DelimiterKind::DoubleQuotes),
                    count: 1,
                }),
            }
        );
    }

    #[test]
    fn normal_mode_yy_yanks_current_line_to_private_register() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('y'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('y'));
        assert_eq!(action, InputAction::YankCurrentLinePrivate { count: 1 });
    }

    #[test]
    fn normal_mode_cc_changes_current_line_to_private_register() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('c'));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('c'));
        assert_eq!(action, InputAction::ChangeCurrentLinePrivate { count: 1 });
    }

    #[test]
    fn visual_mode_iw_resolves_to_inner_word_select() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('i'));
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('w'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Select,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope: TextObjectScope::Inner,
                    kind: TextObjectKind::Word,
                    count: 1,
                }),
            }
        );
    }

    #[test]
    fn visual_mode_a_bracket_resolves_to_around_bracket_select() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('a'));
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('['));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Select,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope: TextObjectScope::Around,
                    kind: TextObjectKind::Delimiter(DelimiterKind::Brackets),
                    count: 1,
                }),
            }
        );
    }

    #[test]
    fn visual_mode_i_big_word_resolves_to_inner_big_word_select() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('i'));
        let action = map_event_with_state(&mut state, InputMode::Visual, &Event::Character('W'));
        assert_eq!(
            action,
            InputAction::OperateTarget {
                operator: TextObjectOperator::Select,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope: TextObjectScope::Inner,
                    kind: TextObjectKind::BigWord,
                    count: 1,
                }),
            }
        );
    }

    #[test]
    fn consumed_prefix_clears_pending_count() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('2'));
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character(' '));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));

        assert_eq!(action, InputAction::None);
        let next = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('j'));
        assert_eq!(
            next,
            InputAction::Motion {
                motion: Motion::Down,
                count: 1,
            }
        );
    }

    #[test]
    fn normal_mode_shift_p_pastes_private_register_before() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('P'));
        assert_eq!(action, InputAction::PastePrivateRegisterBefore);
    }

    #[test]
    fn normal_mode_shift_lowercase_p_key_event_pastes_private_register_before() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('p'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(action, InputAction::PastePrivateRegisterBefore);
    }

    #[test]
    fn normal_mode_shift_lowercase_f_and_t_key_events_start_backward_search_motions() {
        let mut state = InputState::new();
        let f = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('f'),
                mods: KeyModifiers::shift(),
            }),
        );
        let f_target = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(f, InputAction::None);
        assert_eq!(
            f_target,
            InputAction::Motion {
                motion: Motion::FindCharBefore('x'),
                count: 1,
            }
        );

        let mut state = InputState::new();
        let t = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('t'),
                mods: KeyModifiers::shift(),
            }),
        );
        let t_target = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x'));
        assert_eq!(t, InputAction::None);
        assert_eq!(
            t_target,
            InputAction::Motion {
                motion: Motion::TillCharBefore('x'),
                count: 1,
            }
        );
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
    fn visual_mode_shift_lowercase_j_and_k_key_events_move_selection() {
        let mut state = InputState::new();
        let down = map_event_with_state(
            &mut state,
            InputMode::Visual,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('j'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(down, InputAction::MoveVisualSelectionDown { count: 1 });
        let up = map_event_with_state(
            &mut state,
            InputMode::Visual,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('k'),
                mods: KeyModifiers::shift(),
            }),
        );
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
    fn normal_mode_leader_p_pastes_from_system_clipboard() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character(' '));
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('p'));
        assert_eq!(action, InputAction::PasteSystemClipboard);
    }

    #[test]
    fn normal_mode_p_pastes_private_register() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('p'));
        assert_eq!(action, InputAction::PastePrivateRegister);
    }

    #[test]
    fn normal_mode_u_triggers_undo() {
        let mut state = InputState::new();
        let action = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('u'));
        assert_eq!(action, InputAction::Undo);
    }

    #[test]
    fn normal_mode_shift_lowercase_g_key_event_maps_to_file_end() {
        let mut state = InputState::new();
        let _ = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('2'),
                mods: KeyModifiers::none(),
            }),
        );
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('g'),
                mods: KeyModifiers::shift(),
            }),
        );
        assert_eq!(
            action,
            InputAction::Motion {
                motion: Motion::FileEnd,
                count: 2,
            }
        );
    }

    #[test]
    fn normal_mode_y_confirms_explorer_delete() {
        let mut state = InputState::new();
        let action =
            map_event_with_context(&mut state, InputMode::Normal, true, &Event::Character('y'));
        assert_eq!(action, InputAction::ConfirmExplorerDelete);
    }

    #[test]
    fn normal_mode_ctrl_r_triggers_redo() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('r'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::Redo);
    }

    #[test]
    fn normal_mode_ctrl_d_triggers_viewport_down_center() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('d'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::ViewportDownCenter);
    }

    #[test]
    fn normal_mode_ctrl_u_triggers_viewport_up_center() {
        let mut state = InputState::new();
        let action = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &Event::KeyWithModifiers(KeyWithModifiers {
                key: KeyKind::Char('u'),
                mods: KeyModifiers::ctrl(),
            }),
        );
        assert_eq!(action, InputAction::ViewportUpCenter);
    }

    #[test]
    fn normal_mode_zz_triggers_center_cursor_line() {
        let mut state = InputState::new();
        let first = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('z'));
        assert_eq!(first, InputAction::None);

        let second = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('z'));
        assert_eq!(second, InputAction::CenterCursorLine);
    }
}
