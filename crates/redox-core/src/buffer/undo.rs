//! Diff-based undo tree history for text buffers.
//!
//! Undo history is editor-core behaviour: frontends decide when an edit begins
//! and ends, but this module owns how changed buffer states are represented and
//! replayed.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::buffer::{Edit, Pos, TextBuffer};

pub type UndoNodeId = usize;

#[derive(Debug, Clone)]
pub struct UndoCheckpoint {
    buffer: TextBuffer,
    cursor: Pos,
    /// The base node's sequence number, which survives pruning unlike its index.
    base_sequence: u64,
    coalesced: bool,
    replaces_previous_coalesced_record: bool,
}

impl UndoCheckpoint {
    fn new(buffer: TextBuffer, cursor: Pos, base_sequence: u64) -> Self {
        Self {
            buffer,
            cursor,
            base_sequence,
            coalesced: false,
            replaces_previous_coalesced_record: false,
        }
    }

    fn coalesced(buffer: TextBuffer, cursor: Pos, base_sequence: u64) -> Self {
        Self {
            buffer,
            cursor,
            base_sequence,
            coalesced: true,
            replaces_previous_coalesced_record: false,
        }
    }

    pub fn is_coalesced(&self) -> bool {
        self.coalesced
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TextDiff {
    pub start_char: usize,
    pub deleted: String,
    pub inserted: String,
}

/// Where two texts differ, in chars: `start..before_end` of the first
/// became `start..after_end` of the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChangedRange {
    pub start: usize,
    pub before_end: usize,
    pub after_end: usize,
}

impl TextDiff {
    pub fn between(before: &TextBuffer, after: &TextBuffer) -> Option<Self> {
        let change = ChangedRange::between(before, after)?;
        Some(Self {
            start_char: change.start,
            deleted: before.slice_chars(change.start, change.before_end),
            inserted: after.slice_chars(change.start, change.after_end),
        })
    }

    pub fn forward_edit(&self) -> Edit {
        Edit::replace(
            self.start_char..self.start_char + self.deleted.chars().count(),
            self.inserted.clone(),
        )
    }

    pub fn reverse_edit(&self) -> Edit {
        Edit::replace(
            self.start_char..self.start_char + self.inserted.chars().count(),
            self.deleted.clone(),
        )
    }
}

impl ChangedRange {
    pub fn between(before: &TextBuffer, after: &TextBuffer) -> Option<Self> {
        let before_len = before.len_chars();
        let after_len = after.len_chars();
        // Compare equal rope chunks in bulk; only inspect individual bytes in
        // the chunks containing the edit. Round partial UTF-8 matches outwards.
        let prefix_bytes = common_chunk_bytes(before.chunks(), after.chunks(), false);
        if prefix_bytes == before.len_bytes() && prefix_bytes == after.len_bytes() {
            return None;
        }
        let prefix_len = before
            .byte_to_char(prefix_bytes)
            .expect("prefix is in bounds");
        let suffix_bytes = common_chunk_bytes(
            before.chunks_reversed(prefix_len..before_len),
            after.chunks_reversed(prefix_len..after_len),
            true,
        );
        let suffix_start_byte = before.len_bytes() - suffix_bytes;
        let suffix_start_char = before
            .byte_to_char(suffix_start_byte)
            .expect("suffix is in bounds");
        let suffix_len = before_len
            - suffix_start_char
            - usize::from(before.char_to_byte(suffix_start_char) < suffix_start_byte);

        Some(Self {
            start: prefix_len,
            before_end: before_len - suffix_len,
            after_end: after_len - suffix_len,
        })
    }
}

fn common_chunk_bytes<'a>(
    mut before: impl Iterator<Item = &'a str>,
    mut after: impl Iterator<Item = &'a str>,
    from_end: bool,
) -> usize {
    let mut before_chunk = &[][..];
    let mut after_chunk = &[][..];
    let mut matched = 0;
    loop {
        if before_chunk.is_empty() {
            let Some(chunk) = before.next() else {
                return matched;
            };
            before_chunk = chunk.as_bytes();
        }
        if after_chunk.is_empty() {
            let Some(chunk) = after.next() else {
                return matched;
            };
            after_chunk = chunk.as_bytes();
        }
        let length = before_chunk.len().min(after_chunk.len());
        let (before_part, after_part) = if from_end {
            (
                &before_chunk[before_chunk.len() - length..],
                &after_chunk[after_chunk.len() - length..],
            )
        } else {
            (&before_chunk[..length], &after_chunk[..length])
        };
        // A rope and its edited clone share untouched chunks, so equal
        // addresses skip comparing their bytes.
        if !std::ptr::eq(before_part, after_part) && before_part != after_part {
            let matching_bytes = if from_end {
                before_part
                    .iter()
                    .rev()
                    .zip(after_part.iter().rev())
                    .take_while(|(left, right)| left == right)
                    .count()
            } else {
                before_part
                    .iter()
                    .zip(after_part)
                    .take_while(|(left, right)| left == right)
                    .count()
            };
            return matched + matching_bytes;
        }
        matched += length;
        if from_end {
            before_chunk = &before_chunk[..before_chunk.len() - length];
            after_chunk = &after_chunk[..after_chunk.len() - length];
        } else {
            before_chunk = &before_chunk[length..];
            after_chunk = &after_chunk[length..];
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UndoRecord {
    pub diff: TextDiff,
    pub before_cursor: Pos,
    pub after_cursor: Pos,
    coalesced: bool,
    replaces_previous_coalesced_record: bool,
    #[serde(skip)]
    mark_positions: Vec<(char, Pos, Pos)>,
}

impl UndoRecord {
    fn from_checkpoint(
        before: UndoCheckpoint,
        after_buffer: &TextBuffer,
        after_cursor: Pos,
    ) -> Option<Self> {
        let diff = TextDiff::between(&before.buffer, after_buffer)?;
        Some(Self {
            mark_positions: before
                .buffer
                .marks()
                .filter_map(|(name, position)| {
                    after_buffer.mark(name).map(|after| (name, position, after))
                })
                .collect(),
            diff,
            before_cursor: before.cursor,
            after_cursor,
            coalesced: before.coalesced,
            replaces_previous_coalesced_record: before.replaces_previous_coalesced_record,
        })
    }

    fn apply(&self, buffer: &mut TextBuffer, reverse: bool) {
        // Restore existing anchors exactly, while letting marks set later follow the edit.
        let marks = self
            .mark_positions
            .iter()
            .filter_map(|&(name, before, after)| {
                let (expected, restored) = if reverse {
                    (after, before)
                } else {
                    (before, after)
                };
                (buffer.mark(name) == Some(expected)).then_some((name, restored))
            })
            .collect::<Vec<_>>();
        buffer.apply_edit(if reverse {
            self.diff.reverse_edit()
        } else {
            self.diff.forward_edit()
        });
        for (name, position) in marks {
            buffer.set_mark(name, position);
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct UndoNode {
    parent: Option<UndoNodeId>,
    children: Vec<UndoNodeId>,
    record: Option<UndoRecord>,
    sequence: u64,
    created_at_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoTreeEntry {
    pub id: UndoNodeId,
    pub parent: Option<UndoNodeId>,
    pub sequence: u64,
    pub created_at_ms: u128,
    pub is_current: bool,
    pub child_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UndoHistory {
    nodes: Vec<UndoNode>,
    current: UndoNodeId,
    next_sequence: u64,
    #[serde(skip)]
    insert_mode_coalesce_base: Option<UndoCheckpoint>,
    #[serde(skip, default = "unbounded_max_records")]
    max_records: usize,
}

impl Default for UndoHistory {
    fn default() -> Self {
        Self {
            nodes: vec![UndoNode {
                parent: None,
                children: Vec::new(),
                record: None,
                sequence: 0,
                created_at_ms: now_ms(),
            }],
            current: 0,
            next_sequence: 1,
            insert_mode_coalesce_base: None,
            max_records: usize::MAX,
        }
    }
}

impl UndoHistory {
    /// Validate deserialised tree links and restore runtime-only settings.
    pub fn prepare_after_load(&mut self, max_records: usize) -> bool {
        if self.nodes.is_empty() || self.current >= self.nodes.len() {
            return false;
        }
        if self.nodes[0].parent.is_some() || self.nodes[0].record.is_some() {
            return false;
        }

        let mut incoming = vec![0usize; self.nodes.len()];
        for (node_id, node) in self.nodes.iter().enumerate() {
            if node_id > 0 {
                let Some(parent) = node.parent else {
                    return false;
                };
                if parent >= self.nodes.len() || parent == node_id || node.record.is_none() {
                    return false;
                }
            }
            for &child in &node.children {
                if child >= self.nodes.len() || self.nodes[child].parent != Some(node_id) {
                    return false;
                }
                incoming[child] = incoming[child].saturating_add(1);
            }
        }
        if incoming[0] != 0 || incoming.iter().skip(1).any(|&count| count != 1) {
            return false;
        }

        let mut visited = vec![false; self.nodes.len()];
        let mut pending = vec![0];
        while let Some(node_id) = pending.pop() {
            if visited[node_id] {
                return false;
            }
            visited[node_id] = true;
            pending.extend(self.nodes[node_id].children.iter().copied());
        }
        if visited.iter().any(|seen| !seen) {
            return false;
        }

        let max_sequence = self
            .nodes
            .iter()
            .map(|node| node.sequence)
            .max()
            .unwrap_or(0);
        if self.next_sequence <= max_sequence
            || self
                .nodes
                .windows(2)
                .any(|pair| pair[0].sequence >= pair[1].sequence)
        {
            return false;
        }

        self.insert_mode_coalesce_base = None;
        self.set_max_records(max_records);
        true
    }

    pub fn checkpoint(&self, buffer: TextBuffer, cursor: Pos) -> UndoCheckpoint {
        UndoCheckpoint::new(buffer, cursor, self.nodes[self.current].sequence)
    }

    pub fn coalesced_checkpoint(&mut self, buffer: TextBuffer, cursor: Pos) -> UndoCheckpoint {
        if let Some(mut existing) = self.insert_mode_coalesce_base.clone() {
            existing.replaces_previous_coalesced_record = true;
            return existing;
        }

        let checkpoint =
            UndoCheckpoint::coalesced(buffer, cursor, self.nodes[self.current].sequence);
        self.insert_mode_coalesce_base = Some(checkpoint.clone());
        checkpoint
    }

    pub fn clear_coalesce(&mut self) {
        self.insert_mode_coalesce_base = None;
    }

    pub fn clear(&mut self) {
        let max_records = self.max_records;
        // Keep sequences unique so an older checkpoint never matches the new root.
        let sequence = self.next_sequence;
        *self = Self::default();
        self.nodes[0].sequence = sequence;
        self.next_sequence = sequence.saturating_add(1);
        self.max_records = max_records;
    }

    /// Limit the number of stored edits. Return whether older history was pruned.
    pub fn set_max_records(&mut self, max_records: usize) -> bool {
        self.max_records = max_records.max(1);
        self.prune_to_limit()
    }

    pub fn current(&self) -> UndoNodeId {
        self.current
    }

    pub fn record_if_changed(
        &mut self,
        before: UndoCheckpoint,
        after_buffer: &TextBuffer,
        after_cursor: Pos,
    ) -> bool {
        let base_node = self.node_for_sequence(before.base_sequence);
        let base_buffer = before.buffer.clone();
        let Some(record) = UndoRecord::from_checkpoint(before, after_buffer, after_cursor) else {
            return false;
        };
        let Some(base_node) = base_node else {
            // The checkpoint's base was pruned, so its edit can only start a new history.
            self.clear();
            self.push_record(0, record);
            return true;
        };

        if let Some(existing_node) =
            self.find_equivalent_child(base_node, &base_buffer, after_buffer)
        {
            if let Some(existing) = self.nodes[existing_node].record.as_mut() {
                existing.mark_positions = record.mark_positions;
            }
            self.current = existing_node;
            self.clear_coalesce();
            return true;
        }

        if record.coalesced
            && record.replaces_previous_coalesced_record
            && self.current != 0
            && self
                .nodes
                .get(self.current)
                .is_some_and(|node| node.parent == Some(base_node))
        {
            let current = &mut self.nodes[self.current];
            current.record = Some(record);
            current.created_at_ms = now_ms();
            return true;
        }

        self.push_record(base_node, record);
        self.prune_to_limit();
        true
    }

    fn push_record(&mut self, parent: UndoNodeId, record: UndoRecord) {
        let id = self.nodes.len();
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.nodes.push(UndoNode {
            parent: Some(parent),
            children: Vec::new(),
            record: Some(record),
            sequence,
            created_at_ms: now_ms(),
        });
        self.nodes[parent].children.push(id);
        self.current = id;
    }

    /// Nodes are stored in creation order, so sequences are sorted by index.
    fn node_for_sequence(&self, sequence: u64) -> Option<UndoNodeId> {
        self.nodes
            .binary_search_by_key(&sequence, |node| node.sequence)
            .ok()
    }

    /// Drop the oldest history until at most `max_records` edits remain. Branches that
    /// undo cannot reach go first, oldest leaf first; then the oldest undo steps, by
    /// re-rooting the tree along the path to the current node.
    fn prune_to_limit(&mut self) -> bool {
        let records = self.nodes.len().saturating_sub(1);
        if records <= self.max_records {
            return false;
        }
        let mut excess = records - self.max_records;
        let node_count = self.nodes.len();
        let mut keep = vec![true; node_count];
        let path = self.ancestors(self.current);
        let mut on_path = vec![false; node_count];
        for &id in &path {
            on_path[id] = true;
        }

        let mut live_children: Vec<usize> =
            self.nodes.iter().map(|node| node.children.len()).collect();
        let mut leaves: BinaryHeap<_> = (0..node_count)
            .filter(|&id| !on_path[id] && live_children[id] == 0)
            .map(|id| Reverse((self.nodes[id].sequence, id)))
            .collect();
        while excess > 0
            && let Some(Reverse((_, id))) = leaves.pop()
        {
            keep[id] = false;
            excess -= 1;
            if let Some(parent) = self.nodes[id].parent {
                live_children[parent] -= 1;
                if !on_path[parent] && live_children[parent] == 0 {
                    leaves.push(Reverse((self.nodes[parent].sequence, parent)));
                }
            }
        }

        // Only the path remains when excess is still positive; each step drops one edit.
        let path_from_root: Vec<_> = path.into_iter().rev().collect();
        for &id in path_from_root.iter().take(excess) {
            keep[id] = false;
        }

        let mut new_ids = vec![None; node_count];
        let mut next_id = 0;
        for (id, kept) in keep.iter().enumerate() {
            if *kept {
                new_ids[id] = Some(next_id);
                next_id += 1;
            }
        }
        // The new root is an ancestor of every kept node, so it has the lowest index.
        let nodes = std::mem::take(&mut self.nodes);
        self.nodes = nodes
            .into_iter()
            .enumerate()
            .filter(|(id, _)| keep[*id])
            .map(|(_, mut node)| {
                node.parent = node.parent.and_then(|parent| new_ids[parent]);
                if node.parent.is_none() {
                    node.record = None;
                }
                node.children = node
                    .children
                    .iter()
                    .filter_map(|&child| new_ids[child])
                    .collect();
                node
            })
            .collect();
        self.current = new_ids[self.current].expect("the current node is never pruned");
        true
    }

    pub fn undo(&mut self, buffer: &mut TextBuffer) -> Option<Pos> {
        let current = self.current;
        let node = self.nodes.get(current)?;
        let parent = node.parent?;
        let record = node.record.as_ref()?;
        let cursor = record.before_cursor;
        record.apply(buffer, true);
        self.current = parent;
        self.clear_coalesce();
        Some(cursor)
    }

    pub fn redo(&mut self, buffer: &mut TextBuffer) -> Option<Pos> {
        let child = self.nodes.get(self.current)?.children.last().copied()?;
        self.restore(buffer, child)
    }

    pub fn restore(&mut self, buffer: &mut TextBuffer, target: UndoNodeId) -> Option<Pos> {
        if target >= self.nodes.len() {
            return None;
        }
        if target == self.current {
            return Some(self.cursor_for_node(target));
        }

        self.apply_path_between(buffer, self.current, target)?;
        self.current = target;
        self.clear_coalesce();
        Some(self.cursor_for_node(target))
    }

    pub fn undo_len(&self) -> usize {
        self.ancestors(self.current).len().saturating_sub(1)
    }

    pub fn last_undo_record(&self) -> Option<&UndoRecord> {
        self.nodes.get(self.current)?.record.as_ref()
    }

    pub fn record_for_node(&self, node_id: UndoNodeId) -> Option<&UndoRecord> {
        self.nodes.get(node_id)?.record.as_ref()
    }

    /// Whether any edit is recorded, without building `tree_entries`.
    pub fn has_edits(&self) -> bool {
        self.nodes
            .first()
            .is_some_and(|root| root.children.iter().any(|&child| child < self.nodes.len()))
    }

    pub fn tree_entries(&self) -> Vec<UndoTreeEntry> {
        let mut entries = Vec::with_capacity(self.nodes.len());
        let mut pending = vec![0];
        while let Some(node_id) = pending.pop() {
            let Some(node) = self.nodes.get(node_id) else {
                continue;
            };
            entries.push(UndoTreeEntry {
                id: node_id,
                parent: node.parent,
                sequence: node.sequence,
                created_at_ms: node.created_at_ms,
                is_current: node_id == self.current,
                child_count: node.children.len(),
            });
            pending.extend(node.children.iter().rev().copied());
        }
        entries
    }

    fn lowest_common_ancestor(&self, a: UndoNodeId, b: UndoNodeId) -> Option<UndoNodeId> {
        let a_ancestors: HashSet<_> = self.ancestors(a).into_iter().collect();
        self.ancestors(b)
            .into_iter()
            .find(|ancestor| a_ancestors.contains(ancestor))
    }

    fn find_equivalent_child(
        &self,
        base_node: UndoNodeId,
        base_buffer: &TextBuffer,
        target_buffer: &TextBuffer,
    ) -> Option<UndoNodeId> {
        // Reusing a deeper descendant would turn one edit into several undo steps.
        for &candidate in self.nodes.get(base_node)?.children.iter().rev() {
            let mut candidate_buffer = base_buffer.clone();
            self.apply_path_between(&mut candidate_buffer, base_node, candidate)?;
            if candidate_buffer == *target_buffer {
                return Some(candidate);
            }
        }
        None
    }

    fn apply_path_between(
        &self,
        buffer: &mut TextBuffer,
        from: UndoNodeId,
        to: UndoNodeId,
    ) -> Option<()> {
        let lca = self.lowest_common_ancestor(from, to)?;
        let mut node_id = from;
        while node_id != lca {
            let node = self.nodes.get(node_id)?;
            let record = node.record.as_ref()?;
            record.apply(buffer, true);
            node_id = node.parent?;
        }

        let mut path = Vec::new();
        node_id = to;
        while node_id != lca {
            path.push(node_id);
            node_id = self.nodes.get(node_id)?.parent?;
        }
        for id in path.iter().rev().copied() {
            let record = self.nodes.get(id)?.record.as_ref()?;
            record.apply(buffer, false);
        }
        Some(())
    }

    fn ancestors(&self, mut node_id: UndoNodeId) -> Vec<UndoNodeId> {
        let mut ancestors = Vec::new();
        loop {
            ancestors.push(node_id);
            let Some(parent) = self.nodes.get(node_id).and_then(|node| node.parent) else {
                break;
            };
            node_id = parent;
        }
        ancestors
    }

    fn cursor_for_node(&self, node_id: UndoNodeId) -> Pos {
        if node_id == 0 {
            return Pos::zero();
        }
        self.nodes
            .get(node_id)
            .and_then(|node| node.record.as_ref())
            .map(|record| record.after_cursor)
            .unwrap_or_else(Pos::zero)
    }
}

fn unbounded_max_records() -> usize {
    usize::MAX
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(history: &mut UndoHistory, buffer: &mut TextBuffer, text: &str) {
        let checkpoint = history.checkpoint(buffer.clone(), Pos::zero());
        let end = buffer.char_to_pos(buffer.len_chars());
        buffer.insert(end, text);
        assert!(history.record_if_changed(checkpoint, buffer, Pos::zero()));
    }

    fn undo_all(history: &mut UndoHistory, buffer: &mut TextBuffer) -> usize {
        let mut steps = 0;
        while history.undo(buffer).is_some() {
            steps += 1;
        }
        steps
    }

    #[test]
    fn diffs_between_edited_clones_find_only_the_change() {
        let text = "line of text \u{e9}\n".repeat(5_000);
        let before = TextBuffer::from_text(&text);
        assert_eq!(TextDiff::between(&before, &before.clone()), None);
        assert_eq!(
            TextDiff::between(&before, &TextBuffer::from_text(&text)),
            None
        );

        let mut after = before.clone();
        after.insert(Pos::new(2_500, 3), "\u{e9}x");
        let diff = TextDiff::between(&before, &after).unwrap();
        assert_eq!(diff.start_char, after.pos_to_char(Pos::new(2_500, 3)));
        assert_eq!(diff.deleted, "");
        assert_eq!(diff.inserted, "\u{e9}x");

        let mut longer = before.clone();
        longer.insert(before.char_to_pos(before.len_chars()), "end");
        let diff = TextDiff::between(&before, &longer).unwrap();
        assert_eq!(
            (diff.start_char, diff.inserted.as_str()),
            (before.len_chars(), "end")
        );
        let diff = TextDiff::between(&longer, &before).unwrap();
        assert_eq!(
            (diff.start_char, diff.deleted.as_str()),
            (before.len_chars(), "end")
        );
    }

    #[test]
    fn has_edits_matches_the_tree_entry_count() {
        let mut history = UndoHistory::default();
        let mut buffer = TextBuffer::new();
        assert!(!history.has_edits());
        edit(&mut history, &mut buffer, "a");
        assert!(history.has_edits());
        assert_eq!(history.has_edits(), history.tree_entries().len() > 1);
    }

    #[test]
    fn exceeding_the_limit_drops_only_the_oldest_edits() {
        let mut history = UndoHistory::default();
        history.set_max_records(3);
        let mut buffer = TextBuffer::new();
        for text in ["a", "b", "c", "d", "e"] {
            edit(&mut history, &mut buffer, text);
        }

        assert_eq!(history.tree_entries().len(), 4);
        assert_eq!(undo_all(&mut history, &mut buffer), 3);
        assert_eq!(buffer.to_string(), "ab");
        while history.redo(&mut buffer).is_some() {}
        assert_eq!(buffer.to_string(), "abcde");
    }

    #[test]
    fn unreachable_branches_are_pruned_before_undo_steps() {
        let mut history = UndoHistory::default();
        let mut buffer = TextBuffer::new();
        edit(&mut history, &mut buffer, "a");
        edit(&mut history, &mut buffer, "-old");
        history.undo(&mut buffer);
        edit(&mut history, &mut buffer, "b");
        edit(&mut history, &mut buffer, "c");

        // Four edits; the abandoned "-old" branch goes before any undo step.
        assert!(history.set_max_records(3));
        assert_eq!(buffer.to_string(), "abc");
        assert_eq!(undo_all(&mut history, &mut buffer), 3);
        assert_eq!(buffer.to_string(), "");
    }

    #[test]
    fn checkpoints_follow_their_base_node_across_pruning() {
        let mut history = UndoHistory::default();
        history.set_max_records(2);
        let mut buffer = TextBuffer::new();
        edit(&mut history, &mut buffer, "a");
        edit(&mut history, &mut buffer, "b");

        // Taken at "ab"; recording "c" prunes and renumbers the nodes underneath it.
        let checkpoint = history.checkpoint(buffer.clone(), Pos::zero());
        edit(&mut history, &mut buffer, "c");
        history.undo(&mut buffer);
        assert_eq!(buffer.to_string(), "ab");
        buffer.insert(Pos::new(0, 2), "X");
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::zero()));

        assert_eq!(buffer.to_string(), "abX");
        history.undo(&mut buffer);
        assert_eq!(buffer.to_string(), "ab");
        history.undo(&mut buffer);
        assert_eq!(buffer.to_string(), "a");
    }

    #[test]
    fn checkpoints_whose_base_was_pruned_start_a_new_history() {
        let mut history = UndoHistory::default();
        let mut buffer = TextBuffer::new();
        let stale = history.checkpoint(buffer.clone(), Pos::zero());
        edit(&mut history, &mut buffer, "a");
        edit(&mut history, &mut buffer, "b");
        history.set_max_records(1);

        buffer.insert(Pos::new(0, 2), "c");
        assert!(history.record_if_changed(stale, &buffer, Pos::zero()));
        assert_eq!(history.tree_entries().len(), 2);
        history.undo(&mut buffer);
        // The recorded diff runs from the checkpoint's buffer to the result.
        assert_eq!(buffer.to_string(), "");

        let mut cleared = UndoHistory::default();
        let mut buffer = TextBuffer::new();
        let before_clear = cleared.checkpoint(buffer.clone(), Pos::zero());
        cleared.clear();
        edit(&mut cleared, &mut buffer, "x");
        // A pre-clear checkpoint never matches a node created after the clear.
        buffer.insert(Pos::new(0, 1), "y");
        assert!(cleared.record_if_changed(before_clear, &buffer, Pos::zero()));
        assert_eq!(cleared.tree_entries().len(), 2);
    }

    #[test]
    fn chunk_diffs_round_trip_unicode_edits_across_rope_boundaries() {
        // These characters share partial UTF-8 prefixes or suffixes.
        let replacements = ["", "a", "é", "ê", "ࠀ", "😀", "😁", "e\u{301}", "\n"];
        for padding in [0, 700] {
            let prefix = "λ".repeat(padding);
            let suffix = "終".repeat(padding);
            for deleted in replacements {
                for inserted in replacements {
                    let before = TextBuffer::from_text(&format!("{prefix}{deleted}{suffix}"));
                    let after = TextBuffer::from_text(&format!("{prefix}{inserted}{suffix}"));
                    let Some(change) = TextDiff::between(&before, &after) else {
                        assert_eq!(before, after);
                        continue;
                    };
                    assert_eq!(change.start_char, padding);
                    assert_eq!(change.deleted, deleted);
                    assert_eq!(change.inserted, inserted);
                    let mut restored = before.clone();
                    restored.apply_edit(change.forward_edit());
                    assert_eq!(restored, after);
                    restored.apply_edit(change.reverse_edit());
                    assert_eq!(restored, before);
                }
            }
        }
    }
}
