//! Multi-buffer session model for higher-level editor frontends.

mod loading;

use std::collections::HashMap;
use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufWriter, Write};
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
                        buffer.append(&chunk.text);
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
                buffer: TextBuffer::from_text(initial_text),
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

                if !rec.buffer.is_empty()
                    && rec.buffer.char(rec.buffer.len_chars() - 1) != Some('\n')
                {
                    rec.buffer.append("\n");
                }

                let file = File::create(path)
                    .with_context(|| format!("failed to write file: {}", path.display()))?;
                let mut writer = BufWriter::new(file);
                rec.buffer
                    .write_to(&mut writer)
                    .with_context(|| format!("failed to write file: {}", path.display()))?;
                writer
                    .flush()
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
            rec.buffer.append(&chunk.text);
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
    for chunk in buffer.chunks() {
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
