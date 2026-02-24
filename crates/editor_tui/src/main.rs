use std::env;
use std::path::PathBuf;

use editor_core::EditorSession;

use minui::{Window, prelude::*};

mod app;
mod input;
mod ui;

use app::EditorState;
use input::{InputAction, map_event_with_state};

use ui::{
    STATUS_BAR_HEIGHT_CELLS, TextViewport, UiStyle, build_editor_status_bar,
    draw_explorer_popup_view, draw_snapshot, snapshot_lines_wrapped_cached,
};
fn draw_buffer_view(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();

    if let Some(popup) = state.explorer_popup() {
        draw_explorer_popup_view(state, style, window, popup)?;
        return Ok(());
    }

    // Reserve one row for the status bar at the bottom.
    let status_h: u16 = STATUS_BAR_HEIGHT_CELLS;
    let text_h = vh.saturating_sub(status_h);

    let (snapshot, spec) = state.with_active_buffer_view_mut(|buffer, view| {
        let (scroll_x, scroll_y) = view.cursor.viewport_scroll();
        let viewport = TextViewport {
            scroll_x,
            scroll_y,
            width: vw,
            height: text_h,
        };
        let snapshot = snapshot_lines_wrapped_cached(buffer, &viewport, &mut view.grapheme_cache);
        let spec = view
            .cursor
            .cursor_spec(buffer, vw as usize, text_h as usize);
        (snapshot, spec)
    });
    draw_snapshot(&snapshot, window)?;

    // --- Status bar (bottom row) ---
    let status = build_editor_status_bar(state, style);

    status.draw(window)?;

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
    let session = EditorSession::open_initial_file(path).expect("failed to open initial file");

    let mut app = App::new(EditorState::new(session))?;
    let style = UiStyle::default();

    app.run(
        |state, event| {
            let action = match &event {
                Event::Paste(text) => InputAction::Paste(text.clone()),
                _ => map_event_with_state(&mut state.input, state.mode.as_input_mode(), &event),
            };

            let (w, h) = state.viewport_size();
            state.apply_input(action, w, h);
            !state.should_quit
        },
        |state, window| {
            window.clear_cursor_request();
            let (w, h) = window.get_size();
            state.set_viewport_size(w as usize, h as usize);

            draw_buffer_view(state, style, window)?;

            window.end_frame()?;
            Ok(())
        },
    )?;

    Ok(())
}
