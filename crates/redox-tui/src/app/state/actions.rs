use minui::{TabPolicy, cell_width};
use redox_core::{Pos, Selection, TextBuffer, motion::Motion};
use unicode_segmentation::UnicodeSegmentation;

use super::{EditorMode, EditorState};
use crate::SOFT_TAB_WIDTH;
use crate::input::{InputAction, InputMode, InsertKind};
use crate::ui::syntax::smart_newline_insert;
use crate::ui::{STATUS_BAR_HEIGHT_ROWS, language_for_path};

impl EditorState {
    /// Apply a high-level input action using the active viewport size for cursor reconciliation.
    pub fn apply_input(
        &mut self,
        action: InputAction,
        viewport_width_cells: usize,
        viewport_height_rows: usize,
    ) {
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
                if self.mode == EditorMode::Insert {
                    let handled_completion = match motion {
                        Motion::Down => self.completion_move(count as isize),
                        Motion::Up => self.completion_move(-(count as isize)),
                        _ => false,
                    };
                    if handled_completion {
                        return;
                    }
                }

                if !is_char_search_motion(motion) {
                    self.clear_search_highlights();
                }
                if is_char_search_motion(motion) {
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
                if prev_mode == EditorMode::Insert && mode != InputMode::Insert {
                    self.close_completion();
                    self.close_active_snippet();
                }
                if prev_mode == EditorMode::SymbolInfo && mode != InputMode::SymbolInfo {
                    self.clear_symbol_info();
                }
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
                    InputMode::Finder => EditorMode::Finder,
                    InputMode::PinSelect => EditorMode::PinSelect,
                    InputMode::LspMarketplace => EditorMode::LspMarketplace,
                    InputMode::DiagnosticsList => EditorMode::DiagnosticsList,
                    InputMode::CodeActions => EditorMode::CodeActions,
                    InputMode::SymbolInfo => EditorMode::SymbolInfo,
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
                self.close_completion();
                self.clear_active_visual_anchor();
                self.mode = EditorMode::Command;
                self.command_line.clear();
                self.reset_command_history_navigation();
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
                self.reset_command_history_navigation();
                self.input.reset_prefixes();
            }

            InputAction::CommandChar(c) => {
                if self.mode == EditorMode::Command {
                    self.detach_command_history_navigation();
                    self.command_line.push(c);
                }
            }

            InputAction::CommandBackspace => {
                if self.mode == EditorMode::Command {
                    self.detach_command_history_navigation();
                    self.command_line.pop();
                }
            }

            InputAction::CommandHistoryPrev => {
                if self.mode == EditorMode::Command {
                    self.command_history_prev();
                }
            }

            InputAction::CommandHistoryNext => {
                if self.mode == EditorMode::Command {
                    self.command_history_next();
                }
            }

            InputAction::CommandEnter => {
                self.execute_command_line();
            }

            InputAction::OpenFinder => {
                if self.mode == EditorMode::Normal {
                    self.close_completion();
                    self.open_finder();
                }
            }

            InputAction::ToggleDiagnosticsList => {
                if self.mode == EditorMode::Normal || self.mode == EditorMode::DiagnosticsList {
                    self.close_completion();
                    self.toggle_diagnostics_popup();
                }
            }

            InputAction::GotoDefinition => {
                if self.mode == EditorMode::Normal {
                    self.goto_definition();
                }
            }

            InputAction::TriggerCodeActions => {
                if matches!(self.mode, EditorMode::Normal | EditorMode::DiagnosticsList) {
                    self.trigger_code_actions();
                }
            }

            InputAction::TriggerSymbolInfo => {
                if matches!(self.mode, EditorMode::Normal | EditorMode::Insert) {
                    self.trigger_symbol_info();
                }
            }

            InputAction::TriggerCompletion => {
                if self.mode == EditorMode::Insert {
                    self.trigger_completion();
                }
            }

            InputAction::CompletionMoveNext => {
                if self.mode == EditorMode::Insert {
                    let _ = self.completion_move(1);
                }
            }

            InputAction::CompletionMovePrev => {
                if self.mode == EditorMode::Insert {
                    let _ = self.completion_move(-1);
                }
            }

            InputAction::CompletionAccept => {
                if self.mode == EditorMode::Insert {
                    if !self.accept_completion(viewport_width_cells, text_vh) {
                        self.apply_input(
                            InputAction::Enter,
                            viewport_width_cells,
                            viewport_height_rows,
                        );
                    }
                }
            }

            InputAction::CompletionCancel => {
                if self.mode == EditorMode::Insert {
                    let had_visible_completion = self.has_visible_completion_popup();
                    let _ = self.close_completion();
                    if !had_visible_completion {
                        self.apply_input(
                            InputAction::SetMode(InputMode::Normal),
                            viewport_width_cells,
                            viewport_height_rows,
                        );
                    }
                }
            }

            InputAction::SymbolInfoMoveNext => {
                if self.mode == EditorMode::SymbolInfo {
                    self.symbol_info_popup_move(1);
                }
            }

            InputAction::SymbolInfoMovePrev => {
                if self.mode == EditorMode::SymbolInfo {
                    self.symbol_info_popup_move(-1);
                }
            }

            InputAction::SymbolInfoCancel => {
                if self.mode == EditorMode::SymbolInfo {
                    self.close_symbol_info_popup();
                }
            }

            InputAction::SnippetNext => {
                if self.mode == EditorMode::Insert
                    && !self.snippet_jump_next(viewport_width_cells, text_vh)
                {
                    self.apply_input(
                        InputAction::InsertChar('\t'),
                        viewport_width_cells,
                        viewport_height_rows,
                    );
                }
            }

            InputAction::SearchCancel => {
                self.mode = EditorMode::Normal;
                self.command_line.clear();
                self.reset_command_history_navigation();
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

            InputAction::FinderCancel => {
                if self.mode == EditorMode::Finder {
                    self.close_finder();
                }
            }

            InputAction::FinderChar(c) => {
                if self.mode == EditorMode::Finder {
                    self.finder_type_char(c);
                }
            }

            InputAction::FinderBackspace => {
                if self.mode == EditorMode::Finder {
                    self.finder_backspace();
                }
            }

            InputAction::FinderMoveNext => {
                if self.mode == EditorMode::Finder {
                    self.finder_move_selection(1);
                }
            }

            InputAction::FinderMovePrev => {
                if self.mode == EditorMode::Finder {
                    self.finder_move_selection(-1);
                }
            }

            InputAction::FinderEnter => {
                if self.mode == EditorMode::Finder {
                    self.open_selected_finder_entry();
                }
            }

            InputAction::FinderBeginPin => {
                if self.mode == EditorMode::Finder {
                    self.begin_pin_selection_for_finder_entry();
                }
            }

            InputAction::PinSelectorMoveNext => {
                if self.mode == EditorMode::PinSelect {
                    self.pin_selector_move(1);
                }
            }

            InputAction::PinSelectorMovePrev => {
                if self.mode == EditorMode::PinSelect {
                    self.pin_selector_move(-1);
                }
            }

            InputAction::PinSelectorOpenSelected => {
                if self.mode == EditorMode::PinSelect {
                    self.open_selected_pin_selector_entry();
                }
            }

            InputAction::PinSelectorAssign => {
                if self.mode == EditorMode::PinSelect {
                    self.assign_selected_pin_slot();
                }
            }

            InputAction::PinSelectorReorderUp => {
                if self.mode == EditorMode::PinSelect {
                    self.pin_selector_reorder_selected(-1);
                }
            }

            InputAction::PinSelectorReorderDown => {
                if self.mode == EditorMode::PinSelect {
                    self.pin_selector_reorder_selected(1);
                }
            }

            InputAction::PinSelectorDeleteSelected => {
                if self.mode == EditorMode::PinSelect {
                    self.pin_selector_delete_selected();
                }
            }

            InputAction::PinSelectorCancel => {
                if self.mode == EditorMode::PinSelect {
                    self.cancel_pin_selection();
                }
            }

            InputAction::LspMarketplaceMoveNext => {
                if self.mode == EditorMode::LspMarketplace {
                    self.lsp_marketplace_move(1);
                }
            }

            InputAction::LspMarketplaceMovePrev => {
                if self.mode == EditorMode::LspMarketplace {
                    self.lsp_marketplace_move(-1);
                }
            }

            InputAction::LspMarketplaceInstallSelected => {
                if self.mode == EditorMode::LspMarketplace {
                    self.install_selected_lsp();
                }
            }

            InputAction::LspMarketplaceUninstallSelected => {
                if self.mode == EditorMode::LspMarketplace {
                    self.uninstall_selected_lsp();
                }
            }

            InputAction::LspMarketplaceCancel => {
                if self.mode == EditorMode::LspMarketplace {
                    self.close_lsp_marketplace();
                }
            }

            InputAction::DiagnosticsListMoveNext => {
                if self.mode == EditorMode::DiagnosticsList {
                    self.diagnostics_popup_move(1);
                }
            }

            InputAction::DiagnosticsListMovePrev => {
                if self.mode == EditorMode::DiagnosticsList {
                    self.diagnostics_popup_move(-1);
                }
            }

            InputAction::DiagnosticsListOpenSelected => {
                if self.mode == EditorMode::DiagnosticsList {
                    self.diagnostics_popup_open_selected();
                }
            }

            InputAction::DiagnosticsListCancel => {
                if self.mode == EditorMode::DiagnosticsList {
                    self.diagnostics_popup_cancel();
                }
            }

            InputAction::CodeActionsMoveNext => {
                if self.mode == EditorMode::CodeActions {
                    self.code_actions_popup_move(1);
                }
            }

            InputAction::CodeActionsMovePrev => {
                if self.mode == EditorMode::CodeActions {
                    self.code_actions_popup_move(-1);
                }
            }

            InputAction::CodeActionsApplySelected => {
                if self.mode == EditorMode::CodeActions {
                    self.apply_selected_code_action();
                }
            }

            InputAction::CodeActionsCancel => {
                if self.mode == EditorMode::CodeActions {
                    self.close_code_actions_popup();
                }
            }

            InputAction::AssignPinSlot { slot } => {
                if self.mode == EditorMode::PinSelect {
                    self.assign_pin_slot(slot);
                }
            }

            InputAction::OpenPinnedSlot { slot } => {
                self.open_pinned_slot(slot);
            }

            InputAction::QuickPinCurrentFile => {
                if !matches!(
                    self.mode,
                    EditorMode::Command
                        | EditorMode::Search
                        | EditorMode::Finder
                        | EditorMode::PinSelect
                        | EditorMode::LspMarketplace
                        | EditorMode::DiagnosticsList
                        | EditorMode::CodeActions
                        | EditorMode::SymbolInfo
                ) {
                    self.begin_pin_selection_for_current_buffer();
                }
            }

            InputAction::OpenExplorer => {
                if self.mode == EditorMode::Normal {
                    self.close_completion();
                    self.command_open_explorer();
                }
            }

            InputAction::SurfaceOpenSelected => {
                if self.mode == EditorMode::Normal {
                    self.transient_origin_buffer_id = None;
                    self.transient_origin_dir = None;
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
                    self.clear_search_highlights();
                    self.scroll_viewport_and_center_cursor(true, text_vh);
                }
            }

            InputAction::ViewportUpCenter => {
                if self.mode == EditorMode::Normal {
                    self.clear_search_highlights();
                    self.scroll_viewport_and_center_cursor(false, text_vh);
                }
            }

            InputAction::CenterCursorLine => {
                if self.mode == EditorMode::Normal {
                    self.clear_search_highlights();
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
                    self.insert_char_at_cursor(c, viewport_width_cells, text_vh);
                }
            }

            InputAction::Backspace => {
                if self.mode == EditorMode::Insert {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        return;
                    }
                    self.close_completion();
                    let before = self.capture_active_insert_coalesced_snapshot();
                    let active_id = self.session.active_id();
                    let view = self.views.entry(active_id).or_default();
                    let sel = Selection::empty(view.cursor.cursor);
                    let snippet_delete = {
                        let buffer = self.session.active_buffer();
                        let cursor_char = buffer.pos_to_char(view.cursor.cursor);
                        if buffer
                            .char_before(view.cursor.cursor)
                            .and_then(matching_auto_pair_closer)
                            .is_some_and(|closer| {
                                Some(closer) == buffer.char_at(view.cursor.cursor)
                            })
                        {
                            cursor_char
                                .checked_sub(1)
                                .map(|start| (start, cursor_char.saturating_add(1)))
                        } else if let Some((start, end)) =
                            soft_tab_backspace_range(buffer, view.cursor.cursor)
                        {
                            Some((buffer.pos_to_char(start), buffer.pos_to_char(end)))
                        } else {
                            cursor_char.checked_sub(1).map(|start| (start, cursor_char))
                        }
                    };

                    {
                        let buffer = self.session.active_buffer_mut();
                        if let Some(new_cursor) =
                            delete_auto_pair_with_backspace(buffer, view.cursor.cursor)
                        {
                            view.cursor.cursor = new_cursor;
                        } else if let Some((start, end)) =
                            soft_tab_backspace_range(buffer, view.cursor.cursor)
                        {
                            view.cursor.cursor = buffer.delete_range(start, end);
                        } else {
                            let sel = buffer.backspace(sel);
                            view.cursor.cursor = sel.cursor;
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
            }

            InputAction::Enter => {
                if self.mode == EditorMode::Insert {
                    if !self.ensure_active_fully_loaded_for_edit_or_save() {
                        return;
                    }
                    self.close_completion();
                    let before = self.capture_active_insert_coalesced_snapshot();
                    let active_id = self.session.active_id();
                    let cursor = self.views.entry(active_id).or_default().cursor.cursor;
                    let language = language_for_path(self.session.active_meta().path.as_deref());
                    let smart_insert =
                        smart_newline_insert(self.session.active_buffer(), language, cursor);
                    let view = self.views.entry(active_id).or_default();

                    {
                        let buffer = self.session.active_buffer_mut();
                        if let Some((text, cursor)) = smart_insert {
                            let _ = buffer.insert(view.cursor.cursor, &text);
                            view.cursor.cursor = cursor;
                        } else {
                            let sel = Selection::empty(view.cursor.cursor);
                            let sel = buffer.insert_newline(sel);
                            view.cursor.cursor = sel.cursor;
                        }
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
                    self.close_completion();
                    let coalesce = self.mode == EditorMode::Insert;
                    self.insert_text_at_cursor(&text, viewport_width_cells, text_vh, coalesce);
                }
                EditorMode::Command
                | EditorMode::Search
                | EditorMode::Finder
                | EditorMode::PinSelect
                | EditorMode::LspMarketplace
                | EditorMode::CodeActions
                | EditorMode::SymbolInfo
                | EditorMode::DiagnosticsList
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
                    && is_char_search_motion(*motion)
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

            InputAction::ToggleCase { count } => {
                if matches!(
                    self.mode,
                    EditorMode::Normal
                        | EditorMode::Visual
                        | EditorMode::VisualLine
                        | EditorMode::VisualBlock
                ) {
                    self.toggle_case_under_cursor_or_selection(
                        count.max(1),
                        viewport_width_cells,
                        text_vh,
                    );
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
        if coalesce_insert_mode
            && self.replace_active_snippet_selection_text(text, viewport_width_cells, text_vh)
        {
            return;
        }

        let before = if coalesce_insert_mode {
            self.capture_active_insert_coalesced_snapshot()
        } else {
            self.capture_active_undo_snapshot()
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
        let behaviour = {
            let buffer = self.session.active_buffer();
            classify_insert_char(buffer, cursor, ch)
        };

        match behaviour {
            InsertCharBehaviour::Plain => {
                let text = if ch == '\t' {
                    soft_tab_insert_text(self.session.active_buffer(), cursor)
                } else {
                    ch.to_string()
                };
                self.insert_text_at_cursor(&text, viewport_width_cells, text_vh, true);
                self.queue_auto_completion_after_insert(ch);
            }
            InsertCharBehaviour::MoveRight => {
                let view = self.views.entry(active_id).or_default();
                let buffer = self.session.active_buffer();
                view.cursor.cursor = buffer.clamp_pos(Pos::new(cursor.line, cursor.col + 1));
                view.cursor
                    .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
            }
            InsertCharBehaviour::InsertPair(close) => {
                let before = self.capture_active_insert_coalesced_snapshot();
                let view = self.views.entry(active_id).or_default();
                let insert_at_char = {
                    let buffer = self.session.active_buffer();
                    buffer.pos_to_char(view.cursor.cursor)
                };
                let insert = [ch, close].iter().collect::<String>();

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertCharBehaviour {
    Plain,
    MoveRight,
    InsertPair(char),
}

fn classify_insert_char(buffer: &TextBuffer, cursor: Pos, ch: char) -> InsertCharBehaviour {
    if ch == '\t' {
        return buffer
            .char_at(cursor)
            .filter(|current| is_auto_pair_closer(*current))
            .map(|_| InsertCharBehaviour::MoveRight)
            .unwrap_or(InsertCharBehaviour::Plain);
    }

    if buffer.char_at(cursor) == Some(ch) && is_auto_pair_closer(ch) {
        return InsertCharBehaviour::MoveRight;
    }

    match ch {
        '(' => InsertCharBehaviour::InsertPair(')'),
        '[' => InsertCharBehaviour::InsertPair(']'),
        '{' => InsertCharBehaviour::InsertPair('}'),
        '"' | '`' => should_auto_pair_symmetric_delimiter(buffer, cursor, ch)
            .then_some(InsertCharBehaviour::InsertPair(ch))
            .unwrap_or(InsertCharBehaviour::Plain),
        '\'' => should_auto_pair_single_quote(buffer, cursor)
            .then_some(InsertCharBehaviour::InsertPair('\''))
            .unwrap_or(InsertCharBehaviour::Plain),
        _ => InsertCharBehaviour::Plain,
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

fn is_auto_pair_closer(ch: char) -> bool {
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

fn soft_tab_backspace_range(buffer: &TextBuffer, cursor: Pos) -> Option<(Pos, Pos)> {
    if cursor.col == 0 {
        return None;
    }

    let line = buffer.clamp_line(cursor.line);
    let line_text = buffer.line_string(line);
    let chars: Vec<char> = line_text.chars().collect();
    if cursor.col > chars.len() || !chars[..cursor.col].iter().all(|ch| *ch == ' ') {
        return None;
    }

    let spaces_left = cursor.col % SOFT_TAB_WIDTH;
    let remove = if spaces_left == 0 {
        SOFT_TAB_WIDTH
    } else {
        spaces_left
    };

    (cursor.col >= remove).then_some((Pos::new(line, cursor.col - remove), cursor))
}

fn soft_tab_insert_text(buffer: &TextBuffer, cursor: Pos) -> String {
    let line = buffer.clamp_line(cursor.line);
    let line_text = buffer.line_string(line);
    let col = visual_column(&line_text, cursor.col);
    let next_tab = ((col / SOFT_TAB_WIDTH) + 1) * SOFT_TAB_WIDTH;
    " ".repeat(next_tab - col)
}

fn visual_column(line: &str, char_col: usize) -> usize {
    let graphemes: Vec<&str> = line.graphemes(true).collect();
    let cursor_g = char_col_to_grapheme_index(&graphemes, char_col);
    graphemes[..cursor_g.min(graphemes.len())]
        .iter()
        .map(|g| cell_width(*g, TabPolicy::Fixed(SOFT_TAB_WIDTH as u16)) as usize)
        .sum()
}

fn char_col_to_grapheme_index(graphemes: &[&str], cursor_col_chars: usize) -> usize {
    if cursor_col_chars == 0 {
        return 0;
    }

    let mut chars_seen = 0usize;
    for (i, g) in graphemes.iter().enumerate() {
        let next = chars_seen + g.chars().count();
        if cursor_col_chars <= chars_seen {
            return i;
        }
        if cursor_col_chars < next {
            return i;
        }
        if cursor_col_chars == next {
            return i + 1;
        }
        chars_seen = next;
    }
    graphemes.len()
}

fn is_char_search_motion(motion: Motion) -> bool {
    matches!(
        motion,
        Motion::FindChar(_)
            | Motion::TillChar(_)
            | Motion::FindCharBefore(_)
            | Motion::TillCharBefore(_)
    )
}
