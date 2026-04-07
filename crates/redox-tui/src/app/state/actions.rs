use redox_core::{Pos, Selection, motion::Motion};

use super::{EditorMode, EditorState};
use crate::input::{InputAction, InputMode, InsertKind};
use crate::ui::STATUS_BAR_HEIGHT_ROWS;

impl EditorState {
    /// Apply a high-level input action using the active viewport size for cursor reconciliation.
    pub fn apply_input(
        &mut self,
        action: InputAction,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        if self.status_msg_clear_on_input {
            self.clear_status();
        }

        let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);
        let keep_insert_coalesce = self.mode == EditorMode::Insert
            && matches!(
                &action,
                InputAction::InsertChar(_)
                    | InputAction::Backspace
                    | InputAction::Enter
                    | InputAction::Paste(_)
            );
        if !keep_insert_coalesce {
            self.clear_active_insert_undo_coalesce();
        }

        match action {
            InputAction::Motion { motion, count } => {
                if !matches!(motion, Motion::FindChar(_) | Motion::TillChar(_)) {
                    self.clear_search_highlights();
                }
                if matches!(motion, Motion::FindChar(_) | Motion::TillChar(_)) {
                    self.remember_motion_search(motion, count);
                }
                let is_explorer = self.explorer_is_active();
                let active_id = self.session.active_id();
                let view = self.views.entry(active_id).or_default();
                let buffer = self.session.active_buffer();
                if is_explorer {
                    view.cursor.follow.top_margin_rows = 0;
                    view.cursor.follow.bottom_margin_rows = 0;
                }

                view.cursor
                    .apply_motion(buffer, motion, count, viewport_width_cells, text_vh);
                if is_explorer {
                    let total_lines = buffer.len_lines().max(1);
                    let max_top = if text_vh == 0 {
                        total_lines.saturating_sub(1)
                    } else {
                        total_lines.saturating_sub(text_vh)
                    };
                    view.cursor.scroll_y_lines = view.cursor.scroll_y_lines.min(max_top);
                }
            }

            InputAction::SetMode(mode) => {
                let prev_mode = self.mode;
                let leaving_insert_to_normal =
                    prev_mode == EditorMode::Insert && mode == InputMode::Normal;
                let entering_visual = matches!(
                    mode,
                    InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
                );
                let was_visual = matches!(
                    prev_mode,
                    EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
                );

                self.mode = match mode {
                    InputMode::Normal => EditorMode::Normal,
                    InputMode::Insert => EditorMode::Insert,
                    InputMode::Command => EditorMode::Command,
                    InputMode::Search => EditorMode::Search,
                    InputMode::Visual => EditorMode::Visual,
                    InputMode::VisualLine => EditorMode::VisualLine,
                    InputMode::VisualBlock => EditorMode::VisualBlock,
                };

                if leaving_insert_to_normal {
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();
                    let buffer = self.session.active_buffer();

                    if view.cursor.cursor.col > 0
                        && !matches!(buffer.char_before(view.cursor.cursor), Some('\t'))
                    {
                        view.cursor.cursor.col -= 1;
                    }

                    view.cursor
                        .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                }

                if mode == InputMode::Normal {
                    self.clear_search_highlights();
                }

                if entering_visual && !was_visual {
                    self.set_active_visual_anchor_if_missing();
                }

                if was_visual && !entering_visual {
                    self.clear_active_visual_anchor();
                }

                self.input.reset_prefixes();
            }

            InputAction::EnterInsert(kind) => {
                self.clear_active_visual_anchor();
                self.mode = EditorMode::Insert;
                self.clear_status();
                self.input.reset_prefixes();

                {
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();
                    let buffer = self.session.active_buffer();

                    match kind {
                        InsertKind::Insert => {}
                        InsertKind::Append => {
                            let line = buffer.clamp_line(view.cursor.cursor.line);
                            let line_len_chars = buffer.line_len_chars(line);
                            if view.cursor.cursor.col < line_len_chars {
                                view.cursor.cursor.col += 1;
                            }
                        }
                        InsertKind::InsertLineStart => {
                            let line = buffer.clamp_line(view.cursor.cursor.line);
                            view.cursor.cursor.col = buffer.line_first_non_whitespace_col(line);
                        }
                        InsertKind::AppendLineEnd => {
                            let line = buffer.clamp_line(view.cursor.cursor.line);
                            view.cursor.cursor.col = buffer.line_len_chars(line);
                        }
                    }

                    view.cursor
                        .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                }
            }

            InputAction::OpenLineBelow => {
                if self.mode == EditorMode::Normal {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        return;
                    }
                    self.open_line_and_enter_insert(false, viewport_width_cells, text_vh);
                }
            }

            InputAction::OpenLineAbove => {
                if self.mode == EditorMode::Normal {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        return;
                    }
                    self.open_line_and_enter_insert(true, viewport_width_cells, text_vh);
                }
            }

            InputAction::EnterCommand => {
                self.clear_active_visual_anchor();
                self.mode = EditorMode::Command;
                self.command_line.clear();
                self.clear_status();
                self.input.reset_prefixes();
            }

            InputAction::EnterSearch => {
                if self.mode == EditorMode::Normal {
                    self.enter_search_mode();
                }
            }

            InputAction::ClearSearch => {
                self.clear_search_highlights();
            }

            InputAction::CommandCancel => {
                self.mode = EditorMode::Normal;
                self.command_line.clear();
                self.input.reset_prefixes();
            }

            InputAction::CommandChar(c) => {
                if self.mode == EditorMode::Command {
                    self.command_line.push(c);
                }
            }

            InputAction::CommandBackspace => {
                if self.mode == EditorMode::Command {
                    self.command_line.pop();
                }
            }

            InputAction::CommandEnter => {
                self.execute_command_line();
            }

            InputAction::SearchCancel => {
                self.mode = EditorMode::Normal;
                self.command_line.clear();
                self.input.reset_prefixes();
            }

            InputAction::SearchChar(c) => {
                if self.mode == EditorMode::Search {
                    self.command_line.push(c);
                }
            }

            InputAction::SearchBackspace => {
                if self.mode == EditorMode::Search {
                    self.command_line.pop();
                }
            }

            InputAction::SearchEnter => {
                self.execute_search_line(viewport_width_cells, text_vh);
            }

            InputAction::OpenExplorer => {
                if self.mode == EditorMode::Normal {
                    self.command_open_explorer();
                }
            }

            InputAction::SurfaceOpenSelected => {
                if self.mode == EditorMode::Normal {
                    self.surface_open_selected();
                }
            }

            InputAction::SurfaceGoParent => {
                if self.mode == EditorMode::Normal {
                    self.surface_go_parent();
                }
            }

            InputAction::ViewportDownCenter => {
                if self.mode == EditorMode::Normal {
                    self.scroll_viewport_and_center_cursor(true, text_vh);
                }
            }

            InputAction::ViewportUpCenter => {
                if self.mode == EditorMode::Normal {
                    self.scroll_viewport_and_center_cursor(false, text_vh);
                }
            }

            InputAction::CenterCursorLine => {
                if self.mode == EditorMode::Normal {
                    self.center_active_cursor_line(text_vh);
                }
            }

            InputAction::Undo => {
                if self.mode == EditorMode::Normal {
                    self.undo_active(viewport_width_cells, text_vh);
                }
            }

            InputAction::ConfirmExplorerDelete => {
                if self.mode == EditorMode::Normal {
                    self.confirm_pending_explorer_delete();
                }
            }

            InputAction::Redo => {
                if self.mode == EditorMode::Normal {
                    self.redo_active(viewport_width_cells, text_vh);
                }
            }

            InputAction::InsertChar(c) => {
                if self.mode == EditorMode::Insert {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        return;
                    }
                    let s = c.to_string();
                    self.insert_text_at_cursor(&s, viewport_width_cells, text_vh, true);
                }
            }

            InputAction::Backspace => {
                if self.mode == EditorMode::Insert {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        return;
                    }
                    let before = self.capture_active_insert_coalesced_snapshot();
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();
                    let sel = Selection::empty(view.cursor.cursor);

                    {
                        let buffer = self.session.active_buffer_mut();
                        let sel = buffer.backspace(sel);
                        view.cursor.cursor = sel.cursor;
                        view.cursor
                            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                    }

                    self.invalidate_active_render_caches();
                    let _ = self.record_active_undo_if_changed(before);
                    let _ = self.session.recompute_active_dirty();
                }
            }

            InputAction::Enter => {
                if self.mode == EditorMode::Insert {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        return;
                    }
                    let before = self.capture_active_insert_coalesced_snapshot();
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();
                    let sel = Selection::empty(view.cursor.cursor);

                    {
                        let buffer = self.session.active_buffer_mut();
                        let sel = buffer.insert_newline(sel);
                        view.cursor.cursor = sel.cursor;
                        view.cursor
                            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                    }

                    self.invalidate_active_render_caches();
                    let _ = self.record_active_undo_if_changed(before);
                    let _ = self.session.recompute_active_dirty();
                }
            }

            InputAction::Paste(text) => match self.mode {
                EditorMode::Insert | EditorMode::Normal => {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        return;
                    }
                    let coalesce = self.mode == EditorMode::Insert;
                    self.insert_text_at_cursor(&text, viewport_width_cells, text_vh, coalesce);
                }
                EditorMode::Command
                | EditorMode::Search
                | EditorMode::Visual
                | EditorMode::VisualLine
                | EditorMode::VisualBlock => {}
            },

            InputAction::PasteSystemClipboardText(text) => {
                if self.mode == EditorMode::Normal {
                    self.paste_system_clipboard_text(&text, viewport_width_cells, text_vh);
                }
            }

            InputAction::YankSelectionPrivate => {
                if let Some(plan) = self.active_visual_selection_edit_plan() {
                    if let Some((selection, mode)) = self.active_visual_selection() {
                        self.set_one_shot_highlight(selection, mode);
                    }
                    self.private_register = plan.text;
                    self.private_register_kind = Self::register_kind_from_visual_mode(plan.mode);
                    self.mode = EditorMode::Normal;
                    self.clear_active_visual_anchor();
                    self.set_status("yanked");
                }
            }

            InputAction::DeleteSelectionPrivate => {
                if matches!(
                    self.mode,
                    EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
                ) {
                    self.delete_active_visual_selection_to_private_register(
                        viewport_width_cells,
                        text_vh,
                    );
                }
            }

            InputAction::ChangeSelectionPrivate => {
                if matches!(
                    self.mode,
                    EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
                ) {
                    self.change_active_visual_selection_to_private_register(
                        viewport_width_cells,
                        text_vh,
                    );
                }
            }

            InputAction::DeleteSelectionNoYank => {
                if matches!(
                    self.mode,
                    EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
                ) {
                    self.delete_active_visual_selection_without_yank(viewport_width_cells, text_vh);
                }
            }

            InputAction::DeleteCurrentLinePrivate { count } => {
                if self.mode == EditorMode::Normal {
                    self.delete_current_line_to_private_register(
                        count.max(1),
                        viewport_width_cells,
                        text_vh,
                    );
                }
            }

            InputAction::YankCurrentLinePrivate { count } => {
                if self.mode == EditorMode::Normal {
                    self.yank_current_line_to_private_register(count.max(1));
                }
            }

            InputAction::ChangeCurrentLinePrivate { count } => {
                if self.mode == EditorMode::Normal {
                    self.change_current_line_to_private_register(
                        count.max(1),
                        viewport_width_cells,
                        text_vh,
                    );
                }
            }

            InputAction::OperateTarget { operator, target } => {
                if let crate::input::OperatorTarget::Motion { motion, count } = &target
                    && matches!(motion, Motion::FindChar(_) | Motion::TillChar(_))
                {
                    self.remember_motion_search(*motion, *count);
                }
                self.apply_operator_target(operator, &target, viewport_width_cells, text_vh);
            }

            InputAction::YankSelectionSystem => {
                if let Some(plan) = self.active_visual_selection_edit_plan() {
                    if let Some((selection, mode)) = self.active_visual_selection() {
                        self.set_one_shot_highlight(selection, mode);
                    }
                    self.private_register = plan.text.clone();
                    self.private_register_kind = Self::register_kind_from_visual_mode(plan.mode);
                    self.pending_system_clipboard = Some(plan.text);
                    self.mode = EditorMode::Normal;
                    self.clear_active_visual_anchor();
                }
            }

            InputAction::PastePrivateRegister => {
                if self.mode == EditorMode::Normal {
                    self.paste_private_register(viewport_width_cells, text_vh);
                }
            }

            InputAction::PastePrivateRegisterBefore => {
                if self.mode == EditorMode::Normal {
                    self.paste_private_register_before(viewport_width_cells, text_vh);
                }
            }

            InputAction::PasteSystemClipboard => {}

            InputAction::DeleteCharNoYank => {
                if self.mode == EditorMode::Normal {
                    self.delete_char_under_cursor_without_yank(viewport_width_cells, text_vh);
                }
            }

            InputAction::ReplaceChar(ch) => {
                if matches!(
                    self.mode,
                    EditorMode::Normal
                        | EditorMode::Visual
                        | EditorMode::VisualLine
                        | EditorMode::VisualBlock
                ) {
                    self.replace_under_cursor_or_selection(ch, viewport_width_cells, text_vh);
                }
            }

            InputAction::RepeatSearch { forward } => {
                if matches!(
                    self.mode,
                    EditorMode::Normal
                        | EditorMode::Visual
                        | EditorMode::VisualLine
                        | EditorMode::VisualBlock
                ) {
                    self.repeat_search(forward, viewport_width_cells, text_vh);
                }
            }

            InputAction::MoveVisualSelectionUp { count } => {
                if matches!(
                    self.mode,
                    EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
                ) {
                    self.move_visual_selection_lines_up(
                        count.max(1),
                        viewport_width_cells,
                        text_vh,
                    );
                }
            }

            InputAction::MoveVisualSelectionDown { count } => {
                if matches!(
                    self.mode,
                    EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
                ) {
                    self.move_visual_selection_lines_down(
                        count.max(1),
                        viewport_width_cells,
                        text_vh,
                    );
                }
            }

            InputAction::IndentVisualSelection { count } => {
                if matches!(
                    self.mode,
                    EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
                ) {
                    self.indent_visual_selection(count.max(1), viewport_width_cells, text_vh);
                }
            }

            InputAction::OutdentVisualSelection { count } => {
                if self.mode == EditorMode::Visual || self.mode == EditorMode::VisualLine {
                    self.outdent_visual_selection(count.max(1), viewport_width_cells, text_vh);
                }
            }

            InputAction::None => {}
        }

        self.clamp_active_cursor_for_normal_mode();
    }

    fn insert_text_at_cursor(
        &mut self,
        text: &str,
        viewport_width_cells: usize,
        text_vh: usize,
        coalesce_insert_mode: bool,
    ) {
        if text.is_empty() {
            return;
        }

        let before = if coalesce_insert_mode {
            self.capture_active_insert_coalesced_snapshot()
        } else {
            self.capture_active_undo_snapshot()
        };
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();

        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.insert(view.cursor.cursor, text);
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn scroll_viewport_and_center_cursor(&mut self, down: bool, text_vh: usize) {
        if text_vh == 0 {
            return;
        }

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        let total_lines = buffer.len_lines().max(1);
        let center_row = text_vh / 2;
        let max_top = total_lines.saturating_sub(1).saturating_sub(center_row);
        let step = text_vh;
        let prev_top = view.cursor.scroll_y_lines;
        let target_top = if down {
            prev_top.saturating_add(step).min(max_top)
        } else {
            prev_top.saturating_sub(step)
        };
        view.cursor.scroll_y_lines = target_top;

        let target_line = if !down && prev_top == 0 && target_top == 0 {
            0
        } else {
            (target_top + center_row).min(total_lines.saturating_sub(1))
        };
        let target_col = view.cursor.cursor.col;
        view.cursor.cursor = buffer.clamp_pos(Pos::new(target_line, target_col));
    }

    fn center_active_cursor_line(&mut self, text_vh: usize) {
        if text_vh == 0 {
            return;
        }

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        let total_lines = buffer.len_lines().max(1);
        let cursor_line = buffer.clamp_line(view.cursor.cursor.line);

        if cursor_line == 0 {
            view.cursor.scroll_y_lines = 0;
            return;
        }

        let center_row = text_vh / 2;
        let max_top = total_lines.saturating_sub(1);
        view.cursor.scroll_y_lines = cursor_line.saturating_sub(center_row).min(max_top);
    }

    fn clamp_active_cursor_for_normal_mode(&mut self) {
        if self.mode != EditorMode::Normal {
            return;
        }

        let active_id = self.session.active_id();
        let Some(buffer) = self.session.buffer(active_id) else {
            return;
        };
        let view = self.views.entry(active_id).or_default();
        view.cursor.clamp_for_normal_mode(buffer);
    }
}
