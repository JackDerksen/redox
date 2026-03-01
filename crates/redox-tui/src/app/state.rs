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
}

impl Default for BufferViewState {
    fn default() -> Self {
        Self {
            cursor: CursorController::new(),
            grapheme_cache: GraphemeCache::new(512),
            visual_anchor: None,
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
}

impl EditorState {
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
}

#[cfg(test)]
mod tests;
