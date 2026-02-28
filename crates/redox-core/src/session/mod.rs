//! Multi-buffer session model for higher-level editor frontends.

mod loading;

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Component;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use self::loading::IncrementalFileLoader;
use crate::TextBuffer;

const INITIAL_LOAD_BYTES: usize = 64 * 1024;
const FULL_LOAD_CHUNK_BYTES: usize = 64 * 1024;

/// Stable buffer identifier within an [`EditorSession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BufferId(u64);

impl BufferId {
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
    pub id: BufferId,
    pub kind: BufferKind,
    pub display_name: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub is_new_file: bool,
}

/// Listing row for buffer UIs such as `:ls`.
#[derive(Debug, Clone)]
pub struct BufferSummary {
    pub id: BufferId,
    pub kind: BufferKind,
    pub display_name: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub is_new_file: bool,
    pub is_active: bool,
}

/// Loading phase for a file-backed buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferLoadPhase {
    NotLoading,
    Loading,
    Complete,
    Failed,
}

/// Snapshot status for file loading progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferLoadStatus {
    pub phase: BufferLoadPhase,
    pub bytes_loaded: usize,
    pub total_bytes: Option<usize>,
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
    loader: Option<IncrementalFileLoader>,
    load_status: BufferLoadStatus,
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
            is_new_file: !file_exists,
        };
        let clean_fingerprint = if matches!(load_status.phase, BufferLoadPhase::Complete) {
            content_fingerprint(&buffer)
        } else {
            hash_text("")
        };

        self.buffers.insert(
            id,
            BufferRecord {
                meta,
                buffer,
                clean_fingerprint,
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
        let meta = BufferMeta {
            id,
            kind: BufferKind::Ui,
            display_name: name.into(),
            path: None,
            dirty: false,
            is_new_file: false,
        };

        self.buffers.insert(
            id,
            BufferRecord {
                meta,
                buffer: TextBuffer::from_str(initial_text),
                clean_fingerprint: hash_text(initial_text),
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
        let rec = self
            .buffers
            .get_mut(&id)
            .expect("active buffer must exist in session map");

        let current = content_fingerprint(&rec.buffer);
        rec.meta.dirty = current != rec.clean_fingerprint;
        rec.meta.dirty
    }

    /// Record the active buffer's current contents as the clean snapshot.
    pub fn mark_active_clean(&mut self) {
        let id = self.active_id();
        let rec = self
            .buffers
            .get_mut(&id)
            .expect("active buffer must exist in session map");
        rec.clean_fingerprint = content_fingerprint(&rec.buffer);
        rec.meta.dirty = false;
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
                is_new_file: rec.meta.is_new_file,
                is_active: Some(*id) == active,
            })
            .collect()
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
                let content = rec.buffer.to_string();

                std::fs::write(path, &content)
                    .with_context(|| format!("failed to write file: {}", path.display()))?;

                rec.clean_fingerprint = hash_text(&content);
                rec.meta.dirty = false;
                rec.meta.is_new_file = false;
                Ok(())
            }
            BufferKind::Ui => bail!("cannot save UI buffer"),
        }
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
                rec.clean_fingerprint = content_fingerprint(&rec.buffer);
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
            rec.clean_fingerprint = content_fingerprint(&rec.buffer);
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

fn content_fingerprint(buffer: &TextBuffer) -> u64 {
    hash_text(&buffer.to_string())
}

fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
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
        assert_eq!(on_disk, "old_new");

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
}
