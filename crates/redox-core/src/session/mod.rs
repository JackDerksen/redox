//! Multi-buffer session model for higher-level editor frontends.

mod loading;

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};

use self::loading::IncrementalFileLoader;
use crate::TextBuffer;

const INITIAL_LOAD_BYTES: usize = 64 * 1024;
const FULL_LOAD_CHUNK_BYTES: usize = 64 * 1024;

/// Stable buffer identifier within an [`EditorSession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BufferId(u64);

impl BufferId {
    /// Return the stable numeric identifier for this buffer.
    #[inline]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Buffer classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferKind {
    /// File-backed editable buffer.
    File,
    /// Ephemeral UI buffer for editor surfaces.
    Ui,
}

/// Buffer metadata tracked by session management.
#[derive(Debug, Clone)]
pub struct BufferMeta {
    /// Stable identifier for the buffer.
    pub id: BufferId,
    /// Whether the buffer is file-backed or an ephemeral UI surface.
    pub kind: BufferKind,
    /// Human-readable name shown by editor frontends.
    pub display_name: String,
    /// Filesystem path for file-backed buffers.
    pub path: Option<PathBuf>,
    /// Whether current contents differ from the last clean snapshot.
    pub dirty: bool,
    /// Whether the backing file changed after this buffer diverged locally.
    pub external_changed: bool,
    /// Whether this buffer represents a missing path that has not been saved yet.
    pub is_new_file: bool,
}

/// Listing row for buffer UIs such as `:ls`.
#[derive(Debug, Clone)]
pub struct BufferSummary {
    /// Stable identifier for the buffer.
    pub id: BufferId,
    /// Whether the buffer is file-backed or an ephemeral UI surface.
    pub kind: BufferKind,
    /// Human-readable name shown by editor frontends.
    pub display_name: String,
    /// Filesystem path for file-backed buffers.
    pub path: Option<PathBuf>,
    /// Whether current contents differ from the last clean snapshot.
    pub dirty: bool,
    /// Whether the backing file changed after this buffer diverged locally.
    pub external_changed: bool,
    /// Whether this buffer represents a missing path that has not been saved yet.
    pub is_new_file: bool,
    /// Whether this row describes the active buffer.
    pub is_active: bool,
}

/// Loading phase for a file-backed buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferLoadPhase {
    /// The buffer is not currently loading from disk.
    NotLoading,
    /// Incremental file loading is still in progress.
    Loading,
    /// File loading completed successfully.
    Complete,
    /// File loading failed.
    Failed,
}

/// Snapshot status for file loading progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferLoadStatus {
    /// Current loading phase.
    pub phase: BufferLoadPhase,
    /// Number of bytes loaded so far.
    pub bytes_loaded: usize,
    /// Total file size in bytes, when known.
    pub total_bytes: Option<usize>,
    /// Failure detail when `phase` is [`BufferLoadPhase::Failed`].
    pub error: Option<String>,
}

impl BufferLoadStatus {
    #[inline]
    fn not_loading() -> Self {
        Self {
            phase: BufferLoadPhase::NotLoading,
            bytes_loaded: 0,
            total_bytes: None,
            error: None,
        }
    }
}

#[derive(Debug)]
struct BufferRecord {
    meta: BufferMeta,
    buffer: TextBuffer,
    clean_fingerprint: u64,
    clean_normalized_len_chars: usize,
    disk_stamp: Option<FileStamp>,
    loader: Option<IncrementalFileLoader>,
    load_status: BufferLoadStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

/// A file-backed buffer changed outside the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFileChange {
    /// Buffer that observed the external change.
    pub id: BufferId,
    /// Display name for status UIs.
    pub display_name: String,
    /// Backing path, when the buffer still has one.
    pub path: Option<PathBuf>,
    /// How the session handled the change.
    pub kind: ExternalFileChangeKind,
}

/// How an external file change was reconciled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalFileChangeKind {
    /// The clean in-memory buffer was replaced with disk contents.
    Reloaded,
    /// The buffer has local edits, so the disk change was left unresolved.
    Conflict,
    /// The backing path disappeared while the buffer was open.
    Deleted,
    /// Redox noticed the change but could not read the file.
    Failed,
}

/// Multi-buffer editor session with active buffer + MRU ordering.
#[derive(Debug)]
pub struct EditorSession {
    buffers: HashMap<BufferId, BufferRecord>,
    path_index: HashMap<PathBuf, BufferId>,
    mru: Vec<BufferId>,
    active: Option<BufferId>,
    next_id: u64,
    launch_dir: PathBuf,
}

/// Result of reconciling file-backed buffers with paths discovered on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePathSyncResult {
    /// Buffers whose path metadata was updated.
    pub remapped_ids: Vec<BufferId>,
    /// Buffers closed because their backing paths disappeared.
    pub closed_ids: Vec<BufferId>,
}

impl Default for EditorSession {
    fn default() -> Self {
        Self {
            buffers: HashMap::new(),
            path_index: HashMap::new(),
            mru: Vec::new(),
            active: None,
            next_id: 0,
            launch_dir: PathBuf::new(),
        }
    }
}

impl EditorSession {
    /// Build a new session with a single initial file buffer.
    pub fn open_initial_file(path: impl AsRef<Path>) -> Result<Self> {
        let launch_dir = std::env::current_dir().context("failed to resolve current directory")?;
        let launch_dir = std::fs::canonicalize(&launch_dir).unwrap_or(launch_dir);

        let mut session = Self {
            launch_dir,
            ..Self::default()
        };
        let _ = session.open_file(path)?;
        Ok(session)
    }

    /// Build a new session with one empty unnamed file buffer.
    pub fn open_initial_unnamed() -> Result<Self> {
        let launch_dir = std::env::current_dir().context("failed to resolve current directory")?;
        let launch_dir = std::fs::canonicalize(&launch_dir).unwrap_or(launch_dir);

        let mut session = Self {
            launch_dir,
            ..Self::default()
        };
        session.open_unnamed_buffer();
        Ok(session)
    }

    /// Open (or switch to) a file-backed buffer.
    ///
    /// - Existing open path: activates and returns existing buffer ID.
    /// - Missing path: creates an empty file-backed buffer marked as new.
    pub fn open_file(&mut self, path: impl AsRef<Path>) -> Result<BufferId> {
        let normalized = normalize_path(path.as_ref())?;

        if let Some(existing) = self.path_index.get(&normalized).copied() {
            let _ = self.activate(existing);
            return Ok(existing);
        }

        let file_exists = normalized.exists();
        let mut buffer = TextBuffer::new();
        let mut loader = None;
        let mut load_status = BufferLoadStatus::not_loading();

        if file_exists {
            let mut incremental = IncrementalFileLoader::open(&normalized)?;
            load_status = BufferLoadStatus {
                phase: BufferLoadPhase::Loading,
                bytes_loaded: 0,
                total_bytes: incremental.total_bytes(),
                error: None,
            };

            match incremental.read_chunk(INITIAL_LOAD_BYTES) {
                Ok(chunk) => {
                    if !chunk.text.is_empty() {
                        let at = buffer.len_chars();
                        buffer.rope_mut().insert(at, &chunk.text);
                    }

                    load_status.bytes_loaded = incremental.bytes_loaded();
                    load_status.total_bytes = incremental.total_bytes();
                    if chunk.eof {
                        load_status.phase = BufferLoadPhase::Complete;
                    } else {
                        load_status.phase = BufferLoadPhase::Loading;
                        loader = Some(incremental);
                    }
                }
                Err(err) => {
                    load_status.phase = BufferLoadPhase::Failed;
                    load_status.error = Some(err.to_string());
                    load_status.bytes_loaded = incremental.bytes_loaded();
                    load_status.total_bytes = incremental.total_bytes();
                }
            }
        }

        let id = self.alloc_id();
        let meta = BufferMeta {
            id,
            kind: BufferKind::File,
            display_name: self.display_path(&normalized),
            path: Some(normalized.clone()),
            dirty: false,
            external_changed: false,
            is_new_file: !file_exists,
        };
        let disk_stamp = file_exists.then(|| file_stamp(&normalized).ok()).flatten();
        let (clean_fingerprint, clean_normalized_len_chars) =
            if matches!(load_status.phase, BufferLoadPhase::Complete) {
                normalized_content_fingerprint(&buffer)
            } else {
                normalized_text_fingerprint("")
            };

        self.buffers.insert(
            id,
            BufferRecord {
                meta,
                buffer,
                clean_fingerprint,
                clean_normalized_len_chars,
                disk_stamp,
                loader,
                load_status,
            },
        );
        self.path_index.insert(normalized, id);
        let _ = self.activate(id);

        Ok(id)
    }

    /// Open an in-memory UI buffer.
    pub fn open_ui_buffer(&mut self, name: impl Into<String>, initial_text: &str) -> BufferId {
        let id = self.alloc_id();
        let (clean_fingerprint, clean_normalized_len_chars) =
            normalized_text_fingerprint(initial_text);
        let meta = BufferMeta {
            id,
            kind: BufferKind::Ui,
            display_name: name.into(),
            path: None,
            dirty: false,
            external_changed: false,
            is_new_file: false,
        };

        self.buffers.insert(
            id,
            BufferRecord {
                meta,
                buffer: TextBuffer::from_str(initial_text),
                clean_fingerprint,
                clean_normalized_len_chars,
                disk_stamp: None,
                loader: None,
                load_status: BufferLoadStatus::not_loading(),
            },
        );
        let _ = self.activate(id);

        id
    }

    /// Open a new unnamed file buffer and activate it.
    pub fn open_unnamed_buffer(&mut self) -> BufferId {
        let id = self.alloc_id();
        let (clean_fingerprint, clean_normalized_len_chars) = normalized_text_fingerprint("");
        let meta = BufferMeta {
            id,
            kind: BufferKind::File,
            display_name: "[No Name]".to_string(),
            path: None,
            dirty: false,
            external_changed: false,
            is_new_file: true,
        };

        self.buffers.insert(
            id,
            BufferRecord {
                meta,
                buffer: TextBuffer::new(),
                clean_fingerprint,
                clean_normalized_len_chars,
                disk_stamp: None,
                loader: None,
                load_status: BufferLoadStatus::not_loading(),
            },
        );
        let _ = self.activate(id);
        id
    }

    #[inline]
    pub fn active_id(&self) -> BufferId {
        self.active
            .expect("editor session must always have an active buffer")
    }

    /// Activate the target buffer and promote it to the top of MRU order.
    pub fn activate(&mut self, id: BufferId) -> bool {
        if !self.buffers.contains_key(&id) {
            return false;
        }

        self.active = Some(id);
        self.promote_mru(id);
        true
    }

    #[inline]
    pub fn active_buffer(&self) -> &TextBuffer {
        self.buffer(self.active_id())
            .expect("active buffer must exist in session map")
    }

    #[inline]
    pub fn active_buffer_mut(&mut self) -> &mut TextBuffer {
        let id = self.active_id();
        &mut self
            .buffers
            .get_mut(&id)
            .expect("active buffer must exist in session map")
            .buffer
    }

    #[inline]
    pub fn active_meta(&self) -> &BufferMeta {
        self.meta(self.active_id())
            .expect("active metadata must exist in session map")
    }

    #[inline]
    pub fn active_meta_mut(&mut self) -> &mut BufferMeta {
        let id = self.active_id();
        &mut self
            .buffers
            .get_mut(&id)
            .expect("active metadata must exist in session map")
            .meta
    }

    #[inline]
    pub fn active_buffer_load_status(&self) -> BufferLoadStatus {
        self.buffer_load_status(self.active_id())
            .unwrap_or_else(BufferLoadStatus::not_loading)
    }

    #[inline]
    pub fn active_buffer_is_fully_loaded(&self) -> bool {
        self.buffer_is_fully_loaded(self.active_id())
            .unwrap_or(true)
    }

    #[inline]
    pub fn launch_dir(&self) -> &Path {
        &self.launch_dir
    }

    #[inline]
    pub fn buffer_load_status(&self, id: BufferId) -> Option<BufferLoadStatus> {
        self.buffers.get(&id).map(|rec| rec.load_status.clone())
    }

    #[inline]
    pub fn buffer_is_fully_loaded(&self, id: BufferId) -> Option<bool> {
        self.buffers.get(&id).map(|rec| {
            matches!(
                rec.load_status.phase,
                BufferLoadPhase::NotLoading | BufferLoadPhase::Complete
            )
        })
    }

    #[inline]
    pub fn set_active_dirty(&mut self, dirty: bool) {
        self.active_meta_mut().dirty = dirty;
    }

    /// Recompute active buffer dirty state by comparing current contents against
    /// the last clean snapshot (opened-from-disk or last successful save).
    pub fn recompute_active_dirty(&mut self) -> bool {
        let id = self.active_id();
        self.recompute_buffer_dirty(id)
            .expect("active buffer must exist in session map")
    }

    /// Record the active buffer's current contents as the clean snapshot.
    pub fn mark_active_clean(&mut self) {
        let id = self.active_id();
        let rec = self
            .buffers
            .get_mut(&id)
            .expect("active buffer must exist in session map");
        let (fingerprint, normalized_len_chars) = normalized_content_fingerprint(&rec.buffer);
        rec.clean_fingerprint = fingerprint;
        rec.clean_normalized_len_chars = normalized_len_chars;
        rec.meta.dirty = false;
        rec.meta.external_changed = false;
        rec.disk_stamp = rec
            .meta
            .path
            .as_deref()
            .and_then(|path| file_stamp(path).ok());
    }

    /// Refresh the active buffer's known disk metadata after an editor-owned disk write.
    pub fn refresh_active_disk_stamp(&mut self) {
        let id = self.active_id();
        if let Some(rec) = self.buffers.get_mut(&id) {
            rec.disk_stamp = rec
                .meta
                .path
                .as_deref()
                .and_then(|path| file_stamp(path).ok());
            rec.meta.external_changed = false;
        }
    }

    #[inline]
    pub fn any_dirty(&self) -> bool {
        self.buffers.values().any(|rec| rec.meta.dirty)
    }

    /// Poll incremental loaders and append up to `max_bytes` across open buffers.
    ///
    /// Returns the number of bytes read from disk.
    pub fn poll_loading(&mut self, max_bytes: usize) -> usize {
        if max_bytes == 0 {
            return 0;
        }

        let ids: Vec<BufferId> = self.mru.clone();
        let mut remaining = max_bytes;
        let mut total_read = 0usize;

        for id in ids {
            if remaining == 0 {
                break;
            }
            let want = remaining.min(FULL_LOAD_CHUNK_BYTES);
            match self.load_step_for(id, want) {
                Ok(read) => {
                    total_read = total_read.saturating_add(read);
                    remaining = remaining.saturating_sub(read);
                }
                Err(_) => {
                    // Error status is stored in-buffer; continue polling others.
                }
            }
        }

        total_read
    }

    /// Ensure a file-backed buffer has loaded enough text to include `line`,
    /// or until the bounded read budget is exhausted.
    pub fn ensure_buffer_loaded_through_line(
        &mut self,
        id: BufferId,
        line: usize,
        max_bytes: usize,
    ) -> Result<()> {
        let mut remaining = max_bytes;

        while self
            .buffers
            .get(&id)
            .map(|rec| {
                matches!(rec.load_status.phase, BufferLoadPhase::Loading)
                    && rec.buffer.len_lines() <= line
            })
            .unwrap_or(false)
            && remaining > 0
        {
            let want = remaining.min(FULL_LOAD_CHUNK_BYTES);
            let read = self.load_step_for(id, want)?;
            if read == 0 {
                break;
            }
            remaining = remaining.saturating_sub(read);
        }

        let status = self
            .buffers
            .get(&id)
            .map(|rec| rec.load_status.clone())
            .unwrap_or_else(BufferLoadStatus::not_loading);
        if matches!(status.phase, BufferLoadPhase::Failed) {
            let msg = status
                .error
                .unwrap_or_else(|| "buffer load failed".to_string());
            bail!("{msg}");
        }
        Ok(())
    }

    /// Ensure a file-backed buffer is fully loaded to EOF.
    pub fn ensure_buffer_fully_loaded(&mut self, id: BufferId) -> Result<()> {
        loop {
            let phase = self
                .buffers
                .get(&id)
                .map(|rec| rec.load_status.phase)
                .unwrap_or(BufferLoadPhase::NotLoading);
            match phase {
                BufferLoadPhase::NotLoading | BufferLoadPhase::Complete => return Ok(()),
                BufferLoadPhase::Failed => {
                    let msg = self
                        .buffers
                        .get(&id)
                        .and_then(|rec| rec.load_status.error.clone())
                        .unwrap_or_else(|| "buffer load failed".to_string());
                    bail!("{msg}");
                }
                BufferLoadPhase::Loading => {
                    let read = self.load_step_for(id, FULL_LOAD_CHUNK_BYTES)?;
                    if read == 0 {
                        continue;
                    }
                }
            }
        }
    }

    /// Cycle to the next buffer in MRU order.
    pub fn switch_next_mru(&mut self) -> Option<BufferId> {
        if self.mru.is_empty() {
            return None;
        }

        if self.mru.len() > 1 {
            self.mru.rotate_left(1);
        }

        let id = self.mru[0];
        self.active = Some(id);
        Some(id)
    }

    /// Cycle to the previous buffer in MRU order.
    pub fn switch_prev_mru(&mut self) -> Option<BufferId> {
        if self.mru.is_empty() {
            return None;
        }

        if self.mru.len() > 1 {
            self.mru.rotate_right(1);
        }

        let id = self.mru[0];
        self.active = Some(id);
        Some(id)
    }

    pub fn summaries(&self) -> Vec<BufferSummary> {
        let active = self.active;
        self.mru
            .iter()
            .filter_map(|id| self.buffers.get(id).map(|rec| (id, rec)))
            .map(|(id, rec)| BufferSummary {
                id: *id,
                kind: rec.meta.kind,
                display_name: rec.meta.display_name.clone(),
                path: rec.meta.path.clone(),
                dirty: rec.meta.dirty,
                external_changed: rec.meta.external_changed,
                is_new_file: rec.meta.is_new_file,
                is_active: Some(*id) == active,
            })
            .collect()
    }

    /// Reconcile open file buffers after external filesystem renames or deletions.
    pub fn sync_file_buffers_with_paths(
        &mut self,
        renames: &[(PathBuf, PathBuf)],
        deletions: &[PathBuf],
    ) -> FilePathSyncResult {
        let renames: Vec<(PathBuf, PathBuf)> = renames
            .iter()
            .map(|(old_path, new_path)| {
                (normalize_sync_path(old_path), normalize_sync_path(new_path))
            })
            .collect();
        let deletions: Vec<PathBuf> = deletions
            .iter()
            .map(|path| normalize_sync_path(path))
            .collect();

        let mut remaps: Vec<(BufferId, PathBuf, PathBuf)> = Vec::new();
        let mut deletion_candidates = Vec::new();

        for (id, rec) in &self.buffers {
            let Some(path) = rec.meta.path.as_ref() else {
                continue;
            };

            let Some(next_path) = remap_synced_path(path, &renames, &deletions) else {
                deletion_candidates.push(*id);
                continue;
            };

            if next_path != *path {
                remaps.push((*id, path.clone(), next_path));
            }
        }

        let mut remapped_ids = Vec::with_capacity(remaps.len());
        let mut closed_ids = Vec::new();
        for (id, old_path, new_path) in remaps {
            let display_name = self.display_path(&new_path);
            self.path_index.remove(&old_path);
            self.path_index.insert(new_path.clone(), id);

            if let Some(rec) = self.buffers.get_mut(&id) {
                rec.meta.path = Some(new_path.clone());
                rec.meta.display_name = display_name;
                rec.disk_stamp = file_stamp(&new_path).ok();
                rec.meta.external_changed = false;
            }

            remapped_ids.push(id);
        }

        for id in deletion_candidates {
            let Some((old_path, was_dirty)) = self
                .buffers
                .get(&id)
                .and_then(|rec| rec.meta.path.clone().map(|path| (path, rec.meta.dirty)))
            else {
                continue;
            };

            if was_dirty || self.buffers.len() <= 1 {
                orphan_file_buffer(self, id, old_path);
                continue;
            }

            if self.close_buffer(id) {
                closed_ids.push(id);
            } else {
                orphan_file_buffer(self, id, old_path);
            }
        }

        FilePathSyncResult {
            remapped_ids,
            closed_ids,
        }
    }

    /// Close a buffer by id, activating the next MRU buffer if needed.
    ///
    /// Returns `false` if the id does not exist or this is the last remaining buffer.
    pub fn close_buffer(&mut self, id: BufferId) -> bool {
        if !self.buffers.contains_key(&id) || self.buffers.len() <= 1 {
            return false;
        }

        if let Some(rec) = self.buffers.remove(&id)
            && let Some(path) = rec.meta.path
        {
            self.path_index.remove(&path);
        }

        if let Some(pos) = self.mru.iter().position(|cur| *cur == id) {
            self.mru.remove(pos);
        }

        if self.active == Some(id) {
            self.active = self.mru.first().copied();
        }

        self.active.is_some()
    }

    /// Close the currently active buffer.
    #[inline]
    pub fn close_active_buffer(&mut self) -> bool {
        self.close_buffer(self.active_id())
    }

    /// Save the active file-backed buffer.
    pub fn save_active(&mut self) -> Result<()> {
        let id = self.active_id();
        self.ensure_buffer_fully_loaded(id)?;
        let rec = self
            .buffers
            .get_mut(&id)
            .expect("active buffer must exist in session map");

        match rec.meta.kind {
            BufferKind::File => {
                let path = rec
                    .meta
                    .path
                    .as_ref()
                    .context("file buffer is missing path metadata")?;
                if rec.meta.is_new_file && path.exists() {
                    rec.meta.external_changed = true;
                    bail!("file appeared on disk; reload or resolve before writing");
                }
                if rec.meta.external_changed
                    || rec
                        .disk_stamp
                        .is_some_and(|stamp| file_stamp(path).is_ok_and(|current| current != stamp))
                {
                    rec.meta.external_changed = true;
                    bail!("file changed on disk; reload or resolve before writing");
                }

                let mut content = rec.buffer.to_string();
                if !content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                    rec.buffer = TextBuffer::from_str(&content);
                }

                std::fs::write(path, &content)
                    .with_context(|| format!("failed to write file: {}", path.display()))?;

                let (fingerprint, normalized_len_chars) =
                    normalized_content_fingerprint(&rec.buffer);
                rec.clean_fingerprint = fingerprint;
                rec.clean_normalized_len_chars = normalized_len_chars;
                rec.meta.dirty = false;
                rec.meta.external_changed = false;
                rec.meta.is_new_file = false;
                rec.disk_stamp = file_stamp(path).ok();
                Ok(())
            }
            BufferKind::Ui => bail!("cannot save UI buffer"),
        }
    }

    /// Check file-backed buffers for external changes using cheap metadata.
    ///
    /// Clean buffers are reloaded from disk. Dirty buffers are left untouched and
    /// marked as conflicted so saving cannot clobber outside edits.
    pub fn poll_external_file_changes(&mut self) -> Vec<ExternalFileChange> {
        let ids = self.mru.clone();
        let mut changes = Vec::new();

        for id in ids {
            let Some(rec) = self.buffers.get_mut(&id) else {
                continue;
            };
            if rec.meta.kind != BufferKind::File
                || !matches!(
                    rec.load_status.phase,
                    BufferLoadPhase::NotLoading | BufferLoadPhase::Complete
                )
            {
                continue;
            }

            let Some(path) = rec.meta.path.clone() else {
                continue;
            };
            let display_name = rec.meta.display_name.clone();
            let previous = rec.disk_stamp;
            let current = match file_stamp(&path) {
                Ok(stamp) => stamp,
                Err(_) if path.exists() => {
                    changes.push(ExternalFileChange {
                        id,
                        display_name,
                        path: Some(path),
                        kind: ExternalFileChangeKind::Failed,
                    });
                    continue;
                }
                Err(_) if rec.meta.is_new_file => continue,
                Err(_) => {
                    if !rec.meta.external_changed {
                        rec.meta.external_changed = true;
                        changes.push(ExternalFileChange {
                            id,
                            display_name,
                            path: Some(path),
                            kind: ExternalFileChangeKind::Deleted,
                        });
                    }
                    continue;
                }
            };

            let Some(previous) = previous else {
                if rec.meta.is_new_file {
                    if rec.meta.dirty {
                        rec.meta.external_changed = true;
                        changes.push(ExternalFileChange {
                            id,
                            display_name,
                            path: Some(path),
                            kind: ExternalFileChangeKind::Conflict,
                        });
                    } else if let Ok(buffer) = TextBuffer::from_file(&path) {
                        rec.buffer = buffer;
                        let (fingerprint, normalized_len_chars) =
                            normalized_content_fingerprint(&rec.buffer);
                        rec.clean_fingerprint = fingerprint;
                        rec.clean_normalized_len_chars = normalized_len_chars;
                        rec.disk_stamp = Some(current);
                        rec.meta.is_new_file = false;
                        changes.push(ExternalFileChange {
                            id,
                            display_name,
                            path: Some(path),
                            kind: ExternalFileChangeKind::Reloaded,
                        });
                    }
                    continue;
                }
                rec.disk_stamp = Some(current);
                continue;
            };
            if current == previous {
                continue;
            }
            if rec.meta.dirty {
                if !rec.meta.external_changed {
                    rec.meta.external_changed = true;
                    changes.push(ExternalFileChange {
                        id,
                        display_name,
                        path: Some(path),
                        kind: ExternalFileChangeKind::Conflict,
                    });
                }
                continue;
            }

            match TextBuffer::from_file(&path) {
                Ok(buffer) => {
                    rec.buffer = buffer;
                    let (fingerprint, normalized_len_chars) =
                        normalized_content_fingerprint(&rec.buffer);
                    rec.clean_fingerprint = fingerprint;
                    rec.clean_normalized_len_chars = normalized_len_chars;
                    rec.disk_stamp = Some(current);
                    rec.meta.dirty = false;
                    rec.meta.external_changed = false;
                    rec.meta.is_new_file = false;
                    changes.push(ExternalFileChange {
                        id,
                        display_name,
                        path: Some(path),
                        kind: ExternalFileChangeKind::Reloaded,
                    });
                }
                Err(_) => {
                    rec.meta.external_changed = true;
                    changes.push(ExternalFileChange {
                        id,
                        display_name,
                        path: Some(path),
                        kind: ExternalFileChangeKind::Failed,
                    });
                }
            }
        }

        changes
    }

    #[inline]
    pub fn buffer(&self, id: BufferId) -> Option<&TextBuffer> {
        self.buffers.get(&id).map(|rec| &rec.buffer)
    }

    #[inline]
    pub fn buffer_mut(&mut self, id: BufferId) -> Option<&mut TextBuffer> {
        self.buffers.get_mut(&id).map(|rec| &mut rec.buffer)
    }

    #[inline]
    pub fn meta(&self, id: BufferId) -> Option<&BufferMeta> {
        self.buffers.get(&id).map(|rec| &rec.meta)
    }

    pub fn recompute_buffer_dirty(&mut self, id: BufferId) -> Option<bool> {
        let rec = self.buffers.get_mut(&id)?;

        if !matches!(
            rec.load_status.phase,
            BufferLoadPhase::NotLoading | BufferLoadPhase::Complete
        ) {
            return Some(rec.meta.dirty);
        }

        let (current, current_len) = normalized_content_fingerprint(&rec.buffer);
        if current_len != rec.clean_normalized_len_chars {
            rec.meta.dirty = true;
            return Some(true);
        }

        rec.meta.dirty = current != rec.clean_fingerprint;
        Some(rec.meta.dirty)
    }

    fn load_step_for(&mut self, id: BufferId, max_bytes: usize) -> Result<usize> {
        let rec = match self.buffers.get_mut(&id) {
            Some(rec) => rec,
            None => return Ok(0),
        };

        if !matches!(rec.load_status.phase, BufferLoadPhase::Loading) {
            return Ok(0);
        }

        let (chunk, bytes_loaded, total_bytes, is_eof) = match rec.loader.as_mut() {
            Some(loader) => {
                let chunk = match loader.read_chunk(max_bytes) {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        rec.load_status.phase = BufferLoadPhase::Failed;
                        rec.load_status.error = Some(err.to_string());
                        rec.load_status.bytes_loaded = loader.bytes_loaded();
                        rec.load_status.total_bytes = loader.total_bytes();
                        rec.loader = None;
                        return Err(err);
                    }
                };
                (
                    chunk,
                    loader.bytes_loaded(),
                    loader.total_bytes(),
                    loader.is_eof(),
                )
            }
            None => {
                rec.load_status.phase = BufferLoadPhase::Complete;
                rec.load_status.error = None;
                if rec.meta.path.is_some() {
                    let (fingerprint, normalized_len_chars) =
                        normalized_content_fingerprint(&rec.buffer);
                    rec.clean_fingerprint = fingerprint;
                    rec.clean_normalized_len_chars = normalized_len_chars;
                    rec.disk_stamp = rec
                        .meta
                        .path
                        .as_deref()
                        .and_then(|path| file_stamp(path).ok());
                }
                return Ok(0);
            }
        };

        if !chunk.text.is_empty() {
            let at = rec.buffer.len_chars();
            rec.buffer.rope_mut().insert(at, &chunk.text);
        }

        rec.load_status.bytes_loaded = bytes_loaded;
        rec.load_status.total_bytes = total_bytes;

        if chunk.eof || is_eof {
            rec.load_status.phase = BufferLoadPhase::Complete;
            rec.load_status.error = None;
            if rec.meta.path.is_some() {
                let (fingerprint, normalized_len_chars) =
                    normalized_content_fingerprint(&rec.buffer);
                rec.clean_fingerprint = fingerprint;
                rec.clean_normalized_len_chars = normalized_len_chars;
                rec.disk_stamp = rec
                    .meta
                    .path
                    .as_deref()
                    .and_then(|path| file_stamp(path).ok());
            }
            rec.loader = None;
        } else {
            rec.load_status.phase = BufferLoadPhase::Loading;
            rec.load_status.error = None;
        }

        Ok(chunk.bytes_read)
    }

    fn alloc_id(&mut self) -> BufferId {
        self.next_id = self.next_id.saturating_add(1);
        BufferId(self.next_id)
    }

    fn promote_mru(&mut self, id: BufferId) {
        if let Some(pos) = self.mru.iter().position(|cur| *cur == id) {
            self.mru.remove(pos);
        }
        self.mru.insert(0, id);
    }

    fn display_path(&self, path: &Path) -> String {
        if self.launch_dir.as_os_str().is_empty() {
            return path.display().to_string();
        }

        relative_path(path, &self.launch_dir)
            .unwrap_or_else(|| path.to_path_buf())
            .display()
            .to_string()
    }
}

fn normalize_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(path)
    };

    Ok(std::fs::canonicalize(&path).unwrap_or(path))
}

fn orphan_file_buffer(session: &mut EditorSession, id: BufferId, old_path: PathBuf) {
    session.path_index.remove(&old_path);

    if let Some(rec) = session.buffers.get_mut(&id) {
        rec.meta.path = None;
        rec.meta.display_name = orphaned_display_name(&rec.meta.display_name);
        rec.meta.is_new_file = true;
        rec.meta.dirty = true;
        rec.meta.external_changed = false;
        rec.disk_stamp = None;
        let (fingerprint, normalized_len_chars) = normalized_text_fingerprint("");
        rec.clean_fingerprint = fingerprint;
        rec.clean_normalized_len_chars = normalized_len_chars;
    }
}

fn file_stamp(path: &Path) -> std::io::Result<FileStamp> {
    let metadata = std::fs::metadata(path)?;
    Ok(FileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn orphaned_display_name(current_display_name: &str) -> String {
    const ORPHANED_SUFFIX: &str = " [orphaned]";
    if current_display_name.ends_with(ORPHANED_SUFFIX) {
        current_display_name.to_string()
    } else {
        format!("{current_display_name}{ORPHANED_SUFFIX}")
    }
}

fn normalize_sync_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    let Some(parent) = absolute.parent() else {
        return absolute;
    };
    let Some(name) = absolute.file_name() else {
        return absolute;
    };

    let normalized_parent = normalize_sync_path(parent);
    normalized_parent.join(name)
}

fn remap_synced_path(
    path: &Path,
    renames: &[(PathBuf, PathBuf)],
    deletions: &[PathBuf],
) -> Option<PathBuf> {
    let mut best_rename: Option<(&PathBuf, &PathBuf)> = None;
    for (old_path, new_path) in renames {
        if !path_matches_or_is_descendant(path, old_path) {
            continue;
        }

        let replace = match best_rename {
            Some((best_old, _)) => old_path.components().count() > best_old.components().count(),
            None => true,
        };
        if replace {
            best_rename = Some((old_path, new_path));
        }
    }

    let mut mapped = if let Some((old_path, new_path)) = best_rename {
        replace_path_prefix(path, old_path, new_path)
            .expect("matched rename path must support prefix replacement")
    } else {
        path.to_path_buf()
    };

    for deleted_path in deletions {
        if path_matches_or_is_descendant(&mapped, deleted_path) {
            return None;
        }
    }

    mapped = std::fs::canonicalize(&mapped).unwrap_or(mapped);
    Some(mapped)
}

fn path_matches_or_is_descendant(path: &Path, target: &Path) -> bool {
    path == target || path.strip_prefix(target).is_ok()
}

fn replace_path_prefix(path: &Path, old_prefix: &Path, new_prefix: &Path) -> Option<PathBuf> {
    let suffix = path.strip_prefix(old_prefix).ok()?;
    let mut out = new_prefix.to_path_buf();
    if !suffix.as_os_str().is_empty() {
        out.push(suffix);
    }
    Some(out)
}

fn relative_path(path: &Path, base: &Path) -> Option<PathBuf> {
    let path_components: Vec<Component<'_>> = path.components().collect();
    let base_components: Vec<Component<'_>> = base.components().collect();

    let mut shared = 0usize;
    let max_shared = path_components.len().min(base_components.len());
    while shared < max_shared && path_components[shared] == base_components[shared] {
        shared += 1;
    }

    if shared == 0 {
        return None;
    }

    let mut rel = PathBuf::new();

    for comp in &base_components[shared..] {
        if matches!(comp, Component::Normal(_)) {
            rel.push("..");
        }
    }

    for comp in &path_components[shared..] {
        rel.push(comp.as_os_str());
    }

    if rel.as_os_str().is_empty() {
        Some(PathBuf::from("."))
    } else {
        Some(rel)
    }
}

fn normalized_content_fingerprint(buffer: &TextBuffer) -> (u64, usize) {
    let mut hasher = DefaultHasher::new();
    let mut len = 0usize;
    let mut previous_was_cr = false;
    for chunk in buffer.rope().chunks() {
        hash_normalized_newlines(chunk.chars(), &mut hasher, &mut len, &mut previous_was_cr);
    }
    (hasher.finish(), len)
}

fn normalized_text_fingerprint(text: &str) -> (u64, usize) {
    let mut hasher = DefaultHasher::new();
    let mut len = 0usize;
    let mut previous_was_cr = false;
    hash_normalized_newlines(text.chars(), &mut hasher, &mut len, &mut previous_was_cr);
    (hasher.finish(), len)
}

fn hash_normalized_newlines(
    chars: impl Iterator<Item = char>,
    hasher: &mut DefaultHasher,
    len: &mut usize,
    previous_was_cr: &mut bool,
) {
    for ch in chars {
        match ch {
            '\r' => {
                '\n'.hash(hasher);
                *len += 1;
                *previous_was_cr = true;
            }
            '\n' if *previous_was_cr => {
                *previous_was_cr = false;
            }
            _ => {
                ch.hash(hasher);
                *len += 1;
                *previous_was_cr = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos();
        std::env::temp_dir().join(format!("redox_session_test_{tag}_{nanos}.txt"))
    }

    fn large_text(lines: usize) -> String {
        let mut out = String::new();
        for i in 0..lines {
            out.push_str(&format!("line-{i:05} abcdefghijklmnopqrstuvwxyz\n"));
        }
        out
    }

    #[test]
    fn opening_second_file_creates_and_activates_new_buffer() {
        let path_a = temp_path("open_second_a");
        let path_b = temp_path("open_second_b");
        fs::write(&path_a, "aaa").expect("failed to write temp file");
        fs::write(&path_b, "bbb").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path_a).expect("open initial failed");
        let first = session.active_id();
        let second = session.open_file(&path_b).expect("open second failed");

        assert_ne!(first, second);
        assert_eq!(session.active_id(), second);
        assert_eq!(session.active_buffer().to_string(), "bbb");
        assert!(!session.active_meta().display_name.starts_with('/'));

        let _ = fs::remove_file(path_a);
        let _ = fs::remove_file(path_b);
    }

    #[test]
    fn opening_same_path_reuses_existing_buffer() {
        let path = temp_path("dedup");
        fs::write(&path, "hello").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let first = session.active_id();
        let second = session.open_file(&path).expect("open same failed");

        assert_eq!(first, second);
        assert_eq!(session.summaries().len(), 1);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn open_initial_unnamed_creates_empty_file_buffer() {
        let session = EditorSession::open_initial_unnamed().expect("open unnamed failed");
        let meta = session.active_meta();

        assert_eq!(meta.kind, BufferKind::File);
        assert_eq!(meta.display_name, "[No Name]");
        assert!(meta.path.is_none());
        assert!(meta.is_new_file);
        assert_eq!(session.active_buffer().to_string(), "");
    }

    #[test]
    fn missing_path_creates_empty_new_file_buffer() {
        let missing = temp_path("missing");
        if missing.exists() {
            fs::remove_file(&missing).expect("failed to remove existing fixture");
        }

        let session = EditorSession::open_initial_file(&missing).expect("open initial failed");

        assert!(session.active_buffer().is_empty());
        assert!(session.active_meta().is_new_file);
        assert_eq!(
            session.active_meta().path.as_ref(),
            Some(&normalize_path(&missing).unwrap())
        );
    }

    #[test]
    fn mru_switching_rotates_active_buffer() {
        let path_a = temp_path("mru_a");
        let path_b = temp_path("mru_b");
        let path_c = temp_path("mru_c");
        fs::write(&path_a, "a").expect("failed to write temp file");
        fs::write(&path_b, "b").expect("failed to write temp file");
        fs::write(&path_c, "c").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path_a).expect("open initial failed");
        let _ = session.open_file(&path_b).expect("open second failed");
        let _ = session.open_file(&path_c).expect("open third failed");

        let first = session.active_id();
        let second = session.switch_next_mru().expect("switch next failed");
        let third = session.switch_next_mru().expect("switch next failed");
        let back = session.switch_prev_mru().expect("switch prev failed");

        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_eq!(second, back);

        let _ = fs::remove_file(path_a);
        let _ = fs::remove_file(path_b);
        let _ = fs::remove_file(path_c);
    }

    #[test]
    fn any_dirty_detects_hidden_dirty_buffers() {
        let path_a = temp_path("dirty_a");
        let path_b = temp_path("dirty_b");
        fs::write(&path_a, "a").expect("failed to write temp file");
        fs::write(&path_b, "b").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path_a).expect("open initial failed");
        let id_a = session.active_id();
        let _ = session.open_file(&path_b).expect("open second failed");

        let _ = session.activate(id_a);
        let cursor = session.active_buffer().clamp_pos(crate::Pos::new(0, 1));
        let _ = session.active_buffer_mut().insert(cursor, "x");
        let _ = session.recompute_active_dirty();
        let _ = session.switch_next_mru();

        assert!(session.any_dirty());

        let _ = fs::remove_file(path_a);
        let _ = fs::remove_file(path_b);
    }

    #[test]
    fn save_active_writes_and_clears_dirty() {
        let path = temp_path("save_active");
        fs::write(&path, "old").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let cursor = session.active_buffer().clamp_pos(crate::Pos::new(0, 3));
        let _ = session.active_buffer_mut().insert(cursor, "_new");
        let _ = session.recompute_active_dirty();

        session.save_active().expect("save failed");

        assert!(!session.active_meta().dirty);
        let on_disk = fs::read_to_string(&path).expect("failed to read temp file");
        assert_eq!(on_disk, "old_new\n");
        assert_eq!(session.active_buffer().to_string(), "old_new\n");
        assert!(!session.recompute_active_dirty());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn external_clean_file_change_reloads_buffer() {
        let path = temp_path("external_reload");
        fs::write(&path, "old").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        fs::write(&path, "changed on disk\n").expect("failed to change temp file");

        let changes = session.poll_external_file_changes();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ExternalFileChangeKind::Reloaded);
        assert_eq!(session.active_buffer().to_string(), "changed on disk\n");
        assert!(!session.active_meta().dirty);
        assert!(!session.active_meta().external_changed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn external_dirty_file_change_blocks_save() {
        let path = temp_path("external_conflict");
        fs::write(&path, "old").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let cursor = session.active_buffer().clamp_pos(crate::Pos::new(0, 3));
        let _ = session.active_buffer_mut().insert(cursor, " local");
        assert!(session.recompute_active_dirty());
        fs::write(&path, "changed on disk\n").expect("failed to change temp file");

        let changes = session.poll_external_file_changes();
        let err = session
            .save_active()
            .expect_err("save should not overwrite external edits");

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ExternalFileChangeKind::Conflict);
        assert!(session.active_meta().external_changed);
        assert!(err.to_string().contains("file changed on disk"));
        assert_eq!(
            fs::read_to_string(&path).expect("failed to read temp file"),
            "changed on disk\n"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn new_file_created_elsewhere_blocks_save() {
        let path = temp_path("new_file_collision");
        if path.exists() {
            fs::remove_file(&path).expect("failed to remove existing fixture");
        }

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let _ = session
            .active_buffer_mut()
            .insert(crate::Pos::zero(), "local");
        assert!(session.recompute_active_dirty());
        fs::write(&path, "external").expect("failed to create external file");

        let err = session
            .save_active()
            .expect_err("save should not overwrite newly-created file");

        assert!(session.active_meta().external_changed);
        assert!(err.to_string().contains("file appeared on disk"));
        assert_eq!(
            fs::read_to_string(&path).expect("failed to read temp file"),
            "external"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_active_appends_trailing_newline_for_non_empty_file() {
        let path = temp_path("save_active_trailing_newline");
        fs::write(&path, "hello").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        session.save_active().expect("save failed");

        assert_eq!(
            fs::read_to_string(&path).expect("failed to read temp file"),
            "hello\n"
        );
        assert_eq!(session.active_buffer().to_string(), "hello\n");
        assert!(!session.recompute_active_dirty());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn dirty_tracking_clears_when_content_returns_to_clean_snapshot() {
        let path = temp_path("dirty_revert");
        fs::write(&path, "hello").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let end = session.active_buffer().clamp_pos(crate::Pos::new(0, 5));
        let _ = session.active_buffer_mut().insert(end, "!");
        assert!(session.recompute_active_dirty());

        let sel = crate::Selection::empty(crate::Pos::new(0, 6));
        let _ = session.active_buffer_mut().backspace(sel);
        assert!(!session.recompute_active_dirty());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn incremental_open_starts_loading_for_large_file() {
        let path = temp_path("incremental_open");
        let text = large_text(6000);
        fs::write(&path, &text).expect("failed to write temp file");

        let session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let status = session.active_buffer_load_status();

        assert_eq!(status.phase, BufferLoadPhase::Loading);
        assert!(status.bytes_loaded > 0);
        assert!(status.total_bytes.unwrap_or(0) > status.bytes_loaded);
        assert!(!session.active_buffer_is_fully_loaded());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn poll_loading_increases_loaded_bytes_monotonically() {
        let path = temp_path("poll_monotonic");
        let text = large_text(8000);
        fs::write(&path, &text).expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let mut prev = session.active_buffer_load_status().bytes_loaded;

        for _ in 0..10 {
            let _ = session.poll_loading(8 * 1024);
            let now = session.active_buffer_load_status().bytes_loaded;
            assert!(now >= prev);
            prev = now;
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn demand_loading_reaches_target_line_or_eof() {
        let path = temp_path("demand_line");
        let text = large_text(9000);
        fs::write(&path, &text).expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let id = session.active_id();
        let target = 3500usize;
        session
            .ensure_buffer_loaded_through_line(id, target, 256 * 1024)
            .expect("demand load failed");

        let loaded_lines = session.active_buffer().len_lines();
        let phase = session.active_buffer_load_status().phase;
        assert!(loaded_lines > target || phase == BufferLoadPhase::Complete);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn ensure_fully_loaded_completes_and_matches_disk() {
        let path = temp_path("full_load");
        let text = large_text(7500);
        fs::write(&path, &text).expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let id = session.active_id();
        session
            .ensure_buffer_fully_loaded(id)
            .expect("full load should succeed");

        assert_eq!(
            session.active_buffer_load_status().phase,
            BufferLoadPhase::Complete
        );
        assert_eq!(session.active_buffer().to_string(), text);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn full_load_handles_utf8_chunk_boundaries() {
        let path = temp_path("utf8_boundaries");
        let text = "😀alpha\nβeta\nこんにちは\n".repeat(7000);
        fs::write(&path, &text).expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let id = session.active_id();
        session
            .ensure_buffer_fully_loaded(id)
            .expect("full load should succeed");

        assert_eq!(session.active_buffer().to_string(), text);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_utf8_sets_failed_phase_and_blocks_full_load() {
        let path = temp_path("invalid_utf8_incremental");
        let mut file = fs::File::create(&path).expect("failed to create temp file");
        let prefix = "ok\n".repeat(30_000);
        file.write_all(prefix.as_bytes())
            .expect("failed to write prefix");
        file.write_all(&[0xff])
            .expect("failed to write invalid byte");
        file.flush().expect("failed to flush");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let id = session.active_id();
        let err = session
            .ensure_buffer_fully_loaded(id)
            .expect_err("expected invalid utf8 error");
        assert!(err.to_string().contains("not valid UTF-8"));
        assert_eq!(
            session.active_buffer_load_status().phase,
            BufferLoadPhase::Failed
        );
        assert!(!session.active_buffer().is_empty());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn background_loading_does_not_mark_dirty() {
        let path = temp_path("load_not_dirty");
        let text = large_text(7000);
        fs::write(&path, &text).expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let _ = session.poll_loading(128 * 1024);
        assert!(!session.active_meta().dirty);
        assert!(!session.recompute_active_dirty());
        assert!(!session.active_meta().dirty);

        let id = session.active_id();
        session
            .ensure_buffer_fully_loaded(id)
            .expect("full load should succeed");
        let end = session.active_buffer().clamp_pos(crate::Pos::new(0, 5));
        let _ = session.active_buffer_mut().insert(end, "!");
        assert!(session.recompute_active_dirty());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_active_forces_full_load_before_write() {
        let path = temp_path("save_gate");
        let text = large_text(8500);
        fs::write(&path, &text).expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        assert_eq!(
            session.active_buffer_load_status().phase,
            BufferLoadPhase::Loading
        );

        session.save_active().expect("save should force full load");
        assert_eq!(
            session.active_buffer_load_status().phase,
            BufferLoadPhase::Complete
        );

        let on_disk = fs::read_to_string(&path).expect("failed to read file");
        assert_eq!(on_disk, text);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn sync_file_buffers_with_paths_remaps_open_descendants_after_directory_rename() {
        let root = std::env::temp_dir().join(format!(
            "redox_session_sync_dir_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock went backwards")
                .as_nanos()
        ));
        let old_dir = root.join("old");
        let new_dir = root.join("new");
        fs::create_dir_all(&old_dir).expect("failed to create old directory");

        let file_path = old_dir.join("nested.txt");
        fs::write(&file_path, "hello").expect("failed to write nested fixture");

        let mut session =
            EditorSession::open_initial_file(&file_path).expect("open initial failed");
        let file_id = session.active_id();

        fs::rename(&old_dir, &new_dir).expect("failed to rename directory");
        let result =
            session.sync_file_buffers_with_paths(&[(old_dir.clone(), new_dir.clone())], &[]);

        assert_eq!(result.remapped_ids, vec![file_id]);
        assert!(result.closed_ids.is_empty());
        let renamed_file = std::fs::canonicalize(new_dir.join("nested.txt"))
            .expect("renamed nested file should exist");
        assert_eq!(session.active_meta().path.as_ref(), Some(&renamed_file));
        assert_eq!(
            session
                .open_file(&renamed_file)
                .expect("reopen should reuse remapped buffer"),
            file_id
        );

        let _ = fs::remove_file(new_dir.join("nested.txt"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_file_buffers_with_paths_closes_deleted_buffers() {
        let path_a = temp_path("sync_delete_a");
        let path_b = temp_path("sync_delete_b");
        fs::write(&path_a, "a").expect("failed to write temp file");
        fs::write(&path_b, "b").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path_a).expect("open initial failed");
        let doomed_id = session.open_file(&path_b).expect("open second failed");

        fs::remove_file(&path_b).expect("failed to remove doomed file");
        let result = session.sync_file_buffers_with_paths(&[], std::slice::from_ref(&path_b));

        assert!(result.remapped_ids.is_empty());
        assert_eq!(result.closed_ids, vec![doomed_id]);
        assert_eq!(session.summaries().len(), 1);
        assert!(session.meta(doomed_id).is_none());

        let _ = fs::remove_file(path_a);
    }

    #[test]
    fn sync_file_buffers_with_paths_orphans_dirty_deleted_buffer() {
        let path_a = temp_path("sync_orphan_dirty_a");
        let path_b = temp_path("sync_orphan_dirty_b");
        fs::write(&path_a, "a").expect("failed to write temp file");
        fs::write(&path_b, "b").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path_a).expect("open initial failed");
        let dirty_id = session.open_file(&path_b).expect("open second failed");
        let cursor = session.active_buffer().clamp_pos(crate::Pos::new(0, 1));
        let _ = session.active_buffer_mut().insert(cursor, "!");
        assert!(session.recompute_active_dirty());

        fs::remove_file(&path_b).expect("failed to remove doomed file");
        let result = session.sync_file_buffers_with_paths(&[], std::slice::from_ref(&path_b));

        assert!(result.remapped_ids.is_empty());
        assert!(result.closed_ids.is_empty());
        let meta = session.meta(dirty_id).expect("dirty buffer should remain");
        assert!(meta.dirty);
        assert!(meta.path.is_none());
        assert!(meta.display_name.ends_with(" [orphaned]"));
        assert_eq!(session.active_buffer().to_string(), "b!");

        let reopened_id = session
            .open_file(&path_b)
            .expect("reopen should create new buffer");
        assert_ne!(reopened_id, dirty_id);

        let _ = fs::remove_file(path_a);
    }

    #[test]
    fn sync_file_buffers_with_paths_orphans_last_remaining_deleted_buffer() {
        let path = temp_path("sync_orphan_last");
        fs::write(&path, "hello").expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let doomed_id = session.active_id();

        fs::remove_file(&path).expect("failed to remove doomed file");
        let result = session.sync_file_buffers_with_paths(&[], std::slice::from_ref(&path));

        assert!(result.remapped_ids.is_empty());
        assert!(result.closed_ids.is_empty());
        assert_eq!(session.summaries().len(), 1);
        let meta = session.meta(doomed_id).expect("last buffer should remain");
        assert!(meta.path.is_none());
        assert!(meta.dirty);
        assert!(meta.is_new_file);
        assert!(meta.display_name.ends_with(" [orphaned]"));
        assert_eq!(session.active_buffer().to_string(), "hello");
    }

    #[test]
    fn orphaned_loading_buffer_stays_unsaved_after_load_completes() {
        let path = temp_path("sync_orphan_loading");
        let text = large_text(9000);
        fs::write(&path, &text).expect("failed to write temp file");

        let mut session = EditorSession::open_initial_file(&path).expect("open initial failed");
        let doomed_id = session.active_id();
        assert_eq!(
            session.active_buffer_load_status().phase,
            BufferLoadPhase::Loading
        );

        fs::remove_file(&path).expect("failed to remove doomed file");
        let result = session.sync_file_buffers_with_paths(&[], std::slice::from_ref(&path));
        assert!(result.closed_ids.is_empty());

        session
            .ensure_buffer_fully_loaded(doomed_id)
            .expect("orphaned buffer should still finish loading");

        let meta = session
            .meta(doomed_id)
            .expect("orphaned buffer should remain");
        assert!(meta.path.is_none());
        assert!(meta.dirty);
        assert!(meta.is_new_file);
        assert!(session.recompute_active_dirty());
        assert_eq!(session.active_buffer().to_string(), text);
    }

    #[test]
    fn sync_file_buffers_with_paths_deletes_directory_descendants() {
        let root = std::env::temp_dir().join(format!(
            "redox_session_sync_delete_dir_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock went backwards")
                .as_nanos()
        ));
        let doomed_dir = root.join("doomed");
        fs::create_dir_all(&doomed_dir).expect("failed to create doomed directory");

        let clean_path = doomed_dir.join("clean.txt");
        let dirty_path = doomed_dir.join("dirty.txt");
        fs::write(&clean_path, "clean").expect("failed to write clean fixture");
        fs::write(&dirty_path, "dirty").expect("failed to write dirty fixture");

        let mut session =
            EditorSession::open_initial_file(&clean_path).expect("open initial failed");
        let clean_id = session.active_id();
        let dirty_id = session
            .open_file(&dirty_path)
            .expect("open dirty file failed");

        let cursor = session.active_buffer().clamp_pos(crate::Pos::new(0, 5));
        let _ = session.active_buffer_mut().insert(cursor, "!");
        assert!(session.recompute_active_dirty());

        fs::remove_dir_all(&doomed_dir).expect("failed to remove doomed directory");
        let result = session.sync_file_buffers_with_paths(&[], std::slice::from_ref(&doomed_dir));

        assert!(result.remapped_ids.is_empty());
        assert_eq!(result.closed_ids, vec![clean_id]);
        assert!(session.meta(clean_id).is_none());

        let dirty_meta = session
            .meta(dirty_id)
            .expect("dirty descendant should remain");
        assert!(dirty_meta.path.is_none());
        assert!(dirty_meta.display_name.ends_with(" [orphaned]"));
        assert!(dirty_meta.dirty);
        assert!(dirty_meta.is_new_file);
        assert_eq!(session.active_buffer().to_string(), "dirty!");

        let summaries = session.summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, dirty_id);

        let _ = fs::remove_dir_all(root);
    }
}
