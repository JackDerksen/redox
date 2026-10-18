//! Split panes: drawing each pane and locating the active cursor.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use minui::prelude::render::Window;
use minui::widgets::WindowView;
use redox_core::BufferId;

use crate::GUTTER_CONTENT_PADDING;
use crate::app::EditorState;
use crate::ui::widgets::pane::draw_pane_filename;
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, UiStyle, draw_pane_split_lines};
use crate::view::buffer::{BufferDrawOptions, BufferHighlights, draw_buffer_snapshot_for_id};
use crate::view::gutter::{
    PaneGutterLayout, draw_gutter_padding, draw_line_numbers, pane_content_x, pane_gutter_layout,
};

pub(crate) fn draw_split_editor_view(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    (vw, text_h): (u16, u16),
) -> minui::Result<()> {
    if let Some(rect) = state
        .pane_rects(vw, text_h)
        .into_iter()
        .find(|rect| rect.pane_id == state.active_pane_id())
    {
        let content_x = pane_content_x(state, rect.pane_id);
        let pane_text_w = rect.width.saturating_sub(content_x);
        state.set_viewport_size(
            pane_text_w as usize,
            rect.height.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
        );
        start_pending_rain(state, style, pane_text_w, rect.height);
    }
    state.sync_active_pane_view();
    draw_split_editor_panes(state, style, window, vw, text_h)
}

pub(crate) fn draw_split_editor_panes(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    width: u16,
    height: u16,
) -> minui::Result<()> {
    let rects = state.pane_rects(width, height);
    let now = Instant::now();
    for rect in rects.iter().copied() {
        let Some((buffer_id, view, options)) = state
            .panes()
            .iter()
            .find(|pane| pane.id == rect.pane_id)
            .map(|pane| (pane.buffer_id, pane.view.clone(), pane.options))
        else {
            continue;
        };
        state
            .views
            .entry(buffer_id)
            .or_default()
            .copy_pane_state_from(&view);
        let dimming = state.pane_focus_dimming(rect.pane_id, now);
        let pane_style = style.dimmed_by(style.dim_amount * dimming);
        let is_active_pane = rect.pane_id == state.active_pane_id();
        let inactive_filename = (!is_active_pane)
            .then(|| inactive_pane_filename(state, buffer_id))
            .flatten();
        let mut pane_window = WindowView {
            window,
            x_offset: rect.x,
            y_offset: rect.y,
            scroll_x: 0,
            scroll_y: 0,
            width: rect.width,
            height: rect.height,
        };
        if is_active_pane && state.active_rain_animation().is_some() {
            draw_active_split_rain_pane(
                state,
                &pane_style,
                rect.width,
                rect.height,
                &mut pane_window,
            )?;
        } else {
            let visual_selection = is_active_pane
                .then(|| state.active_visual_selection())
                .flatten();
            let one_shot_highlight = is_active_pane.then(|| state.one_shot_highlight()).flatten();
            let total_lines = state
                .session
                .buffer(buffer_id)
                .map_or(0, |buffer| buffer.len_lines().max(1));
            let (_, scroll_y) = view.cursor.viewport_scroll();
            let first_line = scroll_y.min(total_lines.saturating_sub(1));
            let visible_len = (rect.height as usize).min(total_lines.saturating_sub(first_line));
            let diagnostic_lines = if is_active_pane {
                state.active_diagnostic_lines(first_line, visible_len)
            } else {
                BTreeMap::new()
            };
            let snippet_placeholders = if is_active_pane {
                state.active_snippet_placeholder_ranges(first_line, visible_len)
            } else {
                BTreeMap::new()
            };
            let search_highlights = if is_active_pane {
                state.active_search_highlight_ranges(first_line, visible_len)
            } else {
                BTreeMap::new()
            };
            draw_buffer_snapshot_for_id(
                state,
                &pane_style,
                buffer_id,
                BufferDrawOptions {
                    width: rect.width,
                    height: rect.height,
                    has_line_numbers: options.has_line_numbers,
                    colors: pane_style.editor_text,
                },
                &mut pane_window,
                BufferHighlights {
                    visual_selection,
                    one_shot_highlight,
                    search_highlights: &search_highlights,
                    diagnostic_lines: &diagnostic_lines,
                    snippet_placeholders: &snippet_placeholders,
                },
            )?;
        }
        if let Some((filename, dirty)) = inactive_filename {
            draw_pane_filename(&mut pane_window, &filename, dirty, pane_style.pane_title)?;
        }
        state.sync_rendered_pane_view(rect.pane_id, buffer_id);
    }
    state.restore_active_pane_view();
    let style = style.dimmed_by(style.dim_amount * state.popup_background_dimming(now));
    draw_pane_split_lines(window, &style, &rects, width, height)
}

pub(crate) fn inactive_pane_filename(
    state: &EditorState,
    buffer_id: BufferId,
) -> Option<(String, bool)> {
    let meta = state
        .session
        .meta(buffer_id)
        .filter(|meta| meta.kind == redox_core::BufferKind::File)?;
    let filename = if let Some(path) = meta.path.as_deref()
        && let Some(filename) = path.file_name()
    {
        let mut label = PathBuf::from(filename);
        let mut parent = path.parent();
        for _ in 0..2 {
            let duplicate = state
                .panes()
                .iter()
                .filter_map(|pane| state.session.meta(pane.buffer_id))
                .filter_map(|meta| meta.path.as_deref())
                .any(|other| other != path && other.ends_with(&label));
            if !duplicate {
                break;
            }
            let Some(directory) = parent.and_then(std::path::Path::file_name) else {
                break;
            };
            label = PathBuf::from(directory).join(label);
            parent = parent.and_then(std::path::Path::parent);
        }
        label.to_string_lossy().into_owned()
    } else {
        meta.display_name.clone()
    };
    Some((filename, meta.dirty))
}

/// Where the active pane's visible cursor sits on screen, and the text area around it.
pub(crate) struct CursorContext {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) content_x: u16,
    pub(crate) text_w: u16,
    pub(crate) height: u16,
    pub(crate) scroll_x: usize,
}

impl CursorContext {
    pub(crate) fn spec(&self) -> minui::window::CursorSpec {
        minui::window::CursorSpec {
            x: self.x,
            y: self.y,
            visible: true,
        }
    }
}

pub(crate) fn active_split_cursor_context(
    state: &mut EditorState,
    width: u16,
    height: u16,
) -> Option<CursorContext> {
    let rect = state
        .pane_rects(width, height)
        .into_iter()
        .find(|rect| rect.pane_id == state.active_pane_id())?;
    let local_content_x = pane_content_x(state, rect.pane_id);
    let content_x = rect.x.saturating_add(local_content_x);
    let text_w = rect.width.saturating_sub(local_content_x);
    let (spec, scroll_x) = state.with_active_buffer_view_mut(|buffer, view| {
        let (scroll_x, _) = view.cursor.viewport_scroll();
        (
            view.cursor
                .cursor_spec(buffer, text_w as usize, rect.height as usize),
            scroll_x,
        )
    });
    spec.visible.then_some(CursorContext {
        x: content_x.saturating_add(spec.x),
        y: rect.y.saturating_add(spec.y),
        content_x,
        text_w,
        height: rect.height,
        scroll_x,
    })
}

/// Rain freezes the colours it captures, so take them from where the active
/// pane's dimming settles, not from a frame partway through a focus fade
/// (such as the command prompt that ran `:rain` fading out).
pub(crate) fn start_pending_rain(
    state: &mut EditorState,
    style: &UiStyle,
    text_w: u16,
    text_h: u16,
) {
    if !state.rain_is_active() || state.active_rain_animation().is_some() {
        return;
    }
    let resting = style
        .dimmed_by(style.dim_amount * state.resting_pane_focus_dimming(state.active_pane_id()));
    state.ensure_rain_animation(text_w, text_h, resting.editor_text, &resting);
}

pub(crate) fn draw_active_split_rain_pane(
    state: &mut EditorState,
    style: &UiStyle,
    width: u16,
    height: u16,
    window: &mut dyn Window,
) -> minui::Result<()> {
    let gutter = pane_gutter_layout(state, state.active_pane_id());
    let PaneGutterLayout {
        total_lines,
        show_marker_column,
        gutter_w,
        content_x,
    } = gutter;
    let active_cursor_line = state.active_cursor_pos().line;
    let Some(animation) = state.active_rain_animation() else {
        return Ok(());
    };

    draw_line_numbers(
        window,
        style,
        (gutter_w, height),
        u16::from(show_marker_column),
        animation.first_line()..total_lines,
        active_cursor_line,
        None,
    )?;
    draw_gutter_padding(
        window,
        style,
        gutter_w,
        height,
        GUTTER_CONTENT_PADDING,
        animation.first_line(),
        (state.active_git_diff(), Some(state.session.active_buffer())),
    )?;
    animation.draw(
        window,
        0,
        content_x,
        width.saturating_sub(content_x) as usize,
        height as usize,
    )
}
