//! Editor state and action application for `redox-tui`.
//!
//! This module keeps UI-facing state (mode, command line, status, cursor viewport
//! reconciliation) while delegating text editing primitives to `redox-core`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use minui::{TabPolicy, cell_width};
use redox_core::{BufferId, BufferKind, EditorSession, Pos, Selection, TextBuffer, VisualModeKind};
use unicode_segmentation::UnicodeSegmentation;

use crate::input::cursor::CursorController;
use crate::input::{InputMode, InputState};
use crate::ui::overlays::DelimiterPairCache;
use crate::ui::{
    GraphemeCache, RainAnimation, STATUS_BAR_HEIGHT_ROWS, SyntaxHighlighter, language_for_path,
};
mod about;
pub use about::AboutPopup;
use about::AboutState;
mod analysis;
use analysis::AnalysisWorker;
mod explorer;
mod finder;
mod git;
mod lsp;
mod rain_mode;
pub use explorer::ExplorerPopup;
use explorer::ExplorerState;
use finder::{FinderIndexWorker, FinderState, PinSelectorState, PinnedFilesState};
pub use finder::{FinderPopup, FinderPreview, PinSelectorPopup};
pub use git::{GitDiffSnapshot, GitFileStatusKind, GitGutterKind};
pub use lsp::{
    CodeActionPopup, CodeActionPopupEntry, CompletionEntry, CompletionPopup, DiagnosticLine,
    DiagnosticSeverity, DiagnosticsCodeActionsPane, DiagnosticsPopup, DiagnosticsPopupFocus,
    LspEntryStatusKind, LspMarketplacePopup, SymbolInfoBlock, SymbolInfoDisplayKind,
    SymbolInfoDisplayLine, SymbolInfoKind, SymbolInfoPopup,
};
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
const STATUS_MESSAGE_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(test)]
pub(crate) fn global_test_state_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

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

#[derive(Debug, Default, Clone)]
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
    AfterMatch,
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

#[derive(Debug, Default, Clone)]
struct CommandHistoryState {
    entries: Vec<String>,
    nav_index: Option<usize>,
    draft: String,
}

/// Vim-like editor mode for the TUI frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Normal,
    Insert,
    Command,
    Search,
    Finder,
    PinSelect,
    LspMarketplace,
    DiagnosticsList,
    CodeActions,
    SymbolInfo,
    Visual,
    VisualLine,
    VisualBlock,
}

impl EditorMode {
    pub fn has_popup_overlay(self) -> bool {
        matches!(
            self,
            EditorMode::Command
                | EditorMode::Search
                | EditorMode::Finder
                | EditorMode::PinSelect
                | EditorMode::LspMarketplace
                | EditorMode::DiagnosticsList
                | EditorMode::CodeActions
                | EditorMode::SymbolInfo
        )
    }

    pub fn as_input_mode(self) -> InputMode {
        match self {
            EditorMode::Normal => InputMode::Normal,
            EditorMode::Insert => InputMode::Insert,
            EditorMode::Command => InputMode::Command,
            EditorMode::Search => InputMode::Search,
            EditorMode::Finder => InputMode::Finder,
            EditorMode::PinSelect => InputMode::PinSelect,
            EditorMode::LspMarketplace => InputMode::LspMarketplace,
            EditorMode::DiagnosticsList => InputMode::DiagnosticsList,
            EditorMode::CodeActions => InputMode::CodeActions,
            EditorMode::SymbolInfo => InputMode::SymbolInfo,
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

impl Clone for BufferViewState {
    fn clone(&self) -> Self {
        Self {
            cursor: self.cursor.clone(),
            grapheme_cache: GraphemeCache::new(512),
            syntax_highlighter: SyntaxHighlighter::default(),
            delimiter_pair_cache: DelimiterPairCache::default(),
            visual_anchor: self.visual_anchor,
            analysis_version: self.analysis_version,
            undo_history: self.undo_history.clone(),
            insert_mode_coalesce_base: self.insert_mode_coalesce_base.clone(),
        }
    }
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
    pub(crate) fn copy_pane_state_from(&mut self, source: &Self) {
        self.cursor = source.cursor.clone();
        self.visual_anchor = source.visual_anchor;
    }

    fn reset_pane_position(&mut self) {
        self.cursor = CursorController::new();
        self.visual_anchor = None;
    }

    pub(crate) fn analysis_version(&self) -> u64 {
        self.analysis_version
    }

    fn invalidate_render_caches(&mut self) {
        self.grapheme_cache.clear();
        self.syntax_highlighter.mark_cache_stale();
        self.delimiter_pair_cache.mark_stale();
        self.analysis_version = self.analysis_version.wrapping_add(1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMessageStyle {
    Normal,
    Dim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Left,
    Down,
    Up,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneId(usize);

#[derive(Debug, Clone)]
pub struct EditorPane {
    pub id: PaneId,
    pub buffer_id: BufferId,
    pub view: BufferViewState,
    pub last_used: u64,
}

#[derive(Debug, Clone)]
enum SplitNode {
    Pane(PaneId),
    Split {
        axis: SplitAxis,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct PaneRect {
    pub pane_id: PaneId,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Multi-buffer editor state for the TUI frontend.
#[derive(Debug)]
pub struct EditorState {
    pub session: EditorSession,
    pub views: HashMap<BufferId, BufferViewState>,
    about: Option<AboutState>,
    explorer: Option<ExplorerState>,
    finder: Option<FinderState>,
    finder_index_worker: Option<FinderIndexWorker>,
    finder_index_files: Vec<finder::FinderFileCandidate>,
    finder_index_cache: HashMap<PathBuf, Vec<finder::FinderFileCandidate>>,
    pin_selector: Option<PinSelectorState>,
    pinned_files: PinnedFilesState,
    lsp: lsp::LspState,
    pub mode: EditorMode,
    pub input: InputState,
    pub command_line: String,
    pub command_line_cursor: usize,
    pub status_msg: Option<String>,
    pub status_msg_line_styles: Vec<StatusMessageStyle>,
    status_msg_expires_at: Option<Instant>,
    command_history: CommandHistoryState,
    pub should_quit: bool,
    rain_animation: Option<RainAnimation>,
    rain_pending_start: bool,
    viewport_width_cells: usize,
    viewport_height_rows: usize,
    editor_area_width_cells: usize,
    editor_area_height_rows: usize,
    private_register: String,
    private_register_kind: RegisterKind,
    one_shot_highlight: Option<OneShotHighlight>,
    search_state: Option<SearchState>,
    pending_system_clipboard: Option<String>,
    explorer_delete_confirmation_token: Option<String>,
    transient_origin_buffer_id: Option<BufferId>,
    transient_origin_dir: Option<PathBuf>,
    analysis_worker: AnalysisWorker,
    git: git::GitState,
    perf_visible: bool,
    perf_stats: Option<FramePerfStats>,
    panes: Vec<EditorPane>,
    split_root: SplitNode,
    active_pane: PaneId,
    next_pane_id: usize,
    pane_use_tick: u64,
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

        let initial_view = BufferViewState::default();
        let initial_pane = EditorPane {
            id: PaneId(0),
            buffer_id: active,
            view: initial_view.clone(),
            last_used: 1,
        };
        let state = Self {
            session,
            views,
            about: None,
            explorer: None,
            finder: None,
            finder_index_worker: None,
            finder_index_files: Vec::new(),
            finder_index_cache: HashMap::new(),
            pin_selector: None,
            pinned_files: PinnedFilesState::load(),
            lsp: lsp::LspState::default(),
            mode: EditorMode::Normal,
            input: InputState::new(),
            command_line: String::new(),
            command_line_cursor: 0,
            status_msg: None,
            status_msg_line_styles: Vec::new(),
            status_msg_expires_at: None,
            command_history: CommandHistoryState::default(),
            should_quit: false,
            rain_animation: None,
            rain_pending_start: false,
            viewport_width_cells: 80,
            viewport_height_rows: 24,
            editor_area_width_cells: 80,
            editor_area_height_rows: 23,
            private_register: String::new(),
            private_register_kind: RegisterKind::CharWise,
            one_shot_highlight: None,
            search_state: None,
            pending_system_clipboard: None,
            explorer_delete_confirmation_token: None,
            transient_origin_buffer_id: None,
            transient_origin_dir: None,
            analysis_worker: AnalysisWorker::new(),
            git: git::GitState::default(),
            perf_visible: false,
            perf_stats: None,
            panes: vec![initial_pane],
            split_root: SplitNode::Pane(PaneId(0)),
            active_pane: PaneId(0),
            next_pane_id: 1,
            pane_use_tick: 1,
        };
        let mut state = state;
        state.request_analysis(active, 0);
        state.initialise_lsp_state();
        state
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
        self.status_msg_line_styles.clear();
        self.status_msg_expires_at = Some(Instant::now() + STATUS_MESSAGE_TIMEOUT);
    }

    pub fn set_status_sticky_lines(&mut self, lines: Vec<(String, StatusMessageStyle)>) {
        let (lines, styles): (Vec<_>, Vec<_>) = lines.into_iter().unzip();
        self.status_msg = Some(lines.join("\n"));
        self.status_msg_line_styles = styles;
        self.status_msg_expires_at = None;
    }

    pub fn set_status_lines(&mut self, lines: Vec<(String, StatusMessageStyle)>) {
        let (lines, styles): (Vec<_>, Vec<_>) = lines.into_iter().unzip();
        self.status_msg = Some(lines.join("\n"));
        self.status_msg_line_styles = styles;
        self.status_msg_expires_at = Some(Instant::now() + STATUS_MESSAGE_TIMEOUT);
    }

    pub fn clear_status(&mut self) {
        self.status_msg = None;
        self.status_msg_line_styles.clear();
        self.status_msg_expires_at = None;
    }

    pub fn expire_status_message(&mut self, now: Instant) {
        if self
            .status_msg_expires_at
            .is_some_and(|expires_at| now >= expires_at)
        {
            self.clear_status();
        }
    }

    #[cfg(test)]
    pub(crate) fn status_message_is_sticky(&self) -> bool {
        self.status_msg.is_some() && self.status_msg_expires_at.is_none()
    }

    pub fn set_viewport_size(&mut self, width_cells: usize, height_rows: usize) {
        self.viewport_width_cells = width_cells;
        self.viewport_height_rows = height_rows;
    }

    pub fn set_editor_area_size(&mut self, width_cells: usize, height_rows: usize) {
        self.editor_area_width_cells = width_cells;
        self.editor_area_height_rows = height_rows;
    }

    pub fn viewport_size(&self) -> (usize, usize) {
        (self.viewport_width_cells, self.viewport_height_rows)
    }

    pub fn sync_active_pane_view(&mut self) {
        let active_id = self.session.active_id();
        if self
            .session
            .meta(active_id)
            .is_some_and(|meta| meta.kind == BufferKind::Ui)
        {
            return;
        }
        if let Some(view) = self.views.get(&active_id).cloned()
            && let Some(pane) = self
                .panes
                .iter_mut()
                .find(|pane| pane.id == self.active_pane)
        {
            pane.buffer_id = active_id;
            pane.view = view;
        }
    }

    pub fn sync_rendered_pane_view(&mut self, pane_id: PaneId, buffer_id: BufferId) {
        if let Some(view) = self.views.get(&buffer_id)
            && let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == pane_id)
        {
            pane.view.copy_pane_state_from(view);
        }
    }

    pub fn restore_active_pane_view(&mut self) {
        let Some(pane) = self
            .panes
            .iter()
            .find(|pane| pane.id == self.active_pane)
            .cloned()
        else {
            return;
        };
        self.views
            .entry(pane.buffer_id)
            .or_default()
            .copy_pane_state_from(&pane.view);
        let _ = self.session.activate(pane.buffer_id);
    }

    fn activate_pane(&mut self, pane_id: PaneId) -> bool {
        self.activate_pane_with_recent(pane_id, true)
    }

    fn activate_pane_with_recent(&mut self, pane_id: PaneId, mark_recent: bool) -> bool {
        if pane_id == self.active_pane {
            return true;
        }
        self.sync_active_pane_view();
        let Some(pane) = self.panes.iter().find(|pane| pane.id == pane_id).cloned() else {
            return false;
        };
        if !self.session.activate(pane.buffer_id) {
            return false;
        }
        self.views
            .entry(pane.buffer_id)
            .or_default()
            .copy_pane_state_from(&pane.view);
        self.active_pane = pane_id;
        if mark_recent {
            self.pane_use_tick = self.pane_use_tick.saturating_add(1);
            if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == pane_id) {
                pane.last_used = self.pane_use_tick;
            }
        }
        self.clear_active_visual_anchor();
        self.close_completion();
        true
    }

    pub fn split_active_pane(&mut self, axis: SplitAxis) {
        self.sync_active_pane_view();
        self.nudge_active_pane_cursor_before_split(axis);
        let Some(active) = self
            .panes
            .iter()
            .find(|pane| pane.id == self.active_pane)
            .cloned()
        else {
            return;
        };
        let new_id = PaneId(self.next_pane_id);
        self.next_pane_id = self.next_pane_id.saturating_add(1);
        let mut new_view = active.view.clone();
        new_view.reset_pane_position();
        self.panes.push(EditorPane {
            id: new_id,
            buffer_id: active.buffer_id,
            view: new_view,
            last_used: 0,
        });
        if replace_pane_with_split(&mut self.split_root, self.active_pane, axis, new_id) {
            let _ = self.activate_pane(new_id);
            self.refresh_active_split_viewport_size();
        }
    }

    fn nudge_active_pane_cursor_before_split(&mut self, axis: SplitAxis) {
        let Some(pane_index) = self
            .panes
            .iter()
            .position(|pane| pane.id == self.active_pane)
        else {
            return;
        };
        let buffer_id = self.panes[pane_index].buffer_id;
        let Some(buffer) = self.session.buffer(buffer_id).cloned() else {
            return;
        };
        let Some(rect) = self
            .pane_rects(
                self.editor_area_width_cells as u16,
                self.editor_area_height_rows as u16,
            )
            .into_iter()
            .find(|rect| rect.pane_id == self.active_pane)
        else {
            return;
        };
        let total_lines = buffer.len_lines().max(1);
        let show_git_marker_column = self
            .git
            .diff_for(buffer_id)
            .is_some_and(|diff| !diff.stats.is_empty());
        let content_x = split_gutter_width(total_lines, show_git_marker_column).saturating_add(1);
        let view = &mut self.panes[pane_index].view;
        nudge_cursor_out_of_new_split_area(&buffer, view, axis, rect, content_x);
        let adjusted = view.clone();
        self.views
            .entry(buffer_id)
            .or_default()
            .copy_pane_state_from(&adjusted);
    }

    pub fn close_active_split(&mut self) {
        if self.panes.len() <= 1 {
            self.set_status("cannot close the last split");
            return;
        }
        let closing = self.active_pane;
        let next = self
            .panes
            .iter()
            .filter(|pane| pane.id != closing)
            .max_by_key(|pane| pane.last_used)
            .map(|pane| pane.id);
        if remove_pane_from_split(&mut self.split_root, closing) {
            self.panes.retain(|pane| pane.id != closing);
            let next = next.unwrap_or_else(|| first_pane_id(&self.split_root));
            let _ = self.activate_pane(next);
            self.refresh_active_split_viewport_size();
        }
    }

    pub fn focus_split(&mut self, direction: SplitDirection) {
        let rects = self.pane_rects(
            self.editor_area_width_cells as u16,
            self.editor_area_height_rows as u16,
        );
        let Some(current) = rects
            .iter()
            .find(|rect| rect.pane_id == self.active_pane)
            .copied()
        else {
            return;
        };
        let current_left = i32::from(current.x);
        let current_top = i32::from(current.y);
        let current_right = current_left + i32::from(current.width);
        let current_bottom = current_top + i32::from(current.height);
        let candidate = rects
            .iter()
            .filter(|rect| rect.pane_id != self.active_pane)
            .filter_map(|rect| {
                let left = i32::from(rect.x);
                let top = i32::from(rect.y);
                let right = left + i32::from(rect.width);
                let bottom = top + i32::from(rect.height);
                let primary_gap = match direction {
                    SplitDirection::Left => (right <= current_left).then_some(current_left - right),
                    SplitDirection::Right => {
                        (left >= current_right).then_some(left - current_right)
                    }
                    SplitDirection::Up => (bottom <= current_top).then_some(current_top - bottom),
                    SplitDirection::Down => (top >= current_bottom).then_some(top - current_bottom),
                }?;
                let overlap = match direction {
                    SplitDirection::Left | SplitDirection::Right => {
                        current_bottom.min(bottom) - current_top.max(top)
                    }
                    SplitDirection::Up | SplitDirection::Down => {
                        current_right.min(right) - current_left.max(left)
                    }
                };
                let secondary_gap = match direction {
                    SplitDirection::Left | SplitDirection::Right if overlap > 0 => 0,
                    SplitDirection::Left | SplitDirection::Right => {
                        if bottom <= current_top {
                            current_top - bottom
                        } else {
                            top - current_bottom
                        }
                    }
                    SplitDirection::Up | SplitDirection::Down if overlap > 0 => 0,
                    SplitDirection::Up | SplitDirection::Down => {
                        if right <= current_left {
                            current_left - right
                        } else {
                            left - current_right
                        }
                    }
                };
                let last_used = self
                    .panes
                    .iter()
                    .find(|pane| pane.id == rect.pane_id)
                    .map(|pane| pane.last_used)
                    .unwrap_or(0);
                let geometry_bias = i32::from(rect.y) * 10_000 + i32::from(rect.x);
                Some((
                    primary_gap,
                    secondary_gap,
                    std::cmp::Reverse(last_used),
                    -overlap,
                    geometry_bias,
                    rect.pane_id,
                ))
            })
            .min_by_key(|(primary, secondary, recent, overlap, geometry, _)| {
                (*primary, *recent, *secondary, *overlap, *geometry)
            })
            .map(|(_, _, _, _, _, pane_id)| pane_id);
        if let Some(pane_id) = candidate {
            let _ = self.activate_pane(pane_id);
            self.refresh_active_split_viewport_size();
        }
    }

    fn refresh_active_split_viewport_size(&mut self) {
        if self.panes.len() <= 1 {
            self.viewport_width_cells = self.editor_area_width_cells;
            self.viewport_height_rows = self
                .editor_area_height_rows
                .saturating_add(STATUS_BAR_HEIGHT_ROWS);
            return;
        }

        let Some(rect) = self
            .pane_rects(
                self.editor_area_width_cells as u16,
                self.editor_area_height_rows as u16,
            )
            .into_iter()
            .find(|rect| rect.pane_id == self.active_pane)
        else {
            return;
        };
        self.viewport_width_cells = rect.width as usize;
        self.viewport_height_rows = (rect.height as usize).saturating_add(STATUS_BAR_HEIGHT_ROWS);
    }

    pub fn pane_rects(&self, width: u16, height: u16) -> Vec<PaneRect> {
        let mut rects = Vec::new();
        collect_pane_rects(&self.split_root, 0, 0, width, height, &mut rects);
        rects
    }

    pub fn active_pane_id(&self) -> PaneId {
        self.active_pane
    }

    pub fn panes(&self) -> &[EditorPane] {
        &self.panes
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

    pub fn active_git_diff(&self) -> Option<&GitDiffSnapshot> {
        self.git.diff_for(self.session.active_id())
    }

    pub fn git_diff_for_buffer(&self, buffer_id: BufferId) -> Option<&GitDiffSnapshot> {
        self.git.diff_for(buffer_id)
    }

    pub fn git_status_for_path(&self, path: &std::path::Path) -> Option<GitFileStatusKind> {
        self.git.status_for_path(path)
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
            EditorMode::Normal
            | EditorMode::Insert
            | EditorMode::Command
            | EditorMode::Search
            | EditorMode::Finder
            | EditorMode::PinSelect
            | EditorMode::LspMarketplace => return None,
            EditorMode::DiagnosticsList | EditorMode::CodeActions | EditorMode::SymbolInfo => {
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
        self.invalidate_buffer_render_caches(active_id);
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
            let needs_delimiters = !view.delimiter_pair_cache.has_fresh_analysis();
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

    fn invalidate_buffer_render_caches(&mut self, buffer_id: BufferId) {
        let version = {
            let view = self.views.entry(buffer_id).or_default();
            view.invalidate_render_caches();
            view.analysis_version
        };
        self.git.mark_stale(buffer_id);
        self.request_analysis(buffer_id, version);
        if let Some(search) = self.search_state.as_mut()
            && search.buffer_id == buffer_id
        {
            search.dirty = true;
        }
    }

    pub fn poll_analysis_results(&mut self) {
        while let Some(result) = self.analysis_worker.try_recv() {
            self.apply_analysis_result(result);
        }
    }

    pub fn refresh_active_git_diff(&mut self) {
        let active_id = self.session.active_id();
        self.refresh_git_diff_for_buffer(active_id);
    }

    pub fn refresh_git_diff_for_buffer(&mut self, buffer_id: BufferId) {
        self.git.refresh_for_buffer(&self.session, buffer_id);
    }

    pub fn refresh_git_repo_status_for_dir(&mut self, dir: &std::path::Path) {
        self.git.refresh_repo_status_for_dir(dir);
    }

    pub fn mark_git_repo_statuses_stale(&mut self) {
        self.git.mark_all_repo_statuses_stale();
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

fn replace_pane_with_split(
    node: &mut SplitNode,
    target: PaneId,
    axis: SplitAxis,
    new_id: PaneId,
) -> bool {
    match node {
        SplitNode::Pane(id) if *id == target => {
            *node = SplitNode::Split {
                axis,
                first: Box::new(SplitNode::Pane(target)),
                second: Box::new(SplitNode::Pane(new_id)),
            };
            true
        }
        SplitNode::Pane(_) => false,
        SplitNode::Split { first, second, .. } => {
            replace_pane_with_split(first, target, axis, new_id)
                || replace_pane_with_split(second, target, axis, new_id)
        }
    }
}

fn remove_pane_from_split(node: &mut SplitNode, target: PaneId) -> bool {
    match node {
        SplitNode::Pane(_) => false,
        SplitNode::Split { first, second, .. } => {
            if matches!(first.as_ref(), SplitNode::Pane(id) if *id == target) {
                *node = (**second).clone();
                return true;
            }
            if matches!(second.as_ref(), SplitNode::Pane(id) if *id == target) {
                *node = (**first).clone();
                return true;
            }
            remove_pane_from_split(first, target) || remove_pane_from_split(second, target)
        }
    }
}

fn first_pane_id(node: &SplitNode) -> PaneId {
    match node {
        SplitNode::Pane(id) => *id,
        SplitNode::Split { first, .. } => first_pane_id(first),
    }
}

fn collect_pane_rects(
    node: &SplitNode,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    rects: &mut Vec<PaneRect>,
) {
    match node {
        SplitNode::Pane(pane_id) => rects.push(PaneRect {
            pane_id: *pane_id,
            x,
            y,
            width,
            height,
        }),
        SplitNode::Split {
            axis: SplitAxis::Vertical,
            first,
            second,
        } => {
            let first_w = width / 2;
            let second_w = width.saturating_sub(first_w).saturating_sub(1);
            collect_pane_rects(first, x, y, first_w, height, rects);
            collect_pane_rects(
                second,
                x.saturating_add(first_w).saturating_add(1),
                y,
                second_w,
                height,
                rects,
            );
        }
        SplitNode::Split {
            axis: SplitAxis::Horizontal,
            first,
            second,
        } => {
            let first_h = height / 2;
            let second_h = height.saturating_sub(first_h).saturating_sub(1);
            collect_pane_rects(first, x, y, width, first_h, rects);
            collect_pane_rects(
                second,
                x,
                y.saturating_add(first_h).saturating_add(1),
                width,
                second_h,
                rects,
            );
        }
    }
}

fn nudge_cursor_out_of_new_split_area(
    buffer: &TextBuffer,
    view: &mut BufferViewState,
    axis: SplitAxis,
    rect: PaneRect,
    content_x: u16,
) {
    match axis {
        SplitAxis::Horizontal => {
            let retained_height = rect.height / 2;
            if retained_height == 0 {
                return;
            }
            let spec = view.cursor.cursor_spec(
                buffer,
                rect.width.saturating_sub(content_x) as usize,
                rect.height as usize,
            );
            if !spec.visible || spec.y < retained_height {
                return;
            }

            let (_, scroll_y) = view.cursor.viewport_scroll();
            let target_line = scroll_y.saturating_add(retained_height.saturating_sub(1) as usize);
            view.cursor.cursor = buffer.clamp_pos(Pos::new(target_line, view.cursor.cursor.col));
        }
        SplitAxis::Vertical => {
            let retained_width = rect.width / 2;
            if retained_width == 0 {
                return;
            }
            let text_width = rect.width.saturating_sub(content_x);
            let spec = view
                .cursor
                .cursor_spec(buffer, text_width as usize, rect.height as usize);
            let cursor_screen_x = content_x.saturating_add(spec.x);
            if !spec.visible || cursor_screen_x < retained_width {
                return;
            }

            let (scroll_x, _) = view.cursor.viewport_scroll();
            let target_cell =
                retained_width.saturating_sub(1).saturating_sub(content_x) as usize + scroll_x;
            let line = buffer.clamp_line(view.cursor.cursor.line);
            let col = char_col_at_or_before_cell(&buffer.line_string(line), target_cell);
            view.cursor.cursor = buffer.clamp_pos(Pos::new(line, col));
        }
    }
}

fn split_gutter_width(total_lines: usize, show_git_marker_column: bool) -> u16 {
    let digits = total_lines.max(1).ilog10() as u16 + 1;
    let git_marker_width = u16::from(show_git_marker_column);
    digits.saturating_add(git_marker_width).saturating_add(1)
}

fn char_col_at_or_before_cell(line: &str, target_cell: usize) -> usize {
    let mut cells = 0usize;
    let mut chars = 0usize;
    for grapheme in line.graphemes(true) {
        let width = (cell_width(grapheme, TabPolicy::Fixed(4)) as usize).max(1);
        if cells.saturating_add(width) > target_cell {
            return chars;
        }
        cells = cells.saturating_add(width);
        chars = chars.saturating_add(grapheme.chars().count());
    }
    chars
}

#[cfg(test)]
mod tests;
