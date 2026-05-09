use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use redox_core::{BufferId, BufferKind, EditorSession};
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
    pending_repo_status_dirs: HashSet<PathBuf>,
    repo_status_tx: Sender<GitRepoStatusResult>,
    repo_status_rx: Receiver<GitRepoStatusResult>,
    repo_status_job_tx: SyncSender<GitRepoStatusJob>,
    diff_tx: Sender<GitDiffResult>,
    diff_rx: Receiver<GitDiffResult>,
}

#[derive(Debug)]
struct GitDiffCacheEntry {
    path: Option<PathBuf>,
    dirty: bool,
    last_refreshed_at: Instant,
    stale: bool,
    snapshot: Option<GitDiffSnapshot>,
    pending: bool,
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
    dirty: bool,
    snapshot: Option<GitDiffSnapshot>,
}

type RepoStatuses = (
    HashMap<PathBuf, GitFileStatusKind>,
    HashMap<PathBuf, GitFileStatusKind>,
);

impl Default for GitState {
    fn default() -> Self {
        let (repo_status_tx, repo_status_rx) = mpsc::channel();
        let repo_status_job_tx = start_repo_status_workers();
        let (diff_tx, diff_rx) = mpsc::channel();
        Self {
            cache: HashMap::new(),
            repo_status_cache: HashMap::new(),
            pending_repo_status_dirs: HashSet::new(),
            repo_status_tx,
            repo_status_rx,
            repo_status_job_tx,
            diff_tx,
            diff_rx,
        }
    }
}

impl GitState {
    pub fn diff_for(&self, buffer_id: BufferId) -> Option<&GitDiffSnapshot> {
        self.cache.get(&buffer_id)?.snapshot.as_ref()
    }

    pub fn mark_stale(&mut self, buffer_id: BufferId) {
        if let Some(entry) = self.cache.get_mut(&buffer_id) {
            entry.stale = true;
        }
    }

    pub fn mark_all_repo_statuses_stale(&mut self) {
        for entry in self.repo_status_cache.values_mut() {
            entry.stale = true;
        }
    }

    pub fn status_for_path(&self, path: &Path) -> Option<GitFileStatusKind> {
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

    pub fn refresh_repo_status_for_dir(&mut self, dir: &Path) {
        self.drain_repo_status_results();

        let dir = dir.to_path_buf();
        if self.pending_repo_status_dirs.contains(&dir) {
            return;
        };
        if let Some((repo_root, entry)) = self
            .repo_status_cache
            .iter()
            .filter(|(repo_root, _)| dir.starts_with(repo_root))
            .max_by_key(|(repo_root, _)| repo_root.components().count())
            && !entry.stale
            && (repo_root == &dir || !dir_is_separate_repo_from_cached_root(&dir, repo_root))
        {
            return;
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

        let current_text = path
            .as_deref()
            .and_then(|buffer_path| {
                session
                    .buffer(buffer_id)
                    .map(|buffer| (buffer_path, buffer))
            })
            .map(|(buffer_path, buffer)| (buffer_path.to_path_buf(), buffer.to_string()));

        let previous_snapshot = self
            .cache
            .get(&buffer_id)
            .and_then(|entry| entry.snapshot.clone());

        self.cache.insert(
            buffer_id,
            GitDiffCacheEntry {
                path: path.clone(),
                dirty,
                last_refreshed_at: now,
                stale: false,
                snapshot: previous_snapshot,
                pending: current_text.is_some(),
            },
        );

        if let Some((buffer_path, current_text)) = current_text {
            let tx = self.diff_tx.clone();
            thread::spawn(move || {
                let snapshot = load_git_diff(&buffer_path, &current_text);
                let _ = tx.send(GitDiffResult {
                    buffer_id,
                    path: Some(buffer_path),
                    dirty,
                    snapshot,
                });
            });
        }
    }

    fn drain_repo_status_results(&mut self) {
        while let Ok(result) = self.repo_status_rx.try_recv() {
            self.pending_repo_status_dirs.remove(&result.requested_dir);
            let Some(repo_root) = result.repo_root else {
                continue;
            };
            let Some((file_statuses, directory_statuses)) = result.statuses else {
                self.repo_status_cache.remove(&repo_root);
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
        }
    }

    fn drain_diff_results(&mut self) {
        while let Ok(result) = self.diff_rx.try_recv() {
            let Some(entry) = self.cache.get_mut(&result.buffer_id) else {
                continue;
            };
            if entry.path != result.path || entry.dirty != result.dirty {
                continue;
            }
            entry.snapshot = result.snapshot;
            entry.last_refreshed_at = Instant::now();
            entry.stale = false;
            entry.pending = false;
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

fn dir_is_separate_repo_from_cached_root(dir: &Path, cached_repo_root: &Path) -> bool {
    if dir.join(".git").exists() {
        return true;
    }

    git_stdout(dir, &["rev-parse", "--show-toplevel"])
        .map(|repo_root| PathBuf::from(repo_root.trim()))
        .is_some_and(|repo_root| repo_root != cached_repo_root)
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

        let x = raw_entry[0] as char;
        let y = raw_entry[1] as char;
        let is_rename_or_copy = matches!(x, 'R' | 'C');
        let path_bytes = &raw_entry[3..];
        let Ok(path) = String::from_utf8(path_bytes.to_vec()) else {
            if is_rename_or_copy {
                let _ = entries.next();
            }
            continue;
        };
        let Some(status) = classify_repo_status(x, y) else {
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

fn load_git_diff(path: &Path, current_text: &str) -> Option<GitDiffSnapshot> {
    let repo_root = git_stdout(
        path.parent().unwrap_or_else(|| Path::new(".")),
        &["rev-parse", "--show-toplevel"],
    )?;
    let repo_root = PathBuf::from(repo_root.trim());
    let rel_path = path.strip_prefix(&repo_root).ok()?;
    let rel_path = rel_path.to_string_lossy().replace('\\', "/");

    let head_spec = format!("HEAD:{rel_path}");
    let head_text = git_stdout(&repo_root, &["show", &head_spec]).unwrap_or_default();

    let mut old_file = NamedTempFile::new().ok()?;
    let mut new_file = NamedTempFile::new().ok()?;
    old_file.write_all(head_text.as_bytes()).ok()?;
    new_file.write_all(current_text.as_bytes()).ok()?;
    let old_path = old_file.path();
    let new_path = new_file.path();

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .arg("diff")
        .arg("--no-index")
        .arg("--no-ext-diff")
        .arg("--unified=0")
        .arg("--")
        .arg(&old_path)
        .arg(&new_path)
        .output()
        .ok()?;

    if !(output.status.success() || output.status.code() == Some(1)) {
        return None;
    }

    let patch = String::from_utf8(output.stdout).ok()?;
    Some(parse_git_patch(&patch))
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
