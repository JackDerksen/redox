//! Editor state and action application for `redox-tui`.
//!
//! This module keeps UI-facing state (mode, command line, status, cursor viewport
//! reconciliation) while delegating text editing primitives to `redox-core`.

use std::collections::HashMap;

use redox_core::{BufferId, BufferKind, EditorSession, Pos, Selection, TextBuffer, VisualModeKind};

use crate::input::cursor::CursorController;
use crate::input::{InputMode, InputState};
use crate::ui::overlays::DelimiterPairCache;
use crate::ui::{GraphemeCache, RainAnimation, SyntaxHighlighter, language_for_path};
mod about;
pub use about::AboutPopup;
use about::AboutState;
mod analysis;
use analysis::AnalysisWorker;
mod explorer;
mod rain_mode;
pub use explorer::ExplorerPopup;
use explorer::ExplorerState;
mod perf;
pub use perf::{FramePerfSample, FramePerfStats, PerfPopup};
mod actions;
mod commands;
mod editing;
mod search;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OneShotHighlight {
    buffer_id: BufferId,
    selection: Selection,
    mode: VisualModeKind,
    remaining_frames: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchLanding {
    OnMatch,
    BeforeMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchQuery {
    term: String,
    landing: SearchLanding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchMatch {
    start: Pos,
    end: Pos,
}

#[derive(Debug, Clone)]
struct SearchState {
    query: SearchQuery,
    buffer_id: BufferId,
    matches: Vec<SearchMatch>,
    active_match: Option<usize>,
    visible: bool,
    dirty: bool,
}

/// Vim-like editor mode for the TUI frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Normal,
    Insert,
    Command,
    Search,
    Visual,
    VisualLine,
    VisualBlock,
}

impl EditorMode {
    pub fn as_input_mode(self) -> InputMode {
        match self {
            EditorMode::Normal => InputMode::Normal,
            EditorMode::Insert => InputMode::Insert,
            EditorMode::Command => InputMode::Command,
            EditorMode::Search => InputMode::Search,
            EditorMode::Visual => InputMode::Visual,
            EditorMode::VisualLine => InputMode::VisualLine,
            EditorMode::VisualBlock => InputMode::VisualBlock,
        }
    }
}

/// Per-buffer UI/view state that is not persisted in `redox-core`.
#[derive(Debug)]
pub struct BufferViewState {
    pub cursor: CursorController,
    pub grapheme_cache: GraphemeCache,
    pub syntax_highlighter: SyntaxHighlighter,
    pub delimiter_pair_cache: DelimiterPairCache,
    pub visual_anchor: Option<Pos>,
    analysis_version: u64,
    undo_history: UndoHistory,
    insert_mode_coalesce_base: Option<UndoSnapshot>,
}

impl Default for BufferViewState {
    fn default() -> Self {
        Self {
            cursor: CursorController::new(),
            grapheme_cache: GraphemeCache::new(512),
            syntax_highlighter: SyntaxHighlighter::default(),
            delimiter_pair_cache: DelimiterPairCache::default(),
            visual_anchor: None,
            analysis_version: 0,
            undo_history: UndoHistory::default(),
            insert_mode_coalesce_base: None,
        }
    }
}

impl BufferViewState {
    pub(crate) fn analysis_version(&self) -> u64 {
        self.analysis_version
    }

    fn invalidate_render_caches(&mut self) {
        self.grapheme_cache.clear();
        self.syntax_highlighter.replace_cache(None);
        self.analysis_version = self.analysis_version.wrapping_add(1);
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
    status_msg_clear_on_input: bool,
    pub should_quit: bool,
    rain_animation: Option<RainAnimation>,
    rain_pending_start: bool,
    viewport_width_cells: usize,
    viewport_height_rows: usize,
    private_register: String,
    private_register_kind: RegisterKind,
    one_shot_highlight: Option<OneShotHighlight>,
    search_state: Option<SearchState>,
    pending_system_clipboard: Option<String>,
    explorer_delete_confirmation_token: Option<String>,
    analysis_worker: AnalysisWorker,
    perf_visible: bool,
    perf_stats: Option<FramePerfStats>,
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

        let state = Self {
            session,
            views,
            about: None,
            explorer: None,
            mode: EditorMode::Normal,
            input: InputState::new(),
            command_line: String::new(),
            status_msg: None,
            status_msg_clear_on_input: false,
            should_quit: false,
            rain_animation: None,
            rain_pending_start: false,
            viewport_width_cells: 80,
            viewport_height_rows: 24,
            private_register: String::new(),
            private_register_kind: RegisterKind::CharWise,
            one_shot_highlight: None,
            search_state: None,
            pending_system_clipboard: None,
            explorer_delete_confirmation_token: None,
            analysis_worker: AnalysisWorker::new(),
            perf_visible: false,
            perf_stats: None,
        };
        state.request_analysis(active, 0);
        state
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
        self.status_msg_clear_on_input = true;
    }

    pub fn set_status_sticky(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
        self.status_msg_clear_on_input = false;
    }

    pub fn clear_status(&mut self) {
        self.status_msg = None;
        self.status_msg_clear_on_input = false;
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

    pub fn has_pending_explorer_delete_confirmation(&self) -> bool {
        self.explorer_delete_confirmation_token.is_some()
    }

    pub fn one_shot_highlight(&self) -> Option<(Selection, VisualModeKind)> {
        let highlight = self.one_shot_highlight?;
        (highlight.buffer_id == self.session.active_id())
            .then_some((highlight.selection, highlight.mode))
    }

    pub fn advance_one_shot_highlight(&mut self) {
        let Some(mut highlight) = self.one_shot_highlight.take() else {
            return;
        };
        if self.session.buffer(highlight.buffer_id).is_none() {
            return;
        }
        if highlight.buffer_id != self.session.active_id() {
            self.one_shot_highlight = Some(highlight);
            return;
        }

        if highlight.remaining_frames > 1 {
            highlight.remaining_frames -= 1;
            self.one_shot_highlight = Some(highlight);
        }
    }

    fn set_one_shot_highlight(&mut self, selection: Selection, mode: VisualModeKind) {
        self.one_shot_highlight = Some(OneShotHighlight {
            buffer_id: self.session.active_id(),
            selection,
            mode,
            remaining_frames: 2,
        });
    }

    pub fn pump_active_loading(&mut self, viewport_height_rows: usize) {
        let active_id = self.session.active_id();
        let before_len_chars = self.session.active_buffer().len_chars();
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
        if self.session.active_buffer().len_chars() != before_len_chars {
            self.invalidate_active_render_caches();
        }
    }

    fn ensure_active_fully_loaded_for_edit_or_save(&mut self) -> bool {
        let active_id = self.session.active_id();
        let before_len_chars = self.session.active_buffer().len_chars();
        match self.session.ensure_buffer_fully_loaded(active_id) {
            Ok(()) => {
                if self.session.active_buffer().len_chars() != before_len_chars {
                    self.invalidate_active_render_caches();
                }
                true
            }
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

    pub fn active_visual_selection(&self) -> Option<(Selection, VisualModeKind)> {
        let is_visual = matches!(
            self.mode,
            EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
        );
        if !is_visual {
            return None;
        }

        let id = self.session.active_id();
        let view = self.views.get(&id)?;
        let anchor = view.visual_anchor?;
        let mode = match self.mode {
            EditorMode::Visual => VisualModeKind::Char,
            EditorMode::VisualLine => VisualModeKind::Line,
            EditorMode::VisualBlock => VisualModeKind::Block,
            EditorMode::Normal | EditorMode::Insert | EditorMode::Command | EditorMode::Search => {
                return None;
            }
        };
        Some((Selection::new(anchor, view.cursor.cursor), mode))
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

    fn invalidate_active_render_caches(&mut self) {
        let active_id = self.session.active_id();
        let version = {
            let view = self.views.entry(active_id).or_default();
            view.invalidate_render_caches();
            view.analysis_version
        };
        self.request_analysis(active_id, version);
        if let Some(search) = self.search_state.as_mut()
            && search.buffer_id == active_id
        {
            search.dirty = true;
        }
    }

    fn request_analysis(&self, buffer_id: BufferId, version: u64) {
        let Some(meta) = self.session.meta(buffer_id) else {
            return;
        };
        if meta.kind != BufferKind::File {
            return;
        }
        let Some(buffer) = self.session.buffer(buffer_id).cloned() else {
            return;
        };
        let syntax_language = language_for_path(meta.path.as_deref());
        self.analysis_worker
            .request(buffer_id, version, buffer, syntax_language);
    }

    pub(super) fn ensure_buffer_analysis(&mut self, buffer_id: BufferId) {
        let Some(meta) = self.session.meta(buffer_id) else {
            return;
        };
        if meta.kind != BufferKind::File {
            return;
        }

        let syntax_language = language_for_path(meta.path.as_deref());
        let version = {
            let view = self.views.entry(buffer_id).or_default();
            let needs_delimiters = view.delimiter_pair_cache.get().is_none();
            let needs_syntax = syntax_language
                .map(|language| !view.syntax_highlighter.has_cache_for(language))
                .unwrap_or(false);
            if !needs_delimiters && !needs_syntax {
                return;
            }
            view.analysis_version
        };
        self.request_analysis(buffer_id, version);
    }

    pub fn poll_analysis_results(&mut self) {
        while let Some(result) = self.analysis_worker.try_recv() {
            self.apply_analysis_result(result);
        }
    }

    fn apply_analysis_result(&mut self, result: analysis::AnalysisResult) {
        match result {
            analysis::AnalysisResult::Syntax {
                buffer_id,
                version,
                syntax_cache,
            } => {
                let Some(view) = self.views.get_mut(&buffer_id) else {
                    return;
                };
                if view.analysis_version != version {
                    return;
                }

                view.syntax_highlighter.replace_cache(syntax_cache);
            }
            analysis::AnalysisResult::Delimiters {
                buffer_id,
                version,
                delimiter_analysis,
            } => {
                let Some(view) = self.views.get_mut(&buffer_id) else {
                    return;
                };
                if view.analysis_version != version {
                    return;
                }

                view.delimiter_pair_cache.install(delimiter_analysis);
            }
        }
    }

    fn record_active_undo_if_changed(&mut self, before: UndoSnapshot) -> bool {
        let active_id = self.session.active_id();
        let after_buffer = self.session.active_buffer();
        if Self::buffers_equal(after_buffer, &before.buffer) {
            return false;
        }

        let view = self.views.entry(active_id).or_default();
        let duplicate_last = view.undo_history.undo_stack.last().is_some_and(|last| {
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
            self.set_status("nothing to undo");
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
            self.set_status("nothing to redo");
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

        {
            let active_id = self.session.active_id();
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            view.cursor.cursor = buffer.clamp_pos(snapshot.cursor);
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
            view.visual_anchor = None;
            view.insert_mode_coalesce_base = None;
        }

        self.mode = EditorMode::Normal;
        let _ = self.session.recompute_active_dirty();
        self.invalidate_active_render_caches();
        self.clear_status();
    }
}

#[cfg(test)]
mod tests;
