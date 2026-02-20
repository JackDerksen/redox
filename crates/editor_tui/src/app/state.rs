//! Editor state and action application for `editor_tui`.
//!
//! This module keeps UI-facing state (mode, command line, status, cursor viewport
//! reconciliation) while delegating text editing primitives to `editor_core`.

use std::path::PathBuf;

use editor_core::{Selection, TextBuffer};

use crate::input::cursor::CursorController;
use crate::input::{InputAction, InputMode, InputState, InsertKind};
use crate::ui::{GraphemeCache, STATUS_BAR_HEIGHT_ROWS};

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

/// Single-buffer editor state.
#[derive(Debug)]
pub struct EditorState {
    pub path: PathBuf,
    pub buffer: TextBuffer,
    pub cursor: CursorController,
    pub grapheme_cache: GraphemeCache,
    pub mode: EditorMode,
    pub dirty: bool,
    pub input: InputState,
    pub command_line: String,
    pub status_msg: Option<String>,
    pub should_quit: bool,
    viewport_width_cells: usize,
    viewport_height_rows: usize,
}

impl EditorState {
    pub fn new(path: PathBuf, buffer: TextBuffer) -> Self {
        Self {
            path,
            buffer,
            cursor: CursorController::new(),
            grapheme_cache: GraphemeCache::new(512),
            mode: EditorMode::Normal,
            dirty: false,
            input: InputState::new(),
            command_line: String::new(),
            status_msg: None,
            should_quit: false,
            viewport_width_cells: 80,
            viewport_height_rows: 24,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
    }

    pub fn clear_status(&mut self) {
        self.status_msg = None;
    }

    pub fn set_viewport_size(&mut self, width_cells: usize, height_rows: usize) {
        self.viewport_width_cells = width_cells;
        self.viewport_height_rows = height_rows;
    }

    pub fn viewport_size(&self) -> (usize, usize) {
        (self.viewport_width_cells, self.viewport_height_rows)
    }

    /// Apply a high-level input action using the active viewport size for cursor reconciliation.
    pub fn apply_input(
        &mut self,
        action: InputAction,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);

        match action {
            InputAction::Motion { motion, count } => {
                self.cursor.apply_motion(
                    &self.buffer,
                    motion,
                    count,
                    viewport_width_cells,
                    text_vh,
                );
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
                    if self.cursor.cursor.col > 0 {
                        self.cursor.cursor.col -= 1;
                    }
                    self.cursor
                        .reconcile_after_edit(&self.buffer, viewport_width_cells, text_vh);
                }

                self.input.reset_prefixes();
            }

            InputAction::EnterInsert(kind) => {
                match kind {
                    InsertKind::Insert => {}
                    InsertKind::Append => {
                        let line = self.buffer.clamp_line(self.cursor.cursor.line);
                        let line_text = self.buffer.line_string(line);
                        let line_len_chars = line_text.chars().count();
                        if self.cursor.cursor.col < line_len_chars {
                            self.cursor.cursor.col += 1;
                        }
                    }
                    InsertKind::InsertLineStart => {
                        self.cursor.cursor.col = 0;
                    }
                    InsertKind::AppendLineEnd => {
                        let line = self.buffer.clamp_line(self.cursor.cursor.line);
                        let line_text = self.buffer.line_string(line);
                        self.cursor.cursor.col = line_text.chars().count();
                    }
                }

                self.mode = EditorMode::Insert;
                self.clear_status();
                self.input.reset_prefixes();
                self.cursor
                    .reconcile_after_edit(&self.buffer, viewport_width_cells, text_vh);
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

            InputAction::InsertChar(c) => {
                if self.mode == EditorMode::Insert {
                    let s = c.to_string();
                    self.insert_text_at_cursor(&s, viewport_width_cells, text_vh);
                }
            }

            InputAction::Backspace => {
                if self.mode == EditorMode::Insert {
                    let sel = Selection::empty(self.cursor.cursor);
                    let sel = self.buffer.backspace(sel);
                    self.cursor.cursor = sel.cursor;
                    self.dirty = true;

                    self.cursor
                        .reconcile_after_edit(&self.buffer, viewport_width_cells, text_vh);
                }
            }

            InputAction::Enter => {
                if self.mode == EditorMode::Insert {
                    let sel = Selection::empty(self.cursor.cursor);
                    let sel = self.buffer.insert_newline(sel);
                    self.cursor.cursor = sel.cursor;
                    self.dirty = true;

                    self.cursor
                        .reconcile_after_edit(&self.buffer, viewport_width_cells, text_vh);
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

        let new_pos = self.buffer.insert(self.cursor.cursor, text);
        self.cursor.cursor = new_pos;
        self.dirty = true;

        self.cursor
            .reconcile_after_edit(&self.buffer, viewport_width_cells, text_vh);
    }

    fn execute_command_line(&mut self) {
        if self.mode != EditorMode::Command {
            return;
        }

        let cmd = self.command_line.trim().to_string();
        self.command_line.clear();
        self.mode = EditorMode::Normal;

        match cmd.as_str() {
            "" => {}
            "w" => {
                self.write_current_file();
            }
            "q" => {
                if self.dirty {
                    self.set_status("no write since last change (use :q! to quit)");
                } else {
                    self.should_quit = true;
                }
            }
            "q!" => {
                self.should_quit = true;
            }
            "wq" => {
                if self.write_current_file() {
                    self.should_quit = true;
                }
            }
            _ => {
                self.set_status(format!("unknown command: {cmd}"));
            }
        }
    }

    fn write_current_file(&mut self) -> bool {
        match std::fs::write(&self.path, self.buffer.to_string()) {
            Ok(()) => {
                self.dirty = false;
                self.set_status("written");
                true
            }
            Err(e) => {
                self.set_status(format!("write failed: {e}"));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("redox_state_test_{tag}_{nanos}.txt"))
    }

    fn state_with_text(path: PathBuf, text: &str) -> EditorState {
        EditorState::new(path, TextBuffer::from_str(text))
    }

    #[test]
    fn normal_mode_paste_inserts_text_and_marks_dirty() {
        let path = temp_file_path("paste_normal");
        let mut state = state_with_text(path.clone(), "hello");

        state.apply_input(InputAction::Paste(" world".to_string()), 80, 24);

        assert_eq!(state.buffer.to_string(), " worldhello");
        assert!(state.dirty);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn command_q_quits_when_clean() {
        let path = temp_file_path("q_clean");
        let mut state = state_with_text(path.clone(), "abc");
        state.mode = EditorMode::Command;
        state.command_line = "q".to_string();
        state.dirty = false;

        state.apply_input(InputAction::CommandEnter, 80, 24);

        assert!(state.should_quit);
        assert_eq!(state.mode, EditorMode::Normal);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn command_q_does_not_quit_when_dirty() {
        let path = temp_file_path("q_dirty");
        let mut state = state_with_text(path.clone(), "abc");
        state.mode = EditorMode::Command;
        state.command_line = "q".to_string();
        state.dirty = true;

        state.apply_input(InputAction::CommandEnter, 80, 24);

        assert!(!state.should_quit);
        assert_eq!(
            state.status_msg.as_deref(),
            Some("no write since last change (use :q! to quit)")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn command_wq_writes_file_and_quits() {
        let path = temp_file_path("wq_success");
        fs::write(&path, "old").expect("failed to write temp file");

        let mut state = state_with_text(path.clone(), "new");
        state.mode = EditorMode::Command;
        state.command_line = "wq".to_string();
        state.dirty = true;

        state.apply_input(InputAction::CommandEnter, 80, 24);

        assert!(state.should_quit);
        assert!(!state.dirty);
        let on_disk = fs::read_to_string(&path).expect("failed to read temp file");
        assert_eq!(on_disk, "new");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn command_wq_write_failure_does_not_quit() {
        let path = std::env::temp_dir();
        let mut state = state_with_text(path, "new");
        state.mode = EditorMode::Command;
        state.command_line = "wq".to_string();
        state.dirty = true;

        state.apply_input(InputAction::CommandEnter, 80, 24);

        assert!(!state.should_quit);
        assert!(state.dirty);
        assert!(
            state
                .status_msg
                .as_deref()
                .is_some_and(|msg| msg.starts_with("write failed:"))
        );
    }
}
