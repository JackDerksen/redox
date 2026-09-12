use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use redox_core::{BufferId, BufferKind, EditorSession, TextBuffer};
use tempfile::NamedTempFile;

const DIRTY_REFRESH_INTERVAL: Duration = Duration::from_millis(200);
const REPO_STATUS_WORKERS: usize = 2;
const REPO_STATUS_QUEUE_BOUND: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFileStatusKind {
    Added,
    Modified,
    Conflict,
    Removed,
}

impl GitFileStatusKind {
    fn priority(self) -> u8 {
        match self {
            Self::Modified => 1,
            Self::Added => 2,
            Self::Removed => 3,
            Self::Conflict => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitGutterKind {
    Added,
    Modified,
    Removed,
}

impl GitGutterKind {
    fn priority(self) -> u8 {
        match self {
            Self::Added => 1,
            Self::Modified => 2,
            Self::Removed => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GitDiffStats {
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
}

impl GitDiffStats {
    pub fn is_empty(self) -> bool {
        self.added == 0 && self.modified == 0 && self.removed == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitLineMarker {
    pub line: usize,
    pub kind: GitGutterKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitDiffSnapshot {
    pub stats: GitDiffStats,
    pub markers: Vec<GitLineMarker>,
}

impl GitDiffSnapshot {
    pub fn marker_for_line(&self, line: usize) -> Option<GitGutterKind> {
        self.markers
            .iter()
            .find(|marker| marker.line == line)
            .map(|marker| marker.kind)
    }
}

#[derive(Debug)]
pub struct GitState {
    cache: HashMap<BufferId, GitDiffCacheEntry>,
    repo_status_cache: HashMap<PathBuf, GitRepoStatusCacheEntry>,
    repo_status_revision: u64,
    pending_repo_status_dirs: HashSet<PathBuf>,
    known_repo_dirs: HashMap<PathBuf, Option<PathBuf>>,
    repo_status_tx: Sender<GitRepoStatusResult>,
    repo_status_rx: Receiver<GitRepoStatusResult>,
    repo_status_job_tx: SyncSender<GitRepoStatusJob>,
    diff_job_tx: SyncSender<GitDiffJob>,
    next_diff_request: u64,
    base_generation: u64,
    diff_rx: Receiver<GitDiffResult>,
    changed: bool,
}

#[derive(Debug)]
struct GitDiffCacheEntry {
    path: Option<PathBuf>,
    dirty: bool,
    last_refreshed_at: Instant,
    stale: bool,
    snapshot: Option<GitDiffSnapshot>,
    pending: bool,
    request_id: u64,
}

#[derive(Debug)]
struct GitRepoStatusCacheEntry {
    file_statuses: HashMap<PathBuf, GitFileStatusKind>,
    directory_statuses: HashMap<PathBuf, GitFileStatusKind>,
    stale: bool,
}

#[derive(Debug)]
struct GitRepoStatusResult {
    requested_dir: PathBuf,
    repo_root: Option<PathBuf>,
    statuses: Option<RepoStatuses>,
}

struct GitRepoStatusJob {
    dir: PathBuf,
    tx: Sender<GitRepoStatusResult>,
}

#[derive(Debug)]
struct GitDiffResult {
    buffer_id: BufferId,
    path: Option<PathBuf>,
    request_id: u64,
    snapshot: Option<GitDiffSnapshot>,
}

struct GitDiffJob {
    buffer_id: BufferId,
    path: PathBuf,
    buffer: TextBuffer,
    request_id: u64,
    base_generation: u64,
}

type RepoStatuses = (
    HashMap<PathBuf, GitFileStatusKind>,
    HashMap<PathBuf, GitFileStatusKind>,
);

impl Default for GitState {
    fn default() -> Self {
        let (repo_status_tx, repo_status_rx) = mpsc::channel();
        let repo_status_job_tx = start_repo_status_workers();
        let (diff_job_tx, diff_rx) = start_diff_worker();
        Self {
            cache: HashMap::new(),
            repo_status_cache: HashMap::new(),
            repo_status_revision: 0,
            pending_repo_status_dirs: HashSet::new(),
            known_repo_dirs: HashMap::new(),
            repo_status_tx,
            repo_status_rx,
            repo_status_job_tx,
            diff_job_tx,
            next_diff_request: 0,
            base_generation: 0,
            diff_rx,
            changed: false,
        }
    }
}

impl GitState {
    pub(super) fn remove_closed_buffers(&mut self, session: &EditorSession) {
        self.cache.retain(|id, _| session.meta(*id).is_some());
    }

    pub(super) fn take_changed(&mut self) -> bool {
        self.drain_diff_results();
        self.drain_repo_status_results();
        std::mem::take(&mut self.changed)
    }

    pub(super) fn has_pending_work(&self) -> bool {
        !self.pending_repo_status_dirs.is_empty()
            || self
                .cache
                .values()
                .any(|entry| entry.path.is_some() && (entry.pending || entry.stale))
    }

    pub fn diff_for(&self, buffer_id: BufferId) -> Option<&GitDiffSnapshot> {
        self.cache.get(&buffer_id)?.snapshot.as_ref()
    }

    pub fn mark_stale(&mut self, buffer_id: BufferId) {
        if let Some(entry) = self.cache.get_mut(&buffer_id) {
            entry.stale = true;
        }
    }

    pub fn mark_all_repo_statuses_stale(&mut self) {
        self.base_generation = self.base_generation.wrapping_add(1);
        for entry in self.cache.values_mut() {
            entry.stale = true;
        }
        self.known_repo_dirs.clear();
        for entry in self.repo_status_cache.values_mut() {
            entry.stale = true;
        }
    }

    pub(super) fn status_for_path(&self, path: &Path) -> Option<GitFileStatusKind> {
        let entry = self
            .repo_status_cache
            .iter()
            .filter(|(repo_root, _)| path.starts_with(repo_root))
            .max_by_key(|(repo_root, _)| repo_root.components().count())
            .map(|(_, entry)| entry)?;

        entry
            .file_statuses
            .get(path)
            .copied()
            .or_else(|| entry.directory_statuses.get(path).copied())
    }

    pub(super) fn repo_status_revision(&self) -> u64 {
        self.repo_status_revision
    }

    pub fn refresh_repo_status_for_dir(&mut self, dir: &Path) {
        self.drain_repo_status_results();

        let dir = dir.to_path_buf();
        if self.pending_repo_status_dirs.contains(&dir) {
            return;
        }
        if let Some(root) = self.known_repo_dirs.get(&dir) {
            match root {
                None => return,
                Some(root)
                    if self
                        .repo_status_cache
                        .get(root)
                        .is_some_and(|entry| !entry.stale) =>
                {
                    return;
                }
                Some(_) => {}
            }
        }

        let tx = self.repo_status_tx.clone();
        match self.repo_status_job_tx.try_send(GitRepoStatusJob {
            dir: dir.clone(),
            tx,
        }) {
            Ok(()) => {
                self.pending_repo_status_dirs.insert(dir);
            }
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    pub fn refresh_for_buffer(&mut self, session: &EditorSession, buffer_id: BufferId) {
        self.drain_diff_results();

        let Some(meta) = session.meta(buffer_id) else {
            self.cache.remove(&buffer_id);
            return;
        };
        if meta.kind != BufferKind::File {
            self.cache.remove(&buffer_id);
            return;
        }

        let path = meta.path.clone();
        let dirty = meta.dirty;
        let now = Instant::now();
        let should_refresh = match self.cache.get(&buffer_id) {
            Some(entry) if entry.path != path => true,
            Some(entry) if entry.pending => false,
            Some(entry) if entry.dirty != dirty => {
                if dirty {
                    now.duration_since(entry.last_refreshed_at) >= DIRTY_REFRESH_INTERVAL
                } else {
                    true
                }
            }
            Some(entry) if entry.stale => {
                if dirty {
                    now.duration_since(entry.last_refreshed_at) >= DIRTY_REFRESH_INTERVAL
                } else {
                    true
                }
            }
            Some(_) => false,
            None => true,
        };
        if !should_refresh {
            return;
        }

        self.next_diff_request = self.next_diff_request.wrapping_add(1);
        let request_id = self.next_diff_request;
        let job = path
            .as_ref()
            .zip(session.buffer(buffer_id))
            .map(|(path, buffer)| GitDiffJob {
                buffer_id,
                path: path.clone(),
                buffer: buffer.clone(),
                request_id,
                base_generation: self.base_generation,
            });
        let previous_snapshot = self
            .cache
            .remove(&buffer_id)
            .and_then(|entry| entry.snapshot);
        let entry = self.cache.entry(buffer_id).or_insert(GitDiffCacheEntry {
            path,
            dirty,
            last_refreshed_at: now,
            stale: false,
            snapshot: previous_snapshot,
            pending: job.is_some(),
            request_id,
        });
        if let Some(job) = job
            && self.diff_job_tx.try_send(job).is_err()
        {
            entry.pending = false;
            entry.stale = true;
        }
    }

    fn drain_repo_status_results(&mut self) {
        while let Ok(result) = self.repo_status_rx.try_recv() {
            self.pending_repo_status_dirs.remove(&result.requested_dir);
            self.known_repo_dirs
                .insert(result.requested_dir, result.repo_root.clone());
            let Some(repo_root) = result.repo_root else {
                continue;
            };
            self.changed = true;
            let Some((file_statuses, directory_statuses)) = result.statuses else {
                if self.repo_status_cache.remove(&repo_root).is_some() {
                    self.repo_status_revision = self.repo_status_revision.wrapping_add(1);
                }
                continue;
            };
            self.repo_status_cache.insert(
                repo_root,
                GitRepoStatusCacheEntry {
                    file_statuses,
                    directory_statuses,
                    stale: false,
                },
            );
            self.repo_status_revision = self.repo_status_revision.wrapping_add(1);
        }
    }

    fn drain_diff_results(&mut self) {
        while let Ok(result) = self.diff_rx.try_recv() {
            let Some(entry) = self.cache.get_mut(&result.buffer_id) else {
                continue;
            };
            if entry.path != result.path || entry.request_id != result.request_id {
                continue;
            }
            entry.pending = false;
            // An edit made while the job ran still needs its own diff.
            if entry.stale {
                continue;
            }
            self.changed |= entry.snapshot != result.snapshot;
            entry.snapshot = result.snapshot;
            entry.last_refreshed_at = Instant::now();
        }
    }
}

fn start_repo_status_workers() -> SyncSender<GitRepoStatusJob> {
    let (tx, rx) = mpsc::sync_channel::<GitRepoStatusJob>(REPO_STATUS_QUEUE_BOUND);
    let rx = Arc::new(Mutex::new(rx));

    for _ in 0..REPO_STATUS_WORKERS {
        let rx = Arc::clone(&rx);
        thread::spawn(move || {
            loop {
                let job = {
                    let Ok(rx) = rx.lock() else {
                        break;
                    };
                    rx.recv()
                };
                let Ok(job) = job else {
                    break;
                };

                let result = load_repo_statuses_for_dir(&job.dir);
                let _ = job.tx.send(GitRepoStatusResult {
                    requested_dir: job.dir,
                    repo_root: result.as_ref().map(|(repo_root, _)| repo_root.clone()),
                    statuses: result.map(|(_, statuses)| statuses),
                });
            }
        });
    }

    tx
}

fn load_repo_statuses_for_dir(dir: &Path) -> Option<(PathBuf, RepoStatuses)> {
    let repo_root_raw = git_stdout(dir, &["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(repo_root_raw.trim());
    let statuses = load_repo_statuses(&repo_root)?;
    Some((repo_root, statuses))
}

fn load_repo_statuses(repo_root: &Path) -> Option<RepoStatuses> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let mut file_statuses = HashMap::new();
    let mut directory_statuses = HashMap::new();
    let mut entries = output.stdout.split(|byte| *byte == 0);
    while let Some(raw_entry) = entries.next() {
        if raw_entry.is_empty() {
            continue;
        }
        if raw_entry.len() < 4 {
            continue;
        }

        let index_status = raw_entry[0] as char;
        let worktree_status = raw_entry[1] as char;
        let is_rename_or_copy = matches!(index_status, 'R' | 'C');
        let path_bytes = &raw_entry[3..];
        let Ok(path) = String::from_utf8(path_bytes.to_vec()) else {
            if is_rename_or_copy {
                let _ = entries.next();
            }
            continue;
        };
        let Some(status) = classify_repo_status(index_status, worktree_status) else {
            if is_rename_or_copy {
                let _ = entries.next();
            }
            continue;
        };
        let file_path = repo_root.join(path);
        set_repo_status(&mut file_statuses, file_path.clone(), status);
        set_directory_statuses(&mut directory_statuses, repo_root, &file_path, status);

        if is_rename_or_copy {
            let _ = entries.next();
        }
    }

    Some((file_statuses, directory_statuses))
}

fn set_directory_statuses(
    directory_statuses: &mut HashMap<PathBuf, GitFileStatusKind>,
    repo_root: &Path,
    file_path: &Path,
    status: GitFileStatusKind,
) {
    let mut current = file_path.parent();
    while let Some(dir) = current {
        if dir == repo_root.parent().unwrap_or(repo_root) && dir != repo_root {
            break;
        }
        set_repo_status(directory_statuses, dir.to_path_buf(), status);
        if dir == repo_root {
            break;
        }
        current = dir.parent();
    }
}

fn classify_repo_status(x: char, y: char) -> Option<GitFileStatusKind> {
    if matches!((x, y), ('?', '?')) {
        return Some(GitFileStatusKind::Added);
    }
    if matches!((x, y), ('!', '!')) {
        return None;
    }
    if is_conflict_status(x, y) {
        return Some(GitFileStatusKind::Conflict);
    }
    if x == 'D' || y == 'D' {
        return Some(GitFileStatusKind::Removed);
    }
    if matches!(x, 'A' | 'C' | 'R') || matches!(y, 'A' | 'C' | 'R') {
        return Some(GitFileStatusKind::Added);
    }
    if x != ' ' || y != ' ' {
        return Some(GitFileStatusKind::Modified);
    }
    None
}

fn is_conflict_status(x: char, y: char) -> bool {
    matches!((x, y), ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D'))
}

fn set_repo_status(
    statuses: &mut HashMap<PathBuf, GitFileStatusKind>,
    path: PathBuf,
    status: GitFileStatusKind,
) {
    match statuses.get(&path).copied() {
        Some(existing) if existing.priority() >= status.priority() => {}
        _ => {
            statuses.insert(path, status);
        }
    }
}

fn start_diff_worker() -> (SyncSender<GitDiffJob>, Receiver<GitDiffResult>) {
    let (job_tx, job_rx) = mpsc::sync_channel::<GitDiffJob>(REPO_STATUS_QUEUE_BOUND);
    let (result_tx, result_rx) = mpsc::channel();
    thread::Builder::new()
        .name("redox-git-diff".to_string())
        .spawn(move || {
            let mut worker = GitDiffWorker::default();
            while let Ok(job) = job_rx.recv() {
                let snapshot = worker.diff(&job.path, &job.buffer.to_string(), job.base_generation);
                if result_tx
                    .send(GitDiffResult {
                        buffer_id: job.buffer_id,
                        path: Some(job.path),
                        request_id: job.request_id,
                        snapshot,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .expect("failed to start git diff worker");
    (job_tx, result_rx)
}

#[derive(Default)]
struct GitDiffWorker {
    // ponytail: retain one file's baseline; expand only if split-pane profiling warrants it.
    base: Option<GitDiffBase>,
    current_file: Option<NamedTempFile>,
}

struct GitDiffBase {
    path: PathBuf,
    repo_root: PathBuf,
    head: Option<String>,
    generation: u64,
    file: NamedTempFile,
}

impl GitDiffWorker {
    fn diff(
        &mut self,
        path: &Path,
        current_text: &str,
        generation: u64,
    ) -> Option<GitDiffSnapshot> {
        let cached_root = self
            .base
            .as_ref()
            .filter(|base| {
                base.path == path
                    && base.generation == generation
                    && base.repo_root.join(".git").exists()
                    && !path
                        .ancestors()
                        .skip(1)
                        .take_while(|dir| *dir != base.repo_root)
                        .any(|dir| dir.join(".git").exists())
            })
            .map(|base| base.repo_root.clone());
        let repo_root = match cached_root {
            Some(root) => root,
            None => {
                PathBuf::from(git_stdout(path.parent()?, &["rev-parse", "--show-toplevel"])?.trim())
            }
        };
        // Check HEAD on each job so a commit, checkout, reset, or packed ref cannot leave a stale baseline.
        let head = git_stdout(&repo_root, &["rev-parse", "--verify", "HEAD"]);
        if self.base.as_ref().is_none_or(|base| {
            base.path != path
                || base.repo_root != repo_root
                || base.head != head
                || base.generation != generation
        }) {
            let relative = path
                .strip_prefix(&repo_root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let text = head
                .as_ref()
                .and_then(|head| {
                    git_stdout(
                        &repo_root,
                        &["show", &format!("{}:{relative}", head.trim())],
                    )
                })
                .unwrap_or_default();
            let mut file = NamedTempFile::new().ok()?;
            file.write_all(text.as_bytes()).ok()?;
            self.base = Some(GitDiffBase {
                path: path.to_path_buf(),
                repo_root: repo_root.clone(),
                head,
                generation,
                file,
            });
        }
        if self.current_file.is_none() {
            self.current_file = Some(NamedTempFile::new().ok()?);
        }
        let current_file = self.current_file.as_mut()?;
        current_file.as_file_mut().set_len(0).ok()?;
        current_file.as_file_mut().seek(SeekFrom::Start(0)).ok()?;
        current_file.write_all(current_text.as_bytes()).ok()?;
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo_root)
            .args(["diff", "--no-index", "--no-ext-diff", "--unified=0", "--"])
            .arg(self.base.as_ref()?.file.path())
            .arg(current_file.path())
            .output()
            .ok()?;
        if !(output.status.success() || output.status.code() == Some(1)) {
            return None;
        }
        Some(parse_git_patch(&String::from_utf8(output.stdout).ok()?))
    }
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn parse_git_patch(patch: &str) -> GitDiffSnapshot {
    let mut stats = GitDiffStats::default();
    let mut markers = BTreeMap::new();
    let mut lines = patch.lines().peekable();

    while let Some(line) = lines.next() {
        let Some(hunk) = line.strip_prefix("@@ ") else {
            continue;
        };
        let Some((old_range, rest)) = hunk.split_once(" +") else {
            continue;
        };
        let Some((new_range, _)) = rest.split_once(" @@") else {
            continue;
        };

        let (old_start, old_count) = parse_hunk_range(old_range.trim_start_matches('-'));
        let (new_start, new_count) = parse_hunk_range(new_range);
        let header_stats = GitDiffStats {
            modified: old_count.min(new_count),
            added: new_count.saturating_sub(old_count.min(new_count)),
            removed: old_count.saturating_sub(old_count.min(new_count)),
        };
        let mut hunk_stats = GitDiffStats::default();
        let mut hunk_markers = BTreeMap::new();
        let mut _old_line = old_start.saturating_sub(1);
        let mut new_line = new_start.saturating_sub(1);
        let mut removed_at_position = 0usize;
        let mut scanned_body = false;

        while let Some(body_line) = lines.peek().copied() {
            if body_line.starts_with("@@ ") {
                break;
            }

            let body_line = lines.next().unwrap_or_default();
            let Some(prefix) = body_line.as_bytes().first().copied() else {
                continue;
            };

            match prefix {
                b'+' => {
                    scanned_body = true;
                    if removed_at_position > 0 {
                        hunk_stats.modified += 1;
                        removed_at_position -= 1;
                        set_marker(&mut hunk_markers, new_line, GitGutterKind::Modified);
                    } else {
                        hunk_stats.added += 1;
                        set_marker(&mut hunk_markers, new_line, GitGutterKind::Added);
                    }
                    new_line = new_line.saturating_add(1);
                }
                b'-' => {
                    scanned_body = true;
                    removed_at_position = removed_at_position.saturating_add(1);
                    _old_line = _old_line.saturating_add(1);
                }
                b' ' => {
                    for _ in 0..removed_at_position {
                        hunk_stats.removed += 1;
                        set_marker(&mut hunk_markers, new_line, GitGutterKind::Removed);
                    }
                    removed_at_position = 0;
                    _old_line = _old_line.saturating_add(1);
                    new_line = new_line.saturating_add(1);
                }
                _ => {}
            }
        }

        for _ in 0..removed_at_position {
            hunk_stats.removed += 1;
            set_marker(&mut hunk_markers, new_line, GitGutterKind::Removed);
        }

        if scanned_body {
            stats.added += hunk_stats.added;
            stats.modified += hunk_stats.modified;
            stats.removed += hunk_stats.removed;
            for (line, kind) in hunk_markers {
                set_marker(&mut markers, line, kind);
            }
            continue;
        }

        let modified = header_stats.modified;
        let added = header_stats.added;
        let removed = header_stats.removed;

        stats.modified += modified;
        stats.added += added;
        stats.removed += removed;

        for offset in 0..modified {
            let line_idx = new_start.saturating_sub(1).saturating_add(offset);
            set_marker(&mut markers, line_idx, GitGutterKind::Modified);
        }

        for offset in 0..added {
            let line_idx = new_start
                .saturating_sub(1)
                .saturating_add(modified)
                .saturating_add(offset);
            set_marker(&mut markers, line_idx, GitGutterKind::Added);
        }

        if removed > 0 {
            let anchor = if new_count == 0 {
                new_start.saturating_sub(1)
            } else {
                new_start
                    .saturating_sub(1)
                    .saturating_add(modified.min(new_count.saturating_sub(1)))
            };
            set_marker(&mut markers, anchor, GitGutterKind::Removed);
        }

        let _ = old_start;
    }

    GitDiffSnapshot {
        stats,
        markers: markers
            .into_iter()
            .map(|(line, kind)| GitLineMarker { line, kind })
            .collect(),
    }
}

fn set_marker(markers: &mut BTreeMap<usize, GitGutterKind>, line: usize, kind: GitGutterKind) {
    match markers.get(&line).copied() {
        Some(existing) if existing.priority() >= kind.priority() => {}
        _ => {
            markers.insert(line, kind);
        }
    }
}

fn parse_hunk_range(range: &str) -> (usize, usize) {
    if let Some((start, count)) = range.split_once(',') {
        (
            start.parse::<usize>().unwrap_or(0),
            count.parse::<usize>().unwrap_or(0),
        )
    } else {
        (range.parse::<usize>().unwrap_or(0), 1)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{
        GitDiffStats, GitFileStatusKind, GitGutterKind, GitRepoStatusCacheEntry, GitState,
        parse_git_patch,
    };

    #[test]
    fn diff_worker_reuses_files_and_refreshes_head_and_worktree_baselines() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| super::git_stdout(&repo, args).expect("git command failed");
        git(&["init", "-q"]);
        let path = repo.join("example.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let mut worker = super::GitDiffWorker::default();
        assert_eq!(worker.diff(&path, "one\ntwo\n", 0).unwrap().stats.added, 2);
        git(&["add", "example.txt"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "initial",
        ]);
        assert!(
            worker
                .diff(&path, "one\ntwo\n", 0)
                .unwrap()
                .stats
                .is_empty()
        );
        let baseline = worker.base.as_ref().unwrap().file.path().to_path_buf();
        let current = worker.current_file.as_ref().unwrap().path().to_path_buf();
        assert_eq!(
            worker
                .diff(&path, "one\ntwo\nthree\n", 0)
                .unwrap()
                .stats
                .added,
            1
        );
        assert_eq!(worker.base.as_ref().unwrap().file.path(), baseline);
        assert_eq!(worker.current_file.as_ref().unwrap().path(), current);
        // A shorter document must truncate the reused temporary file.
        assert_eq!(worker.diff(&path, "one\n", 0).unwrap().stats.removed, 1);
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        git(&["add", "example.txt"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "updated",
        ]);
        assert!(
            worker
                .diff(&path, "one\ntwo\nthree\n", 0)
                .unwrap()
                .stats
                .is_empty()
        );
        assert_ne!(worker.base.as_ref().unwrap().file.path(), baseline);
        git(&["pack-refs", "--all"]);
        assert!(
            worker
                .diff(&path, "one\ntwo\nthree\n", 0)
                .unwrap()
                .stats
                .is_empty()
        );
        let linked = root.join("linked");
        git(&[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD~1",
        ]);
        assert!(
            worker
                .diff(&linked.join("example.txt"), "one\ntwo\n", 0)
                .unwrap()
                .stats
                .is_empty()
        );
    }

    #[test]
    fn completed_diff_does_not_hide_an_edit_made_while_it_was_running() {
        let mut state = GitState::default();
        let session = redox_core::EditorSession::open_initial_unnamed().unwrap();
        let buffer_id = session.active_id();
        let path = Some(PathBuf::from("/example.txt"));
        state.cache.insert(
            buffer_id,
            super::GitDiffCacheEntry {
                path: path.clone(),
                dirty: true,
                last_refreshed_at: std::time::Instant::now(),
                stale: true,
                snapshot: None,
                pending: true,
                request_id: 1,
            },
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        state.diff_rx = receiver;
        sender
            .send(super::GitDiffResult {
                buffer_id,
                path: path.clone(),
                request_id: 1,
                snapshot: Some(Default::default()),
            })
            .unwrap();
        state.drain_diff_results();
        assert!(!state.cache[&buffer_id].pending);
        assert!(state.cache[&buffer_id].stale);
        assert!(state.cache[&buffer_id].snapshot.is_none());
        let entry = state.cache.get_mut(&buffer_id).unwrap();
        entry.pending = true;
        entry.stale = false;
        entry.request_id = 2;
        sender
            .send(super::GitDiffResult {
                buffer_id,
                path: path.clone(),
                request_id: 1,
                snapshot: Some(Default::default()),
            })
            .unwrap();
        state.drain_diff_results();
        assert!(state.cache[&buffer_id].pending);
        sender
            .send(super::GitDiffResult {
                buffer_id,
                path,
                request_id: 2,
                snapshot: Some(Default::default()),
            })
            .unwrap();
        state.drain_diff_results();
        assert!(!state.cache[&buffer_id].pending);
        assert!(state.cache[&buffer_id].snapshot.is_some());
    }

    #[test]
    fn parse_git_patch_classifies_added_modified_and_removed_lines() {
        let patch = "\
diff --git a/old b/new
@@ -1,2 +1,3 @@
@@ -6 +7 @@
@@ -10,2 +11,0 @@
";

        let snapshot = parse_git_patch(patch);
        assert_eq!(
            snapshot.stats,
            GitDiffStats {
                added: 1,
                modified: 3,
                removed: 2,
            }
        );
        assert_eq!(snapshot.marker_for_line(0), Some(GitGutterKind::Modified));
        assert_eq!(snapshot.marker_for_line(1), Some(GitGutterKind::Modified));
        assert_eq!(snapshot.marker_for_line(2), Some(GitGutterKind::Added));
        assert_eq!(snapshot.marker_for_line(6), Some(GitGutterKind::Modified));
        assert_eq!(snapshot.marker_for_line(10), Some(GitGutterKind::Removed));
    }

    #[test]
    fn status_for_path_prefers_deepest_matching_repo_root() {
        let outer = PathBuf::from("/tmp/work");
        let inner = outer.join("nested");
        let path = inner.join("src/lib.rs");
        let mut state = GitState::default();

        state.repo_status_cache.insert(
            outer.clone(),
            GitRepoStatusCacheEntry {
                file_statuses: HashMap::new(),
                directory_statuses: HashMap::from([(inner.clone(), GitFileStatusKind::Modified)]),
                stale: false,
            },
        );
        state.repo_status_cache.insert(
            inner,
            GitRepoStatusCacheEntry {
                file_statuses: HashMap::from([(path.clone(), GitFileStatusKind::Added)]),
                directory_statuses: HashMap::new(),
                stale: false,
            },
        );

        assert_eq!(state.status_for_path(&path), Some(GitFileStatusKind::Added));
    }
}
