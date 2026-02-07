use std::env;
use std::path::PathBuf;

use editor_core::TextBuffer;
use editor_core::io::load_buffer;

use minui::{Window, prelude::*};

#[derive(Debug)]
struct EditorState {
    buffer: TextBuffer,
    scroll_x: usize,
    scroll_y: usize,
}

impl EditorState {
    fn new(buffer: TextBuffer) -> Self {
        Self {
            buffer,
            scroll_x: 0,
            scroll_y: 0,
        }
    }
}

struct TextViewport {
    scroll_x: usize,
    scroll_y: usize,
    width: u16,
    height: u16,
}

impl TextViewport {
    fn from_window(window: &dyn Window, scroll_x: usize, scroll_y: usize) -> Self {
        let (width, height) = window.get_size();
        Self {
            scroll_x,
            scroll_y,
            width,
            height,
        }
    }
}

fn snapshot_lines(buffer: &TextBuffer, viewport: &TextViewport) -> Vec<String> {
    let mut lines = Vec::with_capacity(viewport.height as usize);
    let first_line = viewport.scroll_y;
    let last_line = first_line.saturating_add(viewport.height as usize);

    for line_idx in first_line..last_line {
        if line_idx >= buffer.len_lines() {
            break;
        }

        let mut line = buffer.line_string(line_idx);

        if viewport.scroll_x > 0 {
            let skip = viewport.scroll_x.min(line.chars().count());
            line = line.chars().skip(skip).collect();
        }

        if line.chars().count() > viewport.width as usize {
            line = line.chars().take(viewport.width as usize).collect();
        }

        lines.push(line);
    }

    lines
}

fn draw_buffer_view(state: &EditorState, window: &mut dyn Window) -> minui::Result<()> {
    let viewport = TextViewport::from_window(window, state.scroll_x, state.scroll_y);
    let lines = snapshot_lines(&state.buffer, &viewport);

    for (row, line) in lines.iter().enumerate() {
        window.write_str(row as u16, 0, line)?;
    }

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

    // Application handler for event loops and rendering updates
    app.run(
        |_state, event| {
            // Closure for handling input and updates.
            match event {
                Event::KeyWithModifiers(k) if matches!(k.key, KeyKind::Char('q')) => false,
                Event::Character('q') => false,
                _ => true,
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
