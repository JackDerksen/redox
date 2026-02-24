//! Editor state and action application for `editor_tui`.
//!
//! This module keeps UI-facing state (mode, command line, status, cursor viewport
//! reconciliation) while delegating text editing primitives to `editor_core`.

use std::collections::HashMap;
use std::path::PathBuf;

use editor_core::{BufferId, EditorSession, Pos, Selection, TextBuffer};

use crate::input::cursor::CursorController;
use crate::input::{InputAction, InputMode, InputState, InsertKind};
use crate::ui::{GraphemeCache, STATUS_BAR_HEIGHT_ROWS};
mod explorer;
pub use explorer::ExplorerPopup;
use explorer::ExplorerState;

/// Vim-like editor mode for the TUI frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Normal,
    Insert,
    Command,
}

impl EditorMode {
    pub fn as_input_mode(self) -> InputMode {
        match self {
            EditorMode::Normal => InputMode::Normal,
            EditorMode::Insert => InputMode::Insert,
            EditorMode::Command => InputMode::Command,
        }
    }
}

/// Per-buffer UI/view state that is not persisted in `editor_core`.
#[derive(Debug)]
pub struct BufferViewState {
    pub cursor: CursorController,
    pub grapheme_cache: GraphemeCache,
}

impl Default for BufferViewState {
    fn default() -> Self {
        Self {
            cursor: CursorController::new(),
            grapheme_cache: GraphemeCache::new(512),
        }
    }
}

/// Multi-buffer editor state for the TUI frontend.
#[derive(Debug)]
pub struct EditorState {
    pub session: EditorSession,
    pub views: HashMap<BufferId, BufferViewState>,
    explorer: Option<ExplorerState>,
    pub mode: EditorMode,
    pub input: InputState,
    pub command_line: String,
    pub status_msg: Option<String>,
    status_msg_ephemeral: bool,
    pub should_quit: bool,
    viewport_width_cells: usize,
    viewport_height_rows: usize,
}

impl EditorState {
    pub fn new(session: EditorSession) -> Self {
        let active = session.active_id();
        let mut views = HashMap::new();
        views.insert(active, BufferViewState::default());

        Self {
            session,
            views,
            explorer: None,
            mode: EditorMode::Normal,
            input: InputState::new(),
            command_line: String::new(),
            status_msg: None,
            status_msg_ephemeral: false,
            should_quit: false,
            viewport_width_cells: 80,
            viewport_height_rows: 24,
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

    /// Apply a high-level input action using the active viewport size for cursor reconciliation.
    pub fn apply_input(
        &mut self,
        action: InputAction,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        if self.status_msg_ephemeral {
            self.clear_status();
        }

        let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);

        match action {
            InputAction::Motion { motion, count } => {
                let active_id = self.session.active_id();
                let view = self.views.entry(active_id).or_default();
                let buffer = self.session.active_buffer();

                view.cursor
                    .apply_motion(buffer, motion, count, viewport_width_cells, text_vh);
            }

            InputAction::SetMode(mode) => {
                let leaving_insert_to_normal =
                    self.mode == EditorMode::Insert && mode == InputMode::Normal;

                self.mode = match mode {
                    InputMode::Normal => EditorMode::Normal,
                    InputMode::Insert => EditorMode::Insert,
                    InputMode::Command => EditorMode::Command,
                };

                if leaving_insert_to_normal {
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();

                    if view.cursor.cursor.col > 0 {
                        view.cursor.cursor.col -= 1;
                    }

                    let buffer = self.session.active_buffer();
                    view.cursor
                        .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                }

                self.input.reset_prefixes();
            }

            InputAction::EnterInsert(kind) => {
                self.mode = EditorMode::Insert;
                self.clear_status();
                self.input.reset_prefixes();

                {
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();
                    let buffer = self.session.active_buffer();

                    match kind {
                        InsertKind::Insert => {}
                        InsertKind::Append => {
                            let line = buffer.clamp_line(view.cursor.cursor.line);
                            let line_len_chars = buffer.line_len_chars(line);
                            if view.cursor.cursor.col < line_len_chars {
                                view.cursor.cursor.col += 1;
                            }
                        }
                        InsertKind::InsertLineStart => {
                            view.cursor.cursor.col = 0;
                        }
                        InsertKind::AppendLineEnd => {
                            let line = buffer.clamp_line(view.cursor.cursor.line);
                            view.cursor.cursor.col = buffer.line_len_chars(line);
                        }
                    }

                    view.cursor
                        .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                }
            }

            InputAction::OpenLineBelow => {
                if self.mode == EditorMode::Normal {
                    self.open_line_and_enter_insert(false, viewport_width_cells, text_vh);
                }
            }

            InputAction::OpenLineAbove => {
                if self.mode == EditorMode::Normal {
                    self.open_line_and_enter_insert(true, viewport_width_cells, text_vh);
                }
            }

            InputAction::EnterCommand => {
                self.mode = EditorMode::Command;
                self.command_line.clear();
                self.clear_status();
                self.input.reset_prefixes();
            }

            InputAction::CommandCancel => {
                self.mode = EditorMode::Normal;
                self.command_line.clear();
                self.input.reset_prefixes();
            }

            InputAction::CommandChar(c) => {
                if self.mode == EditorMode::Command {
                    self.command_line.push(c);
                }
            }

            InputAction::CommandBackspace => {
                if self.mode == EditorMode::Command {
                    self.command_line.pop();
                }
            }

            InputAction::CommandEnter => {
                self.execute_command_line();
            }

            InputAction::OpenExplorer => {
                if self.mode == EditorMode::Normal {
                    self.command_open_explorer();
                }
            }

            InputAction::SurfaceOpenSelected => {
                if self.mode == EditorMode::Normal {
                    self.surface_open_selected();
                }
            }

            InputAction::SurfaceGoParent => {
                if self.mode == EditorMode::Normal {
                    self.surface_go_parent();
                }
            }

            InputAction::InsertChar(c) => {
                if self.mode == EditorMode::Insert {
                    let s = c.to_string();
                    self.insert_text_at_cursor(&s, viewport_width_cells, text_vh);
                }
            }

            InputAction::Backspace => {
                if self.mode == EditorMode::Insert {
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();
                    let sel = Selection::empty(view.cursor.cursor);

                    {
                        let buffer = self.session.active_buffer_mut();
                        let sel = buffer.backspace(sel);
                        view.cursor.cursor = sel.cursor;
                        view.cursor
                            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                    }

                    let _ = self.session.recompute_active_dirty();
                }
            }

            InputAction::Enter => {
                if self.mode == EditorMode::Insert {
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();
                    let sel = Selection::empty(view.cursor.cursor);

                    {
                        let buffer = self.session.active_buffer_mut();
                        let sel = buffer.insert_newline(sel);
                        view.cursor.cursor = sel.cursor;
                        view.cursor
                            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                    }

                    let _ = self.session.recompute_active_dirty();
                }
            }

            InputAction::Paste(text) => match self.mode {
                EditorMode::Insert | EditorMode::Normal => {
                    self.insert_text_at_cursor(&text, viewport_width_cells, text_vh);
                }
                EditorMode::Command => {}
            },

            InputAction::None => {}
        }
    }

    fn insert_text_at_cursor(&mut self, text: &str, viewport_width_cells: usize, text_vh: usize) {
        if text.is_empty() {
            return;
        }

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();

        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.insert(view.cursor.cursor, text);
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        let _ = self.session.recompute_active_dirty();
    }

    fn execute_command_line(&mut self) {
        if self.mode != EditorMode::Command {
            return;
        }

        let cmd_raw = self.command_line.trim().to_string();
        self.command_line.clear();
        self.mode = EditorMode::Normal;

        if cmd_raw.is_empty() {
            return;
        }

        let mut parts = cmd_raw.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).unwrap_or("");

        match cmd {
            "w" => {
                self.write_current_file();
            }
            "q" => {
                if self.active_buffer_is_surface() {
                    if self.close_active_surface_buffer() {
                        self.clear_status();
                    } else {
                        self.set_status("cannot close the last buffer");
                    }
                    return;
                }

                if self.session.any_dirty() {
                    self.set_status(self.unsaved_changes_quit_message());
                } else {
                    self.should_quit = true;
                }
            }
            "q!" => {
                self.should_quit = true;
            }
            "wq" => {
                if self.write_current_file() {
                    if self.session.any_dirty() {
                        self.set_status(self.unsaved_changes_message());
                    } else {
                        self.should_quit = true;
                    }
                }
            }
            "e" => {
                self.command_edit(arg);
            }
            "bn" | "bnext" => {
                self.command_buffer_cycle_next();
            }
            "bp" | "bprev" => {
                self.command_buffer_cycle_prev();
            }
            "ls" => {
                self.command_list_buffers();
            }
            "ex" | "explorer" => {
                self.command_open_explorer();
            }
            _ => {
                self.set_status(format!("unknown command: {cmd_raw}"));
            }
        }
    }

    fn command_edit(&mut self, path_arg: &str) {
        if path_arg.is_empty() {
            self.set_status("usage: e <path>");
            return;
        }

        let path = PathBuf::from(path_arg);
        match self.session.open_file(path) {
            Ok(id) => {
                let _ = self.views.entry(id).or_default();
                self.clear_status();
            }
            Err(e) => {
                self.set_status(format!("open failed: {e}"));
            }
        }
    }

    fn command_buffer_cycle_next(&mut self) {
        let count = self.session.summaries().len();
        if count <= 1 {
            self.set_status("only one buffer");
            return;
        }

        if let Some(id) = self.session.switch_next_mru() {
            let _ = self.views.entry(id).or_default();
            self.clear_status();
        }
    }

    fn command_buffer_cycle_prev(&mut self) {
        let count = self.session.summaries().len();
        if count <= 1 {
            self.set_status("only one buffer");
            return;
        }

        if let Some(id) = self.session.switch_prev_mru() {
            let _ = self.views.entry(id).or_default();
            self.clear_status();
        }
    }

    fn command_list_buffers(&mut self) {
        let summaries = self.session.summaries();
        if summaries.is_empty() {
            self.set_status("no buffers");
            return;
        }

        let mut msg = String::new();

        for (idx, summary) in summaries.iter().enumerate() {
            if idx > 0 {
                msg.push_str(" | ");
            }

            let active = if summary.is_active { '%' } else { '-' };
            let dirty = if summary.dirty { '+' } else { '-' };
            let new_file = if summary.is_new_file { 'n' } else { '-' };
            msg.push_str(&format!(
                "[{active}{dirty}{new_file}]{}:{}",
                summary.id.get(),
                summary.display_name
            ));
        }

        self.set_status_ephemeral(msg);
    }

    fn write_current_file(&mut self) -> bool {
        if self.explorer_is_active() {
            return self.write_explorer_directory();
        }

        match self.session.save_active() {
            Ok(()) => {
                self.set_status("written");
                true
            }
            Err(e) => {
                self.set_status(format!("write failed: {e}"));
                false
            }
        }
    }

    fn open_line_and_enter_insert(
        &mut self,
        above: bool,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        self.mode = EditorMode::Insert;
        self.clear_status();
        self.input.reset_prefixes();

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();

        {
            let buffer = self.session.active_buffer_mut();
            let line = buffer.clamp_line(view.cursor.cursor.line);
            let insert_pos = if above {
                Pos::new(line, 0)
            } else {
                Pos::new(line, buffer.line_len_chars(line))
            };

            let sel = Selection::empty(insert_pos);
            let sel = buffer.insert_newline(sel);
            view.cursor.cursor = if above { Pos::new(line, 0) } else { sel.cursor };
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        let _ = self.session.recompute_active_dirty();
    }

    fn unsaved_changes_message(&self) -> String {
        let dirty: Vec<editor_core::BufferSummary> = self
            .session
            .summaries()
            .into_iter()
            .filter(|summary| summary.dirty)
            .collect();

        if dirty.is_empty() {
            return "unsaved changes".to_string();
        }

        let first_name = dirty[0].display_name.clone();
        if dirty.len() == 1 {
            format!("unsaved changes in {first_name}")
        } else {
            format!("unsaved changes in {first_name} (+{})", dirty.len() - 1)
        }
    }

    fn unsaved_changes_quit_message(&self) -> String {
        format!("{} (use :q! to quit)", self.unsaved_changes_message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::motion::Motion;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("redox_state_test_{tag}_{nanos}.txt"))
    }

    fn state_with_text(path: PathBuf, text: &str) -> EditorState {
        fs::write(&path, text).expect("failed to write test file");
        let session = EditorSession::open_initial_file(&path).expect("failed to open session");
        EditorState::new(session)
    }

    fn run_command(state: &mut EditorState, cmd: &str) {
        state.mode = EditorMode::Command;
        state.command_line = cmd.to_string();
        state.apply_input(InputAction::CommandEnter, 80, 24);
    }

    #[test]
    fn normal_mode_paste_inserts_text_and_marks_dirty() {
        let path = temp_file_path("paste_normal");
        let mut state = state_with_text(path.clone(), "hello");

        state.apply_input(InputAction::Paste(" world".to_string()), 80, 24);

        assert_eq!(state.session.active_buffer().to_string(), " worldhello");
        assert!(state.session.active_meta().dirty);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn switching_buffers_preserves_cursor_and_scroll_state() {
        let path_a = temp_file_path("switch_preserve_a");
        let path_b = temp_file_path("switch_preserve_b");
        let mut state = state_with_text(path_a.clone(), "aaaa\nbbbb\n");
        fs::write(&path_b, "cccc\ndddd\n").expect("failed to write test file");

        let id_a = state.session.active_id();
        {
            let view = state
                .views
                .get_mut(&id_a)
                .expect("missing view for buffer A");
            view.cursor.cursor = Pos::new(1, 2);
            view.cursor.scroll_x_cells = 4;
            view.cursor.scroll_y_lines = 1;
        }

        run_command(&mut state, &format!("e {}", path_b.display()));
        let id_b = state.session.active_id();

        {
            let view = state
                .views
                .get_mut(&id_b)
                .expect("missing view for buffer B");
            view.cursor.cursor = Pos::new(0, 3);
            view.cursor.scroll_x_cells = 7;
            view.cursor.scroll_y_lines = 0;
        }

        run_command(&mut state, "bp");

        assert_eq!(state.session.active_id(), id_a);
        let view_a = state.views.get(&id_a).expect("missing view for buffer A");
        assert_eq!(view_a.cursor.cursor, Pos::new(1, 2));
        assert_eq!(view_a.cursor.scroll_x_cells, 4);
        assert_eq!(view_a.cursor.scroll_y_lines, 1);

        let _ = fs::remove_file(path_a);
        let _ = fs::remove_file(path_b);
    }

    #[test]
    fn command_q_does_not_quit_when_hidden_buffer_is_dirty() {
        let path_a = temp_file_path("q_hidden_dirty_a");
        let path_b = temp_file_path("q_hidden_dirty_b");
        let mut state = state_with_text(path_a.clone(), "aaa");
        fs::write(&path_b, "bbb").expect("failed to write test file");

        run_command(&mut state, &format!("e {}", path_b.display()));
        run_command(&mut state, "bp");
        state.apply_input(InputAction::Paste("x".to_string()), 80, 24);
        run_command(&mut state, "bn");

        run_command(&mut state, "q");

        assert!(!state.should_quit);
        let msg = state.status_msg.as_deref().expect("missing quit warning");
        let leaf_a = path_a
            .file_name()
            .and_then(|name| name.to_str())
            .expect("path should have a file name");
        assert!(msg.contains("unsaved changes in"));
        assert!(msg.contains(leaf_a));
        assert!(msg.contains("use :q! to quit"));

        let _ = fs::remove_file(path_a);
        let _ = fs::remove_file(path_b);
    }

    #[test]
    fn command_w_writes_active_buffer_only() {
        let path_a = temp_file_path("write_active_a");
        let path_b = temp_file_path("write_active_b");
        let mut state = state_with_text(path_a.clone(), "alpha");
        fs::write(&path_b, "bravo").expect("failed to write test file");

        run_command(&mut state, &format!("e {}", path_b.display()));
        let id_b = state.session.active_id();

        state.apply_input(InputAction::Paste("Z".to_string()), 80, 24);
        assert!(state.session.meta(id_b).expect("missing meta").dirty);

        run_command(&mut state, "bp");
        run_command(&mut state, "w");

        assert!(state.session.meta(id_b).expect("missing meta").dirty);
        let on_disk_b = fs::read_to_string(&path_b).expect("failed to read file B");
        assert_eq!(on_disk_b, "bravo");

        let _ = fs::remove_file(path_a);
        let _ = fs::remove_file(path_b);
    }

    #[test]
    fn command_ls_populates_compact_status_summary() {
        let path_a = temp_file_path("ls_a");
        let path_b = temp_file_path("ls_b");
        let mut state = state_with_text(path_a.clone(), "alpha");
        fs::write(&path_b, "bravo").expect("failed to write test file");

        state.apply_input(InputAction::Paste("!".to_string()), 80, 24);
        run_command(&mut state, &format!("e {}", path_b.display()));
        run_command(&mut state, "ls");

        let msg = state.status_msg.as_deref().expect("missing ls status");
        assert!(msg.contains("|"));
        assert!(msg.contains("%"));
        assert!(msg.contains("+"));
        assert!(msg.contains(" | "));
        for summary in state.session.summaries() {
            assert!(msg.contains(&summary.display_name));
        }

        let _ = fs::remove_file(path_a);
        let _ = fs::remove_file(path_b);
    }

    #[test]
    fn command_ls_status_is_cleared_on_next_input() {
        let path = temp_file_path("ls_ephemeral");
        let mut state = state_with_text(path.clone(), "alpha");

        run_command(&mut state, "ls");
        assert!(state.status_msg.is_some());

        state.apply_input(
            InputAction::Motion {
                motion: Motion::Right,
                count: 1,
            },
            80,
            24,
        );

        assert!(state.status_msg.is_none());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn command_e_uses_trimmed_remainder_as_path() {
        let path_a = temp_file_path("e_trimmed_a");
        let path_b = temp_file_path("e_trimmed_b");
        let mut state = state_with_text(path_a.clone(), "alpha");
        fs::write(&path_b, "bravo").expect("failed to write test file");

        run_command(&mut state, &format!("e    {}", path_b.display()));

        assert_eq!(state.session.active_buffer().to_string(), "bravo");

        let _ = fs::remove_file(path_a);
        let _ = fs::remove_file(path_b);
    }

    #[test]
    fn dirty_tracking_clears_after_reverting_to_original_content() {
        let path = temp_file_path("dirty_revert_state");
        let mut state = state_with_text(path.clone(), "hello");

        state.apply_input(InputAction::Paste("x".to_string()), 80, 24);
        assert!(state.session.active_meta().dirty);

        state.apply_input(InputAction::EnterInsert(InsertKind::Insert), 80, 24);
        state.apply_input(InputAction::Backspace, 80, 24);
        assert!(!state.session.active_meta().dirty);

        run_command(&mut state, "q");
        assert!(state.should_quit);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn unknown_command_sets_status_message() {
        let path = temp_file_path("unknown_command");
        let mut state = state_with_text(path.clone(), "alpha");

        run_command(&mut state, "zzzz");

        assert_eq!(state.status_msg.as_deref(), Some("unknown command: zzzz"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn explorer_command_opens_ui_buffer() {
        let path = temp_file_path("explorer_open");
        let mut state = state_with_text(path.clone(), "alpha");

        run_command(&mut state, "explorer");

        assert!(state.explorer_popup().is_some());
        assert!(state.active_display_name().contains("[explorer]"));
        assert!(
            state
                .session
                .active_buffer()
                .to_string()
                .lines()
                .any(|line| line == "..")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn explorer_write_applies_rename_and_create() {
        let dir = std::env::temp_dir().join(format!(
            "redox_explorer_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock went backwards")
                .as_nanos()
        ));
        fs::create_dir(&dir).expect("failed to create temp dir");
        let file_a = dir.join("a.txt");
        let file_open = dir.join("open.txt");
        fs::write(&file_a, "a").expect("failed to write fixture");
        fs::write(&file_open, "open").expect("failed to write fixture");

        let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
        let mut state = EditorState::new(session);

        run_command(&mut state, "explorer");
        {
            let buffer = state.session.active_buffer_mut();
            *buffer = TextBuffer::from_str("..\nrenamed.txt\ncreated.txt");
        }
        let _ = state.session.recompute_active_dirty();

        run_command(&mut state, "w");

        assert!(dir.join("renamed.txt").exists());
        assert!(dir.join("created.txt").exists());
        assert!(!dir.join("a.txt").exists());

        let _ = fs::remove_file(dir.join("renamed.txt"));
        let _ = fs::remove_file(dir.join("created.txt"));
        let _ = fs::remove_file(file_open);
        let _ = fs::remove_dir(dir);
    }

    #[test]
    fn explorer_q_closes_surface_buffer_only() {
        let path = temp_file_path("explorer_q_close");
        let mut state = state_with_text(path.clone(), "alpha");
        let return_to = state.session.active_id();

        run_command(&mut state, "explorer");
        assert!(state.explorer_popup().is_some());

        run_command(&mut state, "q");

        assert!(!state.should_quit);
        assert!(state.explorer_popup().is_none());
        assert_eq!(state.session.active_id(), return_to);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn explorer_command_toggles_visibility() {
        let path = temp_file_path("explorer_toggle");
        let mut state = state_with_text(path.clone(), "alpha");
        let return_to = state.session.active_id();

        run_command(&mut state, "explorer");
        assert!(state.explorer_popup().is_some());

        state.apply_input(InputAction::OpenExplorer, 80, 24);
        assert!(state.explorer_popup().is_none());
        assert_eq!(state.session.active_id(), return_to);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn explorer_enter_opens_file_and_closes_explorer() {
        let dir = std::env::temp_dir().join(format!(
            "redox_explorer_enter_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock went backwards")
                .as_nanos()
        ));
        fs::create_dir(&dir).expect("failed to create temp dir");
        let file_a = dir.join("a.txt");
        let file_open = dir.join("open.txt");
        fs::write(&file_a, "aaa").expect("failed to write fixture");
        fs::write(&file_open, "open").expect("failed to write fixture");

        let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
        let mut state = EditorState::new(session);
        run_command(&mut state, "explorer");

        {
            let text = state.session.active_buffer().to_string();
            let target_line = text
                .lines()
                .position(|line| line == "a.txt")
                .expect("a.txt missing from explorer listing");
            let id = state.session.active_id();
            state
                .views
                .get_mut(&id)
                .expect("missing explorer view")
                .cursor
                .cursor
                .line = target_line;
        }

        state.apply_input(InputAction::SurfaceOpenSelected, 80, 24);

        assert!(state.explorer_popup().is_none());
        assert_eq!(state.session.active_buffer().to_string(), "aaa");

        let _ = fs::remove_file(file_a);
        let _ = fs::remove_file(file_open);
        let _ = fs::remove_dir(dir);
    }

    #[test]
    fn explorer_opens_with_cursor_on_current_file() {
        let dir = std::env::temp_dir().join(format!(
            "redox_explorer_cursor_test_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock went backwards")
                .as_nanos()
        ));
        fs::create_dir(&dir).expect("failed to create temp dir");
        let file_a = dir.join("a.txt");
        let file_open = dir.join("open.txt");
        fs::write(&file_a, "aaa").expect("failed to write fixture");
        fs::write(&file_open, "open").expect("failed to write fixture");

        let session = EditorSession::open_initial_file(&file_open).expect("failed to open session");
        let mut state = EditorState::new(session);
        run_command(&mut state, "explorer");

        let line_idx = state
            .session
            .active_buffer()
            .to_string()
            .lines()
            .position(|line| line == "open.txt")
            .expect("open.txt missing from explorer");
        let active = state.session.active_id();
        let cursor_line = state
            .views
            .get(&active)
            .expect("missing explorer view")
            .cursor
            .cursor
            .line;
        assert_eq!(cursor_line, line_idx);

        let _ = fs::remove_file(file_a);
        let _ = fs::remove_file(file_open);
        let _ = fs::remove_dir(dir);
    }

    #[test]
    fn open_line_below_enters_insert_and_inserts_blank_line() {
        let path = temp_file_path("open_line_below");
        let mut state = state_with_text(path.clone(), "one\ntwo");
        let id = state.session.active_id();
        state
            .views
            .get_mut(&id)
            .expect("missing view")
            .cursor
            .cursor = Pos::new(0, 0);

        state.apply_input(InputAction::OpenLineBelow, 80, 24);

        assert_eq!(state.mode, EditorMode::Insert);
        assert_eq!(state.session.active_buffer().to_string(), "one\n\ntwo");
        assert_eq!(
            state.views.get(&id).expect("missing view").cursor.cursor,
            Pos::new(1, 0)
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn open_line_above_enters_insert_and_inserts_blank_line() {
        let path = temp_file_path("open_line_above");
        let mut state = state_with_text(path.clone(), "one\ntwo");
        let id = state.session.active_id();
        state
            .views
            .get_mut(&id)
            .expect("missing view")
            .cursor
            .cursor = Pos::new(1, 0);

        state.apply_input(InputAction::OpenLineAbove, 80, 24);

        assert_eq!(state.mode, EditorMode::Insert);
        assert_eq!(state.session.active_buffer().to_string(), "one\n\ntwo");
        assert_eq!(
            state.views.get(&id).expect("missing view").cursor.cursor,
            Pos::new(1, 0)
        );

        let _ = fs::remove_file(path);
    }
}
