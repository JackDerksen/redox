use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use redox_core::{Pos, Selection, TextBuffer};

use super::{EditorMode, EditorState};
use crate::SOFT_TAB_WIDTH;
use crate::ui::STATUS_BAR_HEIGHT_ROWS;
use crate::ui::language_for_path;
use crate::ui::syntax::{SyntaxLanguage, smart_open_line_insert};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveFormatter {
    CargoFmt,
    Gofmt,
    Ruff,
}

impl EditorState {
    pub(super) fn execute_configured_command(&mut self, command: String) {
        self.command_line = command;
        self.command_line_cursor = self.command_line.len();
        self.clear_active_visual_anchor();
        self.mode = EditorMode::Command;
        self.execute_command_line();
    }

    pub(super) fn execute_command_line(&mut self) {
        if self.mode != EditorMode::Command {
            return;
        }

        let cmd_raw = self.command_line.trim().to_string();
        self.command_line.clear();
        self.command_line_cursor = 0;
        self.mode = EditorMode::Normal;
        self.reset_command_history_navigation();

        if cmd_raw.is_empty() {
            return;
        }

        self.push_command_history(cmd_raw.clone());

        let mut parts = cmd_raw.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).unwrap_or("");

        if self.command_uses_editor_context(cmd, arg) && !self.close_active_surfaces_for_command() {
            self.set_status("cannot return to an editor buffer");
            return;
        }

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
            "e!" | "reload" => {
                self.command_reload_active();
            }
            "config" => match arg {
                "" => self.request_config_open(),
                "reload" => self.request_config_reload(),
                _ => self.set_status("usage: config [reload]"),
            },
            "colorscheme" => self.request_colorscheme(arg),
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
            "perf" => match arg {
                "" | "popup" => self.command_toggle_perf(),
                _ => self.set_status("usage: perf [popup]"),
            },
            "undo-tree" => {
                self.command_toggle_undo_tree();
            }
            "lsp" => {
                self.command_lsp(arg);
            }
            _ => {
                self.set_status(format!("unknown command: {cmd_raw}"));
            }
        }
    }

    fn command_uses_editor_context(&self, cmd: &str, arg: &str) -> bool {
        match cmd {
            "w" | "wq" => self.active_buffer_is_surface() && !self.explorer_is_active(),
            "e" => !arg.is_empty(),
            "e!" | "reload" | "bn" | "bnext" | "bp" | "bprev" | "rain" => true,
            "config" => arg.is_empty(),
            "perf" => matches!(arg, "" | "popup"),
            "undo-tree" => !self.undo_tree_is_active(),
            "lsp" => matches!(arg, "list" | "status"),
            _ => false,
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
        self.command_line_cursor = self.command_line.len();
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
        self.command_line_cursor = self.command_line.len();
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
            Err(error) => {
                self.set_status(format!("open failed: {error}"));
            }
        }
    }

    pub(super) fn command_reload_active(&mut self) {
        let Some(path) = self.session.active_meta().path.clone() else {
            self.set_status("no file to reload");
            return;
        };
        let (viewport_width_cells, viewport_height_rows) = self.viewport_size();
        let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);

        match self.reload_active_buffer_from_disk(&path, viewport_width_cells, text_vh) {
            Ok(()) => {
                self.mark_git_repo_statuses_stale();
                match self.notify_active_lsp_did_save() {
                    Ok(()) => self.set_status("reloaded from disk"),
                    Err(error) => self.set_status(format!(
                        "reloaded from disk (diagnostics refresh failed: {error})"
                    )),
                }
            }
            Err(error) => self.set_status(format!("reload failed: {error}")),
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

        let mut message = String::new();
        for (idx, summary) in summaries.iter().enumerate() {
            if idx > 0 {
                message.push('\n');
            }
            let active = if summary.is_active { '%' } else { '-' };
            let dirty = if summary.dirty { '+' } else { '-' };
            let new_file = if summary.is_new_file { 'n' } else { '-' };
            let external = if summary.external_changed { '!' } else { '-' };
            message.push_str(&format!(
                "[{active}{dirty}{new_file}{external}]{}:{}",
                summary.id.get(),
                summary.display_name
            ));
        }

        self.set_status(message);
    }

    pub(super) fn command_lsp(&mut self, arg: &str) {
        match arg {
            "list" => self.open_lsp_marketplace(),
            "status" => self.command_lsp_status(),
            "" => self.set_status("usage: lsp list|status"),
            other => self.set_status(format!("unknown lsp command: {other}")),
        }
    }

    pub(super) fn write_current_file(&mut self) -> bool {
        if self.explorer_is_active() {
            return self.write_explorer_directory();
        }

        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return false;
        }

        let before = self.capture_active_undo_checkpoint();
        let (viewport_width_cells, viewport_height_rows) = self.viewport_size();
        let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);

        self.sync_active_lsp_before_save();

        if let Err(error) = self.session.save_active() {
            self.set_status(format!("write failed: {error}"));
            return false;
        }

        let format_result =
            self.format_active_file_after_save_if_available(viewport_width_cells, text_vh);
        let changed_buffer = match format_result {
            Ok(format_changed) => format_changed,
            Err(error) => {
                let _ = self.session.recompute_active_dirty();
                self.mark_git_repo_statuses_stale();
                match self.persist_active_undo_history() {
                    Ok(()) => self.set_status(format!("written (format failed: {error})")),
                    Err(history_error) => self.set_status(format!(
                        "written (format failed: {error}; undo history save failed: {history_error})"
                    )),
                }
                return true;
            }
        };

        if changed_buffer {
            let _ = self.record_active_undo_if_changed(before);
        }
        let _ = self.session.recompute_active_dirty();

        self.mark_git_repo_statuses_stale();
        let history_result = self.persist_active_undo_history();
        let lsp_result = self.notify_active_lsp_did_save();
        match (history_result, lsp_result) {
            (Ok(()), Ok(())) => self.set_status("written"),
            (Err(error), Ok(())) => {
                self.set_status(format!("written (undo history save failed: {error})"))
            }
            (Ok(()), Err(error)) => {
                self.set_status(format!("written (LSP save sync failed: {error})"))
            }
            (Err(history_error), Err(lsp_error)) => self.set_status(format!(
                "written (undo history save failed: {history_error}; LSP save sync failed: {lsp_error})"
            )),
        }
        true
    }

    fn format_active_file_after_save_if_available(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> Result<bool, String> {
        let path = self.session.active_meta().path.clone();
        let buffer_text = self.session.active_buffer().to_string();
        let mut formatted = buffer_text.clone();

        if let Some(path) = path.as_deref()
            && let Some(language) = language_for_path(Some(path))
            && let Some(formatter) = formatter_for(language)
            && formatter_available(formatter, path)
        {
            if let Err(error) = run_formatter(formatter, self.session.launch_dir(), path) {
                if formatter == SaveFormatter::Ruff {
                    self.reload_active_buffer_from_disk(path, viewport_width_cells, text_vh)
                        .map_err(|reload_error| {
                            format!("{error}; failed to reload formatter output: {reload_error}")
                        })?;
                }
                return Err(error);
            }
            formatted = std::fs::read_to_string(path)
                .map_err(|error| format!("failed to read formatter output: {error}"))?;
            self.session.refresh_active_disk_stamp();
        }

        let normalized = apply_save_format_passes(&formatted);

        if formatted == buffer_text && normalized == buffer_text {
            return Ok(false);
        }

        self.replace_active_buffer_text(&normalized, viewport_width_cells, text_vh);
        self.sync_active_lsp_before_save();
        self.session
            .save_active()
            .map_err(|error| format!("failed to save formatted file: {error}"))?;
        Ok(true)
    }

    fn reload_active_buffer_from_disk(
        &mut self,
        path: &Path,
        viewport_width_cells: usize,
        text_vh: usize,
    ) -> Result<(), std::io::Error> {
        let text = std::fs::read_to_string(path)?;
        self.replace_active_buffer_text(&text, viewport_width_cells, text_vh);
        self.session.mark_active_clean();
        let active_id = self.session.active_id();
        self.clear_buffer_undo_history(active_id);
        Ok(())
    }

    fn replace_active_buffer_text(
        &mut self,
        text: &str,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        {
            let buffer = self.session.active_buffer_mut();
            *buffer = TextBuffer::from_text(text);
        }

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        view.cursor.cursor = buffer.clamp_pos(view.cursor.cursor);
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        self.reset_active_render_caches();
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn open_line_and_enter_insert(
        &mut self,
        above: bool,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        let before = self.capture_active_undo_checkpoint();
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
                let selection = Selection::empty(insert_pos);
                let selection = buffer.insert_newline(selection);
                view.cursor.cursor = if above {
                    Pos::new(line, 0)
                } else {
                    selection.cursor
                };
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

fn formatter_for(language: SyntaxLanguage) -> Option<SaveFormatter> {
    match language {
        SyntaxLanguage::Rust => Some(SaveFormatter::CargoFmt),
        SyntaxLanguage::Go => Some(SaveFormatter::Gofmt),
        SyntaxLanguage::Python => Some(SaveFormatter::Ruff),
        _ => None,
    }
}

fn formatter_available(formatter: SaveFormatter, path: &Path) -> bool {
    match formatter {
        SaveFormatter::CargoFmt => {
            cargo_workspace_root(path).is_some()
                && executable_on_path("cargo")
                && executable_on_path("rustfmt")
        }
        SaveFormatter::Gofmt => executable_on_path("gofmt"),
        SaveFormatter::Ruff => executable_on_path("ruff"),
    }
}

fn run_formatter(formatter: SaveFormatter, launch_dir: &Path, path: &Path) -> Result<(), String> {
    match formatter {
        SaveFormatter::CargoFmt => {
            let workspace_root = cargo_workspace_root(path)
                .ok_or_else(|| "cargo fmt unavailable outside a Cargo workspace".to_string())?;
            run_command_status(
                Command::new("cargo")
                    .current_dir(workspace_root)
                    .args(["fmt", "--"])
                    .arg(path),
                "cargo fmt",
            )
        }
        SaveFormatter::Gofmt => run_command_status(
            Command::new("gofmt")
                .current_dir(launch_dir)
                .args(["-w"])
                .arg(path),
            "gofmt",
        ),
        SaveFormatter::Ruff => {
            run_command_status(
                Command::new("ruff")
                    .current_dir(launch_dir)
                    .args(["check", "--fix", "--exit-zero"])
                    .arg(path),
                "ruff check --fix",
            )?;
            run_command_status(
                Command::new("ruff")
                    .current_dir(launch_dir)
                    .args(["format"])
                    .arg(path),
                "ruff format",
            )
        }
    }
}

fn cargo_workspace_root(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|dir| dir.join("Cargo.toml").is_file())
}

fn run_command_status(command: &mut Command, label: &str) -> Result<(), String> {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("{label} failed to start: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("exit status {}", output.status)
    };
    Err(format!("{label} failed: {detail}"))
}

fn executable_on_path(executable: &str) -> bool {
    if executable.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(executable).exists();
    }

    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&paths).any(|dir| dir.join(executable).exists())
}

fn apply_save_format_passes(text: &str) -> String {
    trim_trailing_whitespace_text(&expand_hard_tabs(text, SOFT_TAB_WIDTH))
}

fn expand_hard_tabs(text: &str, tab_width: usize) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut col = 0usize;
    let mut in_indent = true;

    for ch in text.chars() {
        match ch {
            '\t' if in_indent => {
                let spaces = tab_width - (col % tab_width);
                expanded.extend(std::iter::repeat_n(' ', spaces));
                col += spaces;
            }
            '\n' => {
                expanded.push('\n');
                col = 0;
                in_indent = true;
            }
            ' ' if in_indent => {
                expanded.push(ch);
                col += 1;
            }
            _ => {
                expanded.push(ch);
                in_indent = false;
            }
        }
    }

    expanded
}

fn trim_trailing_whitespace_text(text: &str) -> String {
    let mut trimmed = String::with_capacity(text.len());

    for line in text.split_inclusive('\n') {
        let Some(content) = line.strip_suffix('\n') else {
            trimmed.push_str(line.trim_end_matches([' ', '\t']));
            continue;
        };
        trimmed.push_str(content.trim_end_matches([' ', '\t']));
        trimmed.push('\n');
    }

    trimmed
}
