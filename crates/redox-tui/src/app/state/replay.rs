use std::collections::BTreeMap;

use redox_core::{
    BufferId, BufferKind, Pos, Selection, TextBuffer, TextDiff, UndoCheckpoint, UndoHistory,
    VisualModeKind,
};

use super::{EditorMode, EditorState};
use crate::input::{InputAction, InputMode, InsertKind, OperatorTarget, TextObjectOperator};

const MAX_PLAYBACK_ACTIONS: usize = 10_000;
const MAX_PLAYBACK_DEPTH: usize = 32;

#[derive(Debug, Default)]
pub(super) struct ReplayState {
    last_change: Option<RecordedChange>,
    pending_change: Option<PendingChange>,
    macros: BTreeMap<String, RecordedMacro>,
    recording: Option<(String, RecordedMacro)>,
    last_macro: Option<String>,
    depth: usize,
    remaining_actions: usize,
    repeating_change: bool,
    dispatching: bool,
    failure: Option<String>,
    resolved_insert: Option<InputAction>,
    undo_checkpoints: BTreeMap<BufferId, ReplayUndo>,
}

#[derive(Debug)]
struct ReplayUndo {
    checkpoint: UndoCheckpoint,
    history: UndoHistory,
}

#[derive(Debug, Clone, Default)]
struct RecordedMacro {
    actions: Vec<InputAction>,
    keys: String,
}

impl RecordedMacro {
    fn sequence(&self) -> String {
        if self.keys.is_empty() {
            format!("{} actions", self.actions.len())
        } else {
            self.keys.clone()
        }
    }
}

#[derive(Debug, Clone)]
struct RecordedChange {
    actions: Vec<InputAction>,
    visual: Option<VisualExtent>,
    repetitions: usize,
}

#[derive(Debug)]
struct PendingChange {
    buffer_id: BufferId,
    before: TextBuffer,
    change: RecordedChange,
}

#[derive(Debug, Clone, Copy)]
struct VisualExtent {
    mode: VisualModeKind,
    lines: usize,
    columns: usize,
}

impl VisualExtent {
    fn capture(selection: Selection, mode: VisualModeKind) -> Self {
        let (start, end) = selection.ordered();
        Self {
            mode,
            lines: end.line - start.line,
            columns: match mode {
                VisualModeKind::Char if start.line != end.line => end.col,
                VisualModeKind::Char | VisualModeKind::Block => end.col.abs_diff(start.col),
                VisualModeKind::Line => 0,
            },
        }
    }

    fn restore(self, state: &mut EditorState) {
        let start = state.active_cursor_pos();
        let end_line = state
            .session
            .active_buffer()
            .clamp_line(start.line.saturating_add(self.lines));
        let end_column = if self.mode == VisualModeKind::Char && self.lines > 0 {
            self.columns
        } else {
            start.col.saturating_add(self.columns)
        };
        let view = state.views.entry(state.session.active_id()).or_default();
        view.visual_anchor = Some(start);
        view.cursor.cursor = Pos::new(end_line, end_column);
        state.mode = match self.mode {
            VisualModeKind::Char => EditorMode::Visual,
            VisualModeKind::Line => EditorMode::VisualLine,
            VisualModeKind::Block => EditorMode::VisualBlock,
        };
    }
}

impl EditorState {
    pub(super) fn remember_inserted_text(&mut self, text: String) {
        self.replay.last_change = Some(RecordedChange {
            actions: vec![InputAction::Paste(text)],
            visual: None,
            repetitions: 1,
        });
    }

    pub(super) fn command_list_macros(&mut self) {
        if self.replay.macros.is_empty() {
            self.set_status("no macros recorded in this session");
            return;
        }
        let mut message = String::from("Session macros:");
        for (register, recording) in &self.replay.macros {
            message.push_str(&format!("\n@{register}  {}", recording.sequence()));
        }
        self.set_status(message);
    }

    pub fn recording_macro_register(&self) -> Option<&str> {
        self.replay
            .recording
            .as_ref()
            .map(|(register, _)| register.as_str())
    }

    pub(crate) fn record_macro_key(&mut self, key: &str) {
        if self.replay.depth == 0
            && let Some((_, recording)) = &mut self.replay.recording
        {
            recording.keys.push_str(key);
        }
    }

    pub fn apply_input(&mut self, action: InputAction, width: usize, height: usize) {
        self.log_action(&action);
        if self.replay.dispatching {
            self.apply_input_inner(action, width, height);
            return;
        }

        match action {
            InputAction::ToggleMacroRecording => {
                if let Some((register, recording)) = self.replay.recording.take() {
                    let sequence = recording.sequence();
                    self.input.reset_prefixes();
                    self.set_status(format!("Recorded @{register}\n{sequence}"));
                    self.replay.macros.insert(register, recording);
                }
                return;
            }
            InputAction::StartMacroRecording { register } => {
                if register == "@" {
                    self.set_status("@ is reserved for @@ (play the last macro)");
                    return;
                }
                if self.mode == EditorMode::Normal && !register.is_empty() && self.replay.depth == 0
                {
                    let append = register.len() == 1 && register.as_bytes()[0].is_ascii_uppercase();
                    let name = if append {
                        register.to_ascii_lowercase()
                    } else {
                        register
                    };
                    let recording = if append {
                        self.replay.macros.get(&name).cloned().unwrap_or_default()
                    } else {
                        RecordedMacro::default()
                    };
                    self.replay.recording = Some((name, recording));
                    self.clear_status();
                    self.request_redraw();
                }
                return;
            }
            _ => {}
        }

        match &action {
            InputAction::RepeatLastChange { count } => {
                self.record_macro_action(&action);
                self.repeat_last_change(*count, width, height);
                return;
            }
            InputAction::PlayMacro { register, count } => {
                self.record_macro_action(&action);
                self.play_macro(register.as_deref(), *count, width, height);
                return;
            }
            _ => {}
        }

        let active_id = self.session.active_id();
        let was_insert = self.mode == EditorMode::Insert;
        let recorded_action = match &action {
            InputAction::Motion {
                motion: redox_core::motion::Motion::Up | redox_core::motion::Motion::Down,
                ..
            } if was_insert && self.has_visible_completion_popup() => InputAction::None,
            InputAction::CompletionCancel if was_insert => {
                if self.has_visible_completion_popup() {
                    InputAction::None
                } else {
                    InputAction::SetMode(InputMode::Normal)
                }
            }
            InputAction::CompletionAccept if was_insert && !self.has_visible_completion_popup() => {
                InputAction::Enter
            }
            InputAction::SnippetNext if was_insert && !self.has_active_snippet() => {
                InputAction::InsertChar('\t')
            }
            _ => action.clone(),
        };
        let resolved_snapshot = (was_insert
            && (matches!(
                recorded_action,
                InputAction::CompletionAccept | InputAction::SnippetNext
            ) || self.has_active_snippet()
                && matches!(
                    recorded_action,
                    InputAction::InsertChar(_)
                        | InputAction::Backspace
                        | InputAction::Enter
                        | InputAction::Paste(_)
                )))
        .then(|| {
            (
                self.session.active_buffer().clone(),
                self.active_cursor_pos(),
            )
        });
        if self
            .replay
            .pending_change
            .as_ref()
            .is_some_and(|pending| pending.buffer_id != active_id)
        {
            self.finish_recorded_change();
        }
        if !self.replay.repeating_change
            && self.replay.pending_change.is_none()
            && self.session.active_meta().kind == BufferKind::File
            && matches!(
                self.mode,
                EditorMode::Normal
                    | EditorMode::Visual
                    | EditorMode::VisualLine
                    | EditorMode::VisualBlock
                    | EditorMode::Insert
            )
            && (starts_change(&recorded_action) || was_insert && is_insert_action(&recorded_action))
            && self.ensure_active_fully_loaded_for_edit_or_save()
        {
            self.replay.pending_change = Some(PendingChange {
                buffer_id: active_id,
                before: self.session.active_buffer().clone(),
                change: RecordedChange {
                    actions: if was_insert {
                        vec![InputAction::EnterInsert(InsertKind::Insert)]
                    } else {
                        Vec::new()
                    },
                    repetitions: 1,
                    visual: self
                        .active_visual_selection()
                        .map(|(selection, mode)| VisualExtent::capture(selection, mode)),
                },
            });
        }
        self.replay.resolved_insert = None;
        self.replay.dispatching = true;
        self.apply_input_inner(action, width, height);
        self.replay.dispatching = false;
        let recorded_action = self.replay.resolved_insert.take().unwrap_or_else(|| {
            if let Some((before, cursor)) = resolved_snapshot {
                let before_cursor = before.pos_to_char(cursor);
                let after = self.session.active_buffer();
                let after_cursor = after.pos_to_char(self.active_cursor_pos());
                let diff = TextDiff::between(&before, after);
                let start = diff.as_ref().map_or(before_cursor, |diff| diff.start_char);
                InputAction::ApplyRecordedInsert {
                    start_offset: char_offset(start, before_cursor),
                    deleted_chars: diff.as_ref().map_or(0, |diff| diff.deleted.chars().count()),
                    text: diff.map_or_else(String::new, |diff| diff.inserted),
                    cursor_offset: char_offset(after_cursor, start),
                }
            } else {
                recorded_action
            }
        });
        self.record_macro_action(&recorded_action);
        if !matches!(recorded_action, InputAction::None)
            && (!was_insert || is_insert_action(&recorded_action))
            && let Some(pending) = &mut self.replay.pending_change
        {
            pending.change.actions.push(recorded_action);
        }
        if self.mode != EditorMode::Insert || self.session.active_id() != active_id {
            self.finish_recorded_change();
        }
    }

    fn record_macro_action(&mut self, action: &InputAction) {
        if self.replay.depth == 0
            && !matches!(action, InputAction::None)
            && let Some((_, recording)) = &mut self.replay.recording
        {
            recording.actions.push(action.clone());
        }
    }

    pub(super) fn record_resolved_completion(
        &mut self,
        cursor: usize,
        start: usize,
        end: usize,
        text: String,
        cursor_offset: usize,
    ) {
        self.replay.resolved_insert = Some(InputAction::ApplyRecordedInsert {
            start_offset: char_offset(start, cursor),
            deleted_chars: end.saturating_sub(start),
            text,
            cursor_offset: cursor_offset.min(isize::MAX as usize) as isize,
        });
    }

    pub(super) fn apply_recorded_insert(
        &mut self,
        start_offset: isize,
        deleted_chars: usize,
        text: &str,
        cursor_offset: isize,
        width: usize,
        text_height: usize,
    ) {
        if self.mode != EditorMode::Insert || !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let before = self.capture_active_insert_coalesced_checkpoint();
        let cursor = self.active_cursor_pos();
        let buffer = self.session.active_buffer_mut();
        let start = buffer
            .pos_to_char(cursor)
            .saturating_add_signed(start_offset)
            .min(buffer.len_chars());
        let end = start.saturating_add(deleted_chars).min(buffer.len_chars());
        let _ = buffer.replace_selection(
            Selection::new(buffer.char_to_pos(start), buffer.char_to_pos(end)),
            text,
        );
        let cursor = buffer.char_to_pos(start.saturating_add_signed(cursor_offset));
        let view = self.views.entry(self.session.active_id()).or_default();
        view.cursor.cursor = cursor;
        view.cursor
            .reconcile_after_edit(self.session.active_buffer(), width, text_height);
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn finish_recorded_change(&mut self) {
        let Some(mut pending) = self.replay.pending_change.take() else {
            return;
        };
        let Some(buffer) = self.session.buffer(pending.buffer_id) else {
            return;
        };
        if TextDiff::between(&pending.before, buffer).is_none() {
            return;
        }
        if !matches!(
            pending.change.actions.last(),
            Some(InputAction::SetMode(InputMode::Normal))
        ) {
            pending
                .change
                .actions
                .push(InputAction::SetMode(InputMode::Normal));
        }
        self.replay.last_change = Some(pending.change);
    }

    fn repeat_last_change(&mut self, count: Option<usize>, width: usize, height: usize) {
        if self.mode != EditorMode::Normal || self.active_buffer_is_surface() {
            return;
        }
        let Some(mut change) = self.replay.last_change.clone() else {
            self.set_status("nothing to repeat");
            return;
        };
        if let Some(count) = count {
            change.repetitions = if let Some(action) = change.actions.first_mut()
                && replace_change_count(action, count)
            {
                1
            } else {
                count.max(1)
            };
        }
        let was_repeating = self.replay.repeating_change;
        self.replay.repeating_change = true;
        if change.repetitions > 1
            && matches!(change.actions.first(), Some(InputAction::EnterInsert(_)))
        {
            let mut actions = vec![change.actions[0].clone()];
            // Repeat the insertion body before Escape to keep adjacent copies intact.
            let body = &change.actions[1..change.actions.len().saturating_sub(1)];
            if change
                .repetitions
                .saturating_mul(body.len())
                .saturating_add(2)
                <= MAX_PLAYBACK_ACTIONS
            {
                for _ in 0..change.repetitions {
                    actions.extend_from_slice(body);
                }
                actions.push(InputAction::SetMode(InputMode::Normal));
                self.play_actions(&actions, 1, None, width, height);
            } else {
                self.set_status("repeat count exceeds replay limit");
            }
        } else {
            self.play_actions(
                &change.actions,
                change.repetitions,
                change.visual,
                width,
                height,
            );
        }
        self.replay.repeating_change = was_repeating;
        self.replay.last_change = Some(change);
    }

    fn play_macro(&mut self, register: Option<&str>, count: usize, width: usize, height: usize) {
        if self.mode != EditorMode::Normal || self.active_buffer_is_surface() {
            return;
        }
        let Some(register) = register
            .map(|name| {
                if name.len() == 1 {
                    name.to_ascii_lowercase()
                } else {
                    name.to_string()
                }
            })
            .or_else(|| self.replay.last_macro.clone())
        else {
            self.set_status("no macro has been played");
            return;
        };
        let Some(recording) = self.replay.macros.get(&register).cloned() else {
            let message = format!("macro @{register} is empty");
            self.set_status(&message);
            self.replay.failure = Some(message);
            self.replay.remaining_actions = 0;
            return;
        };
        self.replay.last_macro = Some(register);
        self.play_actions(&recording.actions, count.max(1), None, width, height);
    }

    fn play_actions(
        &mut self,
        actions: &[InputAction],
        count: usize,
        visual: Option<VisualExtent>,
        width: usize,
        height: usize,
    ) {
        if actions.is_empty() {
            return;
        }
        if self.replay.depth >= MAX_PLAYBACK_DEPTH {
            self.replay.remaining_actions = 0;
            return;
        }
        let outermost = self.replay.depth == 0;
        if outermost {
            self.replay.remaining_actions = MAX_PLAYBACK_ACTIONS;
            self.replay.failure = None;
            self.close_completion();
        }
        self.replay.depth += 1;
        'playback: for _ in 0..count {
            if let Some(visual) = visual {
                visual.restore(self);
            }
            for action in actions {
                if self.replay.remaining_actions == 0 || self.should_quit {
                    break 'playback;
                }
                self.replay.remaining_actions -= 1;
                let buffer_id = self.session.active_id();
                if !self.replay.undo_checkpoints.contains_key(&buffer_id)
                    && self.session.active_meta().kind == BufferKind::File
                {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        break 'playback;
                    }
                    let checkpoint = self.capture_active_undo_checkpoint();
                    let history = self
                        .views
                        .entry(buffer_id)
                        .or_default()
                        .undo_history
                        .clone();
                    self.replay.undo_checkpoints.insert(
                        buffer_id,
                        ReplayUndo {
                            checkpoint,
                            history,
                        },
                    );
                }
                self.apply_input(action.clone(), width, height);
            }
        }
        self.replay.depth -= 1;
        if outermost {
            let buffers = self
                .replay
                .undo_checkpoints
                .keys()
                .copied()
                .collect::<Vec<_>>();
            for buffer_id in buffers {
                self.flush_replay_undo(buffer_id);
            }
            if self.replay.remaining_actions == 0 {
                let message = self
                    .replay
                    .failure
                    .take()
                    .unwrap_or_else(|| "playback stopped: replay limit reached".to_string());
                self.set_status(message);
            }
        }
    }

    fn flush_replay_undo(&mut self, buffer_id: BufferId) {
        let Some(before) = self.replay.undo_checkpoints.remove(&buffer_id) else {
            return;
        };
        if let Some(view) = self.views.get_mut(&buffer_id) {
            view.pending_insert_undo = None;
            view.undo_history = before.history;
        }
        if !self.record_buffer_undo_if_changed(buffer_id, before.checkpoint) {
            self.refresh_undo_tree_for_buffer(buffer_id);
        }
    }
}

fn char_offset(position: usize, origin: usize) -> isize {
    let distance = position.abs_diff(origin).min(isize::MAX as usize) as isize;
    if position >= origin {
        distance
    } else {
        -distance
    }
}

fn is_insert_action(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::InsertChar(_)
            | InputAction::Backspace
            | InputAction::Enter
            | InputAction::Paste(_)
            | InputAction::ApplyRecordedInsert { .. }
            | InputAction::Motion { .. }
            | InputAction::SetMode(InputMode::Normal)
    )
}

fn starts_change(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::EnterInsert(_)
            | InputAction::SetMode(InputMode::Insert)
            | InputAction::OpenLineBelow
            | InputAction::OpenLineAbove
            | InputAction::JoinLineBelow
            | InputAction::DeleteSelectionPrivate
            | InputAction::ChangeSelectionPrivate
            | InputAction::DeleteSelectionNoYank
            | InputAction::DeleteCurrentLinePrivate { .. }
            | InputAction::ChangeCurrentLinePrivate { .. }
            | InputAction::OperateTarget {
                operator: TextObjectOperator::Delete | TextObjectOperator::Change,
                ..
            }
            | InputAction::Paste(_)
            | InputAction::PasteSystemClipboardText(_)
            | InputAction::PastePrivateRegister
            | InputAction::PastePrivateRegisterBefore
            | InputAction::DeleteCharNoYank
            | InputAction::ToggleCase { .. }
            | InputAction::ReplaceChar(_)
            | InputAction::WrapSelection { .. }
            | InputAction::MoveVisualSelectionUp { .. }
            | InputAction::MoveVisualSelectionDown { .. }
            | InputAction::IndentVisualSelection { .. }
            | InputAction::OutdentVisualSelection { .. }
    )
}

fn replace_change_count(action: &mut InputAction, replacement: usize) -> bool {
    let count = match action {
        InputAction::DeleteCurrentLinePrivate { count }
        | InputAction::ChangeCurrentLinePrivate { count }
        | InputAction::ToggleCase { count }
        | InputAction::MoveVisualSelectionUp { count }
        | InputAction::MoveVisualSelectionDown { count }
        | InputAction::IndentVisualSelection { count }
        | InputAction::OutdentVisualSelection { count } => count,
        InputAction::OperateTarget { target, .. } => match target {
            OperatorTarget::Motion { count, .. } => count,
            OperatorTarget::TextObject(spec) => &mut spec.count,
        },
        _ => return false,
    };
    *count = replacement.max(1);
    true
}
