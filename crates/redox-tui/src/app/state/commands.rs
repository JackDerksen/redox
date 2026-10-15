use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use redox_core::{BufferId, BufferKind, Pos, Selection, TextBuffer, TextDiff};

use super::{EditorMode, EditorState, SplitAxis, calculator};
use crate::input::InputAction;
use crate::ui::STATUS_BAR_HEIGHT_ROWS;
use crate::ui::icons;
use crate::ui::language_for_path;
use crate::ui::syntax::{SyntaxLanguage, smart_open_line_insert};

struct CommandDefinition {
    names: &'static [&'static str],
    editor_context: fn(&EditorState, &str) -> bool,
    run: fn(&mut EditorState, &str),
}

// Dispatch and completion share these names, including aliases and subcommands.
const COMMANDS: &[CommandDefinition] = &[
    CommandDefinition {
        names: &["whichkey", "which-key"],
        editor_context: |_, _| true,
        run: |state, argument| {
            match state
                .input
                .show_which_key(state.mode.as_input_mode(), argument)
            {
                Ok(()) => state.clear_status(),
                Err(error) => state.set_status(error.to_string()),
            }
            state.request_redraw();
        },
    },
    CommandDefinition {
        names: &["zoom"],
        editor_context: |_, _| false,
        run: |state, argument| {
            if argument.is_empty() {
                state.toggle_pane_zoom();
            } else {
                state.set_status("usage: zoom");
            }
        },
    },
    CommandDefinition {
        names: &["split", "sp"],
        editor_context: |_, _| true,
        run: |state, argument| state.command_split(SplitAxis::Horizontal, argument),
    },
    CommandDefinition {
        names: &["vsplit", "vs", "vsp"],
        editor_context: |_, _| true,
        run: |state, argument| state.command_split(SplitAxis::Vertical, argument),
    },
    CommandDefinition {
        names: &["close", "clo"],
        editor_context: |_, _| false,
        run: |state, argument| {
            if !argument.is_empty() {
                state.set_status("usage: close");
            } else if state.active_buffer_is_surface() {
                state.close_active_surface_buffer_without_quit();
            } else {
                state.close_active_split();
            }
        },
    },
    CommandDefinition {
        names: &["only", "on"],
        editor_context: |_, argument| argument.is_empty(),
        run: |state, argument| {
            if argument.is_empty() {
                state.close_other_splits();
            } else {
                state.set_status("usage: only");
            }
        },
    },
    CommandDefinition {
        names: &["terminal", "term"],
        editor_context: |_, _| false,
        run: |state, _| {
            if let Err(error) = state.terminal.toggle(state.session.launch_dir()) {
                state.set_status(format!("could not open terminal: {error:#}"));
            }
        },
    },
    CommandDefinition {
        names: &["log"],
        editor_context: |_, _| false,
        run: |state, argument| state.command_log(argument),
    },
    CommandDefinition {
        names: &["about"],
        editor_context: |_, _| false,
        run: |state, _| state.command_open_about(),
    },
    CommandDefinition {
        names: &["b", "bu", "buf", "buffer"],
        editor_context: |_, _| true,
        run: |state, argument| state.command_buffer(argument),
    },
    CommandDefinition {
        names: &["bd", "bdel", "bdelete"],
        editor_context: |_, _| false,
        run: |state, argument| state.command_delete_buffer(argument, false),
    },
    CommandDefinition {
        names: &["bd!", "bdel!", "bdelete!"],
        editor_context: |_, _| false,
        run: |state, argument| state.command_delete_buffer(argument, true),
    },
    CommandDefinition {
        names: &["enew", "ene"],
        editor_context: |_, argument| argument.is_empty(),
        run: |state, argument| {
            if argument.is_empty() {
                state.sync_active_pane_view();
                let id = state.session.open_unnamed_buffer();
                state.finish_buffer_switch(id);
            } else {
                state.set_status("usage: enew");
            }
        },
    },
    CommandDefinition {
        names: &["bn", "bnext"],
        editor_context: |_, _| true,
        run: |state, _| state.command_buffer_cycle_next(),
    },
    CommandDefinition {
        names: &["bp", "bprev"],
        editor_context: |_, _| true,
        run: |state, _| state.command_buffer_cycle_prev(),
    },
    CommandDefinition {
        names: &["check-update"],
        editor_context: |_, _| false,
        run: |state, argument| {
            if argument.is_empty() {
                state.start_update_check(true);
            } else {
                state.set_status("usage: check-update");
            }
        },
    },
    CommandDefinition {
        names: &["colorscheme"],
        editor_context: |_, _| false,
        run: |state, argument| state.request_colorscheme(argument),
    },
    CommandDefinition {
        names: &["config"],
        editor_context: |_, argument| argument.is_empty(),
        run: |state, argument| {
            if argument.is_empty() {
                state.request_config_open();
            } else {
                state.set_status("usage: config [reload]");
            }
        },
    },
    CommandDefinition {
        names: &["config reload"],
        editor_context: |_, _| false,
        run: |state, _| state.request_config_reload(),
    },
    CommandDefinition {
        names: &["e"],
        editor_context: |_, argument| !argument.is_empty(),
        run: |state, argument| state.command_edit(argument),
    },
    CommandDefinition {
        names: &["delm", "delmarks"],
        editor_context: |_, _| true,
        run: |state, argument| state.command_delete_marks(argument),
    },
    CommandDefinition {
        names: &["delm!", "delmarks!"],
        editor_context: |_, _| true,
        run: |state, argument| {
            if argument.is_empty() {
                let buffer = state.session.active_buffer_mut();
                for name in 'a'..='z' {
                    buffer.delete_mark(name);
                }
                state.clear_status();
            } else {
                state.set_status("usage: delmarks!");
            }
        },
    },
    CommandDefinition {
        names: &["dashboard"],
        editor_context: |_, _| false,
        run: |state, _| state.open_dashboard(),
    },
    CommandDefinition {
        names: &["e!", "reload"],
        editor_context: |_, _| true,
        run: |state, _| state.command_reload_active(),
    },
    CommandDefinition {
        names: &["ex", "explorer"],
        editor_context: |_, _| false,
        run: |state, _| state.command_open_explorer(),
    },
    CommandDefinition {
        names: &["ls"],
        editor_context: |_, _| false,
        run: |state, _| state.command_list_buffers(),
    },
    CommandDefinition {
        names: &["lsp"],
        editor_context: |_, _| false,
        run: |state, argument| {
            if argument.is_empty() {
                state.set_status("usage: lsp list|status");
            } else {
                state.set_status(format!("unknown lsp command: {argument}"));
            }
        },
    },
    CommandDefinition {
        names: &["lsp list"],
        editor_context: |_, _| true,
        run: |state, _| state.open_lsp_marketplace(),
    },
    CommandDefinition {
        names: &["lsp status"],
        editor_context: |_, _| true,
        run: |state, _| state.command_lsp_status(),
    },
    CommandDefinition {
        names: &["macros"],
        editor_context: |_, _| false,
        run: |state, _| state.command_list_macros(),
    },
    CommandDefinition {
        names: &["perf"],
        editor_context: |_, argument| argument.is_empty(),
        run: |state, argument| {
            if argument.is_empty() {
                state.command_toggle_perf();
            } else {
                state.set_status("usage: perf [popup]");
            }
        },
    },
    CommandDefinition {
        names: &["perf popup"],
        editor_context: |_, _| true,
        run: |state, _| state.command_toggle_perf(),
    },
    CommandDefinition {
        names: &["q", "quit"],
        editor_context: |_, _| false,
        run: |state, _| {
            if state.active_buffer_is_surface() {
                if state.close_active_surface_buffer() {
                    state.clear_status();
                } else {
                    state.set_status("cannot close the last buffer");
                }
            } else if state.has_unsaved_changes() {
                state.set_status(state.unsaved_changes_quit_message());
            } else {
                state.should_quit = true;
            }
        },
    },
    CommandDefinition {
        names: &["q!"],
        editor_context: |_, _| false,
        run: |state, _| state.should_quit = true,
    },
    CommandDefinition {
        names: &["qa", "qall"],
        editor_context: |_, _| false,
        run: |state, argument| state.command_quit_all(argument, false),
    },
    CommandDefinition {
        names: &["qa!", "qall!"],
        editor_context: |_, _| false,
        run: |state, argument| state.command_quit_all(argument, true),
    },
    CommandDefinition {
        names: &["rain"],
        editor_context: |_, _| true,
        run: |state, _| state.command_rain(),
    },
    CommandDefinition {
        names: &["zen"],
        editor_context: |_, _| false,
        run: |state, _| state.toggle_zen(),
    },
    CommandDefinition {
        names: &["undo-tree"],
        editor_context: |state, _| !state.undo_tree_is_active(),
        run: |state, _| state.command_toggle_undo_tree(),
    },
    CommandDefinition {
        names: &["w"],
        editor_context: |state, _| state.active_buffer_is_surface() && !state.explorer_is_active(),
        run: |state, argument| {
            state.write_current_file(argument);
        },
    },
    CommandDefinition {
        names: &["wq"],
        editor_context: |state, _| state.active_buffer_is_surface() && !state.explorer_is_active(),
        run: |state, argument| {
            if state.write_current_file(argument) {
                if state.has_unsaved_changes() {
                    state.set_status(state.unsaved_changes_message());
                } else {
                    state.should_quit = true;
                }
            }
        },
    },
    CommandDefinition {
        names: &["wa", "wall"],
        editor_context: |_, _| false,
        run: |state, argument| state.command_write_all(argument, false),
    },
    CommandDefinition {
        names: &["wqa", "wqall"],
        editor_context: |_, _| false,
        run: |state, argument| state.command_write_all(argument, true),
    },
];

fn builtin_commands() -> impl Iterator<Item = (&'static str, &'static CommandDefinition)> {
    COMMANDS
        .iter()
        .flat_map(|definition| definition.names.iter().map(move |&name| (name, definition)))
}

fn resolve_command(command: &str, argument: &str) -> Option<&'static CommandDefinition> {
    builtin_commands()
        .filter(|(name, _)| match name.split_once(' ') {
            Some((head, tail)) => head == command && tail == argument,
            None => *name == command,
        })
        // Prefer a fixed subcommand to its command's argument fallback.
        .max_by_key(|(name, _)| name.len())
        .map(|(_, definition)| definition)
}

fn parse_mark_names(argument: &str) -> Option<Vec<char>> {
    let mut chars = argument.chars().filter(|ch| !ch.is_whitespace()).peekable();
    let mut names = Vec::new();
    while let Some(start) = chars.next() {
        if !start.is_ascii_lowercase() {
            return None;
        }
        let end = if chars.next_if_eq(&'-').is_some() {
            chars
                .next()
                .filter(|end| end.is_ascii_lowercase() && *end >= start)?
        } else {
            start
        };
        names.extend(start..=end);
    }
    (!names.is_empty()).then_some(names)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveFormatter {
    CargoFmt,
    Gofmt,
    Ruff,
    ClangFormat,
}

#[derive(Debug, Default)]
pub(super) struct CommandArgumentCompletions {
    prefix: String,
    candidates: Vec<String>,
}

impl EditorState {
    pub(super) fn refresh_command_argument_completions(&mut self) {
        let input = self.command_line.trim_start();
        if self.mode == EditorMode::Command
            && let Some((command, argument)) = input.split_once(char::is_whitespace)
            && matches!(
                command,
                "b" | "bu"
                    | "buf"
                    | "buffer"
                    | "bd"
                    | "bdel"
                    | "bdelete"
                    | "bd!"
                    | "bdel!"
                    | "bdelete!"
            )
        {
            let prefix = input[..input.len() - argument.trim_start().len()].to_string();
            let mut candidates = Vec::new();
            for summary in self
                .session
                .summaries()
                .into_iter()
                .filter(|summary| summary.kind == BufferKind::File)
            {
                for name in [
                    summary.display_name,
                    summary.id.get().to_string(),
                    summary
                        .path
                        .as_deref()
                        .and_then(Path::file_name)
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ] {
                    if !name.is_empty() && !name.contains(char::is_control) {
                        candidates.push(format!("{prefix}{name}"));
                    }
                }
            }
            candidates.sort();
            candidates.dedup();
            self.command_argument_completions = CommandArgumentCompletions { prefix, candidates };
            return;
        }
        let path_argument = input
            .split_once(char::is_whitespace)
            .filter(|(command, _)| {
                self.mode == EditorMode::Command
                    && matches!(
                        *command,
                        "e" | "w" | "wq" | "split" | "sp" | "vsplit" | "vs" | "vsp"
                    )
            });
        let Some((_, argument)) = path_argument else {
            self.command_argument_completions = CommandArgumentCompletions::default();
            return;
        };
        let argument = argument.trim_start();
        let directory_end = argument
            .rfind(std::path::MAIN_SEPARATOR)
            .map_or(0, |index| index + 1);
        let directory = &argument[..directory_end];
        let prefix = &input[..input.len() - argument.len() + directory_end];
        if self.command_argument_completions.prefix == prefix {
            return;
        }
        let mut candidates = Vec::new();
        if let Ok(entries) = std::fs::read_dir(if directory.is_empty() { "." } else { directory }) {
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                if name.contains(char::is_control) {
                    continue;
                }
                let mut candidate = format!("{prefix}{name}");
                if entry.path().is_dir() {
                    candidate.push(std::path::MAIN_SEPARATOR);
                }
                candidates.push(candidate);
            }
        }
        candidates.sort();
        self.command_argument_completions = CommandArgumentCompletions {
            prefix: prefix.to_owned(),
            candidates,
        };
    }

    pub(crate) fn configure_command_completions<'name>(
        &mut self,
        names: impl IntoIterator<Item = &'name str>,
    ) {
        self.configured_command_completions = names
            .into_iter()
            .map(|name| format!("colorscheme {name}"))
            .chain(self.input.configured_commands().map(str::to_owned))
            .filter(|candidate| {
                candidate != calculator::CONVERT_COMMAND
                    && !builtin_commands().any(|(name, _)| name == candidate)
            })
            .collect();
        self.configured_command_completions.sort();
        self.configured_command_completions.dedup();
        self.command_completion_index = 0;
    }

    fn command_completions(&self) -> impl Iterator<Item = &str> + '_ {
        let prefix = self.command_line.trim_start();
        builtin_commands()
            .map(|(name, _)| name)
            .chain([calculator::CONVERT_COMMAND])
            .chain(
                self.configured_command_completions
                    .iter()
                    .map(String::as_str),
            )
            .chain(
                self.command_argument_completions
                    .candidates
                    .iter()
                    .map(String::as_str)
                    .filter(|candidate| {
                        !self
                            .configured_command_completions
                            .iter()
                            .any(|configured| configured == candidate)
                    }),
            )
            .filter(move |candidate| {
                self.mode == EditorMode::Command
                    && candidate.starts_with(prefix)
                    && *candidate != prefix
            })
    }

    pub(crate) fn command_completion_suffix(&self) -> Option<&str> {
        self.command_completions()
            .nth(self.command_completion_index)
            .map(|candidate| &candidate[self.command_line.trim_start().len()..])
    }

    pub(crate) fn command_calculation_preview(&self) -> Option<String> {
        if self.mode != EditorMode::Command {
            return None;
        }
        let expression = calculator::expression(&self.command_line)?;
        let result = calculator::evaluate(expression).ok()?;
        let separator = if expression.split_whitespace().next() == Some(calculator::CONVERT_COMMAND)
        {
            ": "
        } else {
            " = "
        };
        Some(format!("{separator}{result}"))
    }

    pub(super) fn cycle_command_completion(&mut self, forward: bool) {
        let count = self.command_completions().count();
        if count > 0 {
            self.command_completion_index = if forward {
                (self.command_completion_index + 1) % count
            } else {
                (self.command_completion_index + count - 1) % count
            };
        }
    }

    pub(super) fn accept_command_completion(&mut self) {
        let Some(suffix) = self.command_completion_suffix().map(str::to_owned) else {
            return;
        };
        self.detach_command_history_navigation();
        self.command_line.push_str(&suffix);
        self.command_line_cursor = self.command_line.len();
    }

    pub(super) fn execute_configured_command(&mut self, command: String) {
        self.begin_command();
        self.command_line = command;
        self.command_line_cursor = self.command_line.len();
        self.execute_command_line();
    }

    pub(super) fn execute_command_line(&mut self) {
        if self.mode != EditorMode::Command {
            return;
        }
        if self.execute_substitute_command() {
            return;
        }
        self.log_event("command", serde_json::Value::Null);

        let cmd_raw = self.command_line.trim().to_string();
        let calculation = match calculator::expression(&cmd_raw).map(calculator::evaluate) {
            Some(Ok(result)) => Some(result),
            Some(Err(error)) => {
                self.set_status(format!("calculator: {error}"));
                return;
            }
            None => None,
        };
        self.command_line.clear();
        self.command_line_cursor = 0;
        self.mode = EditorMode::Normal;
        self.reset_command_history_navigation();

        if cmd_raw.is_empty() {
            return;
        }

        self.push_command_history(cmd_raw.clone());

        if cmd_raw.bytes().all(|byte| byte.is_ascii_digit()) {
            self.command_goto_line(cmd_raw.parse().unwrap_or(usize::MAX));
            return;
        }

        if let Some(result) = calculation {
            if !self.close_active_surfaces_for_command() {
                self.set_status("cannot return to an editor buffer");
                return;
            }
            if !self.ensure_active_fully_loaded_for_edit_or_save() {
                return;
            }
            self.clear_status();
            let (width, height) = self.viewport_size();
            self.apply_input(InputAction::Paste(result.clone()), width, height);
            self.remember_inserted_text(result);
            return;
        }

        let mut parts = cmd_raw.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().map(str::trim).unwrap_or("");

        let Some(definition) = resolve_command(cmd, arg) else {
            self.set_status(format!("unknown command: {cmd_raw}"));
            return;
        };
        if (definition.editor_context)(self, arg) && !self.close_active_surfaces_for_command() {
            self.set_status("cannot return to an editor buffer");
            return;
        }
        (definition.run)(self, arg);
    }

    /// Delete named marks given as letters and ranges, as in `:delmarks a c-e`.
    fn command_delete_marks(&mut self, argument: &str) {
        let Some(names) = parse_mark_names(argument) else {
            self.set_status("usage: delmarks {a-z and ranges like b-d} | delmarks!");
            return;
        };
        let buffer = self.session.active_buffer_mut();
        for name in names {
            buffer.delete_mark(name);
        }
        self.clear_status();
    }

    fn command_goto_line(&mut self, line_number: usize) {
        let previous_cursor = self.active_cursor_pos();
        if !self.close_active_surfaces_for_command() {
            self.set_status("cannot return to an editor buffer");
            return;
        }
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let (width, height) = self.viewport_size();
        self.with_active_buffer_view_mut(|buffer, view| {
            let line = buffer.clamp_line(line_number.saturating_sub(1));
            view.cursor.cursor = Pos::new(line, buffer.line_first_non_whitespace_col(line));
            view.cursor.reconcile_after_edit(
                buffer,
                width,
                height.saturating_sub(STATUS_BAR_HEIGHT_ROWS),
            );
        });
        self.center_active_cursor_line(height.saturating_sub(STATUS_BAR_HEIGHT_ROWS));
        self.clear_search_highlights();
        self.clear_status();
        if self.active_cursor_pos() != previous_cursor {
            self.start_jump_highlight(height.saturating_sub(STATUS_BAR_HEIGHT_ROWS));
        }
    }

    pub(super) fn reset_command_history_navigation(&mut self) {
        self.command_completion_index = 0;
        self.command_history.nav_index = None;
        self.command_history.draft.clear();
    }

    pub(super) fn detach_command_history_navigation(&mut self) {
        self.command_completion_index = 0;
        if self.command_history.nav_index.is_some() {
            self.command_history.draft = self.command_line.clone();
            self.command_history.nav_index = None;
        }
    }

    pub(super) fn command_history_prev(&mut self) {
        self.command_completion_index = 0;
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
        self.command_completion_index = 0;
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

    pub(super) fn push_command_history(&mut self, command: String) {
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

    fn command_split(&mut self, axis: SplitAxis, path_arg: &str) {
        let source = self.session.active_id();
        let target = if path_arg.is_empty() {
            source
        } else {
            match self.session.open_file(PathBuf::from(path_arg)) {
                Ok(id) => id,
                Err(error) => {
                    self.set_status(format!("open failed: {error}"));
                    return;
                }
            }
        };
        let _ = self.session.activate(source);
        self.split_active_pane(axis);
        let _ = self.session.activate(target);
        self.transient_origin_buffer_id = None;
        self.transient_origin_dir = None;
        self.ensure_buffer_analysis(target);
        self.sync_active_pane_view();
        self.clear_status();
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

    fn resolve_buffer(&self, argument: &str) -> Result<BufferId, String> {
        if argument.is_empty() {
            return Ok(self.session.active_id());
        }
        let buffers: Vec<_> = self
            .session
            .summaries()
            .into_iter()
            .filter(|summary| summary.kind == BufferKind::File)
            .collect();
        let number = argument.strip_prefix('#').unwrap_or(argument);
        if number.bytes().all(|byte| byte.is_ascii_digit()) && !number.is_empty() {
            let buffer_number = number.parse::<u64>().ok();
            return buffers
                .iter()
                .find(|summary| buffer_number == Some(summary.id.get()))
                .map(|summary| summary.id)
                .ok_or_else(|| format!("buffer not found: {argument}"));
        }
        let mut matches: Vec<_> = buffers
            .iter()
            .filter(|summary| {
                summary.display_name == argument
                    || summary.path.as_deref().is_some_and(|path| {
                        path.to_string_lossy() == argument
                            || path.file_name().is_some_and(|name| name == argument)
                    })
            })
            .collect();
        if matches.is_empty() {
            matches = buffers
                .iter()
                .filter(|summary| {
                    summary.display_name.contains(argument)
                        || summary
                            .path
                            .as_deref()
                            .is_some_and(|path| path.to_string_lossy().contains(argument))
                })
                .collect();
        }
        match matches.as_slice() {
            [summary] => Ok(summary.id),
            [] => Err(format!("buffer not found: {argument}")),
            _ => Err(format!(
                "ambiguous buffer name: {argument}; use a path or number from :ls"
            )),
        }
    }

    fn finish_buffer_switch(&mut self, id: BufferId) {
        self.transient_origin_buffer_id = None;
        self.transient_origin_dir = None;
        if self
            .dashboard
            .as_ref()
            .is_some_and(|dashboard| dashboard.buffer_id == id)
        {
            self.dashboard = None;
        }
        self.ensure_buffer_analysis(id);
        self.sync_active_pane_view();
        self.clear_status();
        self.request_redraw();
    }

    fn command_buffer(&mut self, argument: &str) {
        match self.resolve_buffer(argument) {
            Ok(id) => {
                self.sync_active_pane_view();
                let _ = self.session.activate(id);
                self.finish_buffer_switch(id);
            }
            Err(error) => self.set_status(error),
        }
    }

    fn command_delete_buffer(&mut self, argument: &str, force: bool) {
        let id = match self.resolve_buffer(argument) {
            Ok(id) => id,
            Err(error) => {
                self.set_status(error);
                return;
            }
        };
        if self.session.meta(id).is_some_and(|meta| meta.dirty) && !force {
            self.set_status("buffer has unsaved changes; write it or use :bd! to discard them");
            return;
        }
        if argument.is_empty() && self.active_buffer_is_surface() {
            self.close_active_surface_buffer_without_quit();
            return;
        }
        if !self.close_active_surfaces_for_command() {
            self.set_status("cannot return to an editor buffer");
            return;
        }
        let focused = self.active_pane;
        if self
            .undo_tree
            .as_ref()
            .is_some_and(|tree| tree.source_buffer_id == id)
            && let Some(tree) = self.undo_tree.clone()
        {
            self.close_undo_tree_panel(tree);
        }
        if self
            .dashboard
            .as_ref()
            .is_some_and(|dashboard| dashboard.buffer_id == id)
        {
            self.dashboard = None;
        }
        self.sync_active_pane_view();
        while let Some(pane) = self
            .panes
            .iter()
            .find(|pane| pane.buffer_id == id)
            .map(|pane| pane.id)
        {
            let _ = self.activate_pane(pane);
            if self.panes.len() == 1 {
                let replacement = self
                    .session
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.kind == BufferKind::File && summary.id != id)
                    .map(|summary| summary.id);
                let replacement = match replacement {
                    Some(replacement) => {
                        let _ = self.session.activate(replacement);
                        replacement
                    }
                    None => self.session.open_unnamed_buffer(),
                };
                self.finish_buffer_switch(replacement);
                break;
            }
            self.close_active_split();
        }
        let _ = self.activate_pane(focused);
        if self.session.close_buffer(id) {
            self.close_lsp_document(id);
            self.views.remove(&id);
            self.undo_history_load_attempted.remove(&id);
            self.finish_buffer_switch(self.session.active_id());
        } else {
            self.set_status("cannot delete buffer");
        }
    }

    pub(super) fn command_buffer_cycle_next(&mut self) {
        let count = self.session.summaries().len();
        if count <= 1 {
            self.set_status("only one buffer");
            return;
        }

        self.sync_active_pane_view();
        if let Some(id) = self.session.switch_next_mru() {
            self.finish_buffer_switch(id);
        }
    }

    pub(super) fn command_buffer_cycle_prev(&mut self) {
        let count = self.session.summaries().len();
        if count <= 1 {
            self.set_status("only one buffer");
            return;
        }

        self.sync_active_pane_view();
        if let Some(id) = self.session.switch_prev_mru() {
            self.finish_buffer_switch(id);
        }
    }

    pub(super) fn command_list_buffers(&mut self) {
        self.sync_active_pane_view();
        let summaries = self.session.summaries();
        if summaries.is_empty() {
            self.set_status("no buffers");
            return;
        }

        let mut lines = vec![format!("Buffers ({})", summaries.len())];
        for summary in summaries {
            let file_icon = if self.icons_enabled {
                format!(
                    "{} ",
                    icons::file_icon(
                        summary
                            .path
                            .as_deref()
                            .unwrap_or_else(|| Path::new(&summary.display_name))
                    )
                )
            } else {
                String::new()
            };
            let panes: Vec<_> = self
                .panes
                .iter()
                .filter(|pane| pane.buffer_id == summary.id)
                .map(|pane| {
                    let zoom = if !self.pane_zoomed || pane.id != self.active_pane {
                        ""
                    } else if self.icons_enabled {
                        icons::ZOOM
                    } else {
                        "^"
                    };
                    format!("{}{zoom}", pane.id.0.saturating_add(1))
                })
                .collect();
            let mut details = if !panes.is_empty() {
                format!("[{}]", panes.join(","))
            } else if summary.kind == BufferKind::Ui {
                "[panel]".to_string()
            } else {
                "[hidden]".to_string()
            };
            for (enabled, label) in [
                (summary.dirty, "+"),
                (summary.is_new_file, "new file"),
                (summary.external_changed, "!"),
            ] {
                if enabled {
                    details.push(' ');
                    details.push_str(label);
                }
            }
            lines.push(format!(
                "{} #{} {file_icon}{} {details}",
                if summary.is_active { ">" } else { " " },
                summary.id.get(),
                summary.display_name
            ));
        }
        self.set_status(lines.join("\n"));
    }

    fn command_quit_all(&mut self, argument: &str, force: bool) {
        if !argument.is_empty() {
            self.set_status(if force { "usage: qa!" } else { "usage: qa" });
        } else if !force && self.has_unsaved_changes() {
            self.set_status(format!(
                "{} (use :qa! to quit)",
                self.unsaved_changes_message()
            ));
        } else {
            self.should_quit = true;
        }
    }

    fn command_write_all(&mut self, argument: &str, quit: bool) {
        if !argument.is_empty() {
            self.set_status(if quit { "usage: wqa" } else { "usage: wa" });
            return;
        }

        self.sync_active_pane_view();
        let original_active = self.session.active_id();
        let summaries = self.session.summaries();
        let mut written = 0;
        let mut all_saved = true;
        let mut messages = Vec::new();
        for summary in summaries
            .iter()
            .filter(|summary| summary.kind == BufferKind::File && summary.dirty)
        {
            self.session.activate(summary.id);
            if self.write_current_file("") {
                written += 1;
            } else {
                all_saved = false;
            }
            if let Some(message) = self.status_msg.as_deref() {
                messages.push(format!("{}: {message}", summary.display_name));
            }
        }

        // Temporary activation during saving must preserve the user's buffer order.
        for summary in summaries.iter().rev() {
            self.session.activate(summary.id);
        }
        self.session.activate(original_active);
        self.sync_active_pane_view();

        if quit && all_saved && !self.has_unsaved_changes() {
            self.should_quit = true;
        } else if quit && messages.is_empty() && self.has_unsaved_changes() {
            messages.push(self.unsaved_changes_message());
        }
        if messages.is_empty() {
            self.set_status(match written {
                0 => "no modified file buffers".to_string(),
                1 => "written 1 buffer".to_string(),
                _ => format!("written {written} buffers"),
            });
        } else {
            self.set_status(messages.join("\n"));
        }
    }

    pub(super) fn write_current_file(&mut self, path_arg: &str) -> bool {
        self.save_confirmation = None;
        if self.explorer_is_active() {
            if !path_arg.is_empty() {
                self.set_status("explorer writes do not accept a file name");
                return false;
            }
            return self.write_explorer_directory();
        }

        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return false;
        }

        let before = self.capture_active_undo_checkpoint();
        let (viewport_width_cells, viewport_height_rows) = self.viewport_size();
        let text_vh = viewport_height_rows.saturating_sub(STATUS_BAR_HEIGHT_ROWS);

        self.sync_active_lsp_before_save();

        let previous_path = self.session.active_meta().path.clone();
        let saved = if path_arg.is_empty() {
            self.session.save_active()
        } else {
            self.session.save_active_as(path_arg)
        };
        if let Err(error) = saved {
            self.log_event("write_failed", serde_json::Value::Null);
            self.set_status(format!("write failed: {error}"));
            return false;
        }
        self.log_event("file_written", serde_json::Value::Null);
        if self.session.active_meta().path != previous_path {
            self.reset_active_render_caches();
            self.ensure_active_lsp_client();
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
            (Ok(()), Ok(())) => self.confirm_active_save(),
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
        let mut changed = false;

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
            let formatted = std::fs::read_to_string(path)
                .map_err(|error| format!("failed to read formatter output: {error}"))?;
            self.session.refresh_active_disk_stamp();
            changed |= replace_text_preserving_marks(self.session.active_buffer_mut(), &formatted);
        }

        let passes = SavePasses::for_path(path.as_deref(), self.save);
        changed |= apply_save_format_passes(self.session.active_buffer_mut(), passes);
        if !changed {
            return Ok(false);
        }

        self.reconcile_active_after_buffer_rewrite(viewport_width_cells, text_vh);
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
        replace_text_preserving_marks(self.session.active_buffer_mut(), &text);
        self.reconcile_active_after_buffer_rewrite(viewport_width_cells, text_vh);
        self.session.mark_active_clean();
        let active_id = self.session.active_id();
        self.clear_buffer_undo_history(active_id);
        Ok(())
    }

    fn reconcile_active_after_buffer_rewrite(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
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
        let indent_size = self.active_indent_width();
        let smart_insert = smart_open_line_insert(
            self.session.active_buffer(),
            language,
            line,
            above,
            indent_size,
        );
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

    fn has_unsaved_changes(&self) -> bool {
        self.session.any_dirty() || self.explorer_has_unsaved_changes()
    }

    pub(super) fn unsaved_changes_message(&self) -> String {
        let dirty_explorer = self
            .explorer
            .as_ref()
            .filter(|_| self.explorer_has_unsaved_changes())
            .map(|explorer| explorer.buffer_id);
        let dirty: Vec<redox_core::BufferSummary> = self
            .session
            .summaries()
            .into_iter()
            .filter(|summary| summary.dirty || Some(summary.id) == dirty_explorer)
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
        SyntaxLanguage::C | SyntaxLanguage::Cpp => Some(SaveFormatter::ClangFormat),
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
        SaveFormatter::ClangFormat => executable_on_path("clang-format"),
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
        SaveFormatter::ClangFormat => run_command_status(
            Command::new("clang-format")
                .current_dir(launch_dir)
                .arg("-i")
                .arg(path),
            "clang-format",
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

/// Which whitespace passes a save may apply to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SavePasses {
    expand_tabs: bool,
    trim_trailing_whitespace: bool,
}

impl SavePasses {
    /// Skip passes where the whitespace itself carries meaning: gofmt and Make
    /// require tabs, Markdown uses trailing spaces as hard breaks, a trailing tab in
    /// TSV is an empty column, and patch context lines may consist of whitespace.
    fn for_path(path: Option<&Path>, config: crate::config::SaveConfig) -> Self {
        let file_name = path
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let extension = path
            .and_then(Path::extension)
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let language = language_for_path(path);
        let data_format = matches!(extension.as_str(), "tsv" | "diff" | "patch");
        let tabs_significant = data_format
            || language == Some(SyntaxLanguage::Go)
            || matches!(file_name, "Makefile" | "makefile" | "GNUmakefile")
            || matches!(extension.as_str(), "mk" | "mak");
        let trailing_whitespace_significant =
            data_format || language == Some(SyntaxLanguage::Markdown);
        Self {
            expand_tabs: config.expand_tabs && !tabs_significant,
            trim_trailing_whitespace: config.trim_trailing_whitespace
                && !trailing_whitespace_significant,
        }
    }
}

/// Apply whitespace passes as per-line edits so marks stay on their lines.
fn apply_save_format_passes(buffer: &mut TextBuffer, passes: SavePasses) -> bool {
    let mut changed = false;
    if passes.expand_tabs {
        let tab_width = crate::indentation::detect_buffer_width(buffer)
            .unwrap_or(crate::indentation::DEFAULT_WIDTH);
        for line in 0..buffer.len_lines() {
            let indent: String = buffer
                .line_slice(line)
                .chars()
                .take_while(|character| matches!(character, ' ' | '\t'))
                .collect();
            if indent.contains('\t') {
                let expanded = expand_indent_tabs(&indent, tab_width);
                changed |= buffer.replace_line_indent(line, &expanded).is_some();
            }
        }
    }
    if passes.trim_trailing_whitespace {
        changed |= buffer.trim_trailing_whitespace();
    }
    changed
}

fn expand_indent_tabs(indent: &str, tab_width: usize) -> String {
    let mut expanded = String::with_capacity(indent.len());
    for character in indent.chars() {
        if character == '\t' {
            let spaces = tab_width - (expanded.len() % tab_width);
            expanded.extend(std::iter::repeat_n(' ', spaces));
        } else {
            expanded.push(character);
        }
    }
    expanded
}

/// Replace a buffer's text with one edit spanning only the changed region, so marks
/// outside it keep their positions.
fn replace_text_preserving_marks(buffer: &mut TextBuffer, text: &str) -> bool {
    let Some(diff) = TextDiff::between(buffer, &TextBuffer::from_text(text)) else {
        return false;
    };
    buffer.apply_edit(diff.forward_edit());
    true
}
