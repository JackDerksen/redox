use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use redox_core::{BufferId, Pos, TextBuffer};

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
    pub fn open_explorer_at_path(&mut self, dir_path: PathBuf) -> anyhow::Result<()> {
        if self.active_buffer_is_surface() {
            let _ = self.close_active_surface_buffer();
        }
        self.open_explorer_buffer_with_dir(dir_path)
    }

    pub fn explorer_popup(&self) -> Option<ExplorerPopup> {
        let explorer = self.explorer.as_ref()?;
        if explorer.buffer_id != self.session.active_id() {
            return None;
        }

        Some(ExplorerPopup {
            title: format_explorer_dir_path(&explorer.dir_path),
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

    pub fn explorer_background_is_placeholder_blank(&self) -> bool {
        let Some(explorer) = self.explorer.as_ref() else {
            return false;
        };
        if explorer.buffer_id != self.session.active_id() {
            return false;
        }
        self.is_empty_unnamed_startup_buffer(explorer.return_to_buffer_id)
    }

    pub(super) fn command_open_explorer(&mut self) {
        if self.explorer_is_active() {
            let _ = self.close_active_surface_buffer();
            self.mode = EditorMode::Normal;
            self.clear_status();
            return;
        }

        if self.active_buffer_is_surface() {
            let _ = self.close_active_surface_buffer();
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
        let dir_path = self.explorer_target_directory()?;
        self.open_explorer_buffer_with_dir(dir_path)
    }

    fn open_explorer_buffer_with_dir(&mut self, dir_path: PathBuf) -> anyhow::Result<()> {
        self.explorer_delete_confirmation_token = None;
        let dir_path = std::fs::canonicalize(&dir_path).unwrap_or(dir_path);
        let return_to = self.session.active_id();
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

        let title = format!("[explorer] {}", format_explorer_dir_path(&dir_path));
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
        view.invalidate_render_caches();

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

        let previous_dir_name = explorer
            .dir_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string());
        let parent = explorer
            .dir_path
            .parent()
            .unwrap_or(explorer.dir_path.as_path())
            .to_path_buf();
        if let Err(e) = self.refresh_explorer_directory_with_selection(parent, previous_dir_name) {
            self.set_status(format!("explorer open failed: {e}"));
        }
    }

    pub(super) fn refresh_explorer_directory(&mut self, dir_path: PathBuf) -> anyhow::Result<()> {
        self.refresh_explorer_directory_with_selection(dir_path, None)
    }

    fn refresh_explorer_directory_with_selection(
        &mut self,
        dir_path: PathBuf,
        preferred_entry_name: Option<String>,
    ) -> anyhow::Result<()> {
        self.explorer_delete_confirmation_token = None;
        let dir_path = std::fs::canonicalize(&dir_path).unwrap_or(dir_path);
        let entries = list_explorer_entries(&dir_path)?;
        let initial_cursor_line = preferred_entry_name
            .as_ref()
            .and_then(|name| entries.iter().position(|entry| entry.name == *name))
            .unwrap_or(0);
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
            view.cursor.cursor = Pos::new(initial_cursor_line, 0);
            view.cursor.follow.top_margin_rows = 0;
            view.cursor.follow.bottom_margin_rows = 0;
            view.cursor.reconcile_after_edit(
                buffer,
                self.viewport_width_cells,
                self.viewport_height_rows
                    .saturating_sub(STATUS_BAR_HEIGHT_ROWS),
            );
            view.invalidate_render_caches();
        }

        self.session.mark_active_clean();
        self.clear_status();
        Ok(())
    }

    pub(super) fn write_explorer_directory(&mut self) -> bool {
        self.write_explorer_directory_internal(false)
    }

    pub(super) fn confirm_pending_explorer_delete(&mut self) -> bool {
        if self.explorer_delete_confirmation_token.is_none() {
            return false;
        }
        self.write_explorer_directory_internal(true)
    }

    fn write_explorer_directory_internal(&mut self, confirm_delete: bool) -> bool {
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
                self.explorer_delete_confirmation_token = None;
                self.set_status(format!("explorer parse error: {e}"));
                return false;
            }
        };
        let delete_count =
            pending_explorer_delete_count(&explorer.original_entries, &desired_entries);
        if delete_count > 0 {
            let token = explorer_delete_confirmation_token(&explorer.dir_path, &current_text);
            if !confirm_delete {
                self.explorer_delete_confirmation_token = Some(token);
                let noun = if delete_count == 1 {
                    "entry"
                } else {
                    "entries"
                };
                self.set_status(format!(
                    "confirm deletion of {delete_count} {noun}: press y"
                ));
                return false;
            }
            if self.explorer_delete_confirmation_token.as_deref() != Some(&token) {
                self.set_status("delete target changed; run :w again");
                return false;
            }
        } else {
            self.explorer_delete_confirmation_token = None;
            if confirm_delete {
                return false;
            }
        }

        if let Err(e) = apply_explorer_changes(
            &explorer.dir_path,
            &explorer.original_entries,
            &desired_entries,
        ) {
            self.explorer_delete_confirmation_token = None;
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
        let previous_cursor_line = self
            .views
            .get(&explorer.buffer_id)
            .map(|view| view.cursor.cursor.line)
            .unwrap_or(0);
        if let Some(buffer) = self.session.buffer_mut(explorer.buffer_id) {
            *buffer = TextBuffer::from_str(&refreshed_text);
        }

        if let Some(view) = self.views.get_mut(&explorer.buffer_id) {
            let buffer = self
                .session
                .buffer(explorer.buffer_id)
                .expect("explorer buffer must exist");
            let max_line = buffer.len_lines().saturating_sub(1);
            view.cursor.cursor = Pos::new(previous_cursor_line.min(max_line), 0);
            view.cursor.follow.top_margin_rows = 0;
            view.cursor.follow.bottom_margin_rows = 0;
            view.cursor.reconcile_after_edit(
                buffer,
                self.viewport_width_cells,
                self.viewport_height_rows
                    .saturating_sub(STATUS_BAR_HEIGHT_ROWS),
            );
            view.invalidate_render_caches();
        }

        explorer.original_entries = refreshed_entries;
        self.explorer = Some(explorer);
        self.session.mark_active_clean();
        self.explorer_delete_confirmation_token = None;
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
        if entry.is_parent {
            out.push_str("../");
        } else {
            out.push_str(&entry.name);
        }
        if entry.is_dir && !entry.is_parent {
            out.push('/');
        }
    }
    out
}

fn format_explorer_dir_path(dir_path: &Path) -> String {
    let mut rendered = format!("~{}", dir_path.display());
    if !rendered.ends_with('/') {
        rendered.push('/');
    }
    rendered
}

fn pending_explorer_delete_count(
    old_entries: &[ExplorerEntry],
    new_entries: &[ExplorerEntry],
) -> usize {
    let old_len = old_entries.iter().filter(|entry| !entry.is_parent).count();
    let new_len = new_entries.iter().filter(|entry| !entry.is_parent).count();
    old_len.saturating_sub(new_len)
}

fn explorer_delete_confirmation_token(dir_path: &Path, current_text: &str) -> String {
    format!("{}::{current_text}", dir_path.display())
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

    let mut seen_names = HashSet::new();
    for entry in &new_entries {
        if !seen_names.insert(entry.name.clone()) {
            anyhow::bail!("duplicate entry name '{}'", entry.name);
        }
    }

    #[derive(Debug)]
    struct PlannedRename {
        old_name: String,
        new_name: String,
        old_path: PathBuf,
        new_path: PathBuf,
        temp_path: PathBuf,
    }

    let old_index_by_name: HashMap<&str, usize> = old_entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| (entry.name.as_str(), idx))
        .collect();
    let new_index_by_name: HashMap<&str, usize> = new_entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| (entry.name.as_str(), idx))
        .collect();

    let mut old_missing: Vec<(usize, ExplorerEntry)> = old_entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !new_index_by_name.contains_key(entry.name.as_str()))
        .map(|(idx, entry)| (idx, entry.clone()))
        .collect();
    let mut new_added: Vec<(usize, ExplorerEntry)> = new_entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !old_index_by_name.contains_key(entry.name.as_str()))
        .map(|(idx, entry)| (idx, entry.clone()))
        .collect();

    old_missing.sort_by_key(|(idx, _)| *idx);
    new_added.sort_by_key(|(idx, _)| *idx);

    let paired_rename_count = old_missing.len().min(new_added.len());
    let mut renames: Vec<PlannedRename> = Vec::new();
    for i in 0..paired_rename_count {
        let (_, old) = &old_missing[i];
        let (_, new) = &new_added[i];
        if old.is_dir != new.is_dir {
            anyhow::bail!(
                "cannot change entry kind from '{}' to '{}'",
                old.name,
                new.name
            );
        }

        renames.push(PlannedRename {
            old_name: old.name.clone(),
            new_name: new.name.clone(),
            old_path: dir_path.join(&old.name),
            new_path: dir_path.join(&new.name),
            temp_path: PathBuf::new(),
        });
    }
    let deletions: Vec<ExplorerEntry> = old_missing
        .into_iter()
        .skip(paired_rename_count)
        .map(|(_, entry)| entry)
        .collect();
    let creations: Vec<ExplorerEntry> = new_added
        .into_iter()
        .skip(paired_rename_count)
        .map(|(_, entry)| entry)
        .collect();

    // Allow targets that are part of the rename source set (swap/cycle); reject all others.
    let rename_sources: HashSet<&str> = renames.iter().map(|r| r.old_name.as_str()).collect();
    for rename in &renames {
        if !rename.old_path.exists() {
            anyhow::bail!("rename source '{}' does not exist", rename.old_name);
        }
        if rename.new_path.exists() && !rename_sources.contains(rename.new_name.as_str()) {
            anyhow::bail!("rename target '{}' already exists", rename.new_name);
        }
    }

    // Stage each source to a unique temporary path first so swaps/cycles cannot conflict.
    let mut reserved_names: HashSet<String> = old_entries
        .iter()
        .map(|entry| entry.name.clone())
        .chain(new_entries.iter().map(|entry| entry.name.clone()))
        .collect();

    for (idx, rename) in renames.iter_mut().enumerate() {
        let mut attempt = 0usize;
        loop {
            let candidate = format!(".redox_rename_tmp_{idx}_{attempt}");
            let candidate_path = dir_path.join(&candidate);
            if !reserved_names.contains(&candidate) && !candidate_path.exists() {
                reserved_names.insert(candidate);
                rename.temp_path = candidate_path;
                break;
            }
            attempt = attempt.saturating_add(1);
            if attempt > 10_000 {
                anyhow::bail!("failed to allocate temporary rename path");
            }
        }
    }

    for rename in &renames {
        fs::rename(&rename.old_path, &rename.temp_path)?;
    }
    for rename in &renames {
        if rename.new_path.exists() {
            anyhow::bail!("rename target '{}' already exists", rename.new_name);
        }
        fs::rename(&rename.temp_path, &rename.new_path)?;
    }

    for old in &deletions {
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

    for new in &creations {
        let path = dir_path.join(&new.name);
        if new.is_dir {
            fs::create_dir(&path)?;
        } else {
            let _ = fs::File::create(&path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("redox_explorer_{tag}_{nanos}"))
    }

    fn file_entry(name: &str) -> ExplorerEntry {
        ExplorerEntry {
            name: name.to_string(),
            is_dir: false,
            is_parent: false,
        }
    }

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

    #[test]
    fn reorder_only_does_not_mutate_file_contents() {
        let root = temp_dir_path("reorder_only");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("alpha.txt"), "alpha").expect("failed to write alpha fixture");
        fs::write(root.join("beta.txt"), "beta").expect("failed to write beta fixture");

        let old_entries = vec![file_entry("alpha.txt"), file_entry("beta.txt")];
        let new_entries = vec![file_entry("beta.txt"), file_entry("alpha.txt")];

        apply_explorer_changes(&root, &old_entries, &new_entries)
            .expect("expected reorder-only write to succeed");

        assert_eq!(
            fs::read_to_string(root.join("alpha.txt")).expect("failed to read alpha"),
            "alpha"
        );
        assert_eq!(
            fs::read_to_string(root.join("beta.txt")).expect("failed to read beta"),
            "beta"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_and_keep_entry_succeeds_without_rename_collision() {
        let root = temp_dir_path("delete_keep");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("alpha.txt"), "alpha").expect("failed to write alpha fixture");
        fs::write(root.join("beta.txt"), "beta").expect("failed to write beta fixture");

        let old_entries = vec![file_entry("alpha.txt"), file_entry("beta.txt")];
        let new_entries = vec![file_entry("beta.txt")];

        apply_explorer_changes(&root, &old_entries, &new_entries)
            .expect("expected delete+keep write to succeed");

        assert!(!root.join("alpha.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("beta.txt")).expect("failed to read beta"),
            "beta"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inserting_new_entry_in_middle_preserves_existing_file_contents() {
        let root = temp_dir_path("insert_middle");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("alpha.txt"), "alpha").expect("failed to write alpha fixture");
        fs::write(root.join("beta.txt"), "beta").expect("failed to write beta fixture");

        let old_entries = vec![file_entry("alpha.txt"), file_entry("beta.txt")];
        let new_entries = vec![
            file_entry("alpha.txt"),
            file_entry("new.txt"),
            file_entry("beta.txt"),
        ];

        apply_explorer_changes(&root, &old_entries, &new_entries)
            .expect("expected mid-list insert to succeed");

        assert_eq!(
            fs::read_to_string(root.join("alpha.txt")).expect("failed to read alpha"),
            "alpha"
        );
        assert_eq!(
            fs::read_to_string(root.join("beta.txt")).expect("failed to read beta"),
            "beta"
        );
        assert_eq!(
            fs::read_to_string(root.join("new.txt")).expect("failed to read new file"),
            ""
        );

        let _ = fs::remove_dir_all(root);
    }
}
