use std::env;
use std::path::PathBuf;

use editor_core::io::load_buffer;

use minui::{Window, prelude::*};

mod app;
mod input;
mod ui;

use app::{EditorMode, EditorState};
use input::{InputAction, map_event_with_state};

use ui::{
    Align, EditorStatusBar, Segment, TextViewport, draw_snapshot, snapshot_lines_wrapped_cached,
};

use minui::ColorPair;

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
        EditorMode::Normal => ("NORMAL", ColorPair::new(Color::Black, Color::Red)),
        EditorMode::Insert => ("INSERT", ColorPair::new(Color::Black, Color::Blue)),
        EditorMode::Command => ("COMMAND", ColorPair::new(Color::Black, Color::Cyan)),
    };

    let cursor = state.cursor.cursor;

    let mut left_text = format!(" {} ", mode_label);
    if state.dirty {
        left_text.push('*');
        left_text.push(' ');
    }

    let center_text = if state.mode == EditorMode::Command {
        format!(" :{} ", state.command_line)
    } else if let Some(msg) = &state.status_msg {
        format!(" {} ", msg)
    } else {
        format!(" {} ", state.path.display())
    };

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

    let mut app = App::new(EditorState::new(path, buffer))?;

    app.run(
        |state, event| {
            let action = match &event {
                Event::Paste(text) => InputAction::Paste(text.clone()),
                _ => map_event_with_state(&mut state.input, state.mode.as_input_mode(), &event),
            };

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
            window.clear_cursor_request();

            if let Some(action) = state.pending_action.take() {
                let (w, h) = window.get_size();
                state.apply_input(action, w as usize, h as usize);
            }

            draw_buffer_view(state, window)?;

            window.end_frame()?;
            Ok(())
        },
    )?;

    Ok(())
}
