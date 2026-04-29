use std::path::PathBuf;

use redox_core::Pos;
use redox_core::Selection;

use super::{EditorMode, EditorState};
use crate::ui::STATUS_BAR_HEIGHT_ROWS;
use crate::ui::language_for_path;
use crate::ui::syntax::smart_open_line_insert;

impl EditorState {
    pub(super) fn execute_command_line(&mut self) {
        if self.mode != EditorMode::Command {
            return;
        }

        let cmd_raw = self.command_line.trim().to_string();
        self.command_line.clear();
        self.mode = EditorMode::Normal;
        self.reset_command_history_navigation();

        if cmd_raw.is_empty() {
            return;
        }

        self.push_command_history(cmd_raw.clone());

        let mut parts = cmd_raw.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).unwrap_or("");

        match cmd {
            "w" => {
                self.write_current_file();
            }
            "q" | "quit" => {
                if self.active_buffer_is_surface() {
                    if self.close_active_surface_buffer() {
                        self.clear_status();
                    } else {
                        self.set_status("cannot close the last buffer");
                    }
                    return;
                }

                if self.session.any_dirty() {
                    self.set_status(self.unsaved_changes_quit_message());
                } else {
                    self.should_quit = true;
                }
            }
            "q!" => {
                self.should_quit = true;
            }
            "wq" => {
                if self.write_current_file() {
                    if self.session.any_dirty() {
                        self.set_status(self.unsaved_changes_message());
                    } else {
                        self.should_quit = true;
                    }
                }
            }
            "e" => {
                self.command_edit(arg);
            }
            "bn" | "bnext" => {
                self.command_buffer_cycle_next();
            }
            "bp" | "bprev" => {
                self.command_buffer_cycle_prev();
            }
            "ls" => {
                self.command_list_buffers();
            }
            "ex" | "explorer" => {
                self.command_open_explorer();
            }
            "about" => {
                self.command_open_about();
            }
            "rain" => {
                self.command_rain();
            }
            "perf" => {
                self.command_toggle_perf();
            }
            _ => {
                self.set_status(format!("unknown command: {cmd_raw}"));
            }
        }
    }

    pub(super) fn reset_command_history_navigation(&mut self) {
        self.command_history.nav_index = None;
        self.command_history.draft.clear();
    }

    pub(super) fn detach_command_history_navigation(&mut self) {
        if self.command_history.nav_index.is_some() {
            self.command_history.draft = self.command_line.clone();
            self.command_history.nav_index = None;
        }
    }

    pub(super) fn command_history_prev(&mut self) {
        if self.command_history.entries.is_empty() {
            return;
        }

        let next_index = match self.command_history.nav_index {
            Some(0) => 0,
            Some(idx) => idx.saturating_sub(1),
            None => {
                self.command_history.draft = self.command_line.clone();
                self.command_history.entries.len().saturating_sub(1)
            }
        };

        self.command_history.nav_index = Some(next_index);
        self.command_line = self.command_history.entries[next_index].clone();
    }

    pub(super) fn command_history_next(&mut self) {
        let Some(current_index) = self.command_history.nav_index else {
            return;
        };

        if current_index + 1 < self.command_history.entries.len() {
            let next_index = current_index + 1;
            self.command_history.nav_index = Some(next_index);
            self.command_line = self.command_history.entries[next_index].clone();
        } else {
            self.command_history.nav_index = None;
            self.command_line = std::mem::take(&mut self.command_history.draft);
        }
    }

    fn push_command_history(&mut self, command: String) {
        if self
            .command_history
            .entries
            .last()
            .is_some_and(|previous| previous == &command)
        {
            return;
        }
        self.command_history.entries.push(command);
        const MAX_HISTORY: usize = 100;
        if self.command_history.entries.len() > MAX_HISTORY {
            let overflow = self.command_history.entries.len() - MAX_HISTORY;
            self.command_history.entries.drain(0..overflow);
        }
    }

    pub(super) fn command_edit(&mut self, path_arg: &str) {
        if path_arg.is_empty() {
            self.set_status("usage: e <path>");
            return;
        }

        let path = PathBuf::from(path_arg);
        self.transient_origin_buffer_id = None;
        self.transient_origin_dir = None;
        let previous_id = self.session.active_id();
        let close_previous_placeholder = self.is_empty_unnamed_startup_buffer(previous_id);
        match self.session.open_file(path) {
            Ok(id) => {
                let _ = self.views.entry(id).or_default();
                self.ensure_buffer_analysis(id);
                if close_previous_placeholder && previous_id != id {
                    let _ = self.close_inactive_empty_unnamed_startup_buffer(previous_id);
                }
                self.clear_status();
            }
            Err(e) => {
                self.set_status(format!("open failed: {e}"));
            }
        }
    }

    pub(super) fn command_buffer_cycle_next(&mut self) {
        let count = self.session.summaries().len();
        if count <= 1 {
            self.set_status("only one buffer");
            return;
        }

        if let Some(id) = self.session.switch_next_mru() {
            self.transient_origin_buffer_id = None;
            self.transient_origin_dir = None;
            let _ = self.views.entry(id).or_default();
            self.ensure_buffer_analysis(id);
            self.clear_status();
        }
    }

    pub(super) fn command_buffer_cycle_prev(&mut self) {
        let count = self.session.summaries().len();
        if count <= 1 {
            self.set_status("only one buffer");
            return;
        }

        if let Some(id) = self.session.switch_prev_mru() {
            self.transient_origin_buffer_id = None;
            self.transient_origin_dir = None;
            let _ = self.views.entry(id).or_default();
            self.ensure_buffer_analysis(id);
            self.clear_status();
        }
    }

    pub(super) fn command_list_buffers(&mut self) {
        let summaries = self.session.summaries();
        if summaries.is_empty() {
            self.set_status("no buffers");
            return;
        }

        let mut msg = String::new();
        for (idx, summary) in summaries.iter().enumerate() {
            if idx > 0 {
                msg.push('\n');
            }
            let active = if summary.is_active { '%' } else { '-' };
            let dirty = if summary.dirty { '+' } else { '-' };
            let new_file = if summary.is_new_file { 'n' } else { '-' };
            msg.push_str(&format!(
                "[{active}{dirty}{new_file}]{}:{}",
                summary.id.get(),
                summary.display_name
            ));
        }

        self.set_status(msg);
    }

    pub(super) fn write_current_file(&mut self) -> bool {
        if self.explorer_is_active() {
            return self.write_explorer_directory();
        }

        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return false;
        }

        let before = self.capture_active_undo_snapshot();
        let trimmed = self.trim_active_trailing_whitespace();
        if trimmed {
            let (viewport_width_cells, viewport_height_rows) = self.viewport_size();
            let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);
            let active_id = self.session.active_id();
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
            self.invalidate_active_render_caches();
            let _ = self.record_active_undo_if_changed(before);
            let _ = self.session.recompute_active_dirty();
        }

        match self.session.save_active() {
            Ok(()) => {
                self.set_status("written");
                true
            }
            Err(e) => {
                self.set_status(format!("write failed: {e}"));
                false
            }
        }
    }

    fn trim_active_trailing_whitespace(&mut self) -> bool {
        self.session.active_buffer_mut().trim_trailing_whitespace()
    }

    pub(super) fn open_line_and_enter_insert(
        &mut self,
        above: bool,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        let before = self.capture_active_undo_snapshot();
        self.mode = EditorMode::Insert;
        self.clear_status();
        self.input.reset_prefixes();

        let active_id = self.session.active_id();
        let cursor = self.views.entry(active_id).or_default().cursor.cursor;
        let language = language_for_path(self.session.active_meta().path.as_deref());
        let line = self.session.active_buffer().clamp_line(cursor.line);
        let smart_insert =
            smart_open_line_insert(self.session.active_buffer(), language, line, above);
        let view = self.views.entry(active_id).or_default();

        {
            let buffer = self.session.active_buffer_mut();
            let insert_pos = if above {
                Pos::new(line, 0)
            } else {
                Pos::new(line, buffer.line_len_chars(line))
            };

            if let Some((text, cursor)) = smart_insert {
                let _ = buffer.insert(insert_pos, &text);
                view.cursor.cursor = cursor;
            } else {
                let sel = Selection::empty(insert_pos);
                let sel = buffer.insert_newline(sel);
                view.cursor.cursor = if above { Pos::new(line, 0) } else { sel.cursor };
            }
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn unsaved_changes_message(&self) -> String {
        let dirty: Vec<redox_core::BufferSummary> = self
            .session
            .summaries()
            .into_iter()
            .filter(|summary| summary.dirty)
            .collect();

        if dirty.is_empty() {
            return "unsaved changes".to_string();
        }

        let first_name = dirty[0].display_name.clone();
        if dirty.len() == 1 {
            format!("unsaved changes in {first_name}")
        } else {
            format!("unsaved changes in {first_name} (+{})", dirty.len() - 1)
        }
    }

    pub(super) fn unsaved_changes_quit_message(&self) -> String {
        format!("{} (use :q! to quit)", self.unsaved_changes_message())
    }
}
