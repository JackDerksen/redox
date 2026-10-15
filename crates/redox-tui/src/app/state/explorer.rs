use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use redox_core::{BufferId, BufferKind, Pos, TextBuffer};

use super::{BufferViewState, EditorMode, EditorState, GitFileStatusKind, StatusMessageStyle};
use crate::ui::STATUS_BAR_HEIGHT_ROWS;

/// Stop walking a deleted tree after this many entries; the count is then shown as "N+".
const DELETE_TARGET_SCAN_LIMIT: usize = 10_000;
/// Confirmation lines listed before summarising the rest.
const DELETE_TARGET_DISPLAY_LIMIT: usize = 40;

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

#[derive(Debug)]
pub(super) struct ExplorerState {
    pub(super) buffer_id: BufferId,
    pub(super) dir_path: PathBuf,
    pub(super) directory_drafts: HashMap<PathBuf, ExplorerDirectoryDraft>,
    pub(super) return_to_buffer_id: BufferId,
    render_cache: ExplorerRenderCache,
}

impl Clone for ExplorerState {
    fn clone(&self) -> Self {
        Self {
            buffer_id: self.buffer_id,
            dir_path: self.dir_path.clone(),
            directory_drafts: self.directory_drafts.clone(),
            return_to_buffer_id: self.return_to_buffer_id,
            render_cache: ExplorerRenderCache::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ExplorerDirectoryDraft {
    pub(super) original_entries: Vec<ExplorerEntry>,
    pub(super) text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExplorerRenderRowKind {
    File,
    Directory,
    Hidden,
    Executable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExplorerRenderRow {
    pub(crate) kind: ExplorerRenderRowKind,
    pub(crate) git_status: Option<GitFileStatusKind>,
}

#[derive(Debug, PartialEq, Eq)]
struct ExplorerCachedRow {
    kind: ExplorerRenderRowKind,
    executable_path: Option<PathBuf>,
    git_status: Option<GitFileStatusKind>,
}

#[derive(Debug, PartialEq, Eq)]
struct ExplorerRenderCacheKey {
    dir_path: PathBuf,
    buffer_version: u64,
    git_revision: u64,
}

#[derive(Debug, Default)]
struct ExplorerRenderCache {
    key: Option<ExplorerRenderCacheKey>,
    rows: Vec<ExplorerCachedRow>,
    executable_by_path: HashMap<PathBuf, bool>,
    show_git_status_column: bool,
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

    pub(crate) fn refresh_explorer_render_model(&mut self) -> bool {
        let Some(explorer) = self.explorer.as_ref() else {
            return false;
        };
        if explorer.buffer_id != self.session.active_id() {
            return false;
        }

        let buffer_id = explorer.buffer_id;
        let dir_path = explorer.dir_path.clone();
        let Some(buffer_version) = self
            .views
            .get(&buffer_id)
            .map(BufferViewState::analysis_version)
        else {
            return false;
        };
        let key = ExplorerRenderCacheKey {
            dir_path: dir_path.clone(),
            buffer_version,
            git_revision: self.git.repo_status_revision(),
        };
        if explorer.render_cache.key.as_ref() == Some(&key) {
            return explorer.render_cache.show_git_status_column;
        }

        let Some(buffer) = self.session.buffer(buffer_id) else {
            return false;
        };
        let rows = (0..buffer.len_lines())
            .map(|line_index| {
                let source_slice = buffer.line_slice(line_index);
                let owned_source;
                let source = match source_slice.as_str() {
                    Some(source) => source,
                    None => {
                        owned_source = source_slice.to_string();
                        &owned_source
                    }
                };
                explorer_cached_row(&dir_path, source, |path| self.git.status_for_path(path))
            })
            .collect::<Vec<_>>();
        let show_git_status_column = rows.iter().any(|row| row.git_status.is_some());

        let explorer = self
            .explorer
            .as_mut()
            .expect("explorer state must remain available while rendering");
        let preserve_executable_cache = explorer.render_cache.key.as_ref().is_some_and(|old_key| {
            old_key.dir_path == key.dir_path && old_key.buffer_version == key.buffer_version
        });
        if !preserve_executable_cache {
            explorer.render_cache.executable_by_path.clear();
        }
        explorer.render_cache.key = Some(key);
        explorer.render_cache.rows = rows;
        explorer.render_cache.show_git_status_column = show_git_status_column;
        show_git_status_column
    }

    pub(crate) fn explorer_render_rows(
        &mut self,
        first_line: usize,
        line_count: usize,
    ) -> Vec<ExplorerRenderRow> {
        let Some(explorer) = self.explorer.as_mut() else {
            return Vec::new();
        };
        let cache = &mut explorer.render_cache;
        let end_line = first_line.saturating_add(line_count).min(cache.rows.len());
        let (rows, executable_by_path) = (&cache.rows, &mut cache.executable_by_path);

        rows[first_line.min(end_line)..end_line]
            .iter()
            .map(|row| {
                let kind = if row.kind == ExplorerRenderRowKind::File
                    && row.executable_path.as_ref().is_some_and(|path| {
                        if let Some(cached) = executable_by_path.get(path) {
                            *cached
                        } else {
                            let executable = is_executable(path);
                            executable_by_path.insert(path.clone(), executable);
                            executable
                        }
                    }) {
                    ExplorerRenderRowKind::Executable
                } else {
                    row.kind
                };
                ExplorerRenderRow {
                    kind,
                    git_status: row.git_status,
                }
            })
            .collect()
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
            && self.dashboard_selection_for_buffer(background_id).is_none()
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
            Err(error) => {
                self.set_status(format!("explorer open failed: {error}"));
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
            render_cache: ExplorerRenderCache::default(),
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

    pub(super) fn explorer_has_unsaved_changes(&self) -> bool {
        self.explorer.as_ref().is_some_and(|explorer| {
            let Some(buffer) = self.session.buffer(explorer.buffer_id) else {
                return false;
            };
            let current_text = buffer.to_string();
            explorer.directory_drafts.iter().any(|(directory, draft)| {
                let text = if *directory == explorer.dir_path {
                    &current_text
                } else {
                    &draft.text
                };
                *text != explorer_entries_to_text(&draft.original_entries)
            })
        })
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
            Err(error) => {
                self.set_status(format!("explorer parse error: {error}"));
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
            if let Err(error) = self.refresh_explorer_directory(next_dir) {
                self.set_status(format!("explorer open failed: {error}"));
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
            Err(error) => {
                self.set_status(format!("open failed: {error}"));
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
        if let Err(error) =
            self.refresh_explorer_directory_with_selection(parent, previous_dir_name)
        {
            self.set_status(format!("explorer open failed: {error}"));
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
                Err(error) => {
                    self.explorer_delete_confirmation_token = None;
                    self.set_status(format!("explorer parse error: {error}"));
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
                Err(error) => {
                    self.explorer_delete_confirmation_token = None;
                    self.set_status(format!("explorer write failed: {error}"));
                    return false;
                }
            };

        if let Err(error) = execute_explorer_write(&planned_write) {
            self.explorer_delete_confirmation_token = None;
            self.set_status(format!("explorer write failed: {error}"));
            return false;
        }

        self.sync_session_after_explorer_write(&planned_write.changes);
        let pin_sync_error = self.sync_pinned_files_after_explorer_write(&planned_write.changes);
        self.mark_git_repo_statuses_stale();

        let refreshed_entries = match list_explorer_entries(&explorer.dir_path) {
            Ok(entries) => entries,
            Err(error) => {
                self.set_status(format!("explorer refresh failed: {error}"));
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
        if let Some(error) = pin_sync_error {
            self.set_status(format!("{status}; pin save failed: {error}"));
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

fn explorer_cached_row(
    dir_path: &Path,
    source_line: &str,
    git_status_for_path: impl FnOnce(&Path) -> Option<GitFileStatusKind>,
) -> ExplorerCachedRow {
    let line = source_line.trim();
    if line.is_empty() {
        return ExplorerCachedRow {
            kind: ExplorerRenderRowKind::File,
            executable_path: None,
            git_status: None,
        };
    }
    let is_dir = line == ".." || line.ends_with('/');
    let name = line.strip_suffix('/').unwrap_or(line);
    let kind = if name.starts_with('.') {
        ExplorerRenderRowKind::Hidden
    } else if is_dir {
        ExplorerRenderRowKind::Directory
    } else {
        ExplorerRenderRowKind::File
    };
    let entry_path = (line != "..").then(|| dir_path.join(name));
    let git_status = entry_path.as_deref().and_then(git_status_for_path);
    let executable_path = if kind == ExplorerRenderRowKind::File {
        entry_path
    } else {
        None
    };

    ExplorerCachedRow {
        kind,
        executable_path,
        git_status,
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
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
    diff_explorer_entries(old_entries, new_entries).deletions
}

fn explorer_delete_confirmation_token_for_drafts(
    drafts: &HashMap<PathBuf, ExplorerDirectoryDraft>,
) -> String {
    let mut dirs: Vec<_> = drafts.iter().collect();
    dirs.sort_by_key(|(dir_path, _)| dir_path.as_path());
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

    let mut seen_names = HashSet::new();
    for entry in new_entries.iter().filter(|entry| !entry.is_parent) {
        if !seen_names.insert(entry.name.as_str()) {
            anyhow::bail!("duplicate entry name '{}'", entry.name);
        }
    }

    let mut renames = Vec::new();
    let ExplorerEntryDiff {
        renames: rename_pairs,
        deletions,
        creations,
    } = diff_explorer_entries(old_entries, new_entries);
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
        // A case-only rename on a case-insensitive filesystem "finds" its own source.
        if rename.new_path.exists()
            && !rename_sources.contains(&rename.new_path)
            && !same_filesystem_entry(&rename.new_path, &current_old_path)
        {
            anyhow::bail!("rename target '{}' already exists", rename.new_name);
        }
    }

    // New names never reuse anything on disk. Case-insensitive filesystems treat
    // `Readme.md` as an existing `README.md`, so check the filesystem, not the listing.
    for entry in &creations {
        let occupied = [current_dir_path, resolved_dir_path]
            .iter()
            .any(|dir| fs::symlink_metadata(explorer_entry_path(dir, &entry.name)).is_ok());
        if occupied {
            anyhow::bail!("'{}' already exists", entry.name);
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

/// A completed filesystem step that can be reversed if a later step fails.
#[derive(Debug)]
enum ExplorerUndoStep {
    Moved { from: PathBuf, to: PathBuf },
    CreatedFile(PathBuf),
    CreatedDir(PathBuf),
}

/// Apply a planned write as one transaction. Deleted entries are first moved aside,
/// so every step stays reversible until all renames and creations have succeeded.
fn execute_explorer_write(planned_write: &PlannedExplorerWrite) -> anyhow::Result<()> {
    let mut journal = Vec::new();
    let staged_deletions = match apply_reversible_explorer_steps(planned_write, &mut journal) {
        Ok(staged) => staged,
        Err(error) => {
            let stranded = roll_back_explorer_steps(journal);
            if stranded.is_empty() {
                anyhow::bail!("{error:#}; no changes were made");
            }
            anyhow::bail!(
                "{error:#}; rollback incomplete, check {}",
                stranded.join(", ")
            );
        }
    };

    for (staged, is_dir) in staged_deletions {
        let removed = if is_dir {
            fs::remove_dir_all(&staged)
        } else {
            fs::remove_file(&staged)
        };
        removed.map_err(|error| {
            anyhow::anyhow!("could not finish deleting {}: {error}", staged.display())
        })?;
    }
    Ok(())
}

fn apply_reversible_explorer_steps(
    planned_write: &PlannedExplorerWrite,
    journal: &mut Vec<ExplorerUndoStep>,
) -> anyhow::Result<Vec<(PathBuf, bool)>> {
    for dir_write in &planned_write.dir_writes {
        for rename in &dir_write.renames {
            move_entry(&rename.old_path, &rename.temp_path, journal)?;
        }
        for rename in &dir_write.renames {
            if let Some(parent) = rename.new_path.parent() {
                create_dirs(parent, journal)?;
            }
            if rename.new_path.exists() {
                anyhow::bail!("rename target '{}' already exists", rename.new_name);
            }
            move_entry(&rename.temp_path, &rename.new_path, journal)?;
        }
    }

    let mut staged_deletions = Vec::new();
    for deleted in &planned_write.changes.deleted_entries {
        let staged = unused_sibling_path(&deleted.path, "redox_delete_tmp")?;
        move_entry(&deleted.path, &staged, journal)?;
        staged_deletions.push((staged, deleted.is_dir));
    }

    for created in &planned_write.changes.created_entries {
        if created.is_dir {
            create_dirs(&created.path, journal)?;
            continue;
        }
        if let Some(parent) = created.path.parent() {
            create_dirs(parent, journal)?;
        }
        // Never truncate: the path may have appeared since planning.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&created.path)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => {
                    anyhow::anyhow!("'{}' already exists", created.name)
                }
                _ => error.into(),
            })?;
        journal.push(ExplorerUndoStep::CreatedFile(created.path.clone()));
    }

    Ok(staged_deletions)
}

fn move_entry(from: &Path, to: &Path, journal: &mut Vec<ExplorerUndoStep>) -> anyhow::Result<()> {
    fs::rename(from, to)
        .map_err(|error| anyhow::anyhow!("could not move {}: {error}", from.display()))?;
    journal.push(ExplorerUndoStep::Moved {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
    });
    Ok(())
}

/// Create `dir` and any missing ancestors, journaling each directory actually created.
fn create_dirs(dir: &Path, journal: &mut Vec<ExplorerUndoStep>) -> anyhow::Result<()> {
    let missing: Vec<&Path> = dir
        .ancestors()
        .take_while(|ancestor| fs::symlink_metadata(ancestor).is_err())
        .collect();
    for created in missing.into_iter().rev() {
        fs::create_dir(created)
            .map_err(|error| anyhow::anyhow!("could not create {}: {error}", created.display()))?;
        journal.push(ExplorerUndoStep::CreatedDir(created.to_path_buf()));
    }
    Ok(())
}

fn unused_sibling_path(path: &Path, prefix: &str) -> anyhow::Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    (0..10_000)
        .map(|attempt| parent.join(format!(".{prefix}_{attempt}")))
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
        .ok_or_else(|| anyhow::anyhow!("failed to allocate a temporary path"))
}

/// Undo completed steps newest first. Return the paths that could not be restored.
fn roll_back_explorer_steps(journal: Vec<ExplorerUndoStep>) -> Vec<String> {
    let mut stranded = Vec::new();
    for step in journal.into_iter().rev() {
        match step {
            ExplorerUndoStep::Moved { from, to } => {
                if fs::rename(&to, &from).is_err() {
                    stranded.push(format!("{} (was {})", to.display(), from.display()));
                }
            }
            ExplorerUndoStep::CreatedFile(path) => {
                // Only an empty file was created; leave it if something wrote to it since.
                if fs::metadata(&path).is_ok_and(|metadata| metadata.len() == 0) {
                    let _ = fs::remove_file(&path);
                }
            }
            ExplorerUndoStep::CreatedDir(path) => {
                // Fails harmlessly if anything else now lives inside it.
                let _ = fs::remove_dir(&path);
            }
        }
    }
    stranded
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
    let total = targets.len();
    let more = if total >= DELETE_TARGET_SCAN_LIMIT {
        "+"
    } else {
        ""
    };
    let noun = if total == 1 { "entry" } else { "entries" };
    let mut lines = vec![(
        format!("confirm deletion of {total}{more} {noun}:"),
        StatusMessageStyle::Normal,
    )];
    lines.extend(
        targets
            .into_iter()
            .take(DELETE_TARGET_DISPLAY_LIMIT)
            .map(|target| (format!(" {target}"), StatusMessageStyle::Normal)),
    );
    if total > DELETE_TARGET_DISPLAY_LIMIT {
        lines.push((
            format!(" …and {}{more} more", total - DELETE_TARGET_DISPLAY_LIMIT),
            StatusMessageStyle::Dim,
        ));
    }
    lines.push(("press y".to_string(), StatusMessageStyle::Dim));
    lines
}

fn explorer_delete_confirmation_targets(
    dir_path: &Path,
    deletions: &[ExplorerEntry],
) -> Vec<String> {
    let mut targets = Vec::new();
    for entry in deletions {
        let path = explorer_entry_path(dir_path, &entry.name);
        let descend = entry.is_dir && is_real_directory(&path);
        targets.push(explorer_entry_label(&entry.name, descend));
        if descend {
            collect_directory_delete_targets(dir_path, &path, &mut targets);
        }
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
        if targets.len() >= DELETE_TARGET_SCAN_LIMIT {
            return;
        }
        let child_path = child.path();
        let relative_name = child_path
            .strip_prefix(root_dir)
            .ok()
            .map(|path| path.to_string_lossy().replace('\\', "/"));
        // Deletion removes symlinks themselves, so never list or descend into their targets.
        let is_dir = child.file_type().is_ok_and(|file_type| file_type.is_dir());
        if let Some(relative_name) = relative_name {
            targets.push(explorer_entry_label(&relative_name, is_dir));
        }
        if is_dir {
            collect_directory_delete_targets(root_dir, &child_path, targets);
        }
    }
}

fn is_real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
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

struct ExplorerEntryDiff {
    renames: Vec<(ExplorerEntry, ExplorerEntry)>,
    deletions: Vec<ExplorerEntry>,
    creations: Vec<ExplorerEntry>,
}

/// Compare an edited listing with the original. Unchanged names anchor the listing:
/// a vanished and an added entry of the same kind are a rename only when they sit
/// between the same unchanged neighbours, so deleting one line while typing an
/// unrelated name elsewhere stays a confirmed deletion plus a creation.
fn diff_explorer_entries(
    old_entries: &[ExplorerEntry],
    new_entries: &[ExplorerEntry],
) -> ExplorerEntryDiff {
    fn gapped<'a>(
        entries: &'a [ExplorerEntry],
        others: &HashSet<&str>,
    ) -> Vec<(Option<&'a str>, &'a ExplorerEntry)> {
        let mut anchor = None;
        let mut changed = Vec::new();
        for entry in entries.iter().filter(|entry| !entry.is_parent) {
            if others.contains(entry.name.as_str()) {
                anchor = Some(entry.name.as_str());
            } else {
                changed.push((anchor, entry));
            }
        }
        changed
    }

    fn names(entries: &[ExplorerEntry]) -> HashSet<&str> {
        entries
            .iter()
            .filter(|entry| !entry.is_parent)
            .map(|entry| entry.name.as_str())
            .collect()
    }
    let old_names = names(old_entries);
    let new_names = names(new_entries);

    let old_missing = gapped(old_entries, &new_names);
    let mut new_added: Vec<_> = gapped(new_entries, &old_names)
        .into_iter()
        .map(|added| (added, false))
        .collect();

    let mut diff = ExplorerEntryDiff {
        renames: Vec::new(),
        deletions: Vec::new(),
        creations: Vec::new(),
    };
    for (gap, old) in old_missing {
        let partner = new_added.iter_mut().find(|((added_gap, added), paired)| {
            !*paired && *added_gap == gap && added.is_dir == old.is_dir
        });
        match partner {
            Some(((_, new), paired)) => {
                *paired = true;
                diff.renames.push((old.clone(), (*new).clone()));
            }
            None => diff.deletions.push(old.clone()),
        }
    }
    diff.creations = new_added
        .into_iter()
        .filter(|(_, paired)| !paired)
        .map(|((_, entry), _)| entry.clone())
        .collect();
    diff
}

/// Whether two paths name the same file, as a case-insensitive filesystem can make them.
fn same_filesystem_entry(left: &Path, right: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (Ok(left), Ok(right)) = (fs::symlink_metadata(left), fs::symlink_metadata(right))
        else {
            return false;
        };
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        matches!(
            (fs::canonicalize(left), fs::canonicalize(right)),
            (Ok(left), Ok(right)) if left == right
        )
    }
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

    fn execute_plan(planned: PlannedExplorerDirWrite) -> anyhow::Result<()> {
        execute_explorer_write(&PlannedExplorerWrite {
            changes: planned.changes.clone(),
            dir_writes: vec![planned],
        })
    }

    #[test]
    fn creating_over_an_unlisted_existing_file_is_rejected_without_truncating() {
        let root = temp_dir_path("create_existing");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        // Appeared on disk after the listing was taken.
        fs::write(root.join("notes.txt"), "keep me").expect("failed to write fixture");

        let error = apply_explorer_changes(&root, &[], &[file_entry("notes.txt")])
            .expect_err("planning must refuse to create over an existing file");
        assert!(error.to_string().contains("already exists"), "{error}");
        assert_eq!(
            fs::read_to_string(root.join("notes.txt")).unwrap(),
            "keep me"
        );

        // A path that appears between planning and execution is not truncated either.
        let planned = apply_explorer_changes(&root, &[], &[file_entry("late.txt")]).unwrap();
        fs::write(root.join("late.txt"), "raced").unwrap();
        let error = execute_plan(planned).expect_err("execution must not overwrite");
        assert!(error.to_string().contains("already exists"), "{error}");
        assert_eq!(fs::read_to_string(root.join("late.txt")).unwrap(), "raced");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn creating_a_case_variant_never_truncates_on_case_insensitive_filesystems() {
        let root = temp_dir_path("create_case_variant");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("README.md"), "important").expect("failed to write fixture");
        let case_insensitive = root.join("readme.md").exists();

        let old_entries = vec![file_entry("README.md")];
        let new_entries = vec![file_entry("README.md"), file_entry("Readme.md")];
        let planned = apply_explorer_changes(&root, &old_entries, &new_entries);
        if case_insensitive {
            assert!(
                planned.is_err(),
                "case variant of an existing file must be refused"
            );
        } else {
            execute_plan(planned.unwrap()).unwrap();
            assert_eq!(fs::read_to_string(root.join("Readme.md")).unwrap(), "");
        }
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "important"
        );

        let _ = fs::remove_dir_all(root);
    }

    fn directory_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn failed_rename_rolls_back_without_stranding_temporary_files() {
        let root = temp_dir_path("rollback_rename");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("a.txt"), "a").unwrap();

        let planned =
            apply_explorer_changes(&root, &[file_entry("a.txt")], &[file_entry("b.txt")]).unwrap();
        fs::write(root.join("b.txt"), "someone else's").unwrap();
        let error = execute_plan(planned).expect_err("the rename target appeared");
        assert!(
            format!("{error:#}").contains("no changes were made"),
            "{error:#}"
        );

        assert_eq!(directory_names(&root), ["a.txt", "b.txt"]);
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "a");
        assert_eq!(
            fs::read_to_string(root.join("b.txt")).unwrap(),
            "someone else's"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failure_after_renames_and_deletes_restores_everything() {
        let root = temp_dir_path("rollback_all");
        fs::create_dir_all(root.join("doomed")).expect("failed to create fixture root");
        fs::write(root.join("doomed/inner.txt"), "inner").unwrap();
        fs::write(root.join("old.txt"), "old").unwrap();
        fs::write(root.join("gone.txt"), "gone").unwrap();

        // Rename old->renamed, delete gone.txt and doomed/, create nested/new.txt.
        let old_entries = vec![
            dir_entry("doomed"),
            file_entry("gone.txt"),
            file_entry("old.txt"),
        ];
        let new_entries = vec![file_entry("nested/new.txt"), file_entry("renamed.txt")];
        let mut planned = apply_explorer_changes(&root, &old_entries, &new_entries).unwrap();
        // Force the final step to fail by colliding with a file created after planning.
        planned
            .changes
            .created_entries
            .push(AppliedExplorerEntryChange {
                name: "late.txt".to_string(),
                path: root.join("late.txt"),
                is_dir: false,
            });
        fs::write(root.join("late.txt"), "late").unwrap();

        let error = execute_plan(planned).expect_err("the last creation must fail");
        assert!(
            format!("{error:#}").contains("no changes were made"),
            "{error:#}"
        );
        assert_eq!(
            directory_names(&root),
            ["doomed", "gone.txt", "late.txt", "old.txt"]
        );
        assert_eq!(fs::read_to_string(root.join("old.txt")).unwrap(), "old");
        assert_eq!(fs::read_to_string(root.join("gone.txt")).unwrap(), "gone");
        assert_eq!(
            fs::read_to_string(root.join("doomed/inner.txt")).unwrap(),
            "inner"
        );
        assert_eq!(fs::read_to_string(root.join("late.txt")).unwrap(), "late");

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn delete_confirmation_lists_symlinks_without_following_them() {
        let root = temp_dir_path("delete_symlinks");
        fs::create_dir_all(root.join("doomed")).expect("failed to create fixture root");
        fs::create_dir_all(root.join("elsewhere")).unwrap();
        fs::write(root.join("elsewhere/precious.txt"), "precious").unwrap();
        // A loop back to an ancestor used to recurse until the stack overflowed.
        std::os::unix::fs::symlink(&root, root.join("doomed/loop")).unwrap();
        std::os::unix::fs::symlink(root.join("elsewhere"), root.join("linked")).unwrap();

        let targets = explorer_delete_confirmation_targets(
            &root,
            &[dir_entry("doomed"), dir_entry("linked")],
        );
        assert_eq!(targets, ["linked", "doomed/", "doomed/loop"]);

        let planned = apply_explorer_changes(
            &root,
            &[
                dir_entry("doomed"),
                dir_entry("elsewhere"),
                dir_entry("linked"),
            ],
            &[dir_entry("elsewhere")],
        )
        .unwrap();
        execute_plan(planned).unwrap();
        assert_eq!(directory_names(&root), ["elsewhere"]);
        assert_eq!(
            fs::read_to_string(root.join("elsewhere/precious.txt")).unwrap(),
            "precious"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_confirmation_summarises_large_trees() {
        let root = temp_dir_path("delete_large");
        fs::create_dir_all(root.join("big")).expect("failed to create fixture root");
        for index in 0..100 {
            fs::write(root.join(format!("big/{index:03}.txt")), "").unwrap();
        }

        let lines =
            format_explorer_delete_confirmation_lines(&[(root.clone(), vec![dir_entry("big")])]);
        assert_eq!(lines[0].0, "confirm deletion of 101 entries:");
        assert_eq!(lines.len(), 1 + DELETE_TARGET_DISPLAY_LIMIT + 2);
        assert_eq!(lines[lines.len() - 2].0, " …and 61 more");

        let _ = fs::remove_dir_all(root);
    }

    fn names_of(entries: &[ExplorerEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn unrelated_delete_and_create_are_not_treated_as_a_rename() {
        let old = vec![
            file_entry("a.txt"),
            file_entry("b.txt"),
            file_entry("c.txt"),
        ];
        let new = vec![
            file_entry("b.txt"),
            file_entry("c.txt"),
            file_entry("new.txt"),
        ];

        let diff = diff_explorer_entries(&old, &new);
        assert!(diff.renames.is_empty());
        assert_eq!(names_of(&diff.deletions), ["a.txt"]);
        assert_eq!(names_of(&diff.creations), ["new.txt"]);
        // The confirmation prompt sees exactly the same deletion.
        assert_eq!(names_of(&pending_explorer_deletions(&old, &new)), ["a.txt"]);
    }

    #[test]
    fn renames_survive_deletions_elsewhere_in_the_listing() {
        let old = vec![
            file_entry("a.txt"),
            file_entry("b.txt"),
            file_entry("c.txt"),
            file_entry("d.txt"),
        ];
        let new = vec![
            file_entry("b.txt"),
            file_entry("x.txt"),
            file_entry("d.txt"),
        ];

        let diff = diff_explorer_entries(&old, &new);
        let renames: Vec<_> = diff
            .renames
            .iter()
            .map(|(old, new)| (old.name.as_str(), new.name.as_str()))
            .collect();
        assert_eq!(renames, [("c.txt", "x.txt")]);
        assert_eq!(names_of(&diff.deletions), ["a.txt"]);
        assert!(diff.creations.is_empty());
    }

    #[test]
    fn renames_only_pair_entries_of_the_same_kind() {
        let old = vec![dir_entry("docs"), file_entry("keep.txt")];
        let new = vec![file_entry("notes.txt"), file_entry("keep.txt")];

        let diff = diff_explorer_entries(&old, &new);
        assert!(diff.renames.is_empty());
        assert_eq!(names_of(&diff.deletions), ["docs"]);
        assert_eq!(names_of(&diff.creations), ["notes.txt"]);
    }

    #[test]
    fn case_only_rename_works_on_every_filesystem() {
        let root = temp_dir_path("case_rename");
        fs::create_dir_all(&root).expect("failed to create fixture root");
        fs::write(root.join("readme.md"), "contents").unwrap();

        let planned = apply_explorer_changes(
            &root,
            &[file_entry("readme.md")],
            &[file_entry("README.md")],
        )
        .expect("a case-only rename must be allowed");
        execute_plan(planned).unwrap();

        assert_eq!(directory_names(&root), ["README.md"]);
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "contents"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn creating_new_files_and_directories_still_works() {
        let root = temp_dir_path("create_new");
        fs::create_dir_all(&root).expect("failed to create fixture root");

        let new_entries = vec![dir_entry("src"), file_entry("src/main.rs")];
        let planned = apply_explorer_changes(&root, &[], &new_entries).unwrap();
        execute_plan(planned).unwrap();

        assert!(root.join("src").is_dir());
        assert_eq!(fs::read_to_string(root.join("src/main.rs")).unwrap(), "");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explorer_render_rows_classify_entries_and_keep_git_status() {
        let root = Path::new("/repo");
        let directory = explorer_cached_row(root, "src/", |_| Some(GitFileStatusKind::Added));
        let hidden = explorer_cached_row(root, ".env", |_| Some(GitFileStatusKind::Modified));
        let file = explorer_cached_row(root, "main.rs", |_| None);

        assert_eq!(directory.kind, ExplorerRenderRowKind::Directory);
        assert_eq!(directory.git_status, Some(GitFileStatusKind::Added));
        assert_eq!(hidden.kind, ExplorerRenderRowKind::Hidden);
        assert_eq!(hidden.git_status, Some(GitFileStatusKind::Modified));
        assert_eq!(file.kind, ExplorerRenderRowKind::File);
        assert_eq!(
            file.executable_path.as_deref(),
            Some(root.join("main.rs").as_path())
        );
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
        let error = parse_explorer_entries("../\npath/../file.txt")
            .expect_err("expected parent segments to be rejected");

        assert!(
            error
                .to_string()
                .contains("..' path segments are not allowed")
        );
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
