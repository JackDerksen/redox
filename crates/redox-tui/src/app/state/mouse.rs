use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use minui::{Event, MouseButton};
use redox_core::{BufferKind, Pos, TextBuffer};
use unicode_segmentation::UnicodeSegmentation;

use super::{
    EditorMode, EditorState, PaneId, PaneRect, UndoTreeSurfaceRole, char_col_at_or_before_cell,
};
use crate::input::cursor::CursorController;
use crate::input::{InputAction, InputMode};
use crate::ui::widgets::popup::{MousePopup, MouseScroll, MouseTarget, PopupMouseLayout};
use crate::ui::{STATUS_BAR_HEIGHT_CELLS, STATUS_BAR_HEIGHT_ROWS, UNDO_TREE_HEADER_ROWS};

mod popups;

const SCROLL_GESTURE_PAUSE: Duration = Duration::from_millis(250);

#[derive(Debug, Default)]
pub(crate) struct MouseState {
    pub enabled: bool,
    pub pending_key: Option<Event>,
    pub popups: Vec<PopupMouseLayout>,
    pub list_scroll: BTreeMap<(MousePopup, MouseScroll), usize>,
    invert_vertical: bool,
    invert_horizontal: bool,
    scroll_filter: ScrollFilter,
    wheel_gesture: Option<WheelGesture>,
    drag: Option<(PaneId, Pos)>,
    origin: u16,
    width: u16,
    height: u16,
    last_click: Option<PopupClick>,
}

#[derive(Debug)]
struct PopupClick {
    popup: MousePopup,
    target: MouseTarget,
    position: (u16, u16),
    time: Instant,
}

#[derive(Debug)]
struct WheelGesture {
    popup: Option<MousePopup>,
    last_event: Instant,
}

impl MouseState {
    fn accepts_wheel_context(&mut self, popup: Option<MousePopup>, now: Instant) -> bool {
        let gesture = self.wheel_gesture.get_or_insert(WheelGesture {
            popup,
            last_event: now,
        });
        let closed_popup_momentum = now.duration_since(gesture.last_event) < SCROLL_GESTURE_PAUSE
            && gesture.popup.is_some()
            && gesture.popup != popup;
        gesture.last_event = now;
        if closed_popup_momentum {
            return false;
        }
        gesture.popup = popup;
        true
    }
}

#[derive(Debug, Default)]
struct ScrollFilter {
    horizontal: bool,
    pending_delta: i8,
    last_event: Option<Instant>,
}

impl ScrollFilter {
    fn accepts(&mut self, horizontal: bool, delta: i8, now: Instant) -> bool {
        if delta == 0 {
            return false;
        }
        if self
            .last_event
            .is_some_and(|last| now.duration_since(last) >= SCROLL_GESTURE_PAUSE)
        {
            *self = Self::default();
        }
        self.last_event = Some(now);
        if horizontal == self.horizontal {
            self.pending_delta = 0;
            return true;
        }

        // Prefer vertical movement at gesture start. Require deliberate movement
        // to change axes, discarding noise instead of moving along the wrong axis.
        self.pending_delta += delta.signum();
        let threshold = if horizontal { 3 } else { 2 };
        if self.pending_delta.abs() < threshold {
            return false;
        }
        self.horizontal = horizontal;
        self.pending_delta = 0;
        true
    }
}

impl EditorState {
    pub(crate) fn configure_mouse(
        &mut self,
        enabled: bool,
        invert_vertical: bool,
        invert_horizontal: bool,
    ) {
        if self.mouse.enabled != enabled {
            self.mouse.drag = None;
            self.mouse.scroll_filter = ScrollFilter::default();
            self.mouse.wheel_gesture = None;
            self.mouse.last_click = None;
            self.mouse.list_scroll.clear();
        }
        self.mouse.enabled = enabled;
        self.mouse.invert_vertical = invert_vertical;
        self.mouse.invert_horizontal = invert_horizontal;
    }

    pub(crate) fn set_mouse_viewport(&mut self, origin: u16, width: u16, height: u16) {
        if (origin, width, height) != (self.mouse.origin, self.mouse.width, self.mouse.height) {
            self.mouse.drag = None;
            self.mouse.last_click = None;
            self.mouse.list_scroll.clear();
        }
        self.mouse.origin = origin;
        self.mouse.width = width;
        self.mouse.height = height;
    }

    pub(crate) fn handle_mouse_input(&mut self, event: &Event) -> bool {
        let is_mouse = crate::input::is_mouse_event(event);
        if !is_mouse {
            self.mouse.last_click = None;
            self.mouse.list_scroll.clear();
        }
        if self.mouse.enabled && is_mouse {
            let is_wheel = matches!(
                event,
                Event::MouseScroll { .. } | Event::MouseScrollHorizontal { .. }
            );
            let popup_index = self.mouse.popups.iter().rposition(|popup| {
                self.mouse_popup_is_active(popup.kind)
                    && !(is_wheel && popup.kind == MousePopup::Perf)
            });
            if is_wheel
                && !self.mouse.accepts_wheel_context(
                    popup_index.map(|index| self.mouse.popups[index].kind),
                    Instant::now(),
                )
            {
                return true;
            }
            if let Some(popup_index) = popup_index {
                self.handle_popup_mouse(event, popup_index);
                return true;
            }
        }
        let editor_visible = matches!(
            self.mode,
            EditorMode::Normal
                | EditorMode::Insert
                | EditorMode::Visual
                | EditorMode::VisualLine
                | EditorMode::VisualBlock
        ) && (!self.active_buffer_is_surface() || self.undo_tree_is_active())
            && self.dashboard_selection().is_none()
            && !self.rain_is_active();
        if !self.mouse.enabled || !editor_visible {
            self.mouse.drag = None;
            self.mouse.scroll_filter = ScrollFilter::default();
            return is_mouse;
        }

        match *event {
            Event::MouseClick { x, y, button } => {
                self.mouse.drag = None;
                self.mouse.scroll_filter = ScrollFilter::default();
                if button != MouseButton::Left {
                    return true;
                }
                let Some((pane, position)) = self.mouse_position(x, y, None) else {
                    return true;
                };
                let was_insert = self.mode == EditorMode::Insert;
                let (width, height) = self.viewport_size();
                let scroll =
                    self.with_active_buffer_view_mut(|_, view| view.cursor.viewport_scroll());
                self.apply_input(InputAction::SetMode(InputMode::Normal), width, height);
                self.with_active_buffer_view_mut(|_, view| {
                    view.cursor.scroll_x_cells = scroll.0;
                    view.cursor.scroll_y_lines = scroll.1;
                });
                self.activate_pane(pane);
                if let Some(rect) = self
                    .pane_rects(
                        self.mouse.width,
                        self.mouse.height.saturating_sub(STATUS_BAR_HEIGHT_CELLS),
                    )
                    .into_iter()
                    .find(|rect| rect.pane_id == pane)
                {
                    self.viewport_width_cells =
                        rect.width.saturating_sub(crate::pane_content_x(self, pane)) as usize;
                    self.viewport_height_rows = rect.height as usize + STATUS_BAR_HEIGHT_ROWS;
                }
                self.close_completion();
                self.close_active_snippet();
                let clamp_normal = !was_insert && !self.undo_tree_is_active();
                self.with_active_buffer_view_mut(|buffer, view| {
                    view.cursor.place_cursor(position);
                    if clamp_normal {
                        view.cursor.clamp_for_normal_mode(buffer);
                    }
                });
                if self.undo_tree_is_active() {
                    self.clamp_undo_tree_cursor();
                } else {
                    if was_insert {
                        self.mode = EditorMode::Insert;
                    }
                    self.mouse.drag = Some((pane, position));
                }
                self.clear_status();
                self.input.reset_prefixes();
                self.sync_active_pane_view();
            }
            Event::MouseDrag {
                x,
                y,
                button: MouseButton::Left,
            } => self.drag_mouse_to(x, y),
            Event::MouseRelease {
                x,
                y,
                button: MouseButton::Left,
            } => {
                if self.mode == EditorMode::Visual {
                    self.drag_mouse_to(x, y);
                }
                self.mouse.drag = None;
            }
            Event::MouseScroll { x, y, delta } => {
                if self
                    .mouse
                    .scroll_filter
                    .accepts(false, delta, Instant::now())
                {
                    let direction = if self.mouse.invert_vertical { -1 } else { 1 };
                    self.scroll_mouse_at(x, y, isize::from(delta) * 3 * direction, 0);
                }
            }
            Event::MouseScrollHorizontal { x, y, delta } => {
                if self
                    .mouse
                    .scroll_filter
                    .accepts(true, delta, Instant::now())
                {
                    let direction = if self.mouse.invert_horizontal { -1 } else { 1 };
                    self.scroll_mouse_at(x, y, 0, isize::from(delta) * 3 * direction);
                }
            }
            Event::MouseMove { .. } | Event::MouseRelease { .. } | Event::MouseDrag { .. } => {}
            _ => {
                self.mouse.drag = None;
                self.mouse.scroll_filter = ScrollFilter::default();
                return false;
            }
        }
        true
    }

    fn handle_popup_mouse(&mut self, event: &Event, popup_index: usize) {
        let popup = &mut self.mouse.popups[popup_index];
        let kind = popup.kind;
        self.mouse.drag = None;
        match *event {
            Event::MouseClick {
                button: MouseButton::Left,
                x,
                y,
            } => {
                let pointer = x.checked_sub(self.mouse.origin).map(|x| (x, y));
                self.mouse.scroll_filter = ScrollFilter::default();
                let hit = pointer.and_then(|(x, y)| popup.hit_click(x, y));
                if hit.is_none() {
                    self.mouse.last_click = None;
                    self.mouse.list_scroll.clear();
                    self.dismiss_mouse_popup(kind);
                } else if let Some(Some(target)) = hit {
                    let position = pointer.expect("inside popup has coordinates");
                    let now = Instant::now();
                    let activate = self.mouse.last_click.as_ref().is_some_and(|last| {
                        last.popup == kind
                            && last.target == target
                            && last.position.0.abs_diff(position.0) <= 1
                            && last.position.1.abs_diff(position.1) <= 1
                            && now.duration_since(last.time) <= Duration::from_millis(400)
                    });
                    self.mouse.last_click = (!activate).then(|| PopupClick {
                        popup: kind,
                        target: target.clone(),
                        position,
                        time: now,
                    });
                    self.click_mouse_popup(kind, &target, activate);
                } else {
                    self.mouse.last_click = None;
                }
            }
            Event::MouseScroll { x, y, delta } | Event::MouseScrollHorizontal { x, y, delta } => {
                self.mouse.last_click = None;
                let horizontal = matches!(event, Event::MouseScrollHorizontal { .. });
                let region = x.checked_sub(self.mouse.origin).and_then(|x| {
                    let local = if horizontal {
                        Event::MouseScrollHorizontal { x, y, delta }
                    } else {
                        Event::MouseScroll { x, y, delta }
                    };
                    popup.hit_scroll(&local)
                });
                if let Some((rect, region)) = region
                    && self
                        .mouse
                        .scroll_filter
                        .accepts(horizontal, delta, Instant::now())
                {
                    let invert = if horizontal {
                        self.mouse.invert_horizontal
                    } else {
                        self.mouse.invert_vertical
                    };
                    let distance = isize::from(delta) * 3 * if invert { -1 } else { 1 };
                    let (rows, columns) = if horizontal {
                        (0, distance)
                    } else {
                        (distance, 0)
                    };
                    self.scroll_mouse_popup(kind, region, rows, columns, rect);
                }
            }
            Event::MouseDrag { .. } => self.mouse.last_click = None,
            _ => {}
        }
    }

    fn mouse_position(
        &self,
        column: u16,
        row: u16,
        drag_pane: Option<PaneId>,
    ) -> Option<(PaneId, Pos)> {
        let rect = if let Some(pane) = drag_pane {
            self.pane_rects(
                self.mouse.width,
                self.mouse.height.saturating_sub(STATUS_BAR_HEIGHT_CELLS),
            )
            .into_iter()
            .find(|rect| rect.pane_id == pane)?
        } else {
            self.mouse_pane_at(column, row)?
        };
        let column = column.saturating_sub(self.mouse.origin);
        let pane = self.panes.iter().find(|pane| pane.id == rect.pane_id)?;
        if !pane.options.accessible || rect.width == 0 || rect.height == 0 {
            return None;
        }
        let active = pane.id == self.active_pane_id();
        let buffer_id = if active {
            self.session.active_id()
        } else {
            pane.buffer_id
        };
        let is_tree = self.undo_tree_surface_role(buffer_id) == Some(UndoTreeSurfaceRole::Tree);
        if self.session.meta(buffer_id)?.kind != BufferKind::File && !is_tree {
            return None;
        }
        let buffer = self.session.buffer(buffer_id)?;
        let view = if active {
            self.views.get(&buffer_id)?
        } else {
            &pane.view
        };
        let gutter = if pane.options.has_line_numbers && !(self.zen.enabled && self.zen.hide_gutter)
        {
            crate::line_number_gutter_width(
                buffer.len_lines(),
                u16::from(crate::git_marker_column_visible(
                    self.git_diff_for_buffer(buffer_id),
                )),
            )
            .saturating_add(crate::GUTTER_CONTENT_PADDING)
        } else {
            0
        };
        if rect.width <= gutter {
            return None;
        }
        let header = if is_tree {
            UNDO_TREE_HEADER_ROWS
        } else {
            u16::from(!active)
        };
        let line = buffer.clamp_line(
            view.cursor.scroll_y_lines.saturating_add(
                row.clamp(rect.y, rect.y + rect.height - 1)
                    .saturating_sub(rect.y + header) as usize,
            ),
        );
        let cell = view.cursor.scroll_x_cells.saturating_add(
            column
                .clamp(rect.x, rect.x + rect.width - 1)
                .saturating_sub(rect.x + gutter) as usize,
        );
        let position = Pos::new(
            line,
            char_col_at_or_before_cell(&buffer.line_string(line), cell),
        );
        Some((pane.id, position))
    }

    fn drag_mouse_to(&mut self, column: u16, row: u16) {
        let Some((pane, anchor)) = self.mouse.drag else {
            return;
        };
        let rect = self
            .pane_rects(
                self.mouse.width,
                self.mouse.height.saturating_sub(STATUS_BAR_HEIGHT_CELLS),
            )
            .into_iter()
            .find(|rect| rect.pane_id == pane);
        if let Some(PaneRect {
            x,
            y,
            width,
            height,
            ..
        }) = rect
        {
            let horizontal = if column < self.mouse.origin + x {
                -1
            } else if column >= self.mouse.origin + x + width {
                1
            } else {
                0
            };
            let vertical = if row < y {
                -1
            } else if row >= y + height {
                1
            } else {
                0
            };
            self.scroll_mouse(vertical, horizontal);
        }
        let Some((_, position)) = self.mouse_position(column, row, Some(pane)) else {
            return;
        };
        if position == anchor && self.mode != EditorMode::Visual {
            return;
        }
        let (width, height) = self.viewport_size();
        if self.mode != EditorMode::Visual {
            self.apply_input(InputAction::SetMode(InputMode::Visual), width, height);
        }
        self.with_active_buffer_view_mut(|buffer, view| {
            // Visual selections include their upper endpoint. Keep the entire
            // grapheme, including combining marks and joined emoji.
            let end_of_grapheme = |position: Pos| {
                let text = buffer.line_string(position.line);
                let mut column = 0;
                for grapheme in text.graphemes(true) {
                    let next = column + grapheme.chars().count();
                    if position.col < next {
                        return Pos::new(position.line, next - 1);
                    }
                    column = next;
                }
                position
            };
            if position < anchor {
                view.visual_anchor = Some(end_of_grapheme(anchor));
                view.cursor.place_cursor(position);
            } else {
                view.visual_anchor = Some(anchor);
                view.cursor.place_cursor(end_of_grapheme(position));
            }
        });
        self.sync_active_pane_view();
    }

    fn mouse_pane_at(&self, column: u16, row: u16) -> Option<PaneRect> {
        let column = column.checked_sub(self.mouse.origin)?;
        self.pane_rects(
            self.mouse.width,
            self.mouse.height.saturating_sub(STATUS_BAR_HEIGHT_CELLS),
        )
        .into_iter()
        .find(|rect| {
            minui::widgets::WidgetArea::new(rect.x, rect.y, rect.width, rect.height)
                .contains_point(column, row)
        })
    }

    fn scroll_mouse_at(&mut self, column: u16, row: u16, rows: isize, columns: isize) {
        let Some(rect) = self.mouse_pane_at(column, row) else {
            return;
        };
        let Some(pane_index) = self.panes.iter().position(|pane| pane.id == rect.pane_id) else {
            return;
        };
        let pane = &self.panes[pane_index];
        let buffer_id = pane.buffer_id;
        if self.undo_tree_surface_role(buffer_id).is_some() {
            self.scroll_undo_tree_pane(
                buffer_id,
                rows,
                columns,
                rect.width as usize,
                rect.height as usize,
            );
        } else if pane.options.accessible
            && self
                .session
                .meta(buffer_id)
                .is_some_and(|meta| meta.kind == BufferKind::File)
        {
            if rect.pane_id == self.active_pane_id() {
                self.scroll_mouse(rows, columns);
            } else if let Some(buffer) = self.session.buffer(buffer_id) {
                let cursor = &mut self.panes[pane_index].view.cursor;
                cursor.set_scrolloff_rows(self.scrolloff_rows);
                scroll_mouse_view(
                    buffer,
                    cursor,
                    rows,
                    columns,
                    rect.height.saturating_sub(1) as usize,
                );
            }
        }
    }

    fn scroll_mouse(&mut self, rows: isize, columns: isize) {
        let (_, height) = self.viewport_size();
        self.with_active_buffer_view_mut(|buffer, view| {
            scroll_mouse_view(
                buffer,
                &mut view.cursor,
                rows,
                columns,
                height.saturating_sub(STATUS_BAR_HEIGHT_ROWS),
            );
        });
        self.close_completion();
        self.sync_active_pane_view();
    }
}

fn scroll_mouse_view(
    buffer: &TextBuffer,
    cursor: &mut CursorController,
    rows: isize,
    columns: isize,
    height: usize,
) {
    if height == 0 {
        return;
    }
    cursor.scroll_y_lines = cursor
        .scroll_y_lines
        .saturating_add_signed(rows)
        .min(buffer.len_lines().saturating_sub(1));
    if columns != 0 {
        let first = cursor.scroll_y_lines;
        let last = first.saturating_add(height).min(buffer.len_lines());
        let max_width = (first..last)
            .map(|line| cursor.line_cell_width(buffer, line))
            .max()
            .unwrap_or(0);
        cursor.scroll_x_cells = cursor
            .scroll_x_cells
            .saturating_add_signed(columns)
            .min(max_width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_momentum_is_discarded_until_the_gesture_pauses() {
        let start = Instant::now();
        let mut mouse = MouseState::default();
        for (millis, popup, accepted) in [
            (0, Some(MousePopup::Command), true),
            (50, None, false),
            (200, None, false),
            (400, Some(MousePopup::Finder), false),
            (650, Some(MousePopup::Finder), true),
            (660, Some(MousePopup::Finder), true),
            (670, Some(MousePopup::Command), false),
            (920, None, true),
            (930, None, true),
        ] {
            assert_eq!(
                mouse.accepts_wheel_context(popup, start + Duration::from_millis(millis)),
                accepted
            );
        }
    }

    #[test]
    fn scroll_filter_rejects_off_axis_noise_and_resets_after_a_pause() {
        let start = Instant::now();
        let mut filter = ScrollFilter::default();
        let vertical = false;
        let horizontal = true;
        for (millis, axis, delta, accepted) in [
            (0, horizontal, 1, false),
            (10, horizontal, -1, false),
            (20, vertical, 1, true),
            (30, horizontal, 1, false),
            (40, horizontal, 1, false),
            (50, vertical, -1, true),
            (60, horizontal, -1, false),
            (70, horizontal, -1, false),
            (80, horizontal, -1, true),
            (90, horizontal, 1, true),
            (100, vertical, 1, false),
            (110, horizontal, -1, true),
            (120, vertical, -1, false),
            (130, vertical, -1, true),
            (140, horizontal, 1, false),
            (150, horizontal, 1, false),
            (400, horizontal, 1, false),
            (410, horizontal, 1, false),
            (420, horizontal, 1, true),
            (430, vertical, 0, false),
            (670, vertical, 1, true),
        ] {
            assert_eq!(
                filter.accepts(axis, delta, start + Duration::from_millis(millis)),
                accepted,
                "event at {millis}ms"
            );
        }
    }
}
