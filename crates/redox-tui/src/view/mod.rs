//! Draws the editor frame: the active surface, its popups, and the terminal cursor.

pub(crate) mod buffer;
pub(crate) mod cells;
pub(crate) mod gutter;
pub(crate) mod line;
pub(crate) mod panes;
pub(crate) mod popups;
pub(crate) mod surfaces;

use std::time::Instant;

use minui::prelude::{render::Window, widgets::Widget};
use minui::widgets::WindowView;

use crate::app::{EditorState, FramePerfSample};
use crate::ui::helpers::proportional_size;
use crate::ui::render::LineViewport;
use crate::ui::text_style::TextStyle;
use crate::ui::widgets::popup::{MousePopup, PopupLayout, PopupMouseLayout, popup_occludes_cursor};
use crate::ui::{
    STATUS_BAR_HEIGHT_CELLS, UiStyle, build_editor_status_bar, draw_code_actions_popup,
    draw_command_line_popup, draw_completion_popup, draw_completion_preview,
    draw_diagnostics_popup, draw_finder_popup, draw_lsp_marketplace_popup, draw_perf_popup_view,
    draw_pin_selector_popup, draw_symbol_info_popup, draw_which_key_popup,
    lsp_marketplace_popup_inner_size, perf_popup_layout,
};
use crate::view::buffer::draw_active_buffer;
use crate::view::cells::clipped_cell_width;
use crate::view::gutter::pane_gutter_layout;
use crate::view::line::{clear_inline_diagnostic, draw_inline_diagnostic_shifted};
use crate::view::panes::{
    CursorContext, active_split_cursor_context, draw_split_editor_view, start_pending_rain,
};
use crate::view::popups::{draw_notification_toast, finder_popup_inner_size};
use crate::view::surfaces::{
    draw_about_view, draw_dashboard_view, draw_explorer_view, draw_modal_list_view, draw_rain_view,
    draw_substitute_preview_view,
};

pub(crate) fn draw_buffer_view(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    perf: &mut FramePerfSample,
) -> minui::Result<()> {
    state.sync_focus(Instant::now());
    let (width, height) = window.get_size();
    let viewport_width = if state.zen.enabled {
        proportional_size(width, state.zen.width_percent, state.zen.min_width)
    } else {
        width
    };
    let origin = (width - viewport_width) / 2;
    if viewport_width != width {
        fill_background(
            window,
            width,
            height,
            TextStyle::new(style.zen_margin, style.zen_margin),
        )?;
    }
    let mut viewport = WindowView {
        window,
        x_offset: origin,
        y_offset: 0,
        scroll_x: 0,
        scroll_y: 0,
        width: viewport_width,
        height,
    };
    if let Err(error) = state.terminal.layout(
        origin,
        viewport_width,
        height,
        state.minimum_editor_height(),
    ) {
        state.set_status(format!("could not resize terminal: {error:#}"));
    }
    let editor_height = height.saturating_sub(state.terminal.height(height));
    state.set_mouse_viewport(origin, viewport_width, editor_height);
    {
        let mut editor = WindowView {
            window: &mut viewport,
            x_offset: 0,
            y_offset: 0,
            scroll_x: 0,
            scroll_y: 0,
            width: viewport_width,
            height: editor_height,
        };
        draw_editor_area(state, style, &mut editor, perf)?;
        if state.terminal.is_visible() {
            // Keep the separator visible for dashboards and popups too.
            build_editor_status_bar(state, style).draw(&mut editor)?;
        }
    }
    if state.terminal.is_focused() {
        hide_cursor(&mut viewport);
    }
    state.terminal.draw(
        &mut viewport,
        style,
        style.dim_amount * state.terminal_focus_dimming(Instant::now()),
    )
}

pub(crate) fn draw_editor_area(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    perf: &mut FramePerfSample,
) -> minui::Result<()> {
    // MinUI preserves the previous frame's cursor unless we request a new state.
    // Visible editor and popup cursors override this fallback during drawing.
    hide_cursor(window);
    state.mouse.popups.clear();
    let zen_style;
    let style = if state.zen.enabled && state.zen.hide_color_column {
        let mut without_color_column = *style;
        without_color_column.layout.color_column = None;
        zen_style = without_color_column;
        &zen_style
    } else {
        style
    };
    draw_editor_view(state, style, window, perf)?;
    for popup in &mut state.mouse.popups {
        popup.cache_interactions();
    }
    Ok(())
}

pub(crate) fn draw_editor_view(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    perf: &mut FramePerfSample,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let which_key_popup = state.which_key_popup(Instant::now());
    let background_style = style.dimmed_by(
        style.dim_amount * state.pane_focus_dimming(state.active_pane_id(), Instant::now()),
    );
    fill_background(window, vw, vh, background_style.editor_text)?;
    let text_h = vh.saturating_sub(STATUS_BAR_HEIGHT_CELLS);

    if let Some(selected) = state.dashboard_selection()
        && matches!(
            state.mode,
            crate::app::EditorMode::Normal
                | crate::app::EditorMode::Finder
                | crate::app::EditorMode::PinSelect
        )
    {
        return draw_dashboard_view(state, style, &background_style, window, selected);
    }
    if let Some(popup) = state.explorer_popup() {
        return draw_explorer_view(state, style, window, popup, text_h);
    }
    if let Some(popup) = state.about_popup() {
        return draw_about_view(state, style, window, popup, text_h);
    }
    if let Some(popup) = state.lsp_marketplace_popup() {
        let inner_size = lsp_marketplace_popup_inner_size(vw, vh, style);
        return draw_modal_list_view(state, style, window, inner_size, |window| {
            draw_lsp_marketplace_popup(&popup, style, window)
        });
    }
    if let Some(popup) = state.diagnostics_popup() {
        let inner_size = finder_popup_inner_size(vw, vh, style);
        return draw_modal_list_view(state, style, window, inner_size, |window| {
            draw_diagnostics_popup(&popup, style, window)
        });
    }
    if let Some(popup) = state.code_actions_popup() {
        let inner_size = finder_popup_inner_size(vw, vh, style);
        return draw_modal_list_view(state, style, window, inner_size, |window| {
            draw_code_actions_popup(&popup, style, window)
        });
    }

    let active_cursor_line = state.active_cursor_pos().line;
    let gutter = pane_gutter_layout(state, state.active_pane_id());
    let text_w = vw.saturating_sub(gutter.content_x);
    state.set_editor_area_size(vw as usize, text_h as usize);

    let cursor = if state.panes().len() > 1 {
        draw_split_editor_view(state, style, window, (vw, text_h))?;
        EditorCursor {
            context: active_split_cursor_context(state, vw, text_h),
            suppressed: state.active_rain_animation().is_some()
                || state
                    .undo_tree_surface_role(state.session.active_id())
                    .is_some(),
        }
    } else {
        state.set_viewport_size(text_w as usize, vh as usize);
        start_pending_rain(state, style, text_w, text_h);
        if state.active_rain_animation().is_some() {
            return draw_rain_view(
                state,
                style,
                &background_style,
                window,
                &gutter,
                (text_w, text_h),
                active_cursor_line,
            );
        }
        if state
            .substitute_preview()
            .is_some_and(|preview| preview.buffer.is_some())
        {
            return draw_substitute_preview_view(state, style, &background_style, window, text_h);
        }
        EditorCursor {
            context: draw_active_buffer(
                state,
                &background_style,
                window,
                &gutter,
                (text_w, text_h),
                active_cursor_line,
                perf,
            )?,
            suppressed: false,
        }
    };
    draw_editor_overlays(
        state,
        style,
        &background_style,
        window,
        which_key_popup.as_ref(),
        cursor,
        perf,
    )
}

/// The active buffer's cursor as the overlays see it.
pub(crate) struct EditorCursor {
    /// Where cursor-anchored popups attach, when the cursor is on screen.
    pub(crate) context: Option<CursorContext>,
    /// Rain and the undo tree own the pane, so the cursor stays hidden.
    pub(crate) suppressed: bool,
}

/// Draws the status bar and the popups layered over the buffer, then places the cursor.
pub(crate) fn draw_editor_overlays(
    state: &mut EditorState,
    style: &UiStyle,
    background_style: &UiStyle,
    window: &mut dyn Window,
    which_key_popup: Option<&crate::input::WhichKeyPopup>,
    cursor: EditorCursor,
    perf: &mut FramePerfSample,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let status_start = Instant::now();
    build_editor_status_bar(state, style).draw(window)?;
    perf.status += status_start.elapsed();

    let perf_popup_layout = if let Some(popup) = state.perf_popup()
        && !matches!(
            state.mode,
            crate::app::EditorMode::Command
                | crate::app::EditorMode::Search
                | crate::app::EditorMode::Finder
                | crate::app::EditorMode::PinSelect
        ) {
        let layout = perf_popup_layout(vw, vh, style);
        draw_perf_popup_view(style, window, popup)?;
        let mut mouse = PopupMouseLayout::new(MousePopup::Perf);
        mouse.add_frame(layout);
        state.mouse.popups.push(mouse);
        Some(layout)
    } else {
        None
    };

    let mut cursor_spec = None;
    let mut hide_editor_cursor = false;
    if matches!(
        state.mode,
        crate::app::EditorMode::Command | crate::app::EditorMode::Search
    ) {
        let mouse = draw_command_line_popup(state, style, window)?;
        state.mouse.popups.extend(mouse);
    } else if let Some(popup) = state.finder_popup() {
        let mouse = draw_finder_popup(&popup, style, window)?;
        state.mouse.popups.push(mouse);
    } else if let Some(popup) = state.pin_selector_popup() {
        let mouse = draw_pin_selector_popup(&popup, style, window)?;
        state.mouse.popups.push(mouse);
    } else if let Some(popup) = state.completion_popup() {
        if let Some(context) = &cursor.context {
            draw_completion_preview_at(state, style, background_style, window, context)?;
            let mouse =
                draw_completion_popup(&popup, style, window, context.x, context.y, context.height)?;
            state.mouse.popups.extend(mouse);
            cursor_spec = Some(context.spec());
        }
    } else if cursor.suppressed {
        hide_editor_cursor = true;
    } else if let Some(context) = &cursor.context {
        state.clamp_symbol_info_scroll(vw);
        if let Some(popup) = state.symbol_info_popup(vw) {
            let mouse = draw_symbol_info_popup(&popup, style, window, context.x, context.y)?;
            state.mouse.popups.extend(mouse);
        }
        cursor_spec = Some(context.spec());
    }

    let which_key_layout = if let Some(popup) = which_key_popup {
        draw_which_key_popup(popup, style, window)?.map(|(layout, mouse)| {
            state.mouse.popups.push(mouse);
            layout
        })
    } else {
        None
    };
    let toast_layout = draw_notification_toast(state, style, window)?;
    if hide_editor_cursor {
        hide_cursor(window);
    } else if let Some(cursor) = cursor_spec {
        request_cursor_unless_covered(
            window,
            cursor,
            &[perf_popup_layout, toast_layout],
            which_key_layout,
        );
    }
    Ok(())
}

/// Draws the inline completion ghost text at the cursor, shifting any inline diagnostic on the
/// cursor line past it.
pub(crate) fn draw_completion_preview_at(
    state: &EditorState,
    style: &UiStyle,
    background_style: &UiStyle,
    window: &mut dyn Window,
    context: &CursorContext,
) -> minui::Result<()> {
    let Some(preview) = state.completion_preview() else {
        return Ok(());
    };
    let (vw, _) = window.get_size();
    let preview_width = clipped_cell_width(&preview.text, vw.saturating_sub(context.x) as usize);
    let active_cursor_line = state.active_cursor_pos().line;
    let inline_diagnostic = state
        .active_diagnostic_lines(active_cursor_line, 1)
        .remove(&active_cursor_line);
    let source_line = inline_diagnostic.as_ref().map(|_| {
        state
            .session
            .active_buffer()
            .line_string(active_cursor_line)
    });
    if let Some(source_line) = source_line.as_deref() {
        clear_inline_diagnostic(
            window,
            context.y,
            context.content_x,
            source_line,
            context.scroll_x,
            context.text_w as usize,
            background_style,
        )?;
    }
    draw_completion_preview(
        window,
        style,
        context.x,
        context.y,
        vw.saturating_sub(context.x),
        &preview.text,
        &preview.suffix,
    )?;
    if let Some((diagnostic, source_line)) = inline_diagnostic.as_ref().zip(source_line.as_deref())
    {
        draw_inline_diagnostic_shifted(
            window,
            LineViewport {
                row: context.y,
                column: context.content_x,
                scroll_x: context.scroll_x,
                width: context.text_w as usize,
            },
            source_line,
            background_style,
            diagnostic,
            preview_width,
        )?;
    }
    Ok(())
}

/// Shows `cursor` unless a floating popup is drawn over it.
pub(crate) fn request_cursor_unless_covered(
    window: &mut dyn Window,
    cursor: minui::window::CursorSpec,
    popups: &[Option<PopupLayout>],
    which_key: Option<crate::ui::widgets::which_key::WhichKeyLayout>,
) {
    let covered = popups
        .iter()
        .flatten()
        .any(|layout| popup_occludes_cursor(*layout, cursor.x, cursor.y))
        || which_key.is_some_and(|layout| layout.occludes(cursor.x, cursor.y));
    if covered {
        hide_cursor(window);
    } else {
        window.request_cursor(cursor);
    }
}

pub(crate) fn fill_background(
    window: &mut dyn Window,
    width: u16,
    height: u16,
    colors: TextStyle,
) -> minui::Result<()> {
    if width == 0 || height == 0 {
        return Ok(());
    }

    let row = " ".repeat(width as usize);
    for y in 0..height {
        window.write_str_styled(y, 0, &row, colors.into())?;
    }
    Ok(())
}

pub(crate) fn hide_cursor(window: &mut dyn Window) {
    window.request_cursor(minui::window::CursorSpec {
        x: 0,
        y: 0,
        visible: false,
    });
}
