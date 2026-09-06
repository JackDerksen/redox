use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CodeActionOrigin {
    pub(super) buffer_id: BufferId,
    pub(super) workspace: WorkspaceKey,
}

#[derive(Debug, Clone)]
pub(super) struct CodeActionsPopupState {
    pub(super) origin: CodeActionOrigin,
    pub(super) selected: usize,
    pub(super) requested_at: Pos,
    pub(super) return_mode: EditorMode,
    pub(super) title: String,
    pub(super) actions: Vec<AvailableCodeAction>,
}

#[derive(Debug, Clone)]
pub(super) struct CodeActionsPaneState {
    pub(super) title: String,
    pub(super) selected: usize,
    pub(super) actions: Vec<AvailableCodeAction>,
    pub(super) loading: bool,
}

#[derive(Debug, Clone)]
pub(super) struct CachedCodeActions {
    pub(super) diagnostic: DiagnosticsPopupEntry,
    pub(super) document_version: i32,
    pub(super) title: String,
    pub(super) actions: Vec<AvailableCodeAction>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingDiagnosticsCodeActions {
    pub(super) diagnostic: DiagnosticsPopupEntry,
    pub(super) document_version: i32,
    pub(super) open_on_arrival: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CodeActionRequestTrigger {
    Manual,
    Prefetch,
}

impl EditorState {
    pub fn code_actions_popup(&self) -> Option<CodeActionPopup> {
        let state = self.lsp.code_actions_popup.as_ref()?;
        if self.session.active_id() != state.origin.buffer_id {
            return None;
        }
        if state.return_mode == EditorMode::Normal && self.active_cursor_pos() != state.requested_at
        {
            return None;
        }
        if state.actions.is_empty() {
            return None;
        }
        let max_selected = state.actions.len().saturating_sub(1);
        let selected = state.selected.min(max_selected);
        let scroll = selected.saturating_sub(DIAGNOSTICS_POPUP_VISIBLE_ROWS.saturating_sub(1));
        Some(CodeActionPopup {
            title: state.title.clone(),
            entries: state
                .actions
                .iter()
                .map(|action| CodeActionPopupEntry {
                    title: action.title.clone(),
                    kind: action.kind.clone(),
                    preferred: action.preferred,
                })
                .collect(),
            selected,
            scroll,
        })
    }

    pub(in crate::app::state) fn close_code_actions_popup(&mut self) {
        if self.close_diagnostics_code_actions_pane() {
            return;
        }
        let Some(popup) = self.lsp.code_actions_popup.take() else {
            return;
        };
        if self.mode == EditorMode::CodeActions {
            self.mode = popup.return_mode;
        }
    }

    pub(in crate::app::state) fn code_actions_popup_move(&mut self, delta: isize) {
        if self.diagnostics_code_actions_are_focused() {
            self.move_diagnostics_code_actions(delta);
            return;
        }
        let Some(state) = self.lsp.code_actions_popup.as_mut() else {
            return;
        };
        if state.actions.is_empty() {
            return;
        }
        let max_index = state.actions.len().saturating_sub(1) as isize;
        state.selected = (state.selected as isize + delta).clamp(0, max_index) as usize;
    }

    pub(in crate::app::state) fn trigger_code_actions(&mut self) {
        if self.try_open_cached_diagnostic_code_actions() {
            return;
        }
        if self.promote_pending_diagnostic_code_actions() {
            return;
        }
        self.request_selected_diagnostic_code_actions(CodeActionRequestTrigger::Manual, true);
    }

    pub(super) fn prefetch_selected_diagnostic_code_actions(&mut self) {
        self.request_selected_diagnostic_code_actions(CodeActionRequestTrigger::Prefetch, false);
    }

    fn request_selected_diagnostic_code_actions(
        &mut self,
        trigger: CodeActionRequestTrigger,
        open_on_arrival: bool,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let _ = self.sync_active_lsp_document();

        let active_id = self.session.active_id();
        let Some(document) = self.lsp.documents.get(&active_id).cloned() else {
            if trigger == CodeActionRequestTrigger::Manual {
                self.set_status("no LSP document for current buffer");
            }
            return;
        };
        let origin = CodeActionOrigin {
            buffer_id: active_id,
            workspace: document.workspace.clone(),
        };
        let Some((entry, requested_at, title, range, diagnostics)) =
            self.code_action_request_context()
        else {
            if trigger == CodeActionRequestTrigger::Manual {
                let message = if self.mode == EditorMode::DiagnosticsList {
                    "no diagnostic selected"
                } else {
                    "no diagnostic under cursor"
                };
                self.set_status(message);
            }
            return;
        };
        let return_mode = self.mode;
        let document_version = document.document_version;
        if matches!(trigger, CodeActionRequestTrigger::Prefetch)
            && self
                .lsp
                .diagnostics_popup
                .as_ref()
                .and_then(|state| state.cached_code_actions.as_ref())
                .is_some_and(|cache| {
                    cache.document_version == document_version && cache.diagnostic == entry
                })
        {
            return;
        }

        self.cancel_pending_lsp_requests(
            &document.workspace,
            PendingRequest::CodeActions {
                origin: origin.clone(),
                requested_at,
                return_mode,
                title: title.clone(),
                trigger,
            },
        );
        let client_initialized = self
            .lsp
            .clients
            .get(&document.workspace)
            .is_some_and(|client| client.session.is_initialized());
        if !client_initialized {
            if trigger == CodeActionRequestTrigger::Manual {
                self.set_status("LSP still loading");
            }
            return;
        }
        if self.mode == EditorMode::DiagnosticsList
            && let Some(state) = self.lsp.diagnostics_popup.as_mut()
        {
            state.code_action_origin = Some(origin.clone());
            state.pending_code_actions = Some(PendingDiagnosticsCodeActions {
                diagnostic: entry.clone(),
                document_version,
                open_on_arrival,
            });
        }
        if open_on_arrival {
            self.open_loading_diagnostics_code_actions(title.clone());
        }
        let Some(client) = self.lsp.clients.get_mut(&document.workspace) else {
            if trigger == CodeActionRequestTrigger::Manual {
                self.set_status("no LSP client for current buffer");
            }
            return;
        };
        match client
            .session
            .send_code_actions(&document.path, &range, &diagnostics)
        {
            Ok(id) => {
                self.lsp.pending_requests.insert(
                    RequestKey {
                        workspace: document.workspace,
                        id,
                    },
                    PendingClientRequest {
                        context: self.lsp_request_context(),
                        kind: PendingRequest::CodeActions {
                            origin,
                            requested_at,
                            return_mode,
                            title,
                            trigger,
                        },
                        started_at: Instant::now(),
                    },
                );
                if trigger == CodeActionRequestTrigger::Manual {
                    self.set_status("loading quick fixes...");
                }
            }
            Err(error) => {
                if let Some(state) = self.lsp.diagnostics_popup.as_mut() {
                    state.pending_code_actions = None;
                    if open_on_arrival {
                        state.code_actions = None;
                        state.focus = DiagnosticsPopupFocus::Diagnostics;
                    }
                }
                if trigger == CodeActionRequestTrigger::Manual {
                    self.set_status(format!("code action request failed: {error}"));
                }
            }
        }
    }

    pub(in crate::app::state) fn apply_selected_code_action(&mut self) {
        let Some(origin) = self.selected_code_action_origin() else {
            return;
        };
        if self.session.active_id() != origin.buffer_id {
            self.close_code_actions_popup();
            return;
        }
        let Some(action) = self.selected_code_action() else {
            return;
        };
        let return_mode = if self.diagnostics_code_actions_are_focused() {
            EditorMode::DiagnosticsList
        } else {
            self.lsp
                .code_actions_popup
                .as_ref()
                .map(|popup| popup.return_mode)
                .unwrap_or(EditorMode::Normal)
        };

        let mut edit_applied = false;
        if let Some(edit) = action.edit.as_ref() {
            if let Err(error) = self.apply_workspace_edit(edit) {
                self.set_status(format!("quick fix failed: {error}"));
                return;
            }
            edit_applied = true;
        }

        if let Some(command) = action.command.as_ref() {
            let Some(client) = self.lsp.clients.get_mut(&origin.workspace) else {
                if edit_applied {
                    self.consume_applied_code_action(return_mode);
                    self.set_status("applied quick fix edit, but no LSP client for command");
                } else {
                    self.set_status("no LSP client for current buffer");
                }
                return;
            };
            match client
                .session
                .send_execute_command(&command.command, &command.arguments)
            {
                Ok(id) => {
                    self.lsp.pending_requests.insert(
                        RequestKey {
                            workspace: origin.workspace,
                            id,
                        },
                        PendingClientRequest {
                            context: self.lsp_request_context(),
                            kind: PendingRequest::ExecuteCommand {
                                title: action.title.clone(),
                                edit_applied,
                            },
                            started_at: Instant::now(),
                        },
                    );
                    if edit_applied {
                        self.consume_applied_code_action(return_mode);
                        self.set_status(format!(
                            "applied quick fix edit; running command: {}",
                            action.title
                        ));
                    }
                }
                Err(error) => {
                    if edit_applied {
                        self.consume_applied_code_action(return_mode);
                        self.set_status(format!(
                            "applied quick fix edit, but command failed: {error}"
                        ));
                    } else {
                        self.set_status(format!("quick fix command failed: {error}"));
                    }
                    return;
                }
            }
        } else {
            self.consume_applied_code_action(return_mode);
            self.set_status(format!("applied quick fix: {}", action.title));
        }
    }

    pub(super) fn diagnostics_code_actions_are_focused(&self) -> bool {
        self.lsp
            .diagnostics_popup
            .as_ref()
            .is_some_and(|state| state.focus == DiagnosticsPopupFocus::CodeActions)
    }

    pub(super) fn move_diagnostics_code_actions(&mut self, delta: isize) {
        let Some(state) = self.lsp.diagnostics_popup.as_mut() else {
            return;
        };
        let Some(pane) = state.code_actions.as_mut() else {
            return;
        };
        if pane.actions.is_empty() {
            return;
        }
        let max_index = pane.actions.len().saturating_sub(1) as isize;
        pane.selected = (pane.selected as isize + delta).clamp(0, max_index) as usize;
    }

    pub(super) fn close_diagnostics_code_actions_pane(&mut self) -> bool {
        let Some(state) = self.lsp.diagnostics_popup.as_mut() else {
            return false;
        };
        if state.code_actions.is_none() {
            return false;
        }
        state.code_actions = None;
        if let Some(pending) = state.pending_code_actions.as_mut() {
            pending.open_on_arrival = false;
        }
        state.focus = DiagnosticsPopupFocus::Diagnostics;
        true
    }

    fn consume_applied_code_action(&mut self, return_mode: EditorMode) {
        if return_mode == EditorMode::DiagnosticsList {
            let _ = self.close_diagnostics_code_actions_pane();
        } else {
            self.close_code_actions_popup();
        }
        if return_mode == EditorMode::DiagnosticsList
            && self.current_diagnostic_popup_entries().is_empty()
        {
            self.close_diagnostics_popup();
        }
    }

    fn selected_code_action(&self) -> Option<AvailableCodeAction> {
        if self.diagnostics_code_actions_are_focused() {
            let pane = self.lsp.diagnostics_popup.as_ref()?.code_actions.as_ref()?;
            return pane.actions.get(pane.selected).cloned();
        }

        let popup = self.lsp.code_actions_popup.as_ref()?;
        popup.actions.get(popup.selected).cloned()
    }

    fn selected_code_action_origin(&self) -> Option<CodeActionOrigin> {
        if self.diagnostics_code_actions_are_focused() {
            return self
                .lsp
                .diagnostics_popup
                .as_ref()?
                .code_action_origin
                .clone();
        }
        Some(self.lsp.code_actions_popup.as_ref()?.origin.clone())
    }

    fn try_open_cached_diagnostic_code_actions(&mut self) -> bool {
        let Some(entry) = self.selected_diagnostics_popup_entry() else {
            return false;
        };
        let Some(document_version) = self.active_document_version() else {
            return false;
        };
        let Some(state) = self.lsp.diagnostics_popup.as_mut() else {
            return false;
        };
        let Some(cache) = state.cached_code_actions.as_ref() else {
            return false;
        };
        if cache.document_version != document_version || cache.diagnostic != entry {
            return false;
        }
        state.code_actions = Some(CodeActionsPaneState {
            title: cache.title.clone(),
            selected: 0,
            actions: cache.actions.clone(),
            loading: false,
        });
        state.focus = DiagnosticsPopupFocus::CodeActions;
        self.clear_status();
        true
    }

    fn promote_pending_diagnostic_code_actions(&mut self) -> bool {
        let Some(entry) = self.selected_diagnostics_popup_entry() else {
            return false;
        };
        let Some(document_version) = self.active_document_version() else {
            return false;
        };
        let Some(state) = self.lsp.diagnostics_popup.as_mut() else {
            return false;
        };
        let Some(pending) = state.pending_code_actions.as_mut() else {
            return false;
        };
        if pending.document_version != document_version || pending.diagnostic != entry {
            return false;
        }
        pending.open_on_arrival = true;
        let title = pending.diagnostic.summary.clone();
        if state.code_actions.is_none() {
            state.code_actions = Some(CodeActionsPaneState {
                title,
                selected: 0,
                actions: Vec::new(),
                loading: true,
            });
        }
        state.focus = DiagnosticsPopupFocus::CodeActions;
        self.set_status("loading quick fixes...");
        true
    }

    fn open_loading_diagnostics_code_actions(&mut self, title: String) {
        let Some(state) = self.lsp.diagnostics_popup.as_mut() else {
            return;
        };
        state.code_actions = Some(CodeActionsPaneState {
            title,
            selected: 0,
            actions: Vec::new(),
            loading: true,
        });
        state.focus = DiagnosticsPopupFocus::CodeActions;
    }

    fn selected_diagnostics_popup_entry(&self) -> Option<DiagnosticsPopupEntry> {
        let popup = self.lsp.diagnostics_popup.as_ref()?;
        self.current_diagnostic_popup_entries()
            .get(popup.selected)
            .cloned()
    }

    fn active_document_version(&self) -> Option<i32> {
        let active_id = self.session.active_id();
        self.lsp
            .documents
            .get(&active_id)
            .map(|document| document.document_version)
    }

    fn code_action_request_context(
        &self,
    ) -> Option<(
        DiagnosticsPopupEntry,
        Pos,
        String,
        IncomingRange,
        Vec<Value>,
    )> {
        let entry = if self.mode == EditorMode::DiagnosticsList {
            let popup = self.lsp.diagnostics_popup.as_ref()?;
            self.current_diagnostic_popup_entries()
                .get(popup.selected)
                .cloned()?
        } else {
            self.diagnostic_entry_under_cursor()?
        };
        let requested_at = Pos::new(entry.line, entry.col);
        let range = self.incoming_range_for_diagnostic_entry(&entry)?;
        let diagnostics = vec![self.code_action_diagnostic_value(&entry)?];
        Some((
            entry.clone(),
            requested_at,
            entry.summary.clone(),
            range,
            diagnostics,
        ))
    }

    fn incoming_range_for_diagnostic_entry(
        &self,
        entry: &DiagnosticsPopupEntry,
    ) -> Option<IncomingRange> {
        let buffer = self.session.buffer(self.session.active_id())?;
        let line_text = buffer.line_string(entry.line);
        let start = char_col_to_utf16(&line_text, entry.col);
        let end = char_col_to_utf16(&line_text, entry.end_col.max(entry.col.saturating_add(1)));
        Some(IncomingRange {
            start: IncomingPosition {
                line: entry.line as u64,
                character: start as u64,
            },
            end: IncomingPosition {
                line: entry.line as u64,
                character: end.max(start.saturating_add(1)) as u64,
            },
        })
    }

    fn code_action_diagnostic_value(&self, entry: &DiagnosticsPopupEntry) -> Option<Value> {
        let severity = match entry.severity {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Information => 3,
            DiagnosticSeverity::Hint => 4,
        };
        Some(json!({
            "range": self.incoming_range_for_diagnostic_entry(entry)?,
            "severity": severity,
            "message": entry.message,
        }))
    }

    pub(super) fn apply_workspace_edit(&mut self, edit: &WorkspaceEdit) -> io::Result<()> {
        let original_active = self.session.active_id();
        let (viewport_width_cells, viewport_height_rows) = self.viewport_size();
        let text_vh = viewport_height_rows.saturating_sub(crate::ui::STATUS_BAR_HEIGHT_ROWS);
        let mut touched_buffers = Vec::new();

        for document_edit in &edit.document_edits {
            let Some(expected_version) = document_edit.version else {
                continue;
            };
            let Some((buffer_id, document)) = self
                .lsp
                .documents
                .iter()
                .find(|(_, document)| document.uri == document_edit.uri)
            else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "versioned workspace edit targets an unopened document",
                ));
            };
            let current_analysis_version = self
                .views
                .get(buffer_id)
                .map(|view| view.analysis_version());
            if document.document_version != expected_version
                || document.last_sent_analysis_version != current_analysis_version
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "workspace edit targets a stale document version",
                ));
            }
        }

        let planned_edits = (|| {
            let mut planned_edits = HashMap::<BufferId, Vec<redox_lsp::TextEdit>>::new();
            for document_edit in &edit.document_edits {
                let Some(path) = file_path_from_uri(&document_edit.uri) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "workspace edit target is not a local file",
                    ));
                };
                let buffer_id = self.session.open_file(&path).map_err(io::Error::other)?;
                self.ensure_buffer_analysis(buffer_id);
                self.ensure_active_lsp_client();
                self.session
                    .ensure_buffer_fully_loaded(buffer_id)
                    .map_err(io::Error::other)?;
                planned_edits
                    .entry(buffer_id)
                    .or_default()
                    .extend(document_edit.edits.clone());
            }
            let planned_edits = planned_edits
                .into_iter()
                .map(|(buffer_id, edits)| {
                    let buffer = self.session.buffer(buffer_id).expect("loaded edit buffer");
                    prepare_lsp_edits(buffer, &edits).map(|edits| (buffer_id, edits))
                })
                .collect::<io::Result<Vec<_>>>()?;
            Ok(planned_edits)
        })();

        let _ = self.session.activate(original_active);
        let planned_edits = planned_edits?;

        for (buffer_id, edits) in planned_edits {
            if self
                .lsp
                .active_snippet
                .as_ref()
                .is_some_and(|snippet| snippet.buffer_id == buffer_id)
            {
                self.lsp.active_snippet = None;
            }
            self.finalize_buffer_insert_undo_if_changed(buffer_id);
            let before = self.capture_buffer_undo_checkpoint(buffer_id);
            let Some(buffer) = self.session.buffer_mut(buffer_id) else {
                continue;
            };
            apply_character_edits(buffer, &edits);
            let _ = self.session.recompute_buffer_dirty(buffer_id);
            self.invalidate_buffer_render_caches(buffer_id);
            let _ = self.with_buffer_view_mut(buffer_id, |buffer, view| {
                view.cursor.cursor = buffer.clamp_pos(view.cursor.cursor);
                view.cursor
                    .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
            });
            self.record_buffer_undo_if_changed(buffer_id, before);
            touched_buffers.push(buffer_id);
        }

        let _ = self.session.activate(original_active);
        for buffer_id in touched_buffers {
            let _ = self.sync_lsp_document(buffer_id, SyncPolicy::Immediate);
        }
        Ok(())
    }

    pub(super) fn take_code_action_response(
        &mut self,
        workspace: &WorkspaceKey,
        message: &Value,
    ) -> bool {
        let Some(id) = message.get("id").and_then(Value::as_i64) else {
            return false;
        };
        let key = RequestKey {
            workspace: workspace.clone(),
            id,
        };
        let Some(request) = self.lsp.pending_requests.get(&key).cloned() else {
            return false;
        };
        let PendingRequest::CodeActions {
            origin,
            requested_at,
            return_mode,
            title,
            trigger,
        } = request.kind
        else {
            return false;
        };
        self.lsp.pending_requests.remove(&key);
        if origin.workspace != *workspace || request.context != self.lsp_request_context() {
            return true;
        }

        if let Some(error) = message.get("error") {
            if let Some(popup) = self.lsp.diagnostics_popup.as_mut()
                && popup.code_action_origin.as_ref() == Some(&origin)
            {
                popup.pending_code_actions = None;
                if matches!(trigger, CodeActionRequestTrigger::Manual) {
                    popup.code_actions = None;
                    popup.focus = DiagnosticsPopupFocus::Diagnostics;
                }
            }
            if matches!(trigger, CodeActionRequestTrigger::Prefetch) {
                return true;
            }
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown LSP error");
            self.set_status(format!("code actions failed: {detail}"));
            return true;
        }

        let actions = parse_code_action_response(message);
        if actions.is_empty() {
            if let Some(popup) = self.lsp.diagnostics_popup.as_mut()
                && popup.code_action_origin.as_ref() == Some(&origin)
            {
                let should_close = popup
                    .pending_code_actions
                    .as_ref()
                    .is_some_and(|pending| pending.open_on_arrival);
                popup.pending_code_actions = None;
                popup.cached_code_actions = None;
                if should_close {
                    popup.code_actions = None;
                    popup.focus = DiagnosticsPopupFocus::Diagnostics;
                }
            }
            if matches!(trigger, CodeActionRequestTrigger::Manual) {
                self.set_status("no quick fixes available");
            }
            return true;
        }

        if return_mode == EditorMode::DiagnosticsList {
            let fallback_document_version = self.active_document_version().unwrap_or_default();
            if let Some(popup) = self.lsp.diagnostics_popup.as_mut()
                && popup.code_action_origin.as_ref() == Some(&origin)
            {
                let (diagnostic, document_version, open_on_arrival) = popup
                    .pending_code_actions
                    .as_ref()
                    .map(|pending| {
                        (
                            pending.diagnostic.clone(),
                            pending.document_version,
                            pending.open_on_arrival,
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            DiagnosticsPopupEntry {
                                severity: DiagnosticSeverity::Warning,
                                line: requested_at.line,
                                col: requested_at.col,
                                end_col: requested_at.col.saturating_add(1),
                                summary: title.clone(),
                                message: title.clone(),
                            },
                            fallback_document_version,
                            matches!(trigger, CodeActionRequestTrigger::Manual),
                        )
                    });
                popup.pending_code_actions = None;
                popup.cached_code_actions = Some(CachedCodeActions {
                    diagnostic,
                    document_version,
                    title: title.clone(),
                    actions: actions.clone(),
                });
                if open_on_arrival || matches!(trigger, CodeActionRequestTrigger::Manual) {
                    popup.code_actions = Some(CodeActionsPaneState {
                        title: title.clone(),
                        selected: 0,
                        actions: actions.clone(),
                        loading: false,
                    });
                    popup.focus = DiagnosticsPopupFocus::CodeActions;
                    self.clear_status();
                }
                return true;
            }
            return true;
        }

        if self.session.active_id() != origin.buffer_id {
            return true;
        }

        self.lsp.code_actions_popup = Some(CodeActionsPopupState {
            origin,
            selected: 0,
            requested_at,
            return_mode,
            title,
            actions,
        });
        self.mode = EditorMode::CodeActions;
        self.clear_status();
        true
    }

    pub(super) fn take_execute_command_response(
        &mut self,
        workspace: &WorkspaceKey,
        message: &Value,
    ) -> bool {
        let Some(id) = message.get("id").and_then(Value::as_i64) else {
            return false;
        };
        let key = RequestKey {
            workspace: workspace.clone(),
            id,
        };
        let Some(request) = self.lsp.pending_requests.get(&key).cloned() else {
            return false;
        };
        let PendingRequest::ExecuteCommand {
            title,
            edit_applied,
        } = request.kind
        else {
            return false;
        };
        self.lsp.pending_requests.remove(&key);

        if let Some(error) = message.get("error") {
            let detail = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown LSP error");
            if edit_applied {
                self.close_code_actions_popup();
                self.set_status(format!(
                    "applied quick fix edit, but command failed: {detail}"
                ));
            } else {
                self.set_status(format!("quick fix command failed: {detail}"));
            }
            return true;
        }

        self.close_code_actions_popup();
        self.set_status(format!("applied quick fix: {title}"));
        true
    }
}
