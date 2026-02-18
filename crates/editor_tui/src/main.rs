use std::env;
use std::path::PathBuf;

use editor_core::TextBuffer;
use editor_core::io::load_buffer;

use minui::{Window, prelude::*};

mod input;
mod ui;

use input::cursor::CursorController;
use input::{InputAction, InputState, map_event_with_state};

use ui::{GraphemeCache, TextViewport, draw_snapshot, snapshot_lines_wrapped_cached};

#[derive(Debug)]
struct EditorState {
    buffer: TextBuffer,
    cursor: CursorController,
    grapheme_cache: GraphemeCache,

    /// Stateful key handling (e.g. `gg`, counts).
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
            input: InputState::new(),
            pending_action: None,
        }
    }

    fn apply_input(&mut self, action: InputAction, window: &dyn Window) {
        let (w, h) = window.get_size();
        let vw = w as usize;
        let vh = h as usize;

        match action {
            InputAction::Motion { motion, count } => {
                // Navigation semantics are in `editor_core::motion`.
                // The cursor controller handles viewport following + projection only.
                self.cursor
                    .apply_motion(&self.buffer, motion, count, vw, vh);
            }
            InputAction::Quit | InputAction::None => {}
        }
    }
}

fn draw_buffer_view(state: &mut EditorState, window: &mut dyn Window) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let (scroll_x, scroll_y) = state.cursor.viewport_scroll();

    let viewport = TextViewport {
        scroll_x,
        scroll_y,
        width: vw,
        height: vh,
    };

    let snapshot =
        snapshot_lines_wrapped_cached(&state.buffer, &viewport, &mut state.grapheme_cache);
    draw_snapshot(&snapshot, window)?;

    // Cursor rendering via MinUI deferred cursor request.
    let spec = state
        .cursor
        .cursor_spec(&state.buffer, vw as usize, vh as usize);
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
            let action = map_event_with_state(&mut state.input, &event);
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
