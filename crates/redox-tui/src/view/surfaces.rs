//! Frames for surfaces that replace the buffer view: dashboard, explorer, about, list popups,
//! rain and substitution previews.

use std::collections::BTreeMap;
use std::time::Instant;

use minui::prelude::{render::Window, widgets::Widget};

use crate::app::EditorState;
use crate::ui::widgets::dashboard::draw_dashboard;
use crate::ui::widgets::popup::PopupMouseLayout;
use crate::ui::{
    STATUS_BAR_HEIGHT_CELLS, UiStyle, about_popup_inner_size, build_editor_status_bar,
    draw_about_popup_view, draw_command_line_popup, draw_explorer_popup_view, draw_finder_popup,
    draw_pin_selector_popup, explorer_popup_inner_size,
};
use crate::view::buffer::{BufferDrawOptions, BufferHighlights, draw_buffer_snapshot_for_id};
use crate::view::gutter::{PaneGutterLayout, draw_active_gutter};
use crate::view::popups::{
    PopupStackLayout, draw_command_line_below_popup, draw_modal_popup_background,
    draw_notification_toast, draw_popup_background, popup_stack_layout,
};
use crate::view::{hide_cursor, request_cursor_unless_covered};

pub(crate) fn draw_dashboard_view(
    state: &mut EditorState,
    style: &UiStyle,
    background_style: &UiStyle,
    window: &mut dyn Window,
    selected: usize,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    state.set_viewport_size(vw as usize, vh as usize);
    draw_dashboard(
        window,
        background_style,
        selected,
        state.mode == crate::app::EditorMode::Normal,
        state.dashboard_logo_progress(Instant::now()),
    )?;
    if let Some(popup) = state.finder_popup() {
        let mouse = draw_finder_popup(&popup, style, window)?;
        state.mouse.popups.push(mouse);
    } else if let Some(popup) = state.pin_selector_popup() {
        let mouse = draw_pin_selector_popup(&popup, style, window)?;
        state.mouse.popups.push(mouse);
    }
    let _ = draw_notification_toast(state, style, window)?;
    Ok(())
}

/// Lays out a popup that replaces the editor surface and sizes the viewport to its interior.
pub(crate) fn surface_popup_layout(
    state: &mut EditorState,
    style: &UiStyle,
    window: &dyn Window,
    inner_size: (u16, u16),
) -> PopupStackLayout {
    let stack_layout = popup_stack_layout(state, style, window, inner_size);
    state.set_viewport_size(
        stack_layout.popup.inner_w as usize,
        stack_layout
            .popup
            .inner_h
            .saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
    );
    stack_layout
}

pub(crate) fn draw_explorer_view(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    popup: crate::app::ExplorerPopup,
    text_h: u16,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let fallback_id = state
        .explorer_background_buffer_id()
        .filter(|_| !state.explorer_background_is_placeholder_blank());
    draw_popup_background(state, style, window, vw, text_h, fallback_id)?;
    let inner_size = explorer_popup_inner_size(vw, vh, style);
    let stack_layout = surface_popup_layout(state, style, window, inner_size);
    let (cursor_spec, mouse) = draw_explorer_popup_view(
        state,
        style,
        window,
        popup,
        stack_layout.popup,
        inner_size.1,
    )?;
    state.mouse.popups.push(mouse);
    let in_command_line = matches!(
        state.mode,
        crate::app::EditorMode::Command | crate::app::EditorMode::Search
    );
    if in_command_line && !draw_command_line_below_popup(state, style, window, stack_layout)? {
        let mouse = draw_command_line_popup(state, style, window)?;
        state.mouse.popups.extend(mouse);
    }
    let toast_layout = draw_notification_toast(state, style, window)?;
    if !in_command_line && let Some(cursor) = cursor_spec {
        request_cursor_unless_covered(window, cursor, &[toast_layout], None);
    }
    Ok(())
}

pub(crate) fn draw_about_view(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    popup: crate::app::AboutPopup,
    text_h: u16,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    let fallback_id = state.about_background_buffer_id();
    draw_popup_background(state, style, window, vw, text_h, fallback_id)?;
    let inner_size = about_popup_inner_size(vw, vh, style);
    let stack_layout = surface_popup_layout(state, style, window, inner_size);
    let mouse = draw_about_popup_view(state, style, window, popup, stack_layout.popup)?;
    state.mouse.popups.push(mouse);
    if !draw_command_line_below_popup(state, style, window, stack_layout)? {
        if matches!(
            state.mode,
            crate::app::EditorMode::Command | crate::app::EditorMode::Search
        ) {
            let mouse = draw_command_line_popup(state, style, window)?;
            state.mouse.popups.extend(mouse);
        } else {
            hide_cursor(window);
        }
    }
    let _ = draw_notification_toast(state, style, window)?;
    Ok(())
}

/// Draws a modal list popup (LSP marketplace, diagnostics, code actions) over the active buffer.
pub(crate) fn draw_modal_list_view(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    inner_size: (u16, u16),
    draw_popup: impl FnOnce(&mut dyn Window) -> minui::Result<PopupMouseLayout>,
) -> minui::Result<()> {
    let (vw, vh) = window.get_size();
    draw_modal_popup_background(
        state,
        style,
        window,
        (vw, vh.saturating_sub(STATUS_BAR_HEIGHT_CELLS)),
        Some(state.session.active_id()),
        inner_size,
    )?;
    let mouse = draw_popup(window)?;
    state.mouse.popups.push(mouse);
    let _ = draw_notification_toast(state, style, window)?;
    hide_cursor(window);
    Ok(())
}

pub(crate) fn draw_rain_view(
    state: &mut EditorState,
    style: &UiStyle,
    background_style: &UiStyle,
    window: &mut dyn Window,
    gutter: &PaneGutterLayout,
    (text_w, text_h): (u16, u16),
    active_cursor_line: usize,
) -> minui::Result<()> {
    if let Some(animation) = state.active_rain_animation() {
        draw_active_gutter(
            state,
            background_style,
            window,
            gutter,
            (text_h, animation.first_line(), active_cursor_line),
            None,
        )?;
        animation.draw(
            window,
            0,
            gutter.content_x,
            text_w as usize,
            text_h as usize,
        )?;
    }

    build_editor_status_bar(state, style).draw(window)?;
    if matches!(
        state.mode,
        crate::app::EditorMode::Command | crate::app::EditorMode::Search
    ) {
        let mouse = draw_command_line_popup(state, style, window)?;
        state.mouse.popups.extend(mouse);
    } else {
        hide_cursor(window);
    }
    Ok(())
}

pub(crate) fn draw_substitute_preview_view(
    state: &mut EditorState,
    style: &UiStyle,
    background_style: &UiStyle,
    window: &mut dyn Window,
    text_h: u16,
) -> minui::Result<()> {
    let (vw, _) = window.get_size();
    draw_buffer_snapshot_for_id(
        state,
        background_style,
        state.session.active_id(),
        BufferDrawOptions {
            width: vw,
            height: text_h,
            has_line_numbers: state.pane_options(state.active_pane_id()).has_line_numbers,
            colors: background_style.editor_text,
        },
        window,
        BufferHighlights {
            visual_selection: None,
            one_shot_highlight: None,
            search_highlights: &BTreeMap::new(),
            diagnostic_lines: &BTreeMap::new(),
            snippet_placeholders: &BTreeMap::new(),
        },
    )?;
    build_editor_status_bar(state, style).draw(window)?;
    let mouse = draw_command_line_popup(state, style, window)?;
    state.mouse.popups.extend(mouse);
    let _ = draw_notification_toast(state, style, window)?;
    Ok(())
}
