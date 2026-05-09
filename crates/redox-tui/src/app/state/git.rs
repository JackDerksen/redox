use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redox_core::{BufferId, BufferKind, EditorSession};

const DIRTY_REFRESH_INTERVAL: Duration = Duration::from_millis(200);

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

#[derive(Debug, Default)]
pub struct GitState {
    cache: HashMap<BufferId, GitDiffCacheEntry>,
    repo_status_cache: HashMap<PathBuf, GitRepoStatusCacheEntry>,
}

#[derive(Debug)]
struct GitDiffCacheEntry {
    path: Option<PathBuf>,
    dirty: bool,
    last_refreshed_at: Instant,
    stale: bool,
    snapshot: Option<GitDiffSnapshot>,
}

#[derive(Debug)]
struct GitRepoStatusCacheEntry {
    file_statuses: HashMap<PathBuf, GitFileStatusKind>,
    directory_statuses: HashMap<PathBuf, GitFileStatusKind>,
    stale: bool,
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
        let Some(repo_root_raw) = git_stdout(dir, &["rev-parse", "--show-toplevel"])
            .map(|output| output.trim().to_string())
        else {
            return;
        };
        let repo_root = PathBuf::from(repo_root_raw);
        if self
            .repo_status_cache
            .get(&repo_root)
            .is_some_and(|entry| !entry.stale)
        {
            return;
        }
        let Some((file_statuses, directory_statuses)) = load_repo_statuses(&repo_root) else {
            self.repo_status_cache.remove(&repo_root);
            return;
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

    pub fn refresh_for_buffer(&mut self, session: &EditorSession, buffer_id: BufferId) {
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

        let snapshot = path
            .as_deref()
            .and_then(|buffer_path| {
                session
                    .buffer(buffer_id)
                    .map(|buffer| (buffer_path, buffer))
            })
            .and_then(|(buffer_path, buffer)| load_git_diff(buffer_path, &buffer.to_string()));

        self.cache.insert(
            buffer_id,
            GitDiffCacheEntry {
                path,
                dirty,
                last_refreshed_at: now,
                stale: false,
                snapshot,
            },
        );
    }
}

fn load_repo_statuses(
    repo_root: &Path,
) -> Option<(
    HashMap<PathBuf, GitFileStatusKind>,
    HashMap<PathBuf, GitFileStatusKind>,
)> {
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
        let path_bytes = &raw_entry[3..];
        let path = String::from_utf8(path_bytes.to_vec()).ok()?;
        let status = classify_repo_status(x, y)?;
        let file_path = repo_root.join(path);
        set_repo_status(&mut file_statuses, file_path.clone(), status);
        set_directory_statuses(&mut directory_statuses, repo_root, &file_path, status);

        if matches!(x, 'R' | 'C') {
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

    let old_path = temp_git_diff_path("old");
    let new_path = temp_git_diff_path("new");
    fs::write(&old_path, head_text).ok()?;
    fs::write(&new_path, current_text).ok()?;

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

    let _ = fs::remove_file(&old_path);
    let _ = fs::remove_file(&new_path);

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

fn temp_git_diff_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "redox-git-diff-{label}-{}-{nanos}.tmp",
        std::process::id()
    ))
}

fn parse_git_patch(patch: &str) -> GitDiffSnapshot {
    let mut stats = GitDiffStats::default();
    let mut markers = BTreeMap::new();

    for line in patch.lines() {
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

        let modified = old_count.min(new_count);
        let added = new_count.saturating_sub(modified);
        let removed = old_count.saturating_sub(modified);

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
    use super::{GitDiffStats, GitGutterKind, parse_git_patch};

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
}
