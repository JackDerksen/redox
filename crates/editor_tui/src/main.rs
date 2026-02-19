use std::env;
use std::path::PathBuf;

use editor_core::TextBuffer;
use editor_core::io::load_buffer;

use minui::{Window, prelude::*};

mod input;
mod ui;

use input::cursor::CursorController;
use input::{InputAction, InputMode, InputState, InsertKind, map_event_with_state};

use ui::{
    Align, EditorStatusBar, GraphemeCache, Segment, TextViewport, draw_snapshot,
    snapshot_lines_wrapped_cached,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorMode {
    Normal,
    Insert,
    Command,
}

impl EditorMode {
    fn as_input_mode(self) -> InputMode {
        match self {
            EditorMode::Normal => InputMode::Normal,
            EditorMode::Insert => InputMode::Insert,
            EditorMode::Command => InputMode::Command,
        }
    }
}

#[derive(Debug)]
struct EditorState {
    buffer: TextBuffer,
    cursor: CursorController,
    grapheme_cache: GraphemeCache,

    mode: EditorMode,

    /// Stateful key handling (eg. `gg`, counts).
    input: InputState,

    /// Most recent input action received from the update loop.
    ///
    /// Apply it during draw because draw has access to the current window size.
    pending_action: Option<InputAction>,
}

impl EditorState {
    fn new(buffer: TextBuffer) -> Self {
        Self {
            buffer,
            cursor: CursorController::new(),
            // Cache a few screens worth of lines. Will tune this later.
            grapheme_cache: GraphemeCache::new(512),
            mode: EditorMode::Normal,
            input: InputState::new(),
            pending_action: None,
        }
    }

    fn apply_input(&mut self, action: InputAction, window: &dyn Window) {
        let (w, h) = window.get_size();
        let vw = w as usize;

        // Keep this consistent with rendering (reserve one row at the bottom for the status bar).
        let status_h: usize = 1;
        let text_vh = h as usize;
        let text_vh = text_vh.saturating_sub(status_h);

        match action {
            InputAction::Motion { motion, count } => {
                // Navigation semantics are in `editor_core::motion`.
                // The cursor controller handles viewport following + projection only.
                self.cursor
                    .apply_motion(&self.buffer, motion, count, vw, text_vh);
            }

            InputAction::SetMode(mode) => {
                // If going from Insert -> Normal, apply Vim-like cursor semantics:
                // - `i` then `Esc` ends up with the cursor one char left of the insertion point
                // - `a` then `Esc` ends up back on the original character
                //
                // Both are achieved by moving left one char when exiting Insert mode,
                // because in insert mode the cursor is considered to be *after* the last inserted char.
                let leaving_insert_to_normal =
                    self.mode == EditorMode::Insert && mode == InputMode::Normal;

                self.mode = match mode {
                    InputMode::Normal => EditorMode::Normal,
                    InputMode::Insert => EditorMode::Insert,
                    InputMode::Command => EditorMode::Command,
                };

                if leaving_insert_to_normal {
                    // Move left by one character, staying on the same line when possible.
                    // (At column 0, stay put.)
                    if self.cursor.cursor.col > 0 {
                        self.cursor.cursor.col -= 1;
                    }
                    self.cursor.reconcile_after_edit(&self.buffer, vw, text_vh);
                }

                // Mode changes should clear any pending normal-mode prefixes/counts.
                // This avoids surprising behavior like carrying a count into insert mode.
                self.input.reset_prefixes();
            }

            InputAction::EnterInsert(kind) => {
                match kind {
                    InsertKind::Insert => {
                        // `i`: insert at cursor (no cursor movement)
                    }

                    InsertKind::Append => {
                        // `a`: append after cursor (advance by one char if possible)
                        let line = self.buffer.clamp_line(self.cursor.cursor.line);
                        let line_text = self.buffer.line_string(line);
                        let line_len_chars = line_text.chars().count();

                        if self.cursor.cursor.col < line_len_chars {
                            self.cursor.cursor.col += 1;
                        }
                    }

                    InsertKind::InsertLineStart => {
                        // `I`: insert at beginning of line (for now: true BOL)
                        self.cursor.cursor.col = 0;
                    }

                    InsertKind::AppendLineEnd => {
                        // `A`: append at end of line
                        let line = self.buffer.clamp_line(self.cursor.cursor.line);
                        let line_text = self.buffer.line_string(line);
                        self.cursor.cursor.col = line_text.chars().count();
                    }
                }

                self.mode = EditorMode::Insert;
                self.input.reset_prefixes();

                // Cursor may have moved; ensure viewport follows without applying a motion.
                self.cursor.reconcile_after_edit(&self.buffer, vw, text_vh);
            }

            InputAction::EnterCommand => {
                self.mode = EditorMode::Command;
                self.input.reset_prefixes();
            }

            InputAction::CommandCancel => {
                self.mode = EditorMode::Normal;
                self.input.reset_prefixes();
            }

            // Insert-mode editing (arrow keys are handled as KeyWithModifiers -> Motion).
            InputAction::InsertChar(c) => {
                if self.mode == EditorMode::Insert {
                    let new_pos = self.buffer.insert(self.cursor.cursor, &c.to_string());
                    self.cursor.cursor = new_pos;

                    // After an edit, reconcile scroll/cursor without applying a synthetic motion.
                    self.cursor.reconcile_after_edit(&self.buffer, vw, text_vh);
                }
            }

            InputAction::InsertText(text) => {
                if self.mode == EditorMode::Insert && !text.is_empty() {
                    let new_pos = self.buffer.insert(self.cursor.cursor, &text);
                    self.cursor.cursor = new_pos;

                    self.cursor.reconcile_after_edit(&self.buffer, vw, text_vh);
                }
            }

            InputAction::Backspace => {
                if self.mode == EditorMode::Insert {
                    let sel = editor_core::Selection::empty(self.cursor.cursor);
                    let sel = self.buffer.backspace(sel);
                    self.cursor.cursor = sel.cursor;

                    self.cursor.reconcile_after_edit(&self.buffer, vw, text_vh);
                }
            }

            InputAction::Enter => {
                if self.mode == EditorMode::Insert {
                    let sel = editor_core::Selection::empty(self.cursor.cursor);
                    let sel = self.buffer.insert_newline(sel);
                    self.cursor.cursor = sel.cursor;

                    self.cursor.reconcile_after_edit(&self.buffer, vw, text_vh);
                }
            }

            InputAction::Quit | InputAction::None => {}

            // Not wired yet (command-line buffer + execution).
            InputAction::CommandChar(_)
            | InputAction::CommandBackspace
            | InputAction::CommandEnter => {}
        }
    }
}

fn draw_buffer_view(state: &mut EditorState, window: &mut dyn Window) -> minui::Result<()> {
    let (vw, vh) = window.get_size();

    // Reserve one row for the status bar at the bottom.
    let status_h: u16 = 1;
    let text_h = vh.saturating_sub(status_h);

    let (scroll_x, scroll_y) = state.cursor.viewport_scroll();

    let viewport = TextViewport {
        scroll_x,
        scroll_y,
        width: vw,
        height: text_h,
    };

    let snapshot =
        snapshot_lines_wrapped_cached(&state.buffer, &viewport, &mut state.grapheme_cache);
    draw_snapshot(&snapshot, window)?;

    // --- Status bar (bottom row) ---
    let bar_bg = ColorPair::new(Color::LightGray, Color::Black);

    let (mode_label, mode_colors) = match state.mode {
        EditorMode::Normal => ("NORMAL", ColorPair::new(Color::Black, Color::Magenta)),
        EditorMode::Insert => ("INSERT", ColorPair::new(Color::Black, Color::Blue)),
        EditorMode::Command => ("COMMAND", ColorPair::new(Color::Black, Color::Cyan)),
    };

    let cursor = state.cursor.cursor;
    let left_text = format!(" {} ", mode_label);
    let center_text = " redox ".to_string();
    let right_text = format!(" Ln {}, Col {} ", cursor.line + 1, cursor.col + 1);

    let status = EditorStatusBar::new()
        .with_height(1)
        .with_bg(bar_bg)
        .add_segment(
            Segment::new(left_text)
                .with_color(mode_colors)
                .with_align(Align::Left)
                .with_min_width(12),
        )
        .add_segment(
            Segment::new(center_text)
                .with_color(bar_bg)
                .with_align(Align::Center),
        )
        .add_segment(
            Segment::new(right_text)
                .with_color(bar_bg)
                .with_align(Align::Right)
                .with_min_width(18),
        );

    status.draw(window)?;

    // Cursor rendering via MinUI deferred cursor request.
    let spec = state
        .cursor
        .cursor_spec(&state.buffer, vw as usize, text_h as usize);
    window.request_cursor(spec);

    Ok(())
}

fn parse_path_arg() -> anyhow::Result<PathBuf> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("expected a file path argument"))?;
    Ok(PathBuf::from(path))
}

fn main() -> minui::Result<()> {
    let path = parse_path_arg().expect("file path required (e.g. editor_tui ./file.txt)");
    let buffer = load_buffer(&path).expect("failed to load file");

    let mut app = App::new(EditorState::new(buffer))?;

    app.run(
        |state, event| {
            let action = map_event_with_state(&mut state.input, state.mode.as_input_mode(), &event);

            match action {
                InputAction::Quit => false,
                action => {
                    // Store the action for the next draw call where we know the viewport size.
                    state.pending_action = Some(action);
                    true
                }
            }
        },
        |state, window| {
            // Deferred cursor model (MinUI):
            // - clear per-frame cursor request
            // - draw (requests cursor)
            // - end_frame applies cursor + flushes
            window.clear_cursor_request();

            // Apply pending input based on window size.
            if let Some(action) = state.pending_action.take() {
                state.apply_input(action, window);
            }

            draw_buffer_view(state, window)?;

            window.end_frame()?;
            Ok(())
        },
    )?;

    Ok(())
}
