use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use redox_core::{BufferId, BufferKind, Pos, TextBuffer};

use super::{EditorMode, EditorState, StatusMessageStyle};
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
    pub(super) directory_drafts: HashMap<PathBuf, ExplorerDirectoryDraft>,
    pub(super) return_to_buffer_id: BufferId,
}

#[derive(Debug, Clone)]
pub(super) struct ExplorerDirectoryDraft {
    pub(super) original_entries: Vec<ExplorerEntry>,
    pub(super) text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AppliedExplorerEntryChange {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedExplorerRename {
    old_name: String,
    new_name: String,
    old_path: PathBuf,
    new_path: PathBuf,
    is_dir: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AppliedExplorerChanges {
    renamed_entries: Vec<AppliedExplorerRename>,
    deleted_entries: Vec<AppliedExplorerEntryChange>,
    created_entries: Vec<AppliedExplorerEntryChange>,
}

impl AppliedExplorerChanges {
    fn renamed_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        self.renamed_entries
            .iter()
            .map(|rename| (rename.old_path.clone(), rename.new_path.clone()))
            .collect()
    }

    fn deleted_paths(&self) -> Vec<PathBuf> {
        self.deleted_entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    fn extend(&mut self, other: AppliedExplorerChanges) {
        self.renamed_entries.extend(other.renamed_entries);
        self.deleted_entries.extend(other.deleted_entries);
        self.created_entries.extend(other.created_entries);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedExplorerRename {
    old_name: String,
    new_name: String,
    old_path: PathBuf,
    new_path: PathBuf,
    temp_path: PathBuf,
    is_dir: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PlannedExplorerDirWrite {
    changes: AppliedExplorerChanges,
    renames: Vec<PlannedExplorerRename>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PlannedExplorerWrite {
    changes: AppliedExplorerChanges,
    dir_writes: Vec<PlannedExplorerDirWrite>,
}

impl EditorState {
    pub fn open_explorer_at_path(&mut self, dir_path: PathBuf) -> anyhow::Result<()> {
        if self.active_buffer_is_surface() && !self.about_is_active() {
            let _ = self.close_active_surface_buffer_without_quit();
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
        let background_id = self
            .about
            .as_ref()
            .filter(|about| about.buffer_id == explorer.return_to_buffer_id)
            .map(|about| about.return_to_buffer_id)
            .unwrap_or(explorer.return_to_buffer_id);
        self.session.buffer(background_id).map(|_| background_id)
    }

    pub fn explorer_background_is_placeholder_blank(&self) -> bool {
        let Some(explorer) = self.explorer.as_ref() else {
            return false;
        };
        if explorer.buffer_id != self.session.active_id() {
            return false;
        }
        let background_id = self
            .about
            .as_ref()
            .filter(|about| about.buffer_id == explorer.return_to_buffer_id)
            .map(|about| about.return_to_buffer_id)
            .unwrap_or(explorer.return_to_buffer_id);
        self.is_empty_unnamed_startup_buffer(background_id)
    }

    pub(super) fn command_open_explorer(&mut self) {
        if self.explorer_is_active() {
            let _ = self.close_active_surface_buffer();
            self.mode = EditorMode::Normal;
            self.clear_status();
            return;
        }

        if self.active_buffer_is_surface() && !self.about_is_active() {
            let _ = self.close_active_surface_buffer_without_quit();
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
            dir_path: dir_path.clone(),
            directory_drafts: HashMap::from([(
                dir_path.clone(),
                ExplorerDirectoryDraft {
                    original_entries: entries,
                    text,
                },
            )]),
            return_to_buffer_id: return_to,
        });
        Ok(())
    }

    pub(super) fn explorer_target_directory(&self) -> anyhow::Result<PathBuf> {
        if let Some(dir) = &self.transient_origin_dir {
            return Ok(dir.clone());
        }

        if let Some(origin_id) = self.transient_origin_buffer_id
            && let Some(meta) = self.session.meta(origin_id)
            && let Some(path) = &meta.path
        {
            return Ok(path.parent().unwrap_or(path.as_path()).to_path_buf());
        }

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
        let return_to_id = explorer.return_to_buffer_id;
        let close_return_placeholder = self
            .is_empty_unnamed_startup_buffer(return_to_id)
            .then_some(return_to_id)
            .or_else(|| {
                self.about.as_ref().and_then(|about| {
                    (about.buffer_id == return_to_id
                        && self.is_empty_unnamed_startup_buffer(about.return_to_buffer_id))
                    .then_some(about.return_to_buffer_id)
                })
            });
        match self.session.open_file(&file_path) {
            Ok(file_id) => {
                let _ = self.views.entry(file_id).or_default();
                self.ensure_buffer_analysis(file_id);
                let _ = self.session.close_buffer(explorer.buffer_id);
                self.views.remove(&explorer.buffer_id);
                if let Some(placeholder_id) = close_return_placeholder
                    && placeholder_id != file_id
                {
                    let _ = self.close_inactive_empty_unnamed_startup_buffer(placeholder_id);
                }
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

    fn persist_active_explorer_draft(&mut self) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        if explorer.buffer_id != self.session.active_id() {
            return;
        }
        let current_text = self.session.active_buffer().to_string();
        if let Some(draft) = explorer.directory_drafts.get_mut(&explorer.dir_path) {
            draft.text = current_text;
        }
    }

    fn refresh_explorer_directory_with_selection(
        &mut self,
        dir_path: PathBuf,
        preferred_entry_name: Option<String>,
    ) -> anyhow::Result<()> {
        self.explorer_delete_confirmation_token = None;
        self.persist_active_explorer_draft();
        let dir_path = std::fs::canonicalize(&dir_path).unwrap_or(dir_path);
        let Some(mut explorer) = self.explorer.clone() else {
            return Ok(());
        };
        let draft = if let Some(existing) = explorer.directory_drafts.get(&dir_path) {
            existing.clone()
        } else {
            let entries = list_explorer_entries(&dir_path)?;
            let text = explorer_entries_to_text(&entries);
            let draft = ExplorerDirectoryDraft {
                original_entries: entries,
                text,
            };
            explorer
                .directory_drafts
                .insert(dir_path.clone(), draft.clone());
            draft
        };
        let initial_cursor_line = preferred_entry_name
            .as_ref()
            .and_then(|name| {
                draft
                    .original_entries
                    .iter()
                    .position(|entry| entry.name == *name)
            })
            .unwrap_or(0);
        let explorer_id = explorer.buffer_id;

        if let Some(buffer) = self.session.buffer_mut(explorer_id) {
            *buffer = TextBuffer::from_text(&draft.text);
        }
        explorer.dir_path = dir_path;
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
        if let Some(draft) = explorer.directory_drafts.get_mut(&explorer.dir_path) {
            draft.text = current_text;
        }

        let mut desired_entries_by_dir = Vec::new();
        for (dir_path, draft) in &explorer.directory_drafts {
            let desired_entries = match parse_explorer_entries(&draft.text) {
                Ok(entries) => entries,
                Err(e) => {
                    self.explorer_delete_confirmation_token = None;
                    self.set_status(format!("explorer parse error: {e}"));
                    return false;
                }
            };
            desired_entries_by_dir.push((dir_path.clone(), desired_entries));
        }

        desired_entries_by_dir.sort_by(|(a, _), (b, _)| a.cmp(b));

        let mut pending_deletions_by_dir = Vec::new();
        for (dir_path, desired_entries) in &desired_entries_by_dir {
            let Some(draft) = explorer.directory_drafts.get(dir_path) else {
                continue;
            };
            let pending_deletions =
                pending_explorer_deletions(&draft.original_entries, desired_entries);
            if !pending_deletions.is_empty() {
                pending_deletions_by_dir.push((dir_path.clone(), pending_deletions));
            }
        }

        let delete_count: usize = pending_deletions_by_dir
            .iter()
            .map(|(dir_path, deletions)| {
                explorer_delete_confirmation_targets(dir_path, deletions).len()
            })
            .sum();
        if delete_count > 0 {
            let token = explorer_delete_confirmation_token_for_drafts(&explorer.directory_drafts);
            if !confirm_delete {
                self.explorer_delete_confirmation_token = Some(token);
                self.set_status_sticky_lines(format_explorer_delete_confirmation_lines(
                    &pending_deletions_by_dir,
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

        let planned_write =
            match plan_explorer_write(&explorer.directory_drafts, &desired_entries_by_dir) {
                Ok(plan) => plan,
                Err(e) => {
                    self.explorer_delete_confirmation_token = None;
                    self.set_status(format!("explorer write failed: {e}"));
                    return false;
                }
            };

        if let Err(e) = execute_explorer_write(&planned_write) {
            self.explorer_delete_confirmation_token = None;
            self.set_status(format!("explorer write failed: {e}"));
            return false;
        }

        self.sync_session_after_explorer_write(&planned_write.changes);
        let pin_sync_error = self.sync_pinned_files_after_explorer_write(&planned_write.changes);
        self.mark_git_repo_statuses_stale();

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
            *buffer = TextBuffer::from_text(&refreshed_text);
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

        explorer.directory_drafts = HashMap::from([(
            explorer.dir_path.clone(),
            ExplorerDirectoryDraft {
                original_entries: refreshed_entries,
                text: refreshed_text.clone(),
            },
        )]);
        if let Some(updated_explorer) = self.explorer.as_ref() {
            explorer.return_to_buffer_id = updated_explorer.return_to_buffer_id;
        }
        self.explorer = Some(explorer);
        self.session.mark_active_clean();
        self.explorer_delete_confirmation_token = None;
        let status = format_explorer_write_summary(&planned_write.changes);
        if let Some(err) = pin_sync_error {
            self.set_status(format!("{status}; pin save failed: {err}"));
        } else {
            self.set_status(status);
        }
        true
    }

    fn sync_session_after_explorer_write(&mut self, changes: &AppliedExplorerChanges) {
        let Some(explorer_id) = self.explorer.as_ref().map(|explorer| explorer.buffer_id) else {
            return;
        };

        let result = self
            .session
            .sync_file_buffers_with_paths(&changes.renamed_paths(), &changes.deleted_paths());
        for id in result.closed_ids {
            self.views.remove(&id);
        }

        let Some(return_to_buffer_id) = self
            .explorer
            .as_ref()
            .map(|explorer| explorer.return_to_buffer_id)
        else {
            return;
        };
        if self.session.buffer(return_to_buffer_id).is_some() {
            return;
        }

        let replacement_id = self
            .session
            .summaries()
            .into_iter()
            .find(|summary| summary.kind == BufferKind::File && summary.id != explorer_id)
            .map(|summary| summary.id)
            .unwrap_or_else(|| {
                let id = self.session.open_unnamed_buffer();
                let _ = self.views.entry(id).or_default();
                id
            });

        if let Some(explorer) = self.explorer.as_mut() {
            explorer.return_to_buffer_id = replacement_id;
        }
        let _ = self.session.activate(explorer_id);
    }

    fn sync_pinned_files_after_explorer_write(
        &mut self,
        changes: &AppliedExplorerChanges,
    ) -> Option<std::io::Error> {
        let renamed_paths = changes.renamed_paths();
        if renamed_paths.is_empty() || !self.pinned_files.remap_paths(&renamed_paths) {
            return None;
        }

        self.pinned_files.save().err()
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

fn pending_explorer_deletions(
    old_entries: &[ExplorerEntry],
    new_entries: &[ExplorerEntry],
) -> Vec<ExplorerEntry> {
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

    let old_missing: Vec<(usize, ExplorerEntry)> = old_entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !new_index_by_name.contains_key(entry.name.as_str()))
        .map(|(idx, entry)| (idx, entry.clone()))
        .collect();
    let new_added: Vec<(usize, ExplorerEntry)> = new_entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !old_index_by_name.contains_key(entry.name.as_str()))
        .map(|(idx, entry)| (idx, entry.clone()))
        .collect();

    let (_, deletions, _) = partition_explorer_entry_changes(old_missing, new_added);
    deletions
}

fn explorer_delete_confirmation_token_for_drafts(
    drafts: &HashMap<PathBuf, ExplorerDirectoryDraft>,
) -> String {
    let mut dirs: Vec<_> = drafts.iter().collect();
    dirs.sort_by(|(a, _), (b, _)| a.cmp(b));
    dirs.into_iter()
        .map(|(dir_path, draft)| format!("{}::{}", dir_path.display(), draft.text))
        .collect::<Vec<_>>()
        .join("\n")
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

        let is_parent = name == "..";
        let name = if is_parent {
            if is_dir {
                "..".to_string()
            } else {
                anyhow::bail!("line {}: '..' must remain a directory entry", idx + 1);
            }
        } else {
            normalize_explorer_entry_name(name, idx + 1)?
        };

        out.push(ExplorerEntry {
            name,
            is_dir: is_dir || is_parent,
            is_parent,
        });
    }

    Ok(out)
}

fn plan_explorer_write(
    drafts: &HashMap<PathBuf, ExplorerDirectoryDraft>,
    desired_entries_by_dir: &[(PathBuf, Vec<ExplorerEntry>)],
) -> anyhow::Result<PlannedExplorerWrite> {
    let mut planned_write = PlannedExplorerWrite::default();

    for (original_dir_path, desired_entries) in desired_entries_by_dir {
        let Some(draft) = drafts.get(original_dir_path) else {
            continue;
        };
        let Some(resolved_dir_path) =
            resolve_explorer_draft_dir_path(original_dir_path, &planned_write.changes)
        else {
            continue;
        };
        let dir_write = plan_explorer_dir_changes(
            original_dir_path,
            &resolved_dir_path,
            &draft.original_entries,
            desired_entries,
        )?;
        planned_write.changes.extend(dir_write.changes.clone());
        planned_write.dir_writes.push(dir_write);
    }

    Ok(planned_write)
}

fn resolve_explorer_draft_dir_path(
    dir_path: &Path,
    applied_changes: &AppliedExplorerChanges,
) -> Option<PathBuf> {
    let mut resolved = dir_path.to_path_buf();

    for deleted in applied_changes
        .deleted_entries
        .iter()
        .filter(|entry| entry.is_dir)
    {
        if resolved == deleted.path || resolved.starts_with(&deleted.path) {
            return None;
        }
    }

    for rename in applied_changes
        .renamed_entries
        .iter()
        .filter(|rename| rename.is_dir)
    {
        if resolved == rename.old_path {
            resolved = rename.new_path.clone();
            continue;
        }
        if let Ok(relative) = resolved.strip_prefix(&rename.old_path) {
            resolved = rename.new_path.join(relative);
        }
    }

    for deleted in applied_changes
        .deleted_entries
        .iter()
        .filter(|entry| entry.is_dir)
    {
        if resolved == deleted.path || resolved.starts_with(&deleted.path) {
            return None;
        }
    }

    Some(resolved)
}

#[cfg_attr(not(test), allow(dead_code))]
fn apply_explorer_changes(
    dir_path: &Path,
    old_entries: &[ExplorerEntry],
    new_entries: &[ExplorerEntry],
) -> anyhow::Result<PlannedExplorerDirWrite> {
    plan_explorer_dir_changes(dir_path, dir_path, old_entries, new_entries)
}

fn plan_explorer_dir_changes(
    current_dir_path: &Path,
    resolved_dir_path: &Path,
    old_entries: &[ExplorerEntry],
    new_entries: &[ExplorerEntry],
) -> anyhow::Result<PlannedExplorerDirWrite> {
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

    let mut renames = Vec::new();
    let (rename_pairs, deletions, creations) =
        partition_explorer_entry_changes(old_missing, new_added);
    for (old, new) in rename_pairs {
        renames.push(PlannedExplorerRename {
            old_name: old.name.clone(),
            new_name: new.name.clone(),
            old_path: explorer_entry_path(resolved_dir_path, &old.name),
            new_path: explorer_entry_path(resolved_dir_path, &new.name),
            temp_path: PathBuf::new(),
            is_dir: old.is_dir,
        });
    }

    // Allow targets that are part of the rename source set (swap/cycle); reject all others.
    let rename_sources: HashSet<PathBuf> = renames.iter().map(|r| r.old_path.clone()).collect();
    for rename in &renames {
        let current_old_path = explorer_entry_path(current_dir_path, &rename.old_name);
        if !current_old_path.exists() {
            anyhow::bail!("rename source '{}' does not exist", rename.old_name);
        }
        if rename.new_path.exists() && !rename_sources.contains(&rename.new_path) {
            anyhow::bail!("rename target '{}' already exists", rename.new_name);
        }
    }

    // Stage each source to a unique temporary path first so swaps/cycles cannot conflict.
    let mut reserved_temp_paths: HashSet<PathBuf> = HashSet::new();

    for (idx, rename) in renames.iter_mut().enumerate() {
        let mut attempt = 0usize;
        loop {
            let candidate = format!(".redox_rename_tmp_{idx}_{attempt}");
            let candidate_path = resolved_dir_path.join(&candidate);
            if !reserved_temp_paths.contains(&candidate_path) && !candidate_path.exists() {
                reserved_temp_paths.insert(candidate_path.clone());
                rename.temp_path = candidate_path;
                break;
            }
            attempt = attempt.saturating_add(1);
            if attempt > 10_000 {
                anyhow::bail!("failed to allocate temporary rename path");
            }
        }
    }

    Ok(PlannedExplorerDirWrite {
        changes: AppliedExplorerChanges {
            renamed_entries: renames
                .iter()
                .map(|rename| AppliedExplorerRename {
                    old_name: rename.old_name.clone(),
                    new_name: rename.new_name.clone(),
                    old_path: rename.old_path.clone(),
                    new_path: rename.new_path.clone(),
                    is_dir: rename.is_dir,
                })
                .collect(),
            deleted_entries: deletions
                .iter()
                .map(|entry| AppliedExplorerEntryChange {
                    name: entry.name.clone(),
                    path: explorer_entry_path(resolved_dir_path, &entry.name),
                    is_dir: entry.is_dir,
                })
                .collect(),
            created_entries: creations
                .iter()
                .map(|entry| AppliedExplorerEntryChange {
                    name: entry.name.clone(),
                    path: explorer_entry_path(resolved_dir_path, &entry.name),
                    is_dir: entry.is_dir,
                })
                .collect(),
        },
        renames,
    })
}

fn execute_explorer_write(planned_write: &PlannedExplorerWrite) -> anyhow::Result<()> {
    for dir_write in &planned_write.dir_writes {
        for rename in &dir_write.renames {
            fs::rename(&rename.old_path, &rename.temp_path)?;
        }
        for rename in &dir_write.renames {
            if let Some(parent) = rename.new_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if rename.new_path.exists() {
                anyhow::bail!("rename target '{}' already exists", rename.new_name);
            }
            fs::rename(&rename.temp_path, &rename.new_path)?;
        }
    }

    for deleted in &planned_write.changes.deleted_entries {
        if deleted.is_dir {
            fs::remove_dir_all(&deleted.path)?;
        } else {
            fs::remove_file(&deleted.path)?;
        }
    }

    for created in &planned_write.changes.created_entries {
        if created.is_dir {
            fs::create_dir_all(&created.path)?;
        } else {
            if let Some(parent) = created.path.parent() {
                fs::create_dir_all(parent)?;
            }
            let _ = fs::File::create(&created.path)?;
        }
    }

    Ok(())
}

fn normalize_explorer_entry_name(name: &str, line_number: usize) -> anyhow::Result<String> {
    if name.is_empty() {
        anyhow::bail!("line {line_number}: empty name");
    }

    let mut parts = Vec::new();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {
                anyhow::bail!("line {line_number}: '.' path segments are not allowed")
            }
            Component::ParentDir => {
                anyhow::bail!("line {line_number}: '..' path segments are not allowed")
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("line {line_number}: absolute paths are not allowed")
            }
        }
    }

    if parts.is_empty() {
        anyhow::bail!("line {line_number}: empty name");
    }

    Ok(parts.join("/"))
}

fn explorer_entry_path(dir_path: &Path, name: &str) -> PathBuf {
    name.split('/')
        .fold(dir_path.to_path_buf(), |path, segment| path.join(segment))
}

fn explorer_entry_label(name: &str, is_dir: bool) -> String {
    if is_dir {
        format!("{name}/")
    } else {
        name.to_string()
    }
}

fn format_explorer_delete_confirmation_lines(
    deletions_by_dir: &[(PathBuf, Vec<ExplorerEntry>)],
) -> Vec<(String, StatusMessageStyle)> {
    let dir_context_base = common_path_prefix(
        &deletions_by_dir
            .iter()
            .map(|(dir_path, _)| dir_path.as_path())
            .collect::<Vec<_>>(),
    );
    let include_dir_context = deletions_by_dir.len() > 1;

    let mut targets = Vec::new();
    for (dir_path, deletions) in deletions_by_dir {
        let dir_label = include_dir_context
            .then(|| explorer_delete_confirmation_dir_label(dir_context_base.as_deref(), dir_path));
        targets.extend(explorer_delete_confirmation_targets_with_dir_context(
            dir_path,
            deletions,
            dir_label.as_deref(),
        ));
    }
    let noun = if targets.len() == 1 {
        "entry"
    } else {
        "entries"
    };
    let mut lines = vec![(
        format!("confirm deletion of {} {}:", targets.len(), noun),
        StatusMessageStyle::Normal,
    )];
    lines.extend(
        targets
            .into_iter()
            .map(|target| (format!(" {target}"), StatusMessageStyle::Normal)),
    );
    lines.push(("press y".to_string(), StatusMessageStyle::Dim));
    lines
}

fn explorer_delete_confirmation_targets(
    dir_path: &Path,
    deletions: &[ExplorerEntry],
) -> Vec<String> {
    let mut targets = Vec::new();
    for entry in deletions {
        targets.push(explorer_entry_label(&entry.name, entry.is_dir));
        if !entry.is_dir {
            continue;
        }

        let path = explorer_entry_path(dir_path, &entry.name);
        collect_directory_delete_targets(dir_path, &path, &mut targets);
    }
    targets.sort_by(|a, b| {
        let a_path = a.trim_end_matches('/');
        let b_path = b.trim_end_matches('/');
        let a_depth = a_path.split('/').count();
        let b_depth = b_path.split('/').count();
        let a_parent = a_path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        let b_parent = b_path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        let a_is_dir = a.ends_with('/');
        let b_is_dir = b.ends_with('/');
        a_depth
            .cmp(&b_depth)
            .then_with(|| a_parent.to_lowercase().cmp(&b_parent.to_lowercase()))
            .then_with(|| a_is_dir.cmp(&b_is_dir))
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
            .then_with(|| a.cmp(b))
    });
    targets
}

fn explorer_delete_confirmation_targets_with_dir_context(
    dir_path: &Path,
    deletions: &[ExplorerEntry],
    dir_label: Option<&str>,
) -> Vec<String> {
    explorer_delete_confirmation_targets(dir_path, deletions)
        .into_iter()
        .map(|target| match dir_label {
            Some(dir_label) => format!("{dir_label}{target}"),
            None => target,
        })
        .collect()
}

fn explorer_delete_confirmation_dir_label(base_dir: Option<&Path>, dir_path: &Path) -> String {
    let relative_dir = base_dir
        .and_then(|base_dir| dir_path.strip_prefix(base_dir).ok())
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.to_string_lossy().replace('\\', "/"));

    match relative_dir {
        Some(relative_dir) => format!("{relative_dir}/"),
        None => "./".to_string(),
    }
}

fn common_path_prefix(paths: &[&Path]) -> Option<PathBuf> {
    let mut prefix = paths.first()?.to_path_buf();
    for path in &paths[1..] {
        while !path.starts_with(&prefix) {
            if !prefix.pop() {
                return None;
            }
        }
    }
    Some(prefix)
}

fn collect_directory_delete_targets(root_dir: &Path, dir_path: &Path, targets: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir_path) else {
        return;
    };

    let mut children: Vec<_> = entries.filter_map(Result::ok).collect();
    children.sort_by(|a, b| {
        a.file_name()
            .to_string_lossy()
            .to_lowercase()
            .cmp(&b.file_name().to_string_lossy().to_lowercase())
    });

    for child in children {
        let child_path = child.path();
        let relative_name = child_path
            .strip_prefix(root_dir)
            .ok()
            .map(|path| path.to_string_lossy().replace('\\', "/"));
        let is_dir = child_path.is_dir();
        if let Some(relative_name) = relative_name {
            targets.push(explorer_entry_label(&relative_name, is_dir));
        }
        if is_dir {
            collect_directory_delete_targets(root_dir, &child_path, targets);
        }
    }
}

fn format_explorer_write_summary(changes: &AppliedExplorerChanges) -> String {
    let mut parts = Vec::new();

    push_explorer_entry_change_summary(
        &mut parts,
        "created",
        "file",
        "files",
        &changes
            .created_entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| explorer_entry_label(&entry.name, entry.is_dir))
            .collect::<Vec<_>>(),
    );
    push_explorer_entry_change_summary(
        &mut parts,
        "created",
        "directory",
        "directories",
        &changes
            .created_entries
            .iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| explorer_entry_label(&entry.name, entry.is_dir))
            .collect::<Vec<_>>(),
    );
    push_explorer_entry_change_summary(
        &mut parts,
        "renamed",
        "file",
        "files",
        &changes
            .renamed_entries
            .iter()
            .filter(|rename| !rename.is_dir)
            .map(|rename| {
                format!(
                    "{} -> {}",
                    explorer_entry_label(&rename.old_name, rename.is_dir),
                    explorer_entry_label(&rename.new_name, rename.is_dir)
                )
            })
            .collect::<Vec<_>>(),
    );
    push_explorer_entry_change_summary(
        &mut parts,
        "renamed",
        "directory",
        "directories",
        &changes
            .renamed_entries
            .iter()
            .filter(|rename| rename.is_dir)
            .map(|rename| {
                format!(
                    "{} -> {}",
                    explorer_entry_label(&rename.old_name, rename.is_dir),
                    explorer_entry_label(&rename.new_name, rename.is_dir)
                )
            })
            .collect::<Vec<_>>(),
    );
    push_explorer_entry_change_summary(
        &mut parts,
        "deleted",
        "file",
        "files",
        &changes
            .deleted_entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| explorer_entry_label(&entry.name, entry.is_dir))
            .collect::<Vec<_>>(),
    );
    push_explorer_entry_change_summary(
        &mut parts,
        "deleted",
        "directory",
        "directories",
        &changes
            .deleted_entries
            .iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| explorer_entry_label(&entry.name, entry.is_dir))
            .collect::<Vec<_>>(),
    );

    if parts.is_empty() {
        "no explorer changes".to_string()
    } else {
        parts.join("; ")
    }
}

fn push_explorer_entry_change_summary(
    parts: &mut Vec<String>,
    action: &str,
    singular_kind: &str,
    plural_kind: &str,
    labels: &[String],
) {
    if labels.is_empty() {
        return;
    }

    let kind = if labels.len() == 1 {
        singular_kind
    } else {
        plural_kind
    };
    parts.push(format!("{action} {kind}: {}", labels.join(", ")));
}

fn partition_explorer_entry_changes(
    old_missing: Vec<(usize, ExplorerEntry)>,
    new_added: Vec<(usize, ExplorerEntry)>,
) -> (
    Vec<(ExplorerEntry, ExplorerEntry)>,
    Vec<ExplorerEntry>,
    Vec<ExplorerEntry>,
) {
    fn pair_by_kind(
        old_missing: &[(usize, ExplorerEntry)],
        new_added: &[(usize, ExplorerEntry)],
        is_dir: bool,
        rename_pairs: &mut Vec<(ExplorerEntry, ExplorerEntry)>,
        old_paired: &mut HashSet<usize>,
        new_paired: &mut HashSet<usize>,
    ) {
        let old_candidates: Vec<(usize, &ExplorerEntry)> = old_missing
            .iter()
            .enumerate()
            .filter_map(|(vec_idx, (_, entry))| {
                (entry.is_dir == is_dir).then_some((vec_idx, entry))
            })
            .collect();
        let new_candidates: Vec<(usize, &ExplorerEntry)> = new_added
            .iter()
            .enumerate()
            .filter_map(|(vec_idx, (_, entry))| {
                (entry.is_dir == is_dir).then_some((vec_idx, entry))
            })
            .collect();

        for ((old_vec_idx, old_entry), (new_vec_idx, new_entry)) in
            old_candidates.into_iter().zip(new_candidates.into_iter())
        {
            rename_pairs.push((old_entry.clone(), new_entry.clone()));
            old_paired.insert(old_vec_idx);
            new_paired.insert(new_vec_idx);
        }
    }

    let mut rename_pairs = Vec::new();
    let mut old_paired = HashSet::new();
    let mut new_paired = HashSet::new();

    pair_by_kind(
        &old_missing,
        &new_added,
        false,
        &mut rename_pairs,
        &mut old_paired,
        &mut new_paired,
    );
    pair_by_kind(
        &old_missing,
        &new_added,
        true,
        &mut rename_pairs,
        &mut old_paired,
        &mut new_paired,
    );

    let deletions = old_missing
        .into_iter()
        .enumerate()
        .filter_map(|(vec_idx, (_, entry))| (!old_paired.contains(&vec_idx)).then_some(entry))
        .collect();
    let creations = new_added
        .into_iter()
        .enumerate()
        .filter_map(|(vec_idx, (_, entry))| (!new_paired.contains(&vec_idx)).then_some(entry))
        .collect();

    (rename_pairs, deletions, creations)
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

    fn dir_entry(name: &str) -> ExplorerEntry {
        ExplorerEntry {
            name: name.to_string(),
            is_dir: true,
            is_parent: false,
        }
    }

    fn parent_entry() -> ExplorerEntry {
        ExplorerEntry {
            name: "..".to_string(),
            is_dir: true,
            is_parent: true,
        }
    }

    #[test]
    fn deleting_non_empty_directory_recursively_removes_children() {
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

        let planned = apply_explorer_changes(&root, &old_entries, &new_entries)
            .expect("expected non-empty directory delete to succeed");
        execute_explorer_write(&PlannedExplorerWrite {
            changes: planned.changes.clone(),
            dir_writes: vec![planned.clone()],
        })
        .expect("expected non-empty directory delete execution to succeed");
        assert_eq!(
            planned.changes.deleted_entries,
            vec![AppliedExplorerEntryChange {
                name: "doomed".to_string(),
                path: root.join("doomed"),
                is_dir: true,
            }]
        );
        assert!(!doomed.exists());

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

        let planned = apply_explorer_changes(&root, &old_entries, &new_entries)
            .expect("expected reorder-only plan to succeed");
        execute_explorer_write(&PlannedExplorerWrite {
            changes: planned.changes.clone(),
            dir_writes: vec![planned],
        })
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

        let planned = apply_explorer_changes(&root, &old_entries, &new_entries)
            .expect("expected delete+keep plan to succeed");
        execute_explorer_write(&PlannedExplorerWrite {
            changes: planned.changes.clone(),
            dir_writes: vec![planned],
        })
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

        let planned = apply_explorer_changes(&root, &old_entries, &new_entries)
            .expect("expected mid-list insert plan to succeed");
        execute_explorer_write(&PlannedExplorerWrite {
            changes: planned.changes.clone(),
            dir_writes: vec![planned],
        })
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

    #[test]
    fn parse_explorer_entries_accepts_nested_relative_paths() {
        let parsed = parse_explorer_entries("../\npath/to/file.txt\nnested/dir/\na//b.txt")
            .expect("expected nested paths to parse");

        assert_eq!(
            parsed,
            vec![
                ExplorerEntry {
                    name: "..".to_string(),
                    is_dir: true,
                    is_parent: true,
                },
                file_entry("path/to/file.txt"),
                dir_entry("nested/dir"),
                file_entry("a/b.txt"),
            ]
        );
    }

    #[test]
    fn parse_explorer_entries_rejects_parent_segments_inside_paths() {
        let err = parse_explorer_entries("../\npath/../file.txt")
            .expect_err("expected parent segments to be rejected");

        assert!(
            err.to_string()
                .contains("..' path segments are not allowed")
        );
    }

    #[test]
    fn apply_explorer_changes_creates_parent_directories_for_nested_files() {
        let root = temp_dir_path("nested_create");
        fs::create_dir_all(&root).expect("failed to create fixture root");

        let planned = apply_explorer_changes(&root, &[], &[file_entry("path/to/file.txt")])
            .expect("expected nested file create plan to succeed");
        execute_explorer_write(&PlannedExplorerWrite {
            changes: planned.changes.clone(),
            dir_writes: vec![planned],
        })
        .expect("expected nested file create to succeed");

        assert!(root.join("path").is_dir());
        assert!(root.join("path/to").is_dir());
        assert_eq!(
            fs::read_to_string(root.join("path/to/file.txt")).expect("failed to read nested file"),
            ""
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_explorer_changes_renames_into_nested_path() {
        let root = temp_dir_path("nested_rename");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("alpha.txt"), "alpha").expect("failed to write alpha fixture");

        let planned = apply_explorer_changes(
            &root,
            &[file_entry("alpha.txt")],
            &[file_entry("nested/path/alpha.txt")],
        )
        .expect("expected nested rename plan to succeed");
        execute_explorer_write(&PlannedExplorerWrite {
            changes: planned.changes.clone(),
            dir_writes: vec![planned.clone()],
        })
        .expect("expected nested rename to succeed");

        assert_eq!(
            planned.changes.renamed_entries,
            vec![AppliedExplorerRename {
                old_name: "alpha.txt".to_string(),
                new_name: "nested/path/alpha.txt".to_string(),
                old_path: root.join("alpha.txt"),
                new_path: root.join("nested/path/alpha.txt"),
                is_dir: false,
            }]
        );
        assert_eq!(
            fs::read_to_string(root.join("nested/path/alpha.txt"))
                .expect("failed to read renamed file"),
            "alpha"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_explorer_changes_treats_file_delete_and_dir_create_as_separate_changes() {
        let root = temp_dir_path("delete_file_create_dir");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("alpha.txt"), "alpha").expect("failed to write alpha fixture");

        let planned =
            apply_explorer_changes(&root, &[file_entry("alpha.txt")], &[dir_entry("fresh")])
                .expect("expected delete+create plan to succeed");
        execute_explorer_write(&PlannedExplorerWrite {
            changes: planned.changes.clone(),
            dir_writes: vec![planned.clone()],
        })
        .expect("expected delete+create to succeed");

        assert_eq!(
            planned.changes.renamed_entries,
            Vec::<AppliedExplorerRename>::new()
        );
        assert_eq!(
            planned.changes.deleted_entries,
            vec![AppliedExplorerEntryChange {
                name: "alpha.txt".to_string(),
                path: root.join("alpha.txt"),
                is_dir: false,
            }]
        );
        assert_eq!(
            planned.changes.created_entries,
            vec![AppliedExplorerEntryChange {
                name: "fresh".to_string(),
                path: root.join("fresh"),
                is_dir: true,
            }]
        );
        assert!(!root.join("alpha.txt").exists());
        assert!(root.join("fresh").is_dir());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn planning_explorer_changes_does_not_mutate_filesystem() {
        let root = temp_dir_path("plan_only");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("alpha.txt"), "alpha").expect("failed to write alpha fixture");

        let planned = apply_explorer_changes(
            &root,
            &[file_entry("alpha.txt")],
            &[file_entry("renamed.txt")],
        )
        .expect("expected rename plan to succeed");

        assert_eq!(
            planned.changes.renamed_entries,
            vec![AppliedExplorerRename {
                old_name: "alpha.txt".to_string(),
                new_name: "renamed.txt".to_string(),
                old_path: root.join("alpha.txt"),
                new_path: root.join("renamed.txt"),
                is_dir: false,
            }]
        );
        assert!(root.join("alpha.txt").exists());
        assert!(!root.join("renamed.txt").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plan_explorer_write_remaps_child_drafts_after_parent_rename() {
        let root = temp_dir_path("remap_child_draft");
        let old_parent = root.join("old");
        let child_file = old_parent.join("child.txt");
        fs::create_dir_all(&old_parent).expect("failed to create fixture directories");
        fs::write(&child_file, "child").expect("failed to write child fixture");

        let parent_draft = ExplorerDirectoryDraft {
            original_entries: vec![dir_entry("old")],
            text: "../\nnew/".to_string(),
        };
        let child_draft = ExplorerDirectoryDraft {
            original_entries: vec![file_entry("child.txt")],
            text: "../\nrenamed.txt".to_string(),
        };
        let drafts = HashMap::from([
            (root.clone(), parent_draft),
            (old_parent.clone(), child_draft),
        ]);
        let desired_entries_by_dir = vec![
            (root.clone(), vec![parent_entry(), dir_entry("new")]),
            (
                old_parent.clone(),
                vec![parent_entry(), file_entry("renamed.txt")],
            ),
        ];

        let planned = plan_explorer_write(&drafts, &desired_entries_by_dir)
            .expect("expected parent rename to remap child draft");

        assert_eq!(
            planned.changes.renamed_entries,
            vec![
                AppliedExplorerRename {
                    old_name: "old".to_string(),
                    new_name: "new".to_string(),
                    old_path: root.join("old"),
                    new_path: root.join("new"),
                    is_dir: true,
                },
                AppliedExplorerRename {
                    old_name: "child.txt".to_string(),
                    new_name: "renamed.txt".to_string(),
                    old_path: root.join("new/child.txt"),
                    new_path: root.join("new/renamed.txt"),
                    is_dir: false,
                },
            ]
        );
        assert!(root.join("old/child.txt").exists());
        assert!(!root.join("new").exists());

        execute_explorer_write(&planned).expect("expected remapped child write to execute");
        assert!(!root.join("old").exists());
        assert_eq!(
            fs::read_to_string(root.join("new/renamed.txt")).expect("failed to read renamed child"),
            "child"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plan_explorer_write_drops_child_drafts_under_deleted_parent() {
        let root = temp_dir_path("drop_child_draft");
        let doomed = root.join("doomed");
        fs::create_dir_all(&doomed).expect("failed to create fixture directories");
        fs::write(doomed.join("child.txt"), "child").expect("failed to write child fixture");

        let parent_draft = ExplorerDirectoryDraft {
            original_entries: vec![dir_entry("doomed")],
            text: "../".to_string(),
        };
        let child_draft = ExplorerDirectoryDraft {
            original_entries: vec![file_entry("child.txt")],
            text: "../\nrenamed.txt".to_string(),
        };
        let drafts = HashMap::from([(root.clone(), parent_draft), (doomed.clone(), child_draft)]);
        let desired_entries_by_dir = vec![
            (root.clone(), vec![parent_entry()]),
            (
                doomed.clone(),
                vec![parent_entry(), file_entry("renamed.txt")],
            ),
        ];

        let planned = plan_explorer_write(&drafts, &desired_entries_by_dir)
            .expect("expected parent delete to drop child draft");

        assert_eq!(planned.dir_writes.len(), 1);
        assert_eq!(
            planned.changes.deleted_entries,
            vec![AppliedExplorerEntryChange {
                name: "doomed".to_string(),
                path: root.join("doomed"),
                is_dir: true,
            }]
        );

        execute_explorer_write(&planned).expect("expected parent delete to execute");
        assert!(!root.join("doomed").exists());

        let _ = fs::remove_dir_all(root);
    }
}
