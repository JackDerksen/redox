//! Editor state and action application for `redox-tui`.
//!
//! This module keeps UI-facing state (mode, command line, status, cursor viewport
//! reconciliation) while delegating text editing primitives to `redox-core`.

use std::collections::HashMap;

use redox_core::{BufferId, EditorSession, Pos, Selection, TextBuffer};

use crate::input::cursor::CursorController;
use crate::input::{InputMode, InputState};
use crate::ui::GraphemeCache;
mod about;
pub use about::AboutPopup;
use about::AboutState;
mod explorer;
pub use explorer::ExplorerPopup;
use explorer::ExplorerState;
mod actions;
mod commands;
mod editing;
mod surface;

const PREFETCH_PER_FRAME_BYTES: usize = 64 * 1024;
const DEMAND_LOAD_BUDGET_BYTES: usize = 256 * 1024;
const VIEWPORT_PREFETCH_MULTIPLIER: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterKind {
    CharWise,
    LineWise,
}

#[derive(Debug, Clone)]
struct UndoSnapshot {
    buffer: TextBuffer,
    cursor: Pos,
}

#[derive(Debug, Default)]
struct UndoHistory {
    undo_stack: Vec<UndoSnapshot>,
    redo_stack: Vec<UndoSnapshot>,
}

/// Vim-like editor mode for the TUI frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Normal,
    Insert,
    Command,
    Visual,
    VisualLine,
}

impl EditorMode {
    pub fn as_input_mode(self) -> InputMode {
        match self {
            EditorMode::Normal => InputMode::Normal,
            EditorMode::Insert => InputMode::Insert,
            EditorMode::Command => InputMode::Command,
            EditorMode::Visual => InputMode::Visual,
            EditorMode::VisualLine => InputMode::VisualLine,
        }
    }
}

/// Per-buffer UI/view state that is not persisted in `redox-core`.
#[derive(Debug)]
pub struct BufferViewState {
    pub cursor: CursorController,
    pub grapheme_cache: GraphemeCache,
    pub visual_anchor: Option<Pos>,
    undo_history: UndoHistory,
    insert_mode_coalesce_base: Option<UndoSnapshot>,
}

impl Default for BufferViewState {
    fn default() -> Self {
        Self {
            cursor: CursorController::new(),
            grapheme_cache: GraphemeCache::new(512),
            visual_anchor: None,
            undo_history: UndoHistory::default(),
            insert_mode_coalesce_base: None,
        }
    }
}

/// Multi-buffer editor state for the TUI frontend.
#[derive(Debug)]
pub struct EditorState {
    pub session: EditorSession,
    pub views: HashMap<BufferId, BufferViewState>,
    about: Option<AboutState>,
    explorer: Option<ExplorerState>,
    pub mode: EditorMode,
    pub input: InputState,
    pub command_line: String,
    pub status_msg: Option<String>,
    status_msg_ephemeral: bool,
    pub should_quit: bool,
    viewport_width_cells: usize,
    viewport_height_rows: usize,
    private_register: String,
    private_register_kind: RegisterKind,
    pending_system_clipboard: Option<String>,
    explorer_delete_confirmation_token: Option<String>,
}

impl EditorState {
    #[inline]
    fn buffers_equal(a: &TextBuffer, b: &TextBuffer) -> bool {
        a.rope() == b.rope()
    }

    pub fn new(session: EditorSession) -> Self {
        let active = session.active_id();
        let mut views = HashMap::new();
        views.insert(active, BufferViewState::default());

        Self {
            session,
            views,
            about: None,
            explorer: None,
            mode: EditorMode::Normal,
            input: InputState::new(),
            command_line: String::new(),
            status_msg: None,
            status_msg_ephemeral: false,
            should_quit: false,
            viewport_width_cells: 80,
            viewport_height_rows: 24,
            private_register: String::new(),
            private_register_kind: RegisterKind::CharWise,
            pending_system_clipboard: None,
            explorer_delete_confirmation_token: None,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
        self.status_msg_ephemeral = false;
    }

    fn set_status_ephemeral(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
        self.status_msg_ephemeral = true;
    }

    pub fn clear_status(&mut self) {
        self.status_msg = None;
        self.status_msg_ephemeral = false;
    }

    pub fn set_viewport_size(&mut self, width_cells: usize, height_rows: usize) {
        self.viewport_width_cells = width_cells;
        self.viewport_height_rows = height_rows;
    }

    pub fn viewport_size(&self) -> (usize, usize) {
        (self.viewport_width_cells, self.viewport_height_rows)
    }

    pub fn take_pending_system_clipboard(&mut self) -> Option<String> {
        self.pending_system_clipboard.take()
    }

    pub fn pump_active_loading(&mut self, viewport_height_rows: usize) {
        let active_id = self.session.active_id();
        let scroll_y = self
            .views
            .get(&active_id)
            .map(|v| v.cursor.scroll_y_lines)
            .unwrap_or(0);
        let target_line = scroll_y
            .saturating_add(viewport_height_rows.saturating_mul(VIEWPORT_PREFETCH_MULTIPLIER));

        let _ = self.session.poll_loading(PREFETCH_PER_FRAME_BYTES);
        if let Err(e) = self.session.ensure_buffer_loaded_through_line(
            active_id,
            target_line,
            DEMAND_LOAD_BUDGET_BYTES,
        ) {
            self.set_status(format!("load failed: {e}"));
        }
    }

    fn ensure_active_fully_loaded_for_edit_or_save(&mut self) -> bool {
        let active_id = self.session.active_id();
        match self.session.ensure_buffer_fully_loaded(active_id) {
            Ok(()) => true,
            Err(e) => {
                self.set_status(format!("load failed: {e}"));
                false
            }
        }
    }

    pub fn active_dirty(&self) -> bool {
        self.session.active_meta().dirty
    }

    pub fn active_display_name(&self) -> &str {
        &self.session.active_meta().display_name
    }

    pub fn active_cursor_pos(&self) -> Pos {
        let id = self.session.active_id();
        self.views
            .get(&id)
            .map(|view| view.cursor.cursor)
            .unwrap_or(Pos::zero())
    }

    pub fn active_visual_selection(&self) -> Option<(Selection, bool)> {
        let is_visual = matches!(self.mode, EditorMode::Visual | EditorMode::VisualLine);
        if !is_visual {
            return None;
        }

        let id = self.session.active_id();
        let view = self.views.get(&id)?;
        let anchor = view.visual_anchor?;
        let line_mode = self.mode == EditorMode::VisualLine;
        Some((Selection::new(anchor, view.cursor.cursor), line_mode))
    }

    pub fn with_active_buffer_view_mut<R>(
        &mut self,
        f: impl FnOnce(&TextBuffer, &mut BufferViewState) -> R,
    ) -> R {
        let active_id = self.session.active_id();
        let buffer = self
            .session
            .buffer(active_id)
            .expect("active buffer must exist in session map");
        let view = self.views.entry(active_id).or_default();
        f(buffer, view)
    }

    pub fn with_buffer_view_mut<R>(
        &mut self,
        id: BufferId,
        f: impl FnOnce(&TextBuffer, &mut BufferViewState) -> R,
    ) -> Option<R> {
        let buffer = self.session.buffer(id)?;
        let view = self.views.entry(id).or_default();
        Some(f(buffer, view))
    }

    fn set_active_visual_anchor_if_missing(&mut self) {
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        if view.visual_anchor.is_none() {
            view.visual_anchor = Some(view.cursor.cursor);
        }
    }

    fn clear_active_visual_anchor(&mut self) {
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        view.visual_anchor = None;
    }

    fn capture_active_undo_snapshot(&mut self) -> UndoSnapshot {
        let active_id = self.session.active_id();
        let cursor = self.views.entry(active_id).or_default().cursor.cursor;
        let buffer = self.session.active_buffer().clone();
        UndoSnapshot { buffer, cursor }
    }

    fn capture_active_insert_coalesced_snapshot(&mut self) -> UndoSnapshot {
        let active_id = self.session.active_id();
        if let Some(existing) = self
            .views
            .get(&active_id)
            .and_then(|view| view.insert_mode_coalesce_base.clone())
        {
            return existing;
        }

        let snapshot = self.capture_active_undo_snapshot();
        let view = self.views.entry(active_id).or_default();
        view.insert_mode_coalesce_base = Some(snapshot.clone());
        snapshot
    }

    fn clear_active_insert_undo_coalesce(&mut self) {
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        view.insert_mode_coalesce_base = None;
    }

    fn record_active_undo_if_changed(&mut self, before: UndoSnapshot) -> bool {
        let active_id = self.session.active_id();
        let after_buffer = self.session.active_buffer();
        if Self::buffers_equal(after_buffer, &before.buffer) {
            return false;
        }

        let view = self.views.entry(active_id).or_default();
        let duplicate_last = view
            .undo_history
            .undo_stack
            .last()
            .is_some_and(|last| {
                Self::buffers_equal(&last.buffer, &before.buffer) && last.cursor == before.cursor
            });
        if !duplicate_last {
            view.undo_history.undo_stack.push(before);
        }
        view.undo_history.redo_stack.clear();
        true
    }

    fn undo_active(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let active_id = self.session.active_id();
        let prev = {
            let view = self.views.entry(active_id).or_default();
            view.undo_history.undo_stack.pop()
        };

        let Some(prev) = prev else {
            self.set_status_ephemeral("nothing to undo");
            return;
        };

        let current = self.capture_active_undo_snapshot();
        {
            let view = self.views.entry(active_id).or_default();
            view.undo_history.redo_stack.push(current);
        }

        self.restore_active_snapshot(prev, viewport_width_cells, text_vh);
    }

    fn redo_active(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let active_id = self.session.active_id();
        let next = {
            let view = self.views.entry(active_id).or_default();
            view.undo_history.redo_stack.pop()
        };

        let Some(next) = next else {
            self.set_status_ephemeral("nothing to redo");
            return;
        };

        let current = self.capture_active_undo_snapshot();
        {
            let view = self.views.entry(active_id).or_default();
            view.undo_history.undo_stack.push(current);
        }

        self.restore_active_snapshot(next, viewport_width_cells, text_vh);
    }

    fn restore_active_snapshot(
        &mut self,
        mut snapshot: UndoSnapshot,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        {
            let buffer = self.session.active_buffer_mut();
            *buffer = std::mem::take(&mut snapshot.buffer);
        }

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        view.cursor.cursor = buffer.clamp_pos(snapshot.cursor);
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        view.grapheme_cache.clear();
        view.visual_anchor = None;
        view.insert_mode_coalesce_base = None;

        self.mode = EditorMode::Normal;
        let _ = self.session.recompute_active_dirty();
        self.clear_status();
    }
}

#[cfg(test)]
mod tests;
