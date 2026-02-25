use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use redox_core::{BufferId, BufferKind, Pos, TextBuffer};

use super::{EditorMode, EditorState};
use crate::ui::STATUS_BAR_HEIGHT_ROWS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExplorerEntry {
    name: String,
    is_dir: bool,
    is_parent: bool,
}

#[derive(Debug, Clone)]
pub struct ExplorerPopup {
    pub title: String,
    pub dir_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct ExplorerState {
    pub(super) buffer_id: BufferId,
    pub(super) dir_path: PathBuf,
    pub(super) original_entries: Vec<ExplorerEntry>,
    pub(super) return_to_buffer_id: BufferId,
}

impl EditorState {
    pub fn explorer_popup(&self) -> Option<ExplorerPopup> {
        let explorer = self.explorer.as_ref()?;
        if explorer.buffer_id != self.session.active_id() {
            return None;
        }

        Some(ExplorerPopup {
            title: format!("{}", explorer.dir_path.display()),
            dir_path: explorer.dir_path.clone(),
        })
    }

    pub fn explorer_background_buffer_id(&self) -> Option<BufferId> {
        let explorer = self.explorer.as_ref()?;
        if explorer.buffer_id != self.session.active_id() {
            return None;
        }
        self.session
            .buffer(explorer.return_to_buffer_id)
            .map(|_| explorer.return_to_buffer_id)
    }

    pub(super) fn command_open_explorer(&mut self) {
        if self.explorer_is_active() {
            let _ = self.close_active_surface_buffer();
            self.mode = EditorMode::Normal;
            self.clear_status();
            return;
        }

        match self.open_explorer_buffer() {
            Ok(()) => {
                self.mode = EditorMode::Normal;
                self.clear_status();
            }
            Err(e) => {
                self.set_status(format!("explorer open failed: {e}"));
            }
        }
    }

    pub(super) fn open_explorer_buffer(&mut self) -> anyhow::Result<()> {
        let return_to = self.session.active_id();
        let dir_path = self.explorer_target_directory()?;
        let entries = list_explorer_entries(&dir_path)?;
        let preferred_name = self.session.active_meta().path.as_ref().and_then(|path| {
            let parent = path.parent().unwrap_or(path.as_path());
            if parent == dir_path {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_string())
            } else {
                None
            }
        });
        let initial_cursor_line = preferred_name
            .as_ref()
            .and_then(|name| entries.iter().position(|entry| entry.name == *name))
            .unwrap_or(0);
        let text = explorer_entries_to_text(&entries);

        let title = format!("[explorer] {}", dir_path.display());
        let explorer_id = self.session.open_ui_buffer(title, &text);
        self.session.mark_active_clean();
        let view = self.views.entry(explorer_id).or_default();
        view.cursor.cursor = Pos::new(initial_cursor_line, 0);
        view.cursor.follow.top_margin_rows = 0;
        view.cursor.follow.bottom_margin_rows = 0;
        let buffer = self
            .session
            .buffer(explorer_id)
            .expect("explorer buffer must exist");
        view.cursor.reconcile_after_edit(
            buffer,
            self.viewport_width_cells,
            self.viewport_height_rows
                .saturating_sub(STATUS_BAR_HEIGHT_ROWS),
        );
        view.grapheme_cache.clear();

        self.explorer = Some(ExplorerState {
            buffer_id: explorer_id,
            dir_path,
            original_entries: entries,
            return_to_buffer_id: return_to,
        });
        Ok(())
    }

    pub(super) fn explorer_target_directory(&self) -> anyhow::Result<PathBuf> {
        let active_meta = self.session.active_meta();
        if let Some(path) = &active_meta.path {
            return Ok(path.parent().unwrap_or(path.as_path()).to_path_buf());
        }

        if let Some(explorer) = &self.explorer {
            return Ok(explorer.dir_path.clone());
        }

        Ok(std::env::current_dir()?)
    }

    pub(super) fn explorer_is_active(&self) -> bool {
        self.explorer
            .as_ref()
            .is_some_and(|explorer| explorer.buffer_id == self.session.active_id())
    }

    pub(super) fn active_buffer_is_surface(&self) -> bool {
        self.session.active_meta().kind == BufferKind::Ui
    }

    pub(super) fn close_active_surface_buffer(&mut self) -> bool {
        let active_id = self.session.active_id();
        let is_explorer = self
            .explorer
            .as_ref()
            .is_some_and(|explorer| explorer.buffer_id == active_id);

        let return_to = self.explorer.as_ref().and_then(|explorer| {
            (explorer.buffer_id == active_id).then_some(explorer.return_to_buffer_id)
        });

        if !self.session.close_active_buffer() {
            return false;
        }
        self.views.remove(&active_id);

        if is_explorer {
            self.explorer = None;
            if let Some(target) = return_to {
                let _ = self.session.activate(target);
            }
        }

        true
    }

    pub(super) fn surface_open_selected(&mut self) {
        if !self.explorer_is_active() {
            return;
        }

        let Some(explorer) = self.explorer.clone() else {
            return;
        };

        let active_id = self.session.active_id();
        let cursor_line = self
            .views
            .get(&active_id)
            .map(|view| view.cursor.cursor.line)
            .unwrap_or(0);

        let parsed = match parse_explorer_entries(&self.session.active_buffer().to_string()) {
            Ok(entries) => entries,
            Err(e) => {
                self.set_status(format!("explorer parse error: {e}"));
                return;
            }
        };
        if parsed.is_empty() {
            return;
        }

        let idx = cursor_line.min(parsed.len().saturating_sub(1));
        let entry = parsed[idx].clone();

        if entry.is_parent {
            self.surface_go_parent();
            return;
        }

        if entry.is_dir {
            let next_dir = explorer.dir_path.join(entry.name);
            if let Err(e) = self.refresh_explorer_directory(next_dir) {
                self.set_status(format!("explorer open failed: {e}"));
            }
            return;
        }

        let file_path = explorer.dir_path.join(entry.name);
        match self.session.open_file(&file_path) {
            Ok(file_id) => {
                let _ = self.views.entry(file_id).or_default();
                let _ = self.session.close_buffer(explorer.buffer_id);
                self.views.remove(&explorer.buffer_id);
                self.explorer = None;
                self.mode = EditorMode::Normal;
                self.clear_status();
            }
            Err(e) => {
                self.set_status(format!("open failed: {e}"));
            }
        }
    }

    pub(super) fn surface_go_parent(&mut self) {
        if !self.explorer_is_active() {
            return;
        }
        let Some(explorer) = self.explorer.clone() else {
            return;
        };

        let parent = explorer
            .dir_path
            .parent()
            .unwrap_or(explorer.dir_path.as_path())
            .to_path_buf();
        if let Err(e) = self.refresh_explorer_directory(parent) {
            self.set_status(format!("explorer open failed: {e}"));
        }
    }

    pub(super) fn refresh_explorer_directory(&mut self, dir_path: PathBuf) -> anyhow::Result<()> {
        let entries = list_explorer_entries(&dir_path)?;
        let text = explorer_entries_to_text(&entries);

        let Some(mut explorer) = self.explorer.clone() else {
            return Ok(());
        };
        let explorer_id = explorer.buffer_id;

        if let Some(buffer) = self.session.buffer_mut(explorer_id) {
            *buffer = TextBuffer::from_str(&text);
        }
        explorer.dir_path = dir_path;
        explorer.original_entries = entries;
        self.explorer = Some(explorer);

        if let Some(view) = self.views.get_mut(&explorer_id) {
            let buffer = self
                .session
                .buffer(explorer_id)
                .expect("explorer buffer must exist");
            view.cursor.cursor = Pos::zero();
            view.cursor.follow.top_margin_rows = 0;
            view.cursor.follow.bottom_margin_rows = 0;
            view.cursor.reconcile_after_edit(
                buffer,
                self.viewport_width_cells,
                self.viewport_height_rows
                    .saturating_sub(STATUS_BAR_HEIGHT_ROWS),
            );
            view.grapheme_cache.clear();
        }

        self.session.mark_active_clean();
        self.clear_status();
        Ok(())
    }

    pub(super) fn write_explorer_directory(&mut self) -> bool {
        let Some(mut explorer) = self.explorer.clone() else {
            self.set_status("explorer state missing");
            return false;
        };

        if explorer.buffer_id != self.session.active_id() {
            self.set_status("active buffer is not explorer");
            return false;
        }

        let current_text = self.session.active_buffer().to_string();
        let desired_entries = match parse_explorer_entries(&current_text) {
            Ok(entries) => entries,
            Err(e) => {
                self.set_status(format!("explorer parse error: {e}"));
                return false;
            }
        };

        if let Err(e) = apply_explorer_changes(
            &explorer.dir_path,
            &explorer.original_entries,
            &desired_entries,
        ) {
            self.set_status(format!("explorer write failed: {e}"));
            return false;
        }

        let refreshed_entries = match list_explorer_entries(&explorer.dir_path) {
            Ok(entries) => entries,
            Err(e) => {
                self.set_status(format!("explorer refresh failed: {e}"));
                return false;
            }
        };

        let refreshed_text = explorer_entries_to_text(&refreshed_entries);
        if let Some(buffer) = self.session.buffer_mut(explorer.buffer_id) {
            *buffer = TextBuffer::from_str(&refreshed_text);
        }

        if let Some(view) = self.views.get_mut(&explorer.buffer_id) {
            let buffer = self
                .session
                .buffer(explorer.buffer_id)
                .expect("explorer buffer must exist");
            view.cursor.cursor = Pos::zero();
            view.cursor.follow.top_margin_rows = 0;
            view.cursor.follow.bottom_margin_rows = 0;
            view.cursor.reconcile_after_edit(
                buffer,
                self.viewport_width_cells,
                self.viewport_height_rows
                    .saturating_sub(STATUS_BAR_HEIGHT_ROWS),
            );
            view.grapheme_cache.clear();
        }

        explorer.original_entries = refreshed_entries;
        self.explorer = Some(explorer);
        self.session.mark_active_clean();
        self.set_status("written");
        true
    }
}

fn list_explorer_entries(dir: &Path) -> anyhow::Result<Vec<ExplorerEntry>> {
    let mut entries = Vec::new();

    entries.push(ExplorerEntry {
        name: "..".to_string(),
        is_dir: true,
        is_parent: true,
    });

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let is_dir = path.is_dir();
        let name = entry.file_name().to_string_lossy().to_string();
        entries.push(ExplorerEntry {
            name,
            is_dir,
            is_parent: false,
        });
    }

    entries.sort_by(|a, b| {
        (!a.is_dir)
            .cmp(&!b.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

fn explorer_entries_to_text(entries: &[ExplorerEntry]) -> String {
    let mut out = String::new();
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&entry.name);
        if entry.is_dir && !entry.is_parent {
            out.push('/');
        }
    }
    out
}

fn parse_explorer_entries(text: &str) -> anyhow::Result<Vec<ExplorerEntry>> {
    let mut out = Vec::new();

    for (idx, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let (name, is_dir) = if let Some(stripped) = line.strip_suffix('/') {
            (stripped, true)
        } else {
            (line, false)
        };

        if name.is_empty() {
            anyhow::bail!("line {}: empty name", idx + 1);
        }
        if name.contains('/') {
            anyhow::bail!("line {}: names cannot contain '/'", idx + 1);
        }
        let is_parent = name == "..";

        out.push(ExplorerEntry {
            name: name.to_string(),
            is_dir: is_dir || is_parent,
            is_parent,
        });
    }

    Ok(out)
}

fn apply_explorer_changes(
    dir_path: &Path,
    old_entries: &[ExplorerEntry],
    new_entries: &[ExplorerEntry],
) -> anyhow::Result<()> {
    if let Some(old_first) = old_entries.first()
        && old_first.is_parent
    {
        let Some(new_first) = new_entries.first() else {
            anyhow::bail!("explorer requires '..' entry");
        };
        if !new_first.is_parent || new_first.name != ".." {
            anyhow::bail!("cannot edit or remove '..' entry");
        }
    }

    let old_entries: Vec<ExplorerEntry> = old_entries
        .iter()
        .filter(|entry| !entry.is_parent)
        .cloned()
        .collect();
    let new_entries: Vec<ExplorerEntry> = new_entries
        .iter()
        .filter(|entry| !entry.is_parent)
        .cloned()
        .collect();

    let overlap = old_entries.len().min(new_entries.len());

    for i in 0..overlap {
        let old = &old_entries[i];
        let new = &new_entries[i];
        if old.name == new.name {
            continue;
        }

        // Check if target already exists to prevent collisions
        let old_path = dir_path.join(&old.name);
        let new_path = dir_path.join(&new.name);
        if new_path.exists() && !old_path.exists() {
            anyhow::bail!("rename target '{}' already exists", new.name);
        }
        fs::rename(&old_path, &new_path)?;
    }

    if old_entries.len() > new_entries.len() {
        for old in &old_entries[new_entries.len()..] {
            let path = dir_path.join(&old.name);
            if old.is_dir {
                if let Err(e) = fs::remove_dir(&path) {
                    if e.kind() == ErrorKind::DirectoryNotEmpty {
                        anyhow::bail!(
                            "cannot remove directory '{}': directory is not empty",
                            old.name
                        );
                    }
                    return Err(e.into());
                }
            } else {
                fs::remove_file(&path)?;
            }
        }
    }

    if new_entries.len() > old_entries.len() {
        for new in &new_entries[old_entries.len()..] {
            let path = dir_path.join(&new.name);
            if new.is_dir {
                fs::create_dir(&path)?;
            } else {
                let _ = fs::File::create(&path)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn deleting_non_empty_directory_returns_clear_error() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("redox_explorer_non_empty_{nanos}"));
        let doomed = root.join("doomed");

        fs::create_dir_all(&doomed).expect("failed to create directory fixture");
        fs::write(doomed.join("child.txt"), "x").expect("failed to write child fixture");

        let old_entries = vec![ExplorerEntry {
            name: "doomed".to_string(),
            is_dir: true,
            is_parent: false,
        }];
        let new_entries = Vec::new();

        let err = apply_explorer_changes(&root, &old_entries, &new_entries)
            .expect_err("expected non-empty directory delete to fail");
        let msg = err.to_string();
        assert!(msg.contains("directory is not empty"));
        assert!(msg.contains("doomed"));

        let _ = fs::remove_dir_all(root);
    }
}
