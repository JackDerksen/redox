use std::time::Instant;

use crate::app::state::{EditorMode, EditorState};
use crate::input::{InputAction, InputMode};
use crate::ui::STATUS_BAR_HEIGHT_ROWS;
use crate::ui::widgets::popup::{MousePopup, MouseRect, MouseScroll, MouseTarget};

impl EditorState {
    pub(super) fn mouse_popup_is_active(&self, kind: MousePopup) -> bool {
        match kind {
            MousePopup::Finder => self.mode == EditorMode::Finder && self.finder.is_some(),
            MousePopup::Pinboard => {
                self.mode == EditorMode::PinSelect && self.pin_selector.is_some()
            }
            MousePopup::Explorer => self.explorer_is_active(),
            MousePopup::About => self.about_is_active(),
            MousePopup::LanguageTools => self.mode == EditorMode::LspMarketplace,
            MousePopup::Diagnostics => self.mode == EditorMode::DiagnosticsList,
            MousePopup::CodeActions => self.mode == EditorMode::CodeActions,
            MousePopup::Completion => self.has_visible_completion_popup(),
            MousePopup::SymbolInfo => self.symbol_info_popup_is_visible(),
            MousePopup::Command => self.mode == EditorMode::Command,
            MousePopup::Search => self.mode == EditorMode::Search,
            MousePopup::Perf => self.perf_popup().is_some(),
            MousePopup::WhichKey => self.which_key_popup(Instant::now()).is_some(),
        }
    }

    pub(super) fn dismiss_mouse_popup(&mut self, kind: MousePopup) {
        match kind {
            MousePopup::Finder => self.close_finder(),
            MousePopup::Pinboard => self.cancel_pin_selection(),
            MousePopup::Explorer if self.explorer_has_unsaved_changes() => {
                self.set_status("explorer has unsaved changes; use :w to save or Esc to discard");
            }
            MousePopup::Explorer | MousePopup::About => {
                if self.close_active_surface_buffer_without_quit() {
                    self.mode = EditorMode::Normal;
                    self.clear_status();
                }
            }
            MousePopup::LanguageTools => self.close_lsp_marketplace(),
            MousePopup::Diagnostics => self.close_diagnostics_popup(),
            MousePopup::CodeActions => self.close_code_actions_popup(),
            MousePopup::Completion => {
                self.close_completion();
            }
            MousePopup::SymbolInfo => self.close_symbol_info_popup(),
            MousePopup::Command => {
                let (width, height) = self.viewport_size();
                self.apply_input(InputAction::CommandCancel, width, height);
            }
            MousePopup::Search => self.cancel_search(),
            MousePopup::Perf => {
                self.perf_visible = false;
                self.clear_status();
            }
            MousePopup::WhichKey => self.input.reset_prefixes(),
        }
    }

    pub(super) fn click_mouse_popup(
        &mut self,
        kind: MousePopup,
        target: &MouseTarget,
        activate: bool,
    ) {
        let section = match target {
            MouseTarget::Entry { .. } => Some(MouseScroll::List),
            MouseTarget::DiagnosticAction { .. } => Some(MouseScroll::Actions),
            _ => None,
        };
        if let Some(section) = section
            && matches!(
                kind,
                MousePopup::Completion | MousePopup::Diagnostics | MousePopup::CodeActions
            )
            && let Some((start, _)) = self.rendered_mouse_list_window(kind, section)
        {
            self.mouse.list_scroll.insert((kind, section), start);
        }
        match (kind, target) {
            (MousePopup::Finder, MouseTarget::FinderEntry(path)) => {
                if self.finder_select_path(path) && activate {
                    self.open_selected_finder_entry();
                }
            }
            (MousePopup::Pinboard, MouseTarget::Entry { index, .. }) => {
                if self.pin_selector_select(*index)
                    && activate
                    && self.pin_selector_popup().is_some_and(|popup| {
                        popup
                            .slots
                            .get(*index)
                            .is_some_and(|slot| slot.path_label.is_some())
                    })
                {
                    self.open_selected_pin_selector_entry();
                }
            }
            (MousePopup::Explorer, MouseTarget::BufferPosition(position)) => {
                let was_insert = self.mode == EditorMode::Insert;
                let (width, height) = self.viewport_size();
                let scroll =
                    self.with_active_buffer_view_mut(|_, view| view.cursor.viewport_scroll());
                self.apply_input(InputAction::SetMode(InputMode::Normal), width, height);
                self.with_active_buffer_view_mut(|buffer, view| {
                    view.cursor.scroll_x_cells = scroll.0;
                    view.cursor.scroll_y_lines = scroll.1;
                    view.cursor.place_cursor(buffer.clamp_pos(*position));
                });
                if activate {
                    self.surface_open_selected();
                } else if was_insert {
                    self.mode = EditorMode::Insert;
                }
            }
            (MousePopup::Finder, MouseTarget::InputCursor(cursor)) => {
                self.finder_position_query_cursor(*cursor);
            }
            (MousePopup::Command | MousePopup::Search, MouseTarget::InputCursor(cursor)) => {
                self.command_line_cursor = *cursor;
                super::super::actions::clamp_str_cursor(
                    &self.command_line,
                    &mut self.command_line_cursor,
                );
            }
            (MousePopup::WhichKey, MouseTarget::Key(key)) => {
                self.mouse.pending_key = match key.as_str() {
                    "Space" | "<Space>" => Some(minui::Event::Character(' ')),
                    "Esc" | "<Esc>" => Some(minui::Event::Escape),
                    "Enter" | "<Enter>" => Some(minui::Event::Enter),
                    "Tab" | "<Tab>" => Some(minui::Event::Tab),
                    _ => {
                        let mut characters = key.chars();
                        characters
                            .next()
                            .filter(|_| characters.next().is_none())
                            .map(minui::Event::Character)
                    }
                };
            }
            (MousePopup::Diagnostics, MouseTarget::DiagnosticAction { index, identity }) => {
                if self.select_mouse_diagnostic_action(*index, identity) && activate {
                    self.diagnostics_popup_open_selected();
                }
            }
            (
                MousePopup::Diagnostics
                | MousePopup::CodeActions
                | MousePopup::Completion
                | MousePopup::LanguageTools,
                MouseTarget::Entry { index, identity },
            ) => {
                if !self.select_mouse_lsp_entry(kind, *index, identity) || !activate {
                    return;
                }
                match kind {
                    MousePopup::Diagnostics => self.diagnostics_popup_open_selected(),
                    MousePopup::CodeActions => self.apply_selected_code_action(),
                    MousePopup::Completion => {
                        let (width, height) = self.viewport_size();
                        self.accept_completion(
                            width,
                            height.saturating_sub(STATUS_BAR_HEIGHT_ROWS),
                        );
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn rendered_mouse_list_window(
        &self,
        kind: MousePopup,
        section: MouseScroll,
    ) -> Option<(usize, usize)> {
        let layout = self
            .mouse
            .popups
            .iter()
            .rev()
            .find(|layout| layout.kind == kind)?;
        let mut first_index = usize::MAX;
        let mut count = 0;
        for (_, target) in &layout.clicks {
            let index = match (section, target) {
                (MouseScroll::List, MouseTarget::Entry { index, .. })
                | (MouseScroll::Actions, MouseTarget::DiagnosticAction { index, .. }) => *index,
                _ => continue,
            };
            first_index = first_index.min(index);
            count += 1;
        }
        (count > 0).then_some((first_index, count))
    }

    pub(super) fn scroll_mouse_popup(
        &mut self,
        kind: MousePopup,
        target: MouseScroll,
        rows: isize,
        columns: isize,
        rect: MouseRect,
    ) {
        let width = usize::from(rect.width);
        let height = usize::from(rect.height);
        match (kind, target) {
            (MousePopup::Finder, MouseScroll::Preview) => {
                self.finder_scroll_preview(rows, columns, width, height);
            }
            (MousePopup::Finder, MouseScroll::List) => self.finder_scroll_list(rows),
            (MousePopup::Pinboard, MouseScroll::List) => self.pin_selector_move(rows),
            (MousePopup::Explorer, _) => {
                self.with_active_buffer_view_mut(|buffer, view| {
                    view.cursor.scroll_vertical(buffer, rows, height);
                    view.cursor.scroll_y_lines = view
                        .cursor
                        .scroll_y_lines
                        .min(buffer.len_lines().saturating_sub(height));
                    if columns != 0 {
                        let first = view.cursor.scroll_y_lines;
                        let last = first.saturating_add(height).min(buffer.len_lines());
                        let max_width = (first..last)
                            .map(|line| view.cursor.line_cell_width(buffer, line))
                            .max()
                            .unwrap_or(0);
                        view.cursor.scroll_x_cells = view
                            .cursor
                            .scroll_x_cells
                            .saturating_add_signed(columns)
                            .min(max_width.saturating_sub(width));
                    }
                });
            }
            (MousePopup::LanguageTools, MouseScroll::List) => self.lsp_marketplace_move(rows),
            (MousePopup::Diagnostics, MouseScroll::Details) => {
                self.scroll_mouse_diagnostic_details(rows, width, height);
            }
            (MousePopup::Diagnostics, MouseScroll::Actions | MouseScroll::List)
            | (MousePopup::CodeActions | MousePopup::Completion, MouseScroll::List) => {
                if let Some((start, visible_count)) = self.rendered_mouse_list_window(kind, target)
                {
                    self.scroll_mouse_lsp_list(
                        kind,
                        target,
                        rows,
                        start,
                        visible_count.min(height),
                    );
                }
            }
            (MousePopup::SymbolInfo, MouseScroll::Text) => {
                self.scroll_mouse_symbol_info(rows, height)
            }
            _ => {}
        }
    }
}
