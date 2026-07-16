use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use ignore::WalkBuilder;
use redox_core::{
    FuzzyQuery, PathMatchScore, compare_path_match_scores, fuzzy_match_ranges, path_match_score,
};

use super::actions::{backspace_at_cursor, insert_at_cursor, move_cursor_left, move_cursor_right};
use super::{EditorMode, EditorState};

const MAX_PINNED_FILES: usize = 5;
const PREVIEW_MAX_BYTES: usize = 32 * 1024;
const FINDER_INDEX_BATCH_SIZE: usize = 128;
const MAX_FINDER_INDEX_BATCHES_PER_POLL: usize = 8;

#[derive(Debug, Clone)]
pub struct FinderPopup {
    pub entries: Vec<FinderPopupEntry>,
    pub query: String,
    pub query_cursor: usize,
    pub selected: usize,
    pub result_count: usize,
    pub total_count: usize,
    pub preview: Option<FinderPreview>,
}

#[derive(Debug, Clone)]
pub struct FinderPopupEntry {
    pub label: String,
    pub highlights: Vec<Range<usize>>,
    pub is_pinned: bool,
    pub hotkey: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FinderPreview {
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PinSelectorPopup {
    pub path_label: String,
    pub slots: Vec<PinSelectorSlot>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct PinSelectorSlot {
    pub slot: usize,
    pub path_label: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct FinderState {
    launch_dir: PathBuf,
    query: String,
    query_cursor: usize,
    all_files: Vec<FinderFileCandidate>,
    file_results: Vec<FinderFileResult>,
    combined_entries: Vec<FinderCombinedEntry>,
    selected: usize,
    anchor_selection_to_bottom: bool,
    preview: Option<FinderPreviewCache>,
}

#[derive(Debug)]
pub(super) struct FinderIndexWorker {
    results: Receiver<FinderIndexMessage>,
}

#[derive(Debug, Clone)]
pub(super) struct FinderFileCandidate {
    path: PathBuf,
    label: String,
}

#[derive(Debug, Clone)]
struct FinderFileResult {
    path: PathBuf,
    label: String,
    highlights: Vec<Range<usize>>,
    score: PathMatchScore,
}

#[derive(Debug, Clone)]
struct FinderCombinedEntry {
    path: PathBuf,
    label: String,
    highlights: Vec<Range<usize>>,
    kind: FinderCombinedKind,
}

#[derive(Debug, Clone)]
struct PinnedFileEntry {
    slot: usize,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinderCombinedKind {
    Pinned { slot: usize },
    File,
}

#[derive(Debug, Clone)]
struct FinderPreviewCache {
    path: PathBuf,
    preview: FinderPreview,
}

enum FinderIndexMessage {
    Batch(Vec<FinderFileCandidate>),
    Done,
}

#[derive(Debug, Clone)]
pub(super) struct PinSelectorState {
    source_path: Option<PathBuf>,
    selected_slot: usize,
    return_mode: EditorMode,
}

#[derive(Debug, Clone)]
pub(super) struct PinnedFilesState {
    slots: Vec<Option<PathBuf>>,
    storage_path: PathBuf,
}

impl FinderState {
    fn new(
        launch_dir: PathBuf,
        all_files: Vec<FinderFileCandidate>,
        pinned_files: &[PinnedFileEntry],
    ) -> Self {
        let initial_selected = all_files.len().saturating_sub(1);
        let mut state = Self {
            launch_dir,
            query: String::new(),
            query_cursor: 0,
            all_files,
            file_results: Vec::new(),
            combined_entries: Vec::new(),
            selected: initial_selected,
            anchor_selection_to_bottom: true,
            preview: None,
        };
        state.refresh_results(pinned_files, None);
        state
    }

    fn popup(&self) -> FinderPopup {
        FinderPopup {
            entries: self
                .combined_entries
                .iter()
                .map(|entry| FinderPopupEntry {
                    label: entry.label.clone(),
                    highlights: entry.highlights.clone(),
                    is_pinned: matches!(entry.kind, FinderCombinedKind::Pinned { .. }),
                    hotkey: match entry.kind {
                        FinderCombinedKind::Pinned { slot } => Some(format!("Ctrl+{}", slot + 1)),
                        FinderCombinedKind::File => None,
                    },
                })
                .collect(),
            query: self.query.clone(),
            query_cursor: self.query_cursor,
            selected: self.selected,
            result_count: self.file_results.len(),
            total_count: self.all_files.len(),
            preview: self.preview.as_ref().map(|preview| preview.preview.clone()),
        }
    }

    fn selected_path(&self) -> Option<&Path> {
        self.combined_entries
            .get(self.selected)
            .map(|entry| entry.path.as_path())
    }

    fn selected_entry_is_pinned(&self) -> bool {
        self.combined_entries
            .get(self.selected)
            .is_some_and(|entry| matches!(entry.kind, FinderCombinedKind::Pinned { .. }))
    }

    fn move_selection(&mut self, delta: isize) {
        if self.combined_entries.is_empty() {
            self.selected = 0;
            return;
        }

        let max_index = self.combined_entries.len().saturating_sub(1) as isize;
        let next = (self.selected as isize + delta).clamp(0, max_index) as usize;
        self.selected = next;
        self.anchor_selection_to_bottom = false;
    }

    fn set_query_char(&mut self, ch: char) {
        insert_at_cursor(&mut self.query, &mut self.query_cursor, ch);
    }

    fn pop_query_char(&mut self) {
        backspace_at_cursor(&mut self.query, &mut self.query_cursor);
    }

    fn move_query_cursor_left(&mut self) {
        move_cursor_left(&self.query, &mut self.query_cursor);
    }

    fn move_query_cursor_right(&mut self) {
        move_cursor_right(&self.query, &mut self.query_cursor);
    }

    fn refresh_results(&mut self, pinned_files: &[PinnedFileEntry], preferred_path: Option<&Path>) {
        let query = FuzzyQuery::new(&self.query);
        self.file_results = self
            .all_files
            .iter()
            .filter_map(|candidate| filter_file_result(candidate, &query))
            .collect();
        self.file_results.sort_by(compare_file_results);
        self.file_results.reverse();

        self.rebuild_combined_entries(pinned_files, preferred_path);
        self.refresh_preview();
    }

    fn add_file_candidates(
        &mut self,
        candidates: Vec<FinderFileCandidate>,
        pinned_files: &[PinnedFileEntry],
    ) {
        if candidates.is_empty() {
            return;
        }

        self.all_files.extend(candidates);
        self.all_files
            .sort_by(|left, right| left.path.cmp(&right.path));
        self.all_files
            .dedup_by(|left, right| left.path == right.path);
        self.refresh_results(pinned_files, None);
    }

    fn replace_file_candidates(
        &mut self,
        mut candidates: Vec<FinderFileCandidate>,
        pinned_files: &[PinnedFileEntry],
    ) {
        candidates.sort_by(|left, right| left.path.cmp(&right.path));
        candidates.dedup_by(|left, right| left.path == right.path);
        self.all_files = candidates;
        self.refresh_results(pinned_files, None);
    }

    fn rebuild_combined_entries(
        &mut self,
        pinned_files: &[PinnedFileEntry],
        preferred_path: Option<&Path>,
    ) {
        let previous_path = preferred_path
            .map(Path::to_path_buf)
            .or_else(|| self.selected_path().map(Path::to_path_buf));

        self.combined_entries.clear();
        let query = FuzzyQuery::new(&self.query);
        for pinned_file in pinned_files {
            let slot = pinned_file.slot;
            let path = &pinned_file.path;
            let label = display_path_for_popup(path, &self.launch_dir);
            let highlights = fuzzy_match_ranges(&label, &query)
                .map(|matched| matched.highlights)
                .unwrap_or_default();
            self.combined_entries.push(FinderCombinedEntry {
                path: path.clone(),
                label,
                highlights,
                kind: FinderCombinedKind::Pinned { slot },
            });
        }

        let pinned_paths = pinned_files
            .iter()
            .map(|pinned| pinned.path.as_path())
            .collect::<HashSet<_>>();
        self.combined_entries.extend(
            self.file_results
                .iter()
                .filter(|result| !pinned_paths.contains(result.path.as_path()))
                .map(|result| FinderCombinedEntry {
                    path: result.path.clone(),
                    label: result.label.clone(),
                    highlights: result.highlights.clone(),
                    kind: FinderCombinedKind::File,
                }),
        );

        if self.combined_entries.is_empty() {
            self.selected = 0;
            self.preview = None;
            return;
        }

        self.selected = if self.anchor_selection_to_bottom && preferred_path.is_none() {
            self.last_file_entry_index()
        } else {
            previous_path
                .as_ref()
                .and_then(|path| {
                    self.combined_entries
                        .iter()
                        .position(|entry| &entry.path == path)
                })
                .unwrap_or_else(|| self.last_file_entry_index())
        };
    }

    fn refresh_results_to_bottom(&mut self, pinned_files: &[PinnedFileEntry]) {
        self.refresh_results(pinned_files, None);
        self.selected = self.last_file_entry_index();
        self.anchor_selection_to_bottom = true;
    }

    fn last_file_entry_index(&self) -> usize {
        if self.combined_entries.is_empty() {
            return 0;
        }

        self.combined_entries
            .iter()
            .rposition(|entry| matches!(entry.kind, FinderCombinedKind::File))
            .unwrap_or_else(|| self.combined_entries.len().saturating_sub(1))
    }

    fn refresh_preview(&mut self) {
        let Some(path) = self.selected_path().map(Path::to_path_buf) else {
            self.preview = None;
            return;
        };

        if self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.path == path)
        {
            return;
        }

        self.preview = Some(FinderPreviewCache {
            path: path.clone(),
            preview: load_preview(&path, &self.launch_dir),
        });
    }
}

impl FinderIndexWorker {
    fn spawn(root: PathBuf) -> Self {
        let (result_tx, result_rx) = mpsc::channel::<FinderIndexMessage>();
        thread::Builder::new()
            .name("redox-finder-index".to_string())
            .spawn(move || {
                let mut batch = Vec::with_capacity(FINDER_INDEX_BATCH_SIZE);
                let walker = WalkBuilder::new(&root)
                    .hidden(true)
                    .git_ignore(true)
                    .git_global(true)
                    .git_exclude(true)
                    .require_git(false)
                    .build();

                for entry in walker.filter_map(Result::ok) {
                    let path = entry.into_path();
                    if !path.is_file() {
                        continue;
                    }

                    batch.push(FinderFileCandidate {
                        label: display_path_for_popup(&path, &root),
                        path,
                    });

                    if batch.len() >= FINDER_INDEX_BATCH_SIZE {
                        if result_tx
                            .send(FinderIndexMessage::Batch(std::mem::take(&mut batch)))
                            .is_err()
                        {
                            return;
                        }
                    }
                }

                if !batch.is_empty() && result_tx.send(FinderIndexMessage::Batch(batch)).is_err() {
                    return;
                }
                let _ = result_tx.send(FinderIndexMessage::Done);
            })
            .expect("failed to spawn finder index worker");

        Self { results: result_rx }
    }

    fn try_recv(&self) -> Option<FinderIndexMessage> {
        self.results.try_recv().ok()
    }
}

impl PinnedFilesState {
    pub(super) fn load() -> Self {
        let storage_path = pinned_files_storage_path();
        let mut slots = vec![None; MAX_PINNED_FILES];
        if let Ok(contents) = fs::read_to_string(&storage_path) {
            slots = parse_pinned_files(&contents);
        }

        Self {
            slots,
            storage_path,
        }
    }

    #[cfg(test)]
    fn occupied_paths(&self) -> Vec<PathBuf> {
        self.slots.iter().flatten().cloned().collect()
    }

    fn occupied_entries(&self) -> Vec<PinnedFileEntry> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, path)| {
                path.as_ref().map(|path| PinnedFileEntry {
                    slot,
                    path: path.clone(),
                })
            })
            .collect()
    }

    #[cfg(test)]
    fn slots(&self) -> &[Option<PathBuf>] {
        &self.slots
    }

    fn len(&self) -> usize {
        self.slots.len()
    }

    fn get(&self, slot: usize) -> Option<&PathBuf> {
        self.slots.get(slot).and_then(Option::as_ref)
    }

    fn index_of(&self, path: &Path) -> Option<usize> {
        self.slots
            .iter()
            .position(|existing| existing.as_deref() == Some(path))
    }

    fn first_open_slot(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }

    fn assign_slot(&mut self, slot: usize, path: PathBuf) -> Option<PathBuf> {
        if slot >= self.slots.len() {
            return None;
        }

        for (existing_slot, existing) in self.slots.iter_mut().enumerate() {
            if existing_slot != slot && existing.as_ref() == Some(&path) {
                *existing = None;
            }
        }

        let replaced = self.slots[slot].replace(path.clone());
        replaced.filter(|replaced| replaced != &path)
    }

    fn swap_slots(&mut self, left: usize, right: usize) -> bool {
        if left >= self.slots.len() || right >= self.slots.len() {
            return false;
        }
        self.slots.swap(left, right);
        true
    }

    fn remove_at(&mut self, slot: usize) -> Option<PathBuf> {
        self.slots.get_mut(slot).and_then(Option::take)
    }

    pub(super) fn remap_paths(&mut self, renamed_paths: &[(PathBuf, PathBuf)]) -> bool {
        let mut changed = false;
        for slot in &mut self.slots {
            let Some(path) = slot.as_mut() else {
                continue;
            };

            let mut remapped = path.clone();
            for (old_path, new_path) in renamed_paths {
                if remapped == *old_path {
                    remapped = new_path.clone();
                    changed = true;
                    continue;
                }

                if let Ok(relative) = remapped.strip_prefix(old_path) {
                    remapped = new_path.join(relative);
                    changed = true;
                }
            }

            *path = fs::canonicalize(&remapped).unwrap_or(remapped);
        }

        let mut seen = HashSet::new();
        for slot in &mut self.slots {
            let Some(path) = slot.as_ref() else {
                continue;
            };
            if !seen.insert(path.clone()) {
                *slot = None;
                changed = true;
            }
        }

        changed
    }

    pub(super) fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let output = self
            .slots
            .iter()
            .take(MAX_PINNED_FILES)
            .map(|path| path.as_ref().map(|path| path.display().to_string()))
            .collect::<Vec<Option<String>>>();
        let output = serde_json::to_vec(&output)?;
        let temp_path = self.storage_path.with_extension("tmp");
        fs::write(&temp_path, output)?;
        fs::rename(temp_path, &self.storage_path)
    }
}

fn parse_pinned_files(contents: &str) -> Vec<Option<PathBuf>> {
    serde_json::from_str::<Vec<Option<String>>>(contents)
        .map(|entries| parse_json_pinned_files(entries))
        .unwrap_or_else(|_| parse_legacy_pinned_files(contents))
}

fn parse_json_pinned_files(entries: Vec<Option<String>>) -> Vec<Option<PathBuf>> {
    let mut slots = vec![None; MAX_PINNED_FILES];
    for (slot, path) in entries.into_iter().take(MAX_PINNED_FILES).enumerate() {
        let Some(path) = path else {
            continue;
        };
        store_pinned_path(&mut slots, slot, &path);
    }
    slots
}

fn parse_legacy_pinned_files(contents: &str) -> Vec<Option<PathBuf>> {
    let mut slots = vec![None; MAX_PINNED_FILES];
    for (slot, line) in contents.lines().take(MAX_PINNED_FILES).enumerate() {
        if line.is_empty() || line == "-" {
            continue;
        }
        store_pinned_path(&mut slots, slot, line);
    }
    slots
}

fn store_pinned_path(slots: &mut [Option<PathBuf>], slot: usize, path: &str) {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return;
    }
    let canonical = fs::canonicalize(&path).unwrap_or(path);
    if slots
        .iter()
        .flatten()
        .any(|existing| existing == &canonical)
    {
        return;
    }
    if let Some(slot) = slots.get_mut(slot) {
        *slot = Some(canonical);
    }
}

impl EditorState {
    fn finder_or_pinboard_is_blocked(&self) -> bool {
        self.active_buffer_is_surface()
            || self.perf_visible
            || matches!(self.mode, EditorMode::Command | EditorMode::Search)
    }

    pub fn finder_popup(&self) -> Option<FinderPopup> {
        (self.mode == EditorMode::Finder)
            .then(|| self.finder.as_ref().map(FinderState::popup))
            .flatten()
    }

    pub fn pin_selector_popup(&self) -> Option<PinSelectorPopup> {
        let selector = self.pin_selector.as_ref()?;
        Some(PinSelectorPopup {
            path_label: selector
                .source_path
                .as_ref()
                .map(|path| display_path_for_popup(path, self.session.launch_dir()))
                .unwrap_or_else(|| "Pinned files".to_string()),
            slots: (0..MAX_PINNED_FILES)
                .map(|slot| PinSelectorSlot {
                    slot,
                    path_label: self
                        .pinned_files
                        .get(slot)
                        .map(|path| display_path_for_popup(path, self.session.launch_dir())),
                })
                .collect(),
            selected: selector.selected_slot,
        })
    }

    pub(super) fn open_finder(&mut self) {
        if self.finder_or_pinboard_is_blocked() {
            return;
        }

        let launch_dir = self.session.launch_dir().to_path_buf();
        let pinned_files = self.pinned_files.occupied_entries();
        let cached_files = self
            .finder_index_cache
            .get(&launch_dir)
            .cloned()
            .unwrap_or_default();
        self.finder = Some(FinderState::new(
            launch_dir.clone(),
            cached_files,
            &pinned_files,
        ));
        self.finder_index_files.clear();
        self.finder_index_worker = Some(FinderIndexWorker::spawn(launch_dir));
        self.pin_selector = None;
        self.mode = EditorMode::Finder;
        self.clear_status();
        self.input.reset_prefixes();
    }

    pub(super) fn close_finder(&mut self) {
        self.finder = None;
        self.finder_index_worker = None;
        self.finder_index_files.clear();
        self.mode = EditorMode::Normal;
        self.clear_status();
        self.input.reset_prefixes();
    }

    pub fn poll_finder_results(&mut self) {
        let Some(worker) = self.finder_index_worker.take() else {
            return;
        };

        let mut keep_worker = true;
        let mut pending_candidates = Vec::new();
        let mut replace_candidates = None;

        for _ in 0..MAX_FINDER_INDEX_BATCHES_PER_POLL {
            let Some(message) = worker.try_recv() else {
                break;
            };

            match message {
                FinderIndexMessage::Batch(candidates) => {
                    self.finder_index_files.extend(candidates.iter().cloned());
                    pending_candidates.extend(candidates);
                }
                FinderIndexMessage::Done => {
                    let fresh_files = std::mem::take(&mut self.finder_index_files);
                    let launch_dir = self.session.launch_dir().to_path_buf();
                    self.finder_index_cache
                        .insert(launch_dir, fresh_files.clone());
                    replace_candidates = Some(fresh_files);
                    keep_worker = false;
                    break;
                }
            }
        }

        if let Some(finder) = self.finder.as_mut() {
            let pinned = self.pinned_files.occupied_entries();
            if let Some(candidates) = replace_candidates {
                finder.replace_file_candidates(candidates, &pinned);
            } else {
                finder.add_file_candidates(pending_candidates, &pinned);
            }
        }

        if keep_worker && self.finder.is_some() {
            self.finder_index_worker = Some(worker);
        }
    }

    pub(super) fn finder_type_char(&mut self, ch: char) {
        let pinned = self.pinned_files.occupied_entries();
        if let Some(finder) = self.finder.as_mut() {
            finder.set_query_char(ch);
            finder.refresh_results_to_bottom(&pinned);
        }
    }

    pub(super) fn finder_backspace(&mut self) {
        let pinned = self.pinned_files.occupied_entries();
        if let Some(finder) = self.finder.as_mut() {
            finder.pop_query_char();
            finder.refresh_results_to_bottom(&pinned);
        }
    }

    pub(super) fn finder_move_query_cursor_left(&mut self) {
        if let Some(finder) = self.finder.as_mut() {
            finder.move_query_cursor_left();
        }
    }

    pub(super) fn finder_move_query_cursor_right(&mut self) {
        if let Some(finder) = self.finder.as_mut() {
            finder.move_query_cursor_right();
        }
    }

    pub(super) fn finder_move_selection(&mut self, delta: isize) {
        if let Some(finder) = self.finder.as_mut() {
            finder.move_selection(delta);
            finder.refresh_preview();
        }
    }

    pub(super) fn begin_pin_selection_for_current_buffer(&mut self) {
        if self.finder_or_pinboard_is_blocked() {
            return;
        }

        let source_path = self
            .session
            .active_meta()
            .path
            .clone()
            .map(|path| fs::canonicalize(&path).unwrap_or(path));
        self.begin_pin_selection(source_path, self.mode);
    }

    pub(super) fn begin_pin_selection_for_finder_entry(&mut self) {
        let Some(path) = self
            .finder
            .as_ref()
            .and_then(|finder| finder.selected_path().map(Path::to_path_buf))
        else {
            self.set_status("no file selected");
            return;
        };

        self.begin_pin_selection(Some(path), EditorMode::Finder);
    }

    fn begin_pin_selection(&mut self, source_path: Option<PathBuf>, return_mode: EditorMode) {
        let selected_slot = source_path
            .as_ref()
            .and_then(|path| self.pinned_files.index_of(path))
            .unwrap_or_else(|| {
                self.pinned_files
                    .first_open_slot()
                    .unwrap_or(MAX_PINNED_FILES.saturating_sub(1))
            });
        self.pin_selector = Some(PinSelectorState {
            source_path,
            selected_slot,
            return_mode,
        });
        self.mode = EditorMode::PinSelect;
        self.clear_status();
        self.input.reset_prefixes();
    }

    pub(super) fn cancel_pin_selection(&mut self) {
        let Some(selector) = self.pin_selector.take() else {
            return;
        };
        self.mode = selector.return_mode;
        self.clear_status();
        self.input.reset_prefixes();
    }

    pub(super) fn pin_selector_move(&mut self, delta: isize) {
        let Some(selector) = self.pin_selector.as_mut() else {
            return;
        };
        let max_index = MAX_PINNED_FILES.saturating_sub(1) as isize;
        selector.selected_slot =
            (selector.selected_slot as isize + delta).clamp(0, max_index) as usize;
    }

    pub(super) fn assign_selected_pin_slot(&mut self) {
        let Some(selector) = self.pin_selector.as_ref() else {
            return;
        };
        self.assign_pin_slot(selector.selected_slot);
    }

    pub(super) fn assign_pin_slot(&mut self, slot: usize) {
        let Some(selector) = self.pin_selector.as_ref().cloned() else {
            return;
        };
        let Some(source_path) = selector
            .source_path
            .clone()
            .or_else(|| self.current_active_file_path())
        else {
            self.set_status("no file available to pin");
            return;
        };

        let previous_slots = self.pinned_files.slots.clone();
        let dropped = self.pinned_files.assign_slot(slot, source_path.clone());
        if let Err(err) = self.pinned_files.save() {
            self.pinned_files.slots = previous_slots;
            self.mode = EditorMode::PinSelect;
            self.pin_selector = Some(selector);
            self.set_status(format!("pin save failed: {err}"));
            return;
        }

        self.pin_selector = None;
        self.mode = selector.return_mode;

        if let Some(finder) = self.finder.as_mut() {
            let pinned_files = self.pinned_files.occupied_entries();
            finder.refresh_results(&pinned_files, Some(&source_path));
        }

        let mut status = format!(
            "pinned {} to slot {}",
            display_path_for_popup(&source_path, self.session.launch_dir()),
            slot + 1
        );
        if let Some(dropped) = dropped
            && dropped != source_path
        {
            status.push_str(&format!(
                " and dropped {}",
                display_path_for_popup(&dropped, self.session.launch_dir())
            ));
        }
        self.set_status(status);
    }

    pub(super) fn pin_selector_reorder_selected(&mut self, direction: isize) {
        let Some(selector) = self.pin_selector.as_mut() else {
            return;
        };
        let slot = selector.selected_slot;
        let next_slot = slot as isize + direction;
        if next_slot < 0 || next_slot >= self.pinned_files.len() as isize {
            return;
        }
        let previous_slots = self.pinned_files.slots.clone();
        if !self.pinned_files.swap_slots(slot, next_slot as usize) {
            return;
        }
        if let Err(err) = self.pinned_files.save() {
            self.pinned_files.slots = previous_slots;
            self.set_status(format!("pin save failed: {err}"));
            return;
        }
        selector.selected_slot = next_slot as usize;

        if let Some(finder) = self.finder.as_mut() {
            let preferred_path = selector.source_path.as_deref().or_else(|| {
                self.pinned_files
                    .get(selector.selected_slot)
                    .map(PathBuf::as_path)
            });
            let pinned_files = self.pinned_files.occupied_entries();
            finder.refresh_results(&pinned_files, preferred_path);
        }
    }

    pub(super) fn pin_selector_delete_selected(&mut self) {
        let Some(selector) = self.pin_selector.as_mut() else {
            return;
        };
        let slot = selector.selected_slot;
        let Some(removed) = self.pinned_files.remove_at(slot) else {
            return;
        };
        if let Err(err) = self.pinned_files.save() {
            self.set_status(format!("pin save failed: {err}"));
            let _ = self.pinned_files.assign_slot(slot, removed);
            return;
        }

        if let Some(finder) = self.finder.as_mut() {
            let preferred_path = selector.source_path.as_deref().or_else(|| {
                self.pinned_files
                    .get(selector.selected_slot)
                    .map(PathBuf::as_path)
            });
            let pinned_files = self.pinned_files.occupied_entries();
            finder.refresh_results(&pinned_files, preferred_path);
        }

        self.set_status(format!(
            "unpinned {} from slot {}",
            display_path_for_popup(&removed, self.session.launch_dir()),
            slot + 1
        ));
    }

    pub(super) fn open_selected_finder_entry(&mut self) {
        let Some((path, is_pinned)) = self.finder.as_ref().and_then(|finder| {
            finder
                .selected_path()
                .map(|path| (path.to_path_buf(), finder.selected_entry_is_pinned()))
        }) else {
            return;
        };
        self.finder = None;
        self.finder_index_worker = None;
        self.finder_index_files.clear();
        self.mode = EditorMode::Normal;
        if is_pinned {
            self.open_pinned_path_in_editor(path);
        } else {
            self.open_path_in_editor(path, false);
        }
    }

    pub(super) fn open_pinned_slot(&mut self, slot: usize) {
        let Some(path) = self.pinned_files.get(slot).cloned() else {
            self.set_status(format!("pin slot {} is empty", slot + 1));
            return;
        };

        self.clear_active_visual_anchor();
        self.finder = None;
        self.finder_index_worker = None;
        self.finder_index_files.clear();
        self.pin_selector = None;
        self.mode = EditorMode::Normal;
        self.open_pinned_path_in_editor(path);
    }

    pub(super) fn open_selected_pin_selector_entry(&mut self) {
        let Some(slot) = self
            .pin_selector
            .as_ref()
            .map(|selector| selector.selected_slot)
        else {
            return;
        };
        self.open_pinned_slot(slot);
    }

    fn open_pinned_path_in_editor(&mut self, path: PathBuf) {
        if self.transient_origin_dir.is_none() {
            self.transient_origin_dir = Some(self.session.launch_dir().to_path_buf());
        }
        if self.transient_origin_buffer_id.is_none() {
            self.transient_origin_buffer_id = Some(self.session.active_id());
        }
        self.open_path_in_editor(path, true);
    }

    fn open_path_in_editor(&mut self, path: PathBuf, preserve_transient_origin: bool) {
        if !preserve_transient_origin {
            self.transient_origin_buffer_id = None;
            self.transient_origin_dir = None;
        }
        let previous_id = self.session.active_id();
        let close_previous_placeholder = self.is_empty_unnamed_startup_buffer(previous_id);
        match self.session.open_file(&path) {
            Ok(id) => {
                let _ = self.views.entry(id).or_default();
                self.ensure_buffer_analysis(id);
                if close_previous_placeholder && previous_id != id {
                    let _ = self.close_inactive_empty_unnamed_startup_buffer(previous_id);
                }
                self.clear_status();
            }
            Err(err) => {
                self.set_status(format!("open failed: {err}"));
            }
        }
    }

    fn current_active_file_path(&self) -> Option<PathBuf> {
        self.session
            .active_meta()
            .path
            .clone()
            .map(|path| fs::canonicalize(&path).unwrap_or(path))
    }

    #[cfg(test)]
    pub(crate) fn pinned_files_for_test(&self) -> Vec<PathBuf> {
        self.pinned_files.occupied_paths()
    }

    #[cfg(test)]
    pub(crate) fn pin_slots_for_test(&self) -> Vec<Option<PathBuf>> {
        self.pinned_files.slots().to_vec()
    }
}

fn pinned_files_storage_path() -> PathBuf {
    crate::storage::pinned_files_path()
}

fn filter_file_result(
    candidate: &FinderFileCandidate,
    query: &FuzzyQuery,
) -> Option<FinderFileResult> {
    let matched = fuzzy_match_ranges(&candidate.label, query)?;
    let score = path_match_score(&candidate.label, &matched, query);

    Some(FinderFileResult {
        path: candidate.path.clone(),
        score,
        label: candidate.label.clone(),
        highlights: matched.highlights,
    })
}

fn compare_file_results(left: &FinderFileResult, right: &FinderFileResult) -> Ordering {
    compare_path_match_scores(&left.score, &right.score).then(left.label.cmp(&right.label))
}

fn display_path_for_popup(path: &Path, launch_dir: &Path) -> String {
    path.strip_prefix(launch_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn load_preview(path: &Path, launch_dir: &Path) -> FinderPreview {
    let title = display_path_for_popup(path, launch_dir);
    let mut buffer = Vec::with_capacity(PREVIEW_MAX_BYTES);
    let preview = File::open(path)
        .and_then(|mut file| {
            let mut limited = (&mut file).take(PREVIEW_MAX_BYTES as u64);
            limited.read_to_end(&mut buffer)
        })
        .map(|_| String::from_utf8(buffer))
        .ok();

    let lines = match preview {
        Some(Ok(contents)) => {
            let mut lines = contents.lines().map(str::to_string).collect::<Vec<_>>();
            if contents.ends_with('\n') {
                lines.push(String::new());
            }
            if lines.is_empty() {
                lines.push(String::new());
            }
            lines
        }
        Some(Err(_)) => vec!["<binary file>".to_string()],
        None => vec!["<preview unavailable>".to_string()],
    };

    FinderPreview { title, lines }
}
