use std::path::Path;

use minui::Event;
use redox_core::{BufferId, BufferKind, Pos};

use super::{EditorMode, EditorState};
use crate::input::{InputAction, macro_key_label};
use crate::storage::{self, SessionFile, SessionSnapshot};

pub(crate) const DASHBOARD_ITEMS: [(char, &str); 6] = [
    ('r', "Restore previous session"),
    ('f', "Finder"),
    ('e', "File explorer"),
    ('n', "New file"),
    ('c', "Configuration"),
    ('q', "Quit"),
];

#[derive(Debug)]
pub(super) struct DashboardState {
    pub buffer_id: BufferId,
    return_to_buffer_id: Option<BufferId>,
    selected: usize,
}

impl EditorState {
    pub(crate) fn open_dashboard(&mut self) {
        if self.active_buffer_is_surface() && self.session.active_meta().dirty {
            self.set_status(
                "write or discard pending surface changes before opening the dashboard",
            );
            return;
        }
        if !self.close_active_surfaces_for_command() {
            self.set_status("cannot return to an editor buffer");
            return;
        }
        if let Some(dashboard) = &mut self.dashboard {
            dashboard.selected = 0;
            let _ = self.session.activate(dashboard.buffer_id);
        } else {
            self.sync_active_pane_view();
            let previous_id = self.session.active_id();
            let buffer_id = if self.is_empty_unnamed_startup_buffer(previous_id) {
                previous_id
            } else {
                self.session.open_unnamed_buffer()
            };
            self.views.entry(buffer_id).or_default();
            self.dashboard = Some(DashboardState {
                buffer_id,
                return_to_buffer_id: (buffer_id != previous_id).then_some(previous_id),
                selected: 0,
            });
        }
        self.mode = EditorMode::Normal;
        self.input.reset_prefixes();
        self.clear_status();
        self.request_redraw();
    }

    fn close_dashboard(&mut self) {
        let Some(dashboard) = self.dashboard.take() else {
            return;
        };
        if let Some(previous_id) = dashboard.return_to_buffer_id
            && self.session.activate(previous_id)
        {
            self.close_inactive_empty_unnamed_startup_buffer(dashboard.buffer_id);
            self.ensure_buffer_analysis(previous_id);
            self.sync_active_pane_view();
        }
        self.clear_status();
    }

    pub(crate) fn dashboard_selection(&self) -> Option<usize> {
        self.dashboard_selection_for_buffer(self.session.active_id())
    }

    pub(crate) fn dashboard_selection_for_buffer(&self, buffer_id: BufferId) -> Option<usize> {
        self.dashboard.as_ref().and_then(|dashboard| {
            (dashboard.buffer_id == buffer_id
                && self.is_empty_unnamed_startup_buffer(dashboard.buffer_id))
            .then_some(dashboard.selected)
        })
    }

    pub(crate) fn handle_dashboard_event(&mut self, event: &Event) -> bool {
        let Some(selected) = self.dashboard_selection() else {
            return false;
        };
        if self.mode != EditorMode::Normal {
            return false;
        }
        let key = macro_key_label(event);
        let next = match key.as_str() {
            "j" | "<Down>" => Some((selected + 1).min(DASHBOARD_ITEMS.len() - 1)),
            "k" | "<Up>" => Some(selected.saturating_sub(1)),
            _ => None,
        };
        if let Some(next) = next {
            if let Some(dashboard) = &mut self.dashboard {
                dashboard.selected = next;
            }
        } else {
            let hotkey = if key == "<Enter>" {
                Some(DASHBOARD_ITEMS[selected].0)
            } else {
                DASHBOARD_ITEMS
                    .iter()
                    .find_map(|(hotkey, _)| (key == hotkey.to_string()).then_some(*hotkey))
            };
            match hotkey {
                Some('r') => {
                    let path = storage::session_path(self.session.launch_dir());
                    self.restore_previous_session(&path);
                }
                Some('f') => self.open_finder(),
                Some('e') => self.command_open_explorer(),
                Some('n') => {
                    self.dashboard = None;
                    self.clear_status();
                }
                Some('c') => self.request_config_open(),
                Some('q') => self.execute_configured_command("q".to_string()),
                _ if key == "<Esc>" => self.close_dashboard(),
                _ if key == ":" => {
                    self.close_dashboard();
                    let (width, height) = self.viewport_size();
                    self.apply_input(InputAction::EnterCommand, width, height);
                }
                _ => {}
            }
        }
        self.request_redraw();
        true
    }

    pub(crate) fn save_previous_session(&self, path: &Path) -> std::io::Result<()> {
        let files = self
            .session
            .summaries()
            .into_iter()
            .filter_map(|summary| {
                let path = summary.path.filter(|path| path.is_file())?;
                (summary.kind == BufferKind::File).then(|| SessionFile {
                    path,
                    cursor: self
                        .views
                        .get(&summary.id)
                        .map(|view| view.cursor.cursor)
                        .unwrap_or_else(Pos::zero),
                })
            })
            .collect();
        storage::save_session(
            path,
            &SessionSnapshot {
                directory: self.session.launch_dir().to_path_buf(),
                files,
            },
        )
    }

    fn restore_previous_session(&mut self, path: &Path) {
        let snapshot = match storage::load_session(path) {
            Ok(Some(snapshot)) if snapshot.directory == self.session.launch_dir() => snapshot,
            Ok(_) => {
                self.set_status("no previous session for this directory");
                return;
            }
            Err(error) => {
                self.set_status(format!("session restore failed: {error}"));
                return;
            }
        };
        let previous = self.session.active_id();
        let mut restored = 0;
        let mut skipped = 0;
        for file in snapshot.files.iter().rev() {
            if !file.path.is_file() {
                skipped += 1;
                continue;
            }
            let Ok(buffer_id) = self.session.open_file(&file.path) else {
                skipped += 1;
                continue;
            };
            if self
                .session
                .ensure_buffer_loaded_through_line(buffer_id, file.cursor.line, usize::MAX)
                .is_err()
            {
                self.session.close_buffer(buffer_id);
                skipped += 1;
                continue;
            }
            let cursor = self.session.active_buffer().clamp_pos(file.cursor);
            self.views.entry(buffer_id).or_default().cursor.cursor = cursor;
            self.ensure_buffer_analysis(buffer_id);
            restored += 1;
        }
        if restored > 0 {
            self.close_inactive_empty_unnamed_startup_buffer(previous);
            self.sync_active_pane_view();
        }
        self.set_status(if skipped == 0 {
            format!("restored {restored} files")
        } else {
            format!("restored {restored} files; skipped {skipped} unavailable files")
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_core::EditorSession;

    #[test]
    fn sessions_round_trip_files_focus_and_cursors_without_erasing_on_empty_exit() {
        let _lock = super::super::global_test_state_lock().lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        let missing = directory.path().join("missing.txt");
        let snapshot_path = directory.path().join("session.json");
        for path in [&first, &second, &missing] {
            std::fs::write(path, "alpha\nbeta\ngamma\n").unwrap();
        }
        let mut previous = EditorState::new(EditorSession::open_initial_file(&first).unwrap());
        previous
            .views
            .get_mut(&previous.session.active_id())
            .unwrap()
            .cursor
            .cursor = Pos::new(2, 2);
        previous.command_edit(&missing.to_string_lossy());
        previous.command_edit(&second.to_string_lossy());
        previous
            .views
            .get_mut(&previous.session.active_id())
            .unwrap()
            .cursor
            .cursor = Pos::new(1, 1);
        previous.command_open_about();
        previous.save_previous_session(&snapshot_path).unwrap();
        std::fs::remove_file(&missing).unwrap();

        let mut restored = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        restored.open_dashboard();
        let saved = std::fs::read(&snapshot_path).unwrap();
        restored.save_previous_session(&snapshot_path).unwrap();
        assert_eq!(std::fs::read(&snapshot_path).unwrap(), saved);
        restored.restore_previous_session(&snapshot_path);
        assert!(restored.dashboard_selection().is_none());
        assert_eq!(restored.session.summaries().len(), 2);
        assert_eq!(
            restored.session.active_meta().path,
            Some(second.canonicalize().unwrap())
        );
        assert_eq!(restored.active_cursor_pos(), Pos::new(1, 1));
        restored.command_buffer_cycle_next();
        assert_eq!(
            restored.session.active_meta().path,
            Some(first.canonicalize().unwrap())
        );
        assert_eq!(restored.active_cursor_pos(), Pos::new(2, 2));

        let mut empty = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        empty.open_dashboard();
        empty.restore_previous_session(&directory.path().join("absent.json"));
        assert_eq!(empty.dashboard_selection(), Some(0));
        assert!(
            empty
                .status_msg
                .as_deref()
                .unwrap()
                .contains("no previous session")
        );
        std::fs::write(&snapshot_path, "broken json").unwrap();
        empty.restore_previous_session(&snapshot_path);
        assert_eq!(empty.dashboard_selection(), Some(0));
        assert!(
            empty
                .status_msg
                .as_deref()
                .unwrap()
                .contains("session restore failed")
        );
    }
}
