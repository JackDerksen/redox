use std::path::PathBuf;

use redox_core::Pos;
use redox_core::Selection;

use super::{EditorMode, EditorState};

impl EditorState {
    pub(super) fn execute_command_line(&mut self) {
        if self.mode != EditorMode::Command {
            return;
        }

        let cmd_raw = self.command_line.trim().to_string();
        self.command_line.clear();
        self.mode = EditorMode::Normal;

        if cmd_raw.is_empty() {
            return;
        }

        let mut parts = cmd_raw.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).unwrap_or("");

        match cmd {
            "w" => {
                self.write_current_file();
            }
            "q" => {
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
            _ => {
                self.set_status(format!("unknown command: {cmd_raw}"));
            }
        }
    }

    pub(super) fn command_edit(&mut self, path_arg: &str) {
        if path_arg.is_empty() {
            self.set_status("usage: e <path>");
            return;
        }

        let path = PathBuf::from(path_arg);
        match self.session.open_file(path) {
            Ok(id) => {
                let _ = self.views.entry(id).or_default();
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
            let _ = self.views.entry(id).or_default();
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
            let _ = self.views.entry(id).or_default();
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
                msg.push_str(" | ");
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

        self.set_status_ephemeral(msg);
    }

    pub(super) fn write_current_file(&mut self) -> bool {
        if self.explorer_is_active() {
            return self.write_explorer_directory();
        }

        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return false;
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
        let view = self.views.entry(active_id).or_default();

        {
            let buffer = self.session.active_buffer_mut();
            let line = buffer.clamp_line(view.cursor.cursor.line);
            let insert_pos = if above {
                Pos::new(line, 0)
            } else {
                Pos::new(line, buffer.line_len_chars(line))
            };

            let sel = Selection::empty(insert_pos);
            let sel = buffer.insert_newline(sel);
            view.cursor.cursor = if above { Pos::new(line, 0) } else { sel.cursor };
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

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
