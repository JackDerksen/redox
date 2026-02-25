use std::env;
use std::path::PathBuf;

use redox_core::{BufferId, EditorSession};

use minui::{prelude::*, ColorPair, Window};

mod app;
mod input;
mod ui;

use app::EditorState;
use input::{map_event_with_state, InputAction};

use ui::{
    build_editor_status_bar, draw_explorer_popup_view, explorer_popup_inner_size,
    snapshot_lines_wrapped_cached, TextViewport, UiStyle, STATUS_BAR_HEIGHT_CELLS,
};
fn draw_buffer_view(
    state: &mut EditorState,
    style: UiStyle,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let editor_text = ColorPair::new(style.theme.white, style.theme.bg);
    fill_background(window, vw, vh, editor_text)?;
    let status_h: u16 = STATUS_BAR_HEIGHT_CELLS;
    let text_h = vh.saturating_sub(status_h);

    if let Some(popup) = state.explorer_popup() {
        if let Some(background_id) = state.explorer_background_buffer_id() {
            draw_buffer_snapshot_for_id(state, background_id, vw, text_h, editor_text, window)?;
        }
        let (inner_w, inner_h) = explorer_popup_inner_size(vw, vh, style);
        state.set_viewport_size(
            inner_w as usize,
            inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
        );
        draw_explorer_popup_view(state, style, window, popup)?;
        return Ok(());
    }

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
    for (row, line) in snapshot.lines.iter().enumerate() {
        window.write_str_colored(row as u16, 0, line, editor_text)?;
    }

    // --- Status bar (bottom row) ---
    let status = build_editor_status_bar(state, style);

    status.draw(window)?;

    window.request_cursor(spec);

    Ok(())
}

fn fill_background(
    window: &mut dyn Window,
    width: u16,
    height: u16,
    colors: ColorPair,
) -> minui::Result<()> {
    if width == 0 || height == 0 {
        return Ok(());
    }

    let row = " ".repeat(width as usize);
    for y in 0..height {
        window.write_str_colored(y, 0, &row, colors)?;
    }
    Ok(())
}

fn draw_buffer_snapshot_for_id(
    state: &mut EditorState,
    buffer_id: BufferId,
    width: u16,
    height: u16,
    colors: ColorPair,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let Some(snapshot) = state.with_buffer_view_mut(buffer_id, |buffer, view| {
        let (scroll_x, scroll_y) = view.cursor.viewport_scroll();
        let viewport = TextViewport {
            scroll_x,
            scroll_y,
            width,
            height,
        };
        snapshot_lines_wrapped_cached(buffer, &viewport, &mut view.grapheme_cache)
    }) else {
        return Ok(());
    };

    for (row, line) in snapshot.lines.iter().enumerate() {
        window.write_str_colored(row as u16, 0, line, colors)?;
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
    let path = parse_path_arg().expect("file path required (e.g. redox-tui ./file.txt)");
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
