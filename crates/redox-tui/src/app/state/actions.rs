use crate::ui::helpers::visual_column;
use std::ops::ControlFlow;

use redox_core::{Pos, Selection, TextBuffer, motion::Motion};

use super::{EditorMode, EditorState, SplitAxis, SplitDirection};
use crate::input::{InputAction, InsertKind};
use crate::ui::syntax::{auto_closing_tag, smart_newline_insert};
use crate::ui::{STATUS_BAR_HEIGHT_ROWS, language_for_path};

impl EditorState {
    pub(super) fn apply_input_inner(
        &mut self,
        action: InputAction,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
        if self.handle_substitute_confirmation(&action) {
            return;
        }
        if matches!(
            self.mode,
            EditorMode::Command | EditorMode::Search | EditorMode::Finder
        ) && let InputAction::Paste(text) | InputAction::PasteSystemClipboardText(text) = &action
        {
            self.paste_popup_text(text);
            return;
        }
        self.cancel_obsolete_lsp_requests();
        self.apply_configured_scrolloff(self.session.active_id());
        let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);

        if is_buffer_editing_action(&action) {
            if self
                .undo_tree_surface_role(self.session.active_id())
                .is_some()
            {
                self.mode = EditorMode::Normal;
                self.input.reset_prefixes();
                return;
            }
            if !matches!(action, InputAction::SetMode(_))
                && (self.mode.is_visual()
                    || matches!(self.mode, EditorMode::Normal | EditorMode::Insert))
                && self.session.active_meta().kind == redox_core::BufferKind::File
            {
                self.with_active_buffer_view_mut(|buffer, view| {
                    view.cursor
                        .center_if_outside(buffer, viewport_width_cells, text_vh);
                });
            }
        }

        let keep_insert_coalesce = self.mode == EditorMode::Insert
            && matches!(
                &action,
                InputAction::InsertChar(_)
                    | InputAction::Backspace
                    | InputAction::Enter
                    | InputAction::Paste(_)
            );
        let keep_pending_insert_undo = self.mode == EditorMode::Insert;
        if !keep_insert_coalesce && !keep_pending_insert_undo {
            self.clear_active_insert_undo_coalesce();
        }

        if action_modes(&action).is_none_or(|modes| modes.contains(&self.mode))
            && self
                .dispatch_input_action(action, viewport_width_cells, viewport_height_rows, text_vh)
                .is_break()
        {
            return;
        }

        self.refresh_command_argument_completions();
        self.refresh_substitute_preview();
        self.clamp_active_cursor_for_normal_mode();
    }

    /// Runs an action that [`action_modes`] allows in the current mode. Returns `Break` when the
    /// action bailed out early and the post-action refreshes should be skipped.
    fn dispatch_input_action(
        &mut self,
        action: InputAction,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
        text_vh: usize,
    ) -> ControlFlow<()> {
        match action {
            InputAction::ApplyRecordedInsert {
                start_offset,
                deleted_chars,
                text,
                cursor_offset,
            } => {
                self.apply_recorded_insert(
                    start_offset,
                    deleted_chars,
                    &text,
                    cursor_offset,
                    viewport_width_cells,
                    text_vh,
                );
            }
            // Handled by `apply_input` before dispatch.
            InputAction::RepeatLastChange { .. }
            | InputAction::ToggleMacroRecording
            | InputAction::StartMacroRecording { .. }
            | InputAction::PlayMacro { .. }
            | InputAction::None => {}
            InputAction::ReplaySequence(sequence) => {
                match self.input.expand_replay_sequence(self.mode, &sequence) {
                    Ok(actions) => {
                        for action in actions {
                            self.apply_input(action, viewport_width_cells, viewport_height_rows);
                        }
                    }
                    Err(error) => self.set_status(error.to_string()),
                }
            }
            InputAction::RunCommand(command) => self.execute_configured_command(command),
            InputAction::WriteQuitIfDirty => {
                let command = if self.session.active_meta().dirty {
                    "wq"
                } else {
                    "q"
                };
                self.execute_configured_command(command.to_string());
            }
            InputAction::SetMark(name) => {
                let position = self.active_cursor_pos();
                self.session.active_buffer_mut().set_mark(name, position);
            }
            InputAction::Motion { motion, count } => {
                self.apply_motion_action(motion, count, viewport_width_cells, text_vh)?;
            }
            InputAction::SetMode(mode) => self.set_mode(mode, viewport_width_cells, text_vh),
            InputAction::EnterInsert(kind) => {
                self.enter_insert(kind, viewport_width_cells, text_vh)
            }

            // Buffer edits.
            InputAction::OpenLineBelow => {
                self.require_fully_loaded()?;
                self.open_line_and_enter_insert(false, viewport_width_cells, text_vh);
            }
            InputAction::OpenLineAbove => {
                self.require_fully_loaded()?;
                self.open_line_and_enter_insert(true, viewport_width_cells, text_vh);
            }
            InputAction::JoinLineBelow => self.join_line_below(viewport_width_cells, text_vh),
            InputAction::InsertChar(c) => {
                self.require_fully_loaded()?;
                self.insert_char_at_cursor(c, viewport_width_cells, text_vh);
            }
            InputAction::Backspace => {
                self.require_fully_loaded()?;
                self.backspace_at_insert_cursor(viewport_width_cells, text_vh);
            }
            InputAction::Enter => {
                self.require_fully_loaded()?;
                self.newline_at_insert_cursor(viewport_width_cells, text_vh);
            }
            InputAction::Paste(text) => {
                self.require_fully_loaded()?;
                self.close_completion();
                let coalesce = self.mode == EditorMode::Insert;
                self.insert_text_at_cursor(&text, viewport_width_cells, text_vh, coalesce);
            }
            InputAction::PasteSystemClipboardText(text) => {
                self.paste_system_clipboard_text(&text, viewport_width_cells, text_vh);
            }
            InputAction::PastePrivateRegister => {
                self.paste_private_register(viewport_width_cells, text_vh);
            }
            InputAction::PastePrivateRegisterBefore => {
                self.paste_private_register_before(viewport_width_cells, text_vh);
            }
            InputAction::PasteSystemClipboard => {}
            InputAction::Undo => self.undo_active(viewport_width_cells, text_vh),
            InputAction::Redo => self.redo_active(viewport_width_cells, text_vh),
            InputAction::DeleteCharNoYank => {
                self.delete_char_under_cursor_without_yank(viewport_width_cells, text_vh);
            }
            InputAction::DeleteCurrentLinePrivate { count } => {
                self.delete_current_line_to_private_register(
                    count.max(1),
                    viewport_width_cells,
                    text_vh,
                );
            }
            InputAction::YankCurrentLinePrivate { count } => {
                self.yank_current_line_to_private_register(count.max(1));
            }
            InputAction::ChangeCurrentLinePrivate { count } => {
                self.change_current_line_to_private_register(
                    count.max(1),
                    viewport_width_cells,
                    text_vh,
                );
            }
            InputAction::OperateCurrentLines { operator, count } => {
                self.operate_current_lines(operator, count.max(1), viewport_width_cells, text_vh);
            }
            InputAction::OperateTarget { operator, target } => {
                if let crate::input::OperatorTarget::Motion { motion, count } = &target
                    && is_char_search_motion(*motion)
                {
                    self.remember_motion_search(*motion, *count);
                }
                self.apply_operator_target(operator, &target, viewport_width_cells, text_vh);
            }
            InputAction::IncrementNumbers {
                count,
                decrement,
                progressive,
            } => {
                self.increment_numbers(
                    count,
                    decrement,
                    progressive,
                    viewport_width_cells,
                    text_vh,
                );
            }
            InputAction::ToggleCase { count } => {
                self.toggle_case_under_cursor_or_selection(
                    count.max(1),
                    viewport_width_cells,
                    text_vh,
                );
            }
            InputAction::ReplaceChar(ch) => {
                self.replace_under_cursor_or_selection(ch, viewport_width_cells, text_vh);
            }
            InputAction::ToggleComments => {
                self.toggle_active_comments(viewport_width_cells, text_vh);
            }

            // Visual selections.
            InputAction::YankSelectionPrivate => {
                self.yank_active_visual_selection(false, viewport_width_cells, text_vh);
            }
            InputAction::YankSelectionSystem => {
                self.yank_active_visual_selection(true, viewport_width_cells, text_vh);
            }
            InputAction::DeleteSelectionPrivate => {
                self.delete_active_visual_selection_to_private_register(
                    viewport_width_cells,
                    text_vh,
                );
            }
            InputAction::ChangeSelectionPrivate => {
                self.change_active_visual_selection_to_private_register(
                    viewport_width_cells,
                    text_vh,
                );
            }
            InputAction::DeleteSelectionNoYank => {
                self.delete_active_visual_selection_without_yank(viewport_width_cells, text_vh);
            }
            InputAction::ChangeSelectionCase { upper } => {
                let case = if upper {
                    super::editing::CaseChange::Upper
                } else {
                    super::editing::CaseChange::Lower
                };
                self.change_case_active_visual_selection(case, viewport_width_cells, text_vh);
            }
            InputAction::RestoreVisualSelection => {
                self.restore_visual_selection(viewport_width_cells, text_vh);
            }
            InputAction::SwapVisualSelectionEnds => {
                self.swap_visual_selection_ends(viewport_width_cells, text_vh);
            }
            InputAction::SelectSearchMatch { forward } => {
                self.select_search_match(forward, viewport_width_cells, text_vh);
            }
            InputAction::WrapSelection { opening, closing } => {
                self.wrap_active_visual_selection(opening, closing, viewport_width_cells, text_vh);
            }
            InputAction::MoveVisualSelectionUp { count } => {
                self.move_visual_selection_lines_up(count.max(1), viewport_width_cells, text_vh);
            }
            InputAction::MoveVisualSelectionDown { count } => {
                self.move_visual_selection_lines_down(count.max(1), viewport_width_cells, text_vh);
            }
            InputAction::IndentVisualSelection { count } => {
                self.indent_visual_selection(count.max(1), viewport_width_cells, text_vh);
            }
            InputAction::OutdentVisualSelection { count } => {
                self.outdent_visual_selection(count.max(1), viewport_width_cells, text_vh);
            }

            // Viewport.
            InputAction::ScrollHalfPage { down, count } => {
                self.scroll_active_half_page(down, count, viewport_width_cells, text_vh)?;
            }
            InputAction::CenterCursorLine => {
                self.clear_search_highlights();
                self.center_active_cursor_line(text_vh);
            }
            InputAction::SplitFocusLeft => self.focus_split(SplitDirection::Left),
            InputAction::SplitFocusDown => self.focus_split(SplitDirection::Down),
            InputAction::SplitFocusUp => self.focus_split(SplitDirection::Up),
            InputAction::SplitFocusRight => self.focus_split(SplitDirection::Right),
            InputAction::SplitHorizontal => self.split_active_pane(SplitAxis::Horizontal),
            InputAction::SplitVertical => self.split_active_pane(SplitAxis::Vertical),
            InputAction::CloseSplit => self.close_active_split(),
            InputAction::TogglePaneZoom => self.toggle_pane_zoom(),
            InputAction::ToggleZen => self.toggle_zen(),

            // Command line.
            InputAction::EnterCommand => self.begin_command(),
            InputAction::CommandCancel => {
                self.mode = EditorMode::Normal;
                self.command_line.clear();
                self.command_line_cursor = 0;
                self.reset_command_history_navigation();
                self.input.reset_prefixes();
            }
            InputAction::CommandChar(c) => {
                self.detach_command_history_navigation();
                insert_at_cursor(
                    &mut self.command_line,
                    &mut self.command_line_cursor,
                    c.encode_utf8(&mut [0; 4]),
                );
            }
            InputAction::CommandBackspace => {
                self.detach_command_history_navigation();
                backspace_at_cursor(&mut self.command_line, &mut self.command_line_cursor);
            }
            InputAction::CommandMoveLeft => {
                self.command_completion_index = 0;
                move_cursor_left(&self.command_line, &mut self.command_line_cursor);
            }
            InputAction::CommandMoveRight => {
                self.command_completion_index = 0;
                move_cursor_right(&self.command_line, &mut self.command_line_cursor);
            }
            InputAction::CommandHistoryPrev => self.command_history_prev(),
            InputAction::CommandHistoryNext => self.command_history_next(),
            InputAction::CommandEnter => self.execute_command_line(),
            InputAction::CommandComplete => self.accept_command_completion(),
            InputAction::CommandCompletionNext => self.cycle_command_completion(true),
            InputAction::CommandCompletionPrev => self.cycle_command_completion(false),

            // Search.
            InputAction::EnterSearch => self.enter_search_mode(),
            InputAction::ClearSearch => self.clear_search_highlights(),
            InputAction::SearchCancel => self.cancel_search(),
            InputAction::SearchChar(c) => {
                insert_at_cursor(
                    &mut self.command_line,
                    &mut self.command_line_cursor,
                    c.encode_utf8(&mut [0; 4]),
                );
                self.schedule_search_preview(std::time::Instant::now());
            }
            InputAction::SearchBackspace => {
                backspace_at_cursor(&mut self.command_line, &mut self.command_line_cursor);
                self.schedule_search_preview(std::time::Instant::now());
            }
            InputAction::SearchMoveLeft => {
                move_cursor_left(&self.command_line, &mut self.command_line_cursor);
            }
            InputAction::SearchMoveRight => {
                move_cursor_right(&self.command_line, &mut self.command_line_cursor);
            }
            InputAction::SearchEnter => self.execute_search_line(viewport_width_cells, text_vh),
            InputAction::RepeatSearch { forward } => {
                self.repeat_search(forward, viewport_width_cells, text_vh);
            }

            // Insert-mode completion and snippets.
            InputAction::TriggerCompletion => self.trigger_completion(),
            InputAction::CompletionMoveNext => {
                let _ = self.completion_move(1);
            }
            InputAction::CompletionMovePrev => {
                let _ = self.completion_move(-1);
            }
            InputAction::CompletionAccept => {
                if !self.accept_completion(viewport_width_cells, text_vh) {
                    self.apply_input(
                        InputAction::Enter,
                        viewport_width_cells,
                        viewport_height_rows,
                    );
                }
            }
            InputAction::CompletionCancel => {
                let had_visible_completion = self.has_visible_completion_popup();
                let _ = self.close_completion();
                if !had_visible_completion {
                    self.apply_input(
                        InputAction::SetMode(EditorMode::Normal),
                        viewport_width_cells,
                        viewport_height_rows,
                    );
                }
            }
            InputAction::SnippetNext => {
                if !self.snippet_jump_next(viewport_width_cells, text_vh) {
                    self.apply_input(
                        InputAction::InsertChar('\t'),
                        viewport_width_cells,
                        viewport_height_rows,
                    );
                }
            }

            // LSP.
            InputAction::GotoDefinition => self.goto_definition(),
            InputAction::TriggerCodeActions => self.trigger_code_actions(),
            InputAction::TriggerSymbolInfo => self.trigger_symbol_info(),
            InputAction::ToggleDiagnosticsList => {
                self.close_completion();
                self.toggle_diagnostics_popup();
            }
            InputAction::SymbolInfoMoveNext => self.symbol_info_popup_move(1),
            InputAction::SymbolInfoMovePrev => self.symbol_info_popup_move(-1),
            InputAction::SymbolInfoCancel => self.close_symbol_info_popup(),
            InputAction::DiagnosticsListMoveNext => self.diagnostics_popup_move(1),
            InputAction::DiagnosticsListMovePrev => self.diagnostics_popup_move(-1),
            InputAction::DiagnosticsListOpenSelected => self.diagnostics_popup_open_selected(),
            InputAction::DiagnosticsListCancel => self.diagnostics_popup_cancel(),
            InputAction::CodeActionsMoveNext => self.code_actions_popup_move(1),
            InputAction::CodeActionsMovePrev => self.code_actions_popup_move(-1),
            InputAction::CodeActionsApplySelected => self.apply_selected_code_action(),
            InputAction::CodeActionsCancel => self.close_code_actions_popup(),
            InputAction::LspMarketplaceMoveNext => self.lsp_marketplace_move(1),
            InputAction::LspMarketplaceMovePrev => self.lsp_marketplace_move(-1),
            InputAction::LspMarketplaceInstallSelected => self.install_selected_lsp(),
            InputAction::LspMarketplaceUninstallSelected => self.uninstall_selected_lsp(),
            InputAction::LspMarketplaceCancel => self.close_lsp_marketplace(),

            // Finder.
            InputAction::OpenFinder => {
                self.close_completion();
                self.open_finder();
            }
            InputAction::FinderCancel => self.close_finder(),
            InputAction::FinderChar(c) => self.finder_insert_text(c.encode_utf8(&mut [0; 4])),
            InputAction::FinderBackspace => self.finder_backspace(),
            InputAction::FinderMoveLeft => self.finder_move_query_cursor_left(),
            InputAction::FinderMoveRight => self.finder_move_query_cursor_right(),
            InputAction::FinderMoveNext => self.finder_move_selection(1),
            InputAction::FinderMovePrev => self.finder_move_selection(-1),
            InputAction::FinderEnter => self.open_selected_finder_entry(),
            InputAction::FinderBeginPin => self.begin_pin_selection_for_finder_entry(),

            // Pins.
            InputAction::QuickPinCurrentFile => self.begin_pin_selection_for_current_buffer(),
            InputAction::OpenPinnedSlot { slot } => self.open_pinned_slot(slot),
            InputAction::AssignPinSlot { slot } => self.assign_pin_slot(slot),
            InputAction::PinSelectorMoveNext => self.pin_selector_move(1),
            InputAction::PinSelectorMovePrev => self.pin_selector_move(-1),
            InputAction::PinSelectorOpenSelected => self.open_selected_pin_selector_entry(),
            InputAction::PinSelectorAssign => self.assign_selected_pin_slot(),
            InputAction::PinSelectorReorderUp => self.pin_selector_reorder_selected(-1),
            InputAction::PinSelectorReorderDown => self.pin_selector_reorder_selected(1),
            InputAction::PinSelectorDeleteSelected => self.pin_selector_delete_selected(),
            InputAction::PinSelectorCancel => self.cancel_pin_selection(),

            // Explorer and undo-tree surfaces.
            InputAction::OpenExplorer => {
                self.close_completion();
                self.command_open_explorer();
            }
            InputAction::ToggleUndoTree => {
                self.close_completion();
                self.command_toggle_undo_tree();
            }
            InputAction::SurfaceOpenSelected => {
                if self.undo_tree_is_active() {
                    self.undo_tree_open_selected(viewport_width_cells, text_vh);
                    return ControlFlow::Break(());
                }
                self.transient_origin_buffer_id = None;
                self.transient_origin_dir = None;
                self.surface_open_selected();
            }
            InputAction::SurfaceGoParent => self.surface_go_parent(),
            InputAction::ConfirmExplorerDelete => {
                self.confirm_pending_explorer_delete();
            }
        }
        ControlFlow::Continue(())
    }

    fn require_fully_loaded(&mut self) -> ControlFlow<()> {
        if self.ensure_active_fully_loaded_for_edit_or_save() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    }

    fn apply_motion_action(
        &mut self,
        motion: Motion,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> ControlFlow<()> {
        if let Motion::MarkLine(name) | Motion::MarkExact(name) = motion
            && self.session.active_buffer().mark(name).is_none()
        {
            self.set_status(format!("Mark {name} is not set"));
            return ControlFlow::Break(());
        }
        if self.mode == EditorMode::Insert {
            let handled_completion = match motion {
                Motion::Down => self.completion_move(count as isize),
                Motion::Up => self.completion_move(-(count as isize)),
                _ => false,
            };
            if handled_completion {
                return ControlFlow::Break(());
            }
        }

        if is_char_search_motion(motion) {
            self.remember_motion_search(motion, count);
        } else {
            self.clear_search_highlights();
        }
        if self.apply_undo_tree_motion(motion, count) {
            return ControlFlow::Break(());
        }
        let previous_cursor = self.active_cursor_pos();
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
        if self.undo_tree_is_active() {
            self.clamp_undo_tree_cursor();
        }
        let cursor = self.active_cursor_pos();
        if cursor != previous_cursor
            && (cursor.line.abs_diff(previous_cursor.line) >= 5
                || matches!(
                    motion,
                    Motion::FileStart
                        | Motion::FileEnd
                        | Motion::LineNumber
                        | Motion::MatchDelimiter
                        | Motion::MarkLine(_)
                        | Motion::MarkExact(_)
                ))
        {
            self.start_jump_highlight(text_vh);
        }
        ControlFlow::Continue(())
    }

    fn set_mode(&mut self, next_mode: EditorMode, viewport_width_cells: usize, text_vh: usize) {
        let prev_mode = self.mode;
        if prev_mode == EditorMode::Insert && next_mode != EditorMode::Insert {
            let _ = self.finalize_active_insert_undo_if_changed();
            self.close_completion();
            self.close_active_snippet();
        }
        if prev_mode == EditorMode::SymbolInfo && next_mode != EditorMode::SymbolInfo {
            self.clear_symbol_info();
        }

        self.mode = next_mode;

        if prev_mode == EditorMode::Insert && next_mode == EditorMode::Normal {
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

        if next_mode == EditorMode::Normal {
            self.clear_search_highlights();
        }

        match (prev_mode.is_visual(), next_mode.is_visual()) {
            (false, true) => self.set_active_visual_anchor_if_missing(),
            (true, false) => self.clear_active_visual_anchor(),
            _ => {}
        }

        self.input.reset_prefixes();
    }

    fn enter_insert(&mut self, kind: InsertKind, viewport_width_cells: usize, text_vh: usize) {
        self.clear_active_visual_anchor();
        self.mode = EditorMode::Insert;
        self.clear_status();
        self.input.reset_prefixes();

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        let line = buffer.clamp_line(view.cursor.cursor.line);

        match kind {
            InsertKind::Insert => {}
            InsertKind::Append => {
                if view.cursor.cursor.col < buffer.line_len_chars(line) {
                    view.cursor.cursor.col += 1;
                }
            }
            InsertKind::InsertLineStart => {
                view.cursor.cursor.col = buffer.line_first_non_whitespace_col(line);
            }
            InsertKind::AppendLineEnd => {
                view.cursor.cursor.col = buffer.line_len_chars(line);
            }
        }

        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
    }

    fn yank_active_visual_selection(
        &mut self,
        to_system_clipboard: bool,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        let Some(plan) = self.active_visual_selection_edit_plan() else {
            return;
        };
        if let Some((selection, mode)) = self.active_visual_selection() {
            self.set_one_shot_highlight(selection, mode, super::HighlightKind::Yank);
            // Like Vim, the cursor returns to the start of the yanked text: the top-left
            // corner of a block.
            let (start, end) = selection.ordered();
            let yank_start = match mode {
                redox_core::VisualModeKind::Block => Pos::new(start.line, start.col.min(end.col)),
                redox_core::VisualModeKind::Char | redox_core::VisualModeKind::Line => start,
            };
            self.with_active_buffer_view_mut(|buffer, view| {
                view.cursor.cursor = buffer.clamp_pos(yank_start);
                view.cursor
                    .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
            });
        }
        self.private_register_kind = Self::register_kind_from_visual_mode(plan.mode);
        self.private_register = plan.text;
        if to_system_clipboard {
            self.pending_system_clipboard = Some(self.private_register.clone());
        }
        self.mode = EditorMode::Normal;
        self.clear_active_visual_anchor();
        if !to_system_clipboard {
            self.confirm_yank("yanked");
        }
    }

    fn scroll_active_half_page(
        &mut self,
        down: bool,
        count: Option<usize>,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> ControlFlow<()> {
        if text_vh == 0 {
            return ControlFlow::Continue(());
        }
        let Some(pane) = self
            .panes
            .iter_mut()
            .find(|pane| pane.id == self.active_pane)
        else {
            return ControlFlow::Break(());
        };
        if let Some(count) = count {
            pane.half_page_rows = Some(count.clamp(1, text_vh));
        }
        let rows = pane.half_page_rows.unwrap_or(text_vh / 2).clamp(1, text_vh);
        self.clear_search_highlights();
        self.with_active_buffer_view_mut(|buffer, view| {
            view.cursor
                .scroll_half_page(buffer, down, rows, viewport_width_cells, text_vh);
        });
        self.center_active_cursor_line(text_vh);
        self.request_redraw();
        ControlFlow::Continue(())
    }

    fn backspace_at_insert_cursor(&mut self, viewport_width_cells: usize, text_vh: usize) {
        self.close_completion();
        let before = self.capture_active_insert_coalesced_checkpoint();
        let indent_size = self.active_indent_width();
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let selection = Selection::empty(view.cursor.cursor);
        let snippet_delete = {
            let buffer = self.session.active_buffer();
            let cursor_char = buffer.pos_to_char(view.cursor.cursor);
            if buffer
                .char_before(view.cursor.cursor)
                .and_then(matching_auto_pair_closer)
                .is_some_and(|closer| Some(closer) == buffer.char_at(view.cursor.cursor))
            {
                cursor_char
                    .checked_sub(1)
                    .map(|start| (start, cursor_char.saturating_add(1)))
            } else if let Some((start, end)) =
                soft_tab_backspace_range(buffer, view.cursor.cursor, indent_size)
            {
                Some((buffer.pos_to_char(start), buffer.pos_to_char(end)))
            } else {
                cursor_char.checked_sub(1).map(|start| (start, cursor_char))
            }
        };

        {
            let buffer = self.session.active_buffer_mut();
            if let Some(new_cursor) = delete_auto_pair_with_backspace(buffer, view.cursor.cursor) {
                view.cursor.cursor = new_cursor;
            } else if let Some((start, end)) =
                soft_tab_backspace_range(buffer, view.cursor.cursor, indent_size)
            {
                view.cursor.cursor = buffer.delete_range(start, end);
            } else {
                let selection = buffer.backspace(selection);
                view.cursor.cursor = selection.cursor;
            }
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        if let Some((start_char, end_char)) = snippet_delete {
            let _ = self.mirror_active_snippet_delete_after_cursor_delete(
                start_char,
                end_char,
                viewport_width_cells,
                text_vh,
            );
        }
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn newline_at_insert_cursor(&mut self, viewport_width_cells: usize, text_vh: usize) {
        self.close_completion();
        let before = self.capture_active_insert_coalesced_checkpoint();
        let active_id = self.session.active_id();
        let cursor = self.views.entry(active_id).or_default().cursor.cursor;
        let indent_size = self.active_indent_width();
        self.seed_indent_parser();
        let smart_insert = smart_newline_insert(
            &mut self.indent_parser,
            self.session.active_buffer(),
            self.session.active_meta().path.as_deref(),
            cursor,
            indent_size,
        );
        let comment_prefix = smart_insert
            .as_ref()
            .filter(|insert| insert.comment_prefix_bytes > 0)
            .map(|insert| (insert.cursor.line, insert.comment_prefix_bytes));
        let view = self.views.entry(active_id).or_default();

        {
            let buffer = self.session.active_buffer_mut();
            if let Some(insert) = smart_insert {
                let _ = buffer.insert(view.cursor.cursor, &insert.text);
                view.cursor.cursor = insert.cursor;
            } else {
                let selection = Selection::empty(view.cursor.cursor);
                let selection = buffer.insert_newline(selection);
                view.cursor.cursor = selection.cursor;
            }
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.invalidate_active_render_caches();
        if let Some((line, end_byte)) = comment_prefix {
            self.views
                .entry(active_id)
                .or_default()
                .syntax_highlighter
                .preserve_comment_prefix(line, end_byte);
        }
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn paste_popup_text(&mut self, text: &str) {
        let text: String = text
            .replace("\r\n", "\n")
            .chars()
            .filter_map(|character| match character {
                '\r' | '\n' => Some(' '),
                '\t' => Some(character),
                character if character.is_control() => None,
                character => Some(character),
            })
            .collect();
        if text.is_empty() {
            return;
        }
        if self.mode == EditorMode::Finder {
            self.finder_insert_text(&text);
        } else {
            if self.mode == EditorMode::Command {
                self.detach_command_history_navigation();
            }
            insert_at_cursor(&mut self.command_line, &mut self.command_line_cursor, &text);
            if self.mode == EditorMode::Search {
                self.schedule_search_preview(std::time::Instant::now());
            }
        }
        self.refresh_command_argument_completions();
        self.refresh_substitute_preview();
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
        if coalesce_insert_mode
            && self.replace_active_snippet_selection_text(text, viewport_width_cells, text_vh)
        {
            if text.contains('\n') {
                self.refresh_active_indentation();
            }
            return;
        }

        let before = if coalesce_insert_mode {
            self.capture_active_insert_coalesced_checkpoint()
        } else {
            self.capture_active_undo_checkpoint()
        };
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let insert_at_char = {
            let buffer = self.session.active_buffer();
            buffer.pos_to_char(view.cursor.cursor)
        };

        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.insert(view.cursor.cursor, text);
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        if text.contains('\n') {
            self.refresh_active_indentation();
        }
        self.invalidate_active_render_caches();
        if coalesce_insert_mode {
            let _ = self.mirror_active_snippet_insert_after_cursor_insert(
                insert_at_char,
                text,
                viewport_width_cells,
                text_vh,
            );
        }
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn insert_char_at_cursor(&mut self, ch: char, viewport_width_cells: usize, text_vh: usize) {
        if !self.completion_survives_insert(ch) {
            self.close_completion();
        }
        if let Some(close) = matching_auto_pair_closer(ch) {
            let text = [ch, close].iter().collect::<String>();
            if self.replace_active_snippet_selection_text_with_cursor_offset(
                &text,
                1,
                viewport_width_cells,
                text_vh,
            ) {
                self.queue_auto_completion_after_insert(ch);
                return;
            }
        }
        let active_id = self.session.active_id();
        let cursor = self.views.entry(active_id).or_default().cursor.cursor;
        if ch == '>' {
            self.seed_indent_parser();
        }
        let behavior = {
            let buffer = self.session.active_buffer();
            if ch == '>' {
                let language = language_for_path(self.session.active_meta().path.as_deref());
                auto_closing_tag(&mut self.indent_parser, buffer, language, cursor)
                    .map(InsertCharBehavior::InsertPair)
                    .unwrap_or(InsertCharBehavior::Plain)
            } else {
                classify_insert_char(buffer, cursor, ch)
            }
        };

        match behavior {
            InsertCharBehavior::Plain => {
                let text = if ch == '\t' {
                    let indent_size = self.active_indent_width();
                    soft_tab_insert_text(self.session.active_buffer(), cursor, indent_size)
                } else {
                    ch.to_string()
                };
                self.insert_text_at_cursor(&text, viewport_width_cells, text_vh, true);
                self.queue_auto_completion_after_insert(ch);
                self.start_delimiter_blink();
            }
            InsertCharBehavior::MoveRight => {
                let view = self.views.entry(active_id).or_default();
                let buffer = self.session.active_buffer();
                view.cursor.cursor = buffer.clamp_pos(Pos::new(cursor.line, cursor.col + 1));
                view.cursor
                    .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                self.start_delimiter_blink();
            }
            InsertCharBehavior::InsertPair(close) => {
                let before = self.capture_active_insert_coalesced_checkpoint();
                let view = self.views.entry(active_id).or_default();
                let insert_at_char = {
                    let buffer = self.session.active_buffer();
                    buffer.pos_to_char(view.cursor.cursor)
                };
                let insert = format!("{ch}{close}");

                {
                    let buffer = self.session.active_buffer_mut();
                    let _ = buffer.insert(view.cursor.cursor, &insert);
                    view.cursor.cursor = Pos::new(cursor.line, cursor.col + 1);
                    view.cursor
                        .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                }

                self.invalidate_active_render_caches();
                let cursor_between_pair = self.views.entry(active_id).or_default().cursor.cursor;
                let _ = self.mirror_active_snippet_insert_after_cursor_insert(
                    insert_at_char,
                    &insert,
                    viewport_width_cells,
                    text_vh,
                );
                {
                    let view = self.views.entry(active_id).or_default();
                    let buffer = self.session.active_buffer();
                    view.cursor.cursor = buffer.clamp_pos(cursor_between_pair);
                    view.cursor
                        .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
                }
                let _ = self.record_active_undo_if_changed(before);
                let _ = self.session.recompute_active_dirty();
                self.queue_auto_completion_after_insert(ch);
            }
        }
    }

    fn start_delimiter_blink(&mut self) {
        if self.session.active_meta().kind != redox_core::BufferKind::File {
            return;
        }
        let cursor = self.active_cursor_pos();
        let Some(column) = cursor.col.checked_sub(1) else {
            return;
        };
        let closing = Pos::new(cursor.line, column);
        let buffer = self.session.active_buffer();
        if !buffer.char_at(closing).is_some_and(is_auto_pair_closer) {
            return;
        }
        let Some(opening) = buffer
            .matching_delimiter(closing)
            .filter(|opening| *opening < closing)
        else {
            return;
        };
        self.set_one_shot_highlight(
            Selection::empty(opening),
            redox_core::VisualModeKind::Char,
            super::HighlightKind::Delimiter,
        );
        self.request_redraw();
    }

    pub(super) fn center_active_cursor_line(&mut self, text_vh: usize) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum InsertCharBehavior {
    Plain,
    MoveRight,
    InsertPair(String),
}

fn classify_insert_char(buffer: &TextBuffer, cursor: Pos, ch: char) -> InsertCharBehavior {
    if ch == '\t' {
        return buffer
            .char_at(cursor)
            .filter(|current| is_auto_pair_closer(*current))
            .map(|_| InsertCharBehavior::MoveRight)
            .unwrap_or(InsertCharBehavior::Plain);
    }

    if buffer.char_at(cursor) == Some(ch) && is_auto_pair_closer(ch) {
        return InsertCharBehavior::MoveRight;
    }

    match ch {
        '(' => InsertCharBehavior::InsertPair(")".into()),
        '[' => InsertCharBehavior::InsertPair("]".into()),
        '{' => InsertCharBehavior::InsertPair("}".into()),
        '"' | '`' => {
            if should_auto_pair_symmetric_delimiter(buffer, cursor, ch) {
                if ch == '`'
                    && cursor.col >= 2
                    && buffer.char_before(cursor) == Some('`')
                    && buffer.char_at(Pos::new(cursor.line, cursor.col - 2)) == Some('`')
                    && (cursor.col == 2
                        || buffer.char_at(Pos::new(cursor.line, cursor.col - 3)) != Some('`'))
                {
                    InsertCharBehavior::InsertPair("```".into())
                } else {
                    InsertCharBehavior::InsertPair(ch.to_string())
                }
            } else {
                InsertCharBehavior::Plain
            }
        }
        '\'' => {
            if should_auto_pair_single_quote(buffer, cursor) {
                InsertCharBehavior::InsertPair("'".into())
            } else {
                InsertCharBehavior::Plain
            }
        }
        _ => InsertCharBehavior::Plain,
    }
}

fn should_auto_pair_symmetric_delimiter(buffer: &TextBuffer, cursor: Pos, ch: char) -> bool {
    if ch != '`' && buffer.char_before(cursor) == Some('\\') {
        return false;
    }

    buffer.char_at(cursor).is_none_or(is_auto_pair_boundary)
}

fn should_auto_pair_single_quote(buffer: &TextBuffer, cursor: Pos) -> bool {
    if buffer
        .char_before(cursor)
        .is_some_and(is_word_like_for_single_quote)
    {
        return false;
    }

    buffer
        .char_at(cursor)
        .is_none_or(|ch| is_auto_pair_boundary(ch) && !is_word_like_for_single_quote(ch))
}

fn is_auto_pair_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ')' | ']' | '}' | ',' | '.' | ';' | ':' | '!' | '?' | '>' | '/'
        )
}

fn is_word_like_for_single_quote(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

pub(super) fn is_auto_pair_closer(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}' | '"' | '\'' | '`')
}

fn matching_auto_pair_closer(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

fn delete_auto_pair_with_backspace(buffer: &mut TextBuffer, cursor: Pos) -> Option<Pos> {
    let opener = buffer.char_before(cursor)?;
    let closer = buffer.char_at(cursor)?;
    if matching_auto_pair_closer(opener) != Some(closer) {
        return None;
    }

    let start = Pos::new(cursor.line, cursor.col.saturating_sub(1));
    let end = Pos::new(cursor.line, cursor.col + 1);
    Some(buffer.delete_range(start, end))
}

fn soft_tab_backspace_range(
    buffer: &TextBuffer,
    cursor: Pos,
    indent_size: usize,
) -> Option<(Pos, Pos)> {
    if cursor.col == 0 {
        return None;
    }

    let line = buffer.clamp_line(cursor.line);
    let line_text = buffer.line_string(line);
    let mut seen = 0usize;
    for ch in line_text.chars() {
        if seen == cursor.col {
            break;
        }
        if ch != ' ' {
            return None;
        }
        seen += 1;
    }
    if seen < cursor.col {
        return None;
    }

    let spaces_left = cursor.col % indent_size;
    let remove = if spaces_left == 0 {
        indent_size
    } else {
        spaces_left
    };

    (cursor.col >= remove).then_some((Pos::new(line, cursor.col - remove), cursor))
}

fn soft_tab_insert_text(buffer: &TextBuffer, cursor: Pos, indent_size: usize) -> String {
    let line = buffer.clamp_line(cursor.line);
    let line_text = buffer.line_string(line);
    let col = visual_column(&line_text, cursor.col);
    let next_tab = ((col / indent_size) + 1) * indent_size;
    " ".repeat(next_tab - col)
}

pub(super) fn insert_at_cursor(text: &mut String, cursor: &mut usize, inserted: &str) {
    *cursor = text.floor_char_boundary(*cursor);
    text.insert_str(*cursor, inserted);
    *cursor += inserted.len();
}

pub(super) fn backspace_at_cursor(text: &mut String, cursor: &mut usize) {
    *cursor = text.floor_char_boundary(*cursor);
    let Some(prev) = previous_char_boundary(text, *cursor) else {
        return;
    };
    text.replace_range(prev..*cursor, "");
    *cursor = prev;
}

pub(super) fn move_cursor_left(text: &str, cursor: &mut usize) {
    *cursor = text.floor_char_boundary(*cursor);
    if let Some(prev) = previous_char_boundary(text, *cursor) {
        *cursor = prev;
    }
}

pub(super) fn move_cursor_right(text: &str, cursor: &mut usize) {
    *cursor = text.floor_char_boundary(*cursor);
    if *cursor >= text.len() {
        return;
    }
    *cursor += text[*cursor..]
        .chars()
        .next()
        .map(char::len_utf8)
        .unwrap_or(0);
}

fn previous_char_boundary(text: &str, cursor: usize) -> Option<usize> {
    text[..cursor].char_indices().last().map(|(idx, _)| idx)
}

/// The modes in which `action` takes effect, or `None` when it applies in every mode. Actions
/// outside their modes are ignored, though the post-action refreshes still run.
fn action_modes(action: &InputAction) -> Option<&'static [EditorMode]> {
    use EditorMode as M;
    use InputAction as A;

    const NORMAL: &[EditorMode] = &[M::Normal];
    const INSERT: &[EditorMode] = &[M::Insert];
    const NORMAL_OR_INSERT: &[EditorMode] = &[M::Normal, M::Insert];
    const VISUAL: &[EditorMode] = &[M::Visual, M::VisualLine, M::VisualBlock];
    const NORMAL_OR_VISUAL: &[EditorMode] = &[M::Normal, M::Visual, M::VisualLine, M::VisualBlock];

    let modes = match action {
        A::OpenLineBelow
        | A::OpenLineAbove
        | A::JoinLineBelow
        | A::PasteSystemClipboardText(_)
        | A::PastePrivateRegister
        | A::PastePrivateRegisterBefore
        | A::Undo
        | A::Redo
        | A::DeleteCharNoYank
        | A::DeleteCurrentLinePrivate { .. }
        | A::YankCurrentLinePrivate { .. }
        | A::ChangeCurrentLinePrivate { .. }
        | A::OperateCurrentLines { .. }
        | A::CenterCursorLine
        | A::EnterSearch
        | A::GotoDefinition
        | A::OpenFinder
        | A::OpenExplorer
        | A::ToggleUndoTree
        | A::SurfaceOpenSelected
        | A::SurfaceGoParent
        | A::ConfirmExplorerDelete => NORMAL,
        A::InsertChar(_)
        | A::Backspace
        | A::Enter
        | A::TriggerCompletion
        | A::CompletionMoveNext
        | A::CompletionMovePrev
        | A::CompletionAccept
        | A::CompletionCancel
        | A::SnippetNext => INSERT,
        A::Paste(_) | A::TriggerSymbolInfo => NORMAL_OR_INSERT,
        A::ToggleCase { .. } | A::ReplaceChar(_) | A::ScrollHalfPage { .. } => NORMAL_OR_VISUAL,
        A::RepeatSearch { .. } => &[
            M::Normal,
            M::Search,
            M::Visual,
            M::VisualLine,
            M::VisualBlock,
        ],
        A::QuickPinCurrentFile => &[
            M::Normal,
            M::Insert,
            M::Visual,
            M::VisualLine,
            M::VisualBlock,
        ],
        A::DeleteSelectionPrivate
        | A::ChangeSelectionPrivate
        | A::DeleteSelectionNoYank
        | A::ChangeSelectionCase { .. }
        | A::MoveVisualSelectionUp { .. }
        | A::MoveVisualSelectionDown { .. }
        | A::IndentVisualSelection { .. } => VISUAL,
        A::OutdentVisualSelection { .. } => &[M::Visual, M::VisualLine],
        A::ToggleDiagnosticsList | A::TriggerCodeActions => &[M::Normal, M::DiagnosticsList],
        A::CommandChar(_)
        | A::CommandBackspace
        | A::CommandMoveLeft
        | A::CommandMoveRight
        | A::CommandHistoryPrev
        | A::CommandHistoryNext => &[M::Command],
        A::SearchChar(_) | A::SearchBackspace | A::SearchMoveLeft | A::SearchMoveRight => {
            &[M::Search]
        }
        A::FinderCancel
        | A::FinderChar(_)
        | A::FinderBackspace
        | A::FinderMoveLeft
        | A::FinderMoveRight
        | A::FinderMoveNext
        | A::FinderMovePrev
        | A::FinderEnter
        | A::FinderBeginPin => &[M::Finder],
        A::AssignPinSlot { .. }
        | A::PinSelectorMoveNext
        | A::PinSelectorMovePrev
        | A::PinSelectorOpenSelected
        | A::PinSelectorAssign
        | A::PinSelectorReorderUp
        | A::PinSelectorReorderDown
        | A::PinSelectorDeleteSelected
        | A::PinSelectorCancel => &[M::PinSelect],
        A::LspMarketplaceMoveNext
        | A::LspMarketplaceMovePrev
        | A::LspMarketplaceInstallSelected
        | A::LspMarketplaceUninstallSelected
        | A::LspMarketplaceCancel => &[M::LspMarketplace],
        A::DiagnosticsListMoveNext
        | A::DiagnosticsListMovePrev
        | A::DiagnosticsListOpenSelected
        | A::DiagnosticsListCancel => &[M::DiagnosticsList],
        A::CodeActionsMoveNext
        | A::CodeActionsMovePrev
        | A::CodeActionsApplySelected
        | A::CodeActionsCancel => &[M::CodeActions],
        A::SymbolInfoMoveNext | A::SymbolInfoMovePrev | A::SymbolInfoCancel => &[M::SymbolInfo],
        A::None
        | A::ApplyRecordedInsert { .. }
        | A::RepeatLastChange { .. }
        | A::ToggleMacroRecording
        | A::StartMacroRecording { .. }
        | A::PlayMacro { .. }
        | A::ReplaySequence(_)
        | A::RunCommand(_)
        | A::WriteQuitIfDirty
        | A::SetMark(_)
        | A::Motion { .. }
        | A::SetMode(_)
        | A::EnterInsert(_)
        | A::PasteSystemClipboard
        | A::OperateTarget { .. }
        | A::IncrementNumbers { .. }
        | A::ToggleComments
        | A::YankSelectionPrivate
        | A::YankSelectionSystem
        | A::RestoreVisualSelection
        | A::SwapVisualSelectionEnds
        | A::SelectSearchMatch { .. }
        | A::WrapSelection { .. }
        | A::SplitFocusLeft
        | A::SplitFocusDown
        | A::SplitFocusUp
        | A::SplitFocusRight
        | A::SplitHorizontal
        | A::SplitVertical
        | A::CloseSplit
        | A::TogglePaneZoom
        | A::ToggleZen
        | A::EnterCommand
        | A::CommandCancel
        | A::CommandEnter
        | A::CommandComplete
        | A::CommandCompletionNext
        | A::CommandCompletionPrev
        | A::ClearSearch
        | A::SearchCancel
        | A::SearchEnter
        | A::OpenPinnedSlot { .. } => return None,
    };
    Some(modes)
}
fn is_char_search_motion(motion: Motion) -> bool {
    matches!(
        motion,
        Motion::FindChar(_)
            | Motion::TillChar(_)
            | Motion::FindCharBefore(_)
            | Motion::TillCharBefore(_)
            | Motion::RepeatTillChar(_)
            | Motion::RepeatTillCharBefore(_)
    )
}

fn is_buffer_editing_action(action: &InputAction) -> bool {
    match action {
        InputAction::SetMode(mode) => matches!(
            mode,
            EditorMode::Insert
                | EditorMode::Visual
                | EditorMode::VisualLine
                | EditorMode::VisualBlock
        ),
        InputAction::Paste(_)
        | InputAction::EnterInsert(_)
        | InputAction::OpenLineBelow
        | InputAction::OpenLineAbove
        | InputAction::JoinLineBelow
        | InputAction::Undo
        | InputAction::Redo
        | InputAction::YankSelectionPrivate
        | InputAction::DeleteSelectionPrivate
        | InputAction::ChangeSelectionPrivate
        | InputAction::DeleteSelectionNoYank
        | InputAction::OperateTarget { .. }
        | InputAction::DeleteCurrentLinePrivate { .. }
        | InputAction::YankCurrentLinePrivate { .. }
        | InputAction::ChangeCurrentLinePrivate { .. }
        | InputAction::OperateCurrentLines { .. }
        | InputAction::ChangeSelectionCase { .. }
        | InputAction::IncrementNumbers { .. }
        | InputAction::RestoreVisualSelection
        | InputAction::SwapVisualSelectionEnds
        | InputAction::SelectSearchMatch { .. }
        | InputAction::YankSelectionSystem
        | InputAction::PasteSystemClipboard
        | InputAction::PasteSystemClipboardText(_)
        | InputAction::PastePrivateRegister
        | InputAction::PastePrivateRegisterBefore
        | InputAction::DeleteCharNoYank
        | InputAction::ToggleCase { .. }
        | InputAction::ReplaceChar(_)
        | InputAction::WrapSelection { .. }
        | InputAction::ToggleComments
        | InputAction::MoveVisualSelectionUp { .. }
        | InputAction::MoveVisualSelectionDown { .. }
        | InputAction::IndentVisualSelection { .. }
        | InputAction::OutdentVisualSelection { .. }
        | InputAction::InsertChar(_)
        | InputAction::ApplyRecordedInsert { .. }
        | InputAction::Backspace
        | InputAction::Enter => true,
        _ => false,
    }
}
