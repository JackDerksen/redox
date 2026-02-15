use std::env;
use std::path::PathBuf;

use editor_core::TextBuffer;
use editor_core::io::load_buffer;

use minui::{Window, prelude::*};

mod input;
mod ui;

use input::{InputAction, map_event};
use ui::{GraphemeCache, TextViewport, draw_snapshot, snapshot_lines_wrapped_cached};

#[derive(Debug)]
struct EditorState {
    buffer: TextBuffer,
    scroll_x: usize,
    scroll_y: usize,
    grapheme_cache: GraphemeCache,
}

impl EditorState {
    fn new(buffer: TextBuffer) -> Self {
        Self {
            buffer,
            scroll_x: 0,
            scroll_y: 0,
            // Cache a few screens worth of lines. Will tune this later.
            grapheme_cache: GraphemeCache::new(512),
        }
    }

    fn apply_input(&mut self, action: InputAction) {
        match action {
            InputAction::ScrollBy { dx, dy } => {
                self.scroll_x = apply_scroll_delta(self.scroll_x, dx);
                self.scroll_y = apply_scroll_delta(self.scroll_y, dy);
            }
            InputAction::Quit | InputAction::None => {}
        }
    }
}

fn apply_scroll_delta(current: usize, delta: i32) -> usize {
    if delta >= 0 {
        current.saturating_add(delta as usize)
    } else {
        current.saturating_sub((-delta) as usize)
    }
}

fn draw_buffer_view(state: &mut EditorState, window: &mut dyn Window) -> minui::Result<()> {
    let viewport = TextViewport::from_window(window, state.scroll_x, state.scroll_y);
    let snapshot =
        snapshot_lines_wrapped_cached(&state.buffer, &viewport, &mut state.grapheme_cache);
    draw_snapshot(&snapshot, window)
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

    // Application handler for event loops and rendering updates
    app.run(
        |state, event| {
            // Closure for handling input and updates.
            match map_event(&event) {
                InputAction::Quit => false,
                action => {
                    state.apply_input(action);
                    true
                }
            }
        },
        |state, window| {
            // Closure for rendering the application state.
            draw_buffer_view(state, window)?;

            window.flush()?;

            Ok(()) // Drawing succeeded
        },
    )?;

    Ok(())
}
