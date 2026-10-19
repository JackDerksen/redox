//! Popup placement, backgrounds and toasts shared by the editor surfaces.

use std::collections::BTreeMap;
use std::time::Instant;

use minui::prelude::{
    render::{TabPolicy, Window, cell_width},
    widgets::Widget,
};
use redox_core::BufferId;

use crate::app::EditorState;
use crate::ui::widgets::dashboard::draw_dashboard;
use crate::ui::widgets::popup::{PopupLayout, anchored_popup_origin};
use crate::ui::{
    STATUS_BAR_HEIGHT_CELLS, UiStyle, build_editor_status_bar, draw_command_line_popup_below,
    draw_status_toast,
};
use crate::view::buffer::{BufferDrawOptions, BufferHighlights, draw_buffer_snapshot_for_id};
use crate::view::panes::draw_split_editor_panes;

pub(crate) fn draw_loading_toast(
    state: &EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
) -> minui::Result<Option<crate::ui::widgets::popup::PopupLayout>> {
    let Some(message) = state.active_loading_toast(Instant::now()) else {
        return Ok(None);
    };
    let (term_w, term_h) = window.get_size();
    let y = crate::ui::widgets::command_line::search_toast_layout(state, term_w, term_h)
        .map_or(0, |layout| layout.y.saturating_add(layout.outer_h()));
    let width = (cell_width(&message, TabPolicy::Fixed(4)) as u16)
        .saturating_add(2)
        .min(term_w.saturating_sub(2));
    if width <= 2 || term_h.saturating_sub(y) <= 2 {
        return Ok(None);
    }
    let popup_w = width.saturating_add(2);
    let x = term_w.saturating_sub(popup_w.saturating_add(1));
    let layout = crate::ui::widgets::popup::draw_popup_frame_at(
        window,
        x,
        y,
        width,
        1,
        "",
        crate::ui::widgets::popup::PopupChrome::command_line(style),
    )?;
    let mut view = crate::ui::widgets::popup::popup_window_view(window, layout);
    view.write_str_styled(0, 1, &message, style.command_line.text.into())?;
    Ok(Some(layout))
}

pub(crate) fn draw_notification_toast(
    state: &EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
) -> minui::Result<Option<PopupLayout>> {
    let layout = draw_status_toast(state, style, window)?;
    if layout.is_some() {
        Ok(layout)
    } else {
        draw_loading_toast(state, style, window)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PopupStackLayout {
    pub(crate) popup: PopupLayout,
    pub(crate) command_padding: Option<u16>,
}

pub(crate) fn anchored_popup_layout(window: &dyn Window, inner_size: (u16, u16)) -> PopupLayout {
    let (term_w, term_h) = window.get_size();
    let (inner_w, inner_h) = inner_size;
    let (x, y) = anchored_popup_origin(term_w, term_h, inner_w, inner_h);
    PopupLayout {
        inner_w,
        inner_h,
        x,
        y,
    }
}

pub(crate) fn popup_stack_layout(
    state: &EditorState,
    style: &UiStyle,
    window: &dyn Window,
    inner_size: (u16, u16),
) -> PopupStackLayout {
    let (_, term_h) = window.get_size();
    let (_, inner_h) = inner_size;
    let popup = anchored_popup_layout(window, inner_size);
    if !matches!(
        state.mode,
        crate::app::EditorMode::Command | crate::app::EditorMode::Search
    ) {
        return PopupStackLayout {
            popup,
            command_padding: None,
        };
    }

    let available_h = term_h.saturating_sub(STATUS_BAR_HEIGHT_CELLS);
    if popup.y.saturating_add(popup.outer_h()) > available_h {
        return PopupStackLayout {
            popup,
            command_padding: None,
        };
    }

    const MIN_POPUP_INNER_HEIGHT: u16 = 1;
    let command_outer_h = style
        .command_line
        .inner_height_rows
        .max(1)
        .saturating_add(2);
    let required_inner_h = command_outer_h.saturating_add(MIN_POPUP_INNER_HEIGHT);
    if inner_h < required_inner_h {
        return PopupStackLayout {
            popup,
            command_padding: None,
        };
    }

    let max_padding = inner_h.saturating_sub(required_inner_h);
    let command_padding = style.command_line.stacked_padding.min(max_padding);
    let popup = PopupLayout {
        inner_h: inner_h
            .saturating_sub(command_outer_h)
            .saturating_sub(command_padding),
        ..popup
    };
    PopupStackLayout {
        popup,
        command_padding: Some(command_padding),
    }
}

pub(crate) fn draw_command_line_below_popup(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    stack_layout: PopupStackLayout,
) -> minui::Result<bool> {
    let Some(command_padding) = stack_layout.command_padding else {
        return Ok(false);
    };
    let mouse =
        draw_command_line_popup_below(state, style, window, stack_layout.popup, command_padding)?;
    let drawn = mouse.is_some();
    state.mouse.popups.extend(mouse);
    Ok(drawn)
}

pub(crate) fn draw_modal_popup_background(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    (width, text_height): (u16, u16),
    fallback_buffer_id: Option<BufferId>,
    inner_size: (u16, u16),
) -> minui::Result<()> {
    draw_popup_background(state, style, window, width, text_height, fallback_buffer_id)?;
    let (inner_w, inner_h) = inner_size;
    state.set_viewport_size(
        inner_w as usize,
        inner_h.saturating_add(STATUS_BAR_HEIGHT_CELLS) as usize,
    );
    build_editor_status_bar(state, style).draw(window)
}

pub(crate) fn finder_popup_inner_size(term_w: u16, term_h: u16, style: &UiStyle) -> (u16, u16) {
    crate::ui::widgets::popup::popup_inner_size(
        term_w,
        term_h,
        style.finder.width_percent,
        style.finder.height_percent,
        style.finder.min_width,
        style.finder.min_height,
    )
}

pub(crate) fn draw_popup_background(
    state: &mut EditorState,
    style: &UiStyle,
    window: &mut dyn Window,
    width: u16,
    height: u16,
    fallback_buffer_id: Option<BufferId>,
) -> minui::Result<()> {
    let background_style = style.dimmed_by(
        style.dim_amount * state.pane_focus_dimming(state.active_pane_id(), Instant::now()),
    );
    if let Some(selected) =
        fallback_buffer_id.and_then(|id| state.dashboard_selection_for_buffer(id))
    {
        return draw_dashboard(
            window,
            &background_style,
            selected,
            false,
            state.dashboard_logo_progress(Instant::now()),
        );
    }
    if state.panes().len() > 1 {
        let active_before_draw = state.session.active_id();
        state.sync_active_pane_view();
        draw_split_editor_panes(state, style, window, width, height)?;
        let _ = state.session.activate(active_before_draw);
        return Ok(());
    }

    if let Some(buffer_id) = fallback_buffer_id {
        draw_buffer_snapshot_for_id(
            state,
            &background_style,
            buffer_id,
            BufferDrawOptions {
                width,
                height,
                has_line_numbers: true,
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
    }
    Ok(())
}
