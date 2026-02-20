//! Editor application state for `editor_tui`.
//!
//! This module owns:
//! - the current buffer + cursor controller
//! - modal state (normal/insert/command)
//! - command-line buffer and status messages
//! - applying high-level `InputAction`s to mutate state
//!
//! Rendering and event-loop glue stay in `main.rs`.
//! Terminal-agnostic editing logic stays in `editor_core`.

use std::path::PathBuf;

use editor_core::{Selection, TextBuffer};

use crate::input::cursor::CursorController;
use crate::input::{InputAction, InputMode, InputState, InsertKind};

use crate::ui::GraphemeCache;

/// Vim-like editor mode for the frontend.
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

    pub fn label(self) -> &'static str {
        match self {
            EditorMode::Normal => "NORMAL",
            EditorMode::Insert => "INSERT",
            EditorMode::Command => "COMMAND",
        }
    }
}

/// TUI app state for a single-buffer editor MVP.
#[derive(Debug)]
pub struct EditorState {
    /// Path of the file currently being edited (single-buffer editor for now).
    pub path: PathBuf,

    pub buffer: TextBuffer,
    pub cursor: CursorController,
    pub grapheme_cache: GraphemeCache,

    pub mode: EditorMode,

    /// Whether the in-memory buffer has diverged from the file on disk.
    pub dirty: bool,

    /// Stateful key handling (eg. `gg`, counts).
    pub input: InputState,

    /// Command-line buffer for `:` commands.
    pub command_line: String,

    /// Status / error message (rendered in status bar).
    pub status_msg: Option<String>,

    /// Most recent input action received from the update loop.
    ///
    /// Apply it during draw because draw has access to the current window size.
    pub pending_action: Option<InputAction>,
}

impl EditorState {
    pub fn new(path: PathBuf, buffer: TextBuffer) -> Self {
        Self {
            path,
            buffer,
            cursor: CursorController::new(),
            // Cache a few screens worth of lines. Will tune this later.
            grapheme_cache: GraphemeCache::new(512),
            mode: EditorMode::Normal,
            dirty: false,
            input: InputState::new(),
            command_line: String::new(),
            status_msg: None,
            pending_action: None,
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
    }

    pub fn clear_status(&mut self) {
        self.status_msg = None;
    }

    /// Apply an input action to this state, using the current viewport size to reconcile cursor+scroll.
    ///
    /// This is kept in `editor_tui` because it is UI/terminal aware (viewport dimensions)
    /// and orchestrates `editor_core` operations.
    pub fn apply_input(
        &mut self,
        action: InputAction,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        // Keep this consistent with rendering (reserve one row at the bottom for the status bar).
        let status_h: usize = 1;
        let text_vh = viewport_height_rows.saturating_sub(status_h);

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

                // Vim behavior: when leaving insert mode, cursor rests on the previous char.
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
                if self.mode == EditorMode::Command {
                    let cmd = self.command_line.trim().to_string();
                    self.command_line.clear();
                    self.mode = EditorMode::Normal;

                    match cmd.as_str() {
                        "w" => {
                            // MVP: write whole buffer back to the original file.
                            match std::fs::write(&self.path, self.buffer.to_string()) {
                                Ok(()) => {
                                    self.dirty = false;
                                    self.set_status("written");
                                }
                                Err(e) => {
                                    self.set_status(format!("write failed: {e}"));
                                }
                            }
                        }
                        "q" => {
                            if self.dirty {
                                self.set_status("no write since last change (use :q! to quit)");
                            } else {
                                // Quit is handled elsewhere (event loop). Keep messaging consistent for now.
                                self.set_status("use q to quit (temporary)");
                            }
                        }
                        "q!" => {
                            self.set_status("use q to quit (temporary)");
                        }
                        "wq" => match std::fs::write(&self.path, self.buffer.to_string()) {
                            Ok(()) => {
                                self.dirty = false;
                                self.set_status("written (use q to quit)");
                            }
                            Err(e) => {
                                self.set_status(format!("write failed: {e}"));
                            }
                        },
                        "" => {
                            // Empty command: no-op.
                        }
                        _ => {
                            self.set_status(format!("unknown command: {cmd}"));
                        }
                    }
                }
            }

            InputAction::InsertChar(c) => {
                if self.mode == EditorMode::Insert {
                    let new_pos = self.buffer.insert(self.cursor.cursor, &c.to_string());
                    self.cursor.cursor = new_pos;
                    self.dirty = true;

                    self.cursor
                        .reconcile_after_edit(&self.buffer, viewport_width_cells, text_vh);
                }
            }

            InputAction::InsertText(text) => {
                if self.mode == EditorMode::Insert && !text.is_empty() {
                    let new_pos = self.buffer.insert(self.cursor.cursor, &text);
                    self.cursor.cursor = new_pos;
                    self.dirty = true;

                    self.cursor
                        .reconcile_after_edit(&self.buffer, viewport_width_cells, text_vh);
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

            // Paste handling: treat as bulk insert in Insert mode.
            InputAction::Paste(text) => {
                if self.mode == EditorMode::Insert && !text.is_empty() {
                    let new_pos = self.buffer.insert(self.cursor.cursor, &text);
                    self.cursor.cursor = new_pos;
                    self.dirty = true;

                    self.cursor
                        .reconcile_after_edit(&self.buffer, viewport_width_cells, text_vh);
                }
            }

            InputAction::Quit | InputAction::None => {}
        }
    }
}
