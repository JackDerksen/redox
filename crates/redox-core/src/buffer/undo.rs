//! Diff-based undo tree history for text buffers.
//!
//! Undo history is editor-core behaviour: frontends decide when an edit begins
//! and ends, but this module owns how changed buffer states are represented and
//! replayed.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::buffer::{Edit, Pos, TextBuffer};

pub type UndoNodeId = usize;

#[derive(Debug, Clone)]
pub struct UndoCheckpoint {
    buffer: TextBuffer,
    cursor: Pos,
    base_node: UndoNodeId,
    coalesced: bool,
    replaces_previous_coalesced_record: bool,
}

impl UndoCheckpoint {
    fn new(buffer: TextBuffer, cursor: Pos, base_node: UndoNodeId) -> Self {
        Self {
            buffer,
            cursor,
            base_node,
            coalesced: false,
            replaces_previous_coalesced_record: false,
        }
    }

    fn coalesced(buffer: TextBuffer, cursor: Pos, base_node: UndoNodeId) -> Self {
        Self {
            buffer,
            cursor,
            base_node,
            coalesced: true,
            replaces_previous_coalesced_record: false,
        }
    }

    pub fn is_coalesced(&self) -> bool {
        self.coalesced
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDiff {
    pub start_char: usize,
    pub deleted: String,
    pub inserted: String,
}

impl TextDiff {
    pub fn between(before: &TextBuffer, after: &TextBuffer) -> Option<Self> {
        if before.rope() == after.rope() {
            return None;
        }

        let before_len = before.len_chars();
        let after_len = after.len_chars();
        let min_len = before_len.min(after_len);

        let prefix_len = before
            .rope()
            .chars()
            .zip(after.rope().chars())
            .take(min_len)
            .take_while(|(before_char, after_char)| before_char == after_char)
            .count();

        let max_suffix_len = before_len
            .saturating_sub(prefix_len)
            .min(after_len.saturating_sub(prefix_len));

        let suffix_len = before
            .rope()
            .chars_at(before_len)
            .reversed()
            .zip(after.rope().chars_at(after_len).reversed())
            .take(max_suffix_len)
            .take_while(|(before_char, after_char)| before_char == after_char)
            .count();

        let before_changed_end = before_len - suffix_len;
        let after_changed_end = after_len - suffix_len;
        Some(Self {
            start_char: prefix_len,
            deleted: before.slice_chars(prefix_len, before_changed_end),
            inserted: after.slice_chars(prefix_len, after_changed_end),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoRecord {
    pub diff: TextDiff,
    pub before_cursor: Pos,
    pub after_cursor: Pos,
    coalesced: bool,
    replaces_previous_coalesced_record: bool,
}

impl UndoRecord {
    fn from_checkpoint(
        before: UndoCheckpoint,
        after_buffer: &TextBuffer,
        after_cursor: Pos,
    ) -> Option<Self> {
        let diff = TextDiff::between(&before.buffer, after_buffer)?;
        Some(Self {
            diff,
            before_cursor: before.cursor,
            after_cursor,
            coalesced: before.coalesced,
            replaces_previous_coalesced_record: before.replaces_previous_coalesced_record,
        })
    }
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct UndoHistory {
    nodes: Vec<UndoNode>,
    current: UndoNodeId,
    next_sequence: u64,
    insert_mode_coalesce_base: Option<UndoCheckpoint>,
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
    pub fn checkpoint(&self, buffer: TextBuffer, cursor: Pos) -> UndoCheckpoint {
        UndoCheckpoint::new(buffer, cursor, self.current)
    }

    pub fn coalesced_checkpoint(&mut self, buffer: TextBuffer, cursor: Pos) -> UndoCheckpoint {
        if let Some(mut existing) = self.insert_mode_coalesce_base.clone() {
            existing.replaces_previous_coalesced_record = true;
            return existing;
        }

        let checkpoint = UndoCheckpoint::coalesced(buffer, cursor, self.current);
        self.insert_mode_coalesce_base = Some(checkpoint.clone());
        checkpoint
    }

    pub fn clear_coalesce(&mut self) {
        self.insert_mode_coalesce_base = None;
    }

    pub fn clear(&mut self) {
        let max_records = self.max_records;
        *self = Self::default();
        self.max_records = max_records;
    }

    pub fn set_max_records(&mut self, max_records: usize) -> bool {
        self.max_records = max_records.max(1);
        if self.nodes.len().saturating_sub(1) > self.max_records {
            self.clear();
            true
        } else {
            false
        }
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
        let base_node = before.base_node.min(self.nodes.len().saturating_sub(1));
        let base_buffer = before.buffer.clone();
        let Some(record) = UndoRecord::from_checkpoint(before, after_buffer, after_cursor) else {
            return false;
        };

        if let Some(existing_node) =
            self.find_equivalent_node(base_node, &base_buffer, after_buffer)
        {
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

        let parent = if self.nodes.len().saturating_sub(1) >= self.max_records {
            // Start a fresh tree whose root represents this edit's checkpoint. This keeps
            // history strictly bounded without retaining full buffer snapshots in each node.
            self.clear();
            0
        } else {
            base_node
        };
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
        true
    }

    pub fn undo(&mut self, buffer: &mut TextBuffer) -> Option<Pos> {
        let current = self.current;
        let node = self.nodes.get(current)?;
        let parent = node.parent?;
        let record = node.record.as_ref()?;
        let cursor = record.before_cursor;
        let _ = buffer.apply_edit(record.diff.reverse_edit());
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

    fn find_equivalent_node(
        &self,
        base_node: UndoNodeId,
        base_buffer: &TextBuffer,
        target_buffer: &TextBuffer,
    ) -> Option<UndoNodeId> {
        let mut candidates = self.nodes.get(base_node)?.children.clone();
        while let Some(candidate) = candidates.pop() {
            let mut candidate_buffer = base_buffer.clone();
            self.apply_path_between(&mut candidate_buffer, base_node, candidate)?;
            if candidate_buffer.rope() == target_buffer.rope() {
                return Some(candidate);
            }
            candidates.extend(self.nodes.get(candidate)?.children.iter().copied());
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
            let _ = buffer.apply_edit(record.diff.reverse_edit());
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
            let _ = buffer.apply_edit(record.diff.forward_edit());
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

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_diff_records_only_changed_span_with_unicode_text() {
        let before = TextBuffer::from_str("aébc🙂d\n");
        let after = TextBuffer::from_str("aéXYZ🙂d\n");
        let diff = TextDiff::between(&before, &after).expect("buffers should differ");

        assert_eq!(diff.start_char, 2);
        assert_eq!(diff.deleted, "bc");
        assert_eq!(diff.inserted, "XYZ");

        let mut patched = before.clone();
        let _ = patched.apply_edit(diff.forward_edit());
        assert_eq!(patched.to_string(), after.to_string());

        let _ = patched.apply_edit(diff.reverse_edit());
        assert_eq!(patched.to_string(), before.to_string());
    }

    #[test]
    fn history_limit_starts_a_fresh_bounded_tree() {
        let mut history = UndoHistory::default();
        history.set_max_records(2);
        let mut buffer = TextBuffer::from_str("");
        for value in ["a", "ab", "abc", "abcd"] {
            let checkpoint = history.checkpoint(buffer.clone(), Pos::zero());
            buffer = TextBuffer::from_str(value);
            assert!(history.record_if_changed(checkpoint, &buffer, Pos::zero()));
            assert!(history.tree_entries().len() <= 3);
        }
        assert_eq!(history.undo(&mut buffer), Some(Pos::zero()));
        assert_eq!(buffer.to_string(), "abc");

        assert!(history.set_max_records(1));
        assert_eq!(history.tree_entries().len(), 1);
        assert_eq!(history.undo(&mut buffer), None);
    }

    #[test]
    fn coalesced_checkpoints_replace_only_the_current_insert_session_record() {
        let mut history = UndoHistory::default();
        let before = TextBuffer::from_str("hello");
        let mut after = before.clone();
        let checkpoint = history.coalesced_checkpoint(before.clone(), Pos::new(0, 0));
        let _ = after.insert(Pos::new(0, 0), "a");
        assert!(history.record_if_changed(checkpoint, &after, Pos::new(0, 1)));

        let checkpoint = history.coalesced_checkpoint(before.clone(), Pos::new(0, 0));
        let mut later = before.clone();
        let _ = later.insert(Pos::new(0, 0), "ab");
        assert!(history.record_if_changed(checkpoint, &later, Pos::new(0, 2)));
        assert_eq!(history.undo_len(), 1);
        assert_eq!(
            history
                .last_undo_record()
                .expect("missing record")
                .diff
                .inserted,
            "ab"
        );

        history.clear_coalesce();
        let checkpoint = history.coalesced_checkpoint(later.clone(), Pos::new(0, 0));
        let mut separate = later.clone();
        let _ = separate.insert(Pos::new(0, 0), "z");
        assert!(history.record_if_changed(checkpoint, &separate, Pos::new(0, 1)));
        assert_eq!(history.undo_len(), 2);
    }

    #[test]
    fn edit_after_undo_branches_from_restored_node() {
        let mut history = UndoHistory::default();
        let mut buffer = TextBuffer::from_str("a");

        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 0));
        let _ = buffer.insert(Pos::new(0, 1), "b");
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 2)));

        assert_eq!(history.undo(&mut buffer), Some(Pos::new(0, 0)));
        assert_eq!(buffer.to_string(), "a");

        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 0));
        let _ = buffer.insert(Pos::new(0, 1), "c");
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 2)));

        let root = history.tree_entries()[0].id;
        assert_eq!(history.tree_entries()[root].child_count, 2);
        assert_eq!(buffer.to_string(), "ac");
    }

    #[test]
    fn restore_moves_between_branches_through_common_ancestor() {
        let mut history = UndoHistory::default();
        let mut buffer = TextBuffer::from_str("a");

        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 0));
        let _ = buffer.insert(Pos::new(0, 1), "b");
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 2)));
        let first_branch = history.current();

        let _ = history.undo(&mut buffer);
        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 0));
        let _ = buffer.insert(Pos::new(0, 1), "c");
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 2)));

        assert_eq!(buffer.to_string(), "ac");
        assert_eq!(
            history.restore(&mut buffer, first_branch),
            Some(Pos::new(0, 2))
        );
        assert_eq!(buffer.to_string(), "ab");
    }

    #[test]
    fn edit_that_reaches_existing_descendant_reuses_that_node() {
        let mut history = UndoHistory::default();
        let mut buffer = TextBuffer::from_str("");

        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 0));
        let _ = buffer.insert(Pos::new(0, 0), "a");
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 1)));
        let parent = history.current();

        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 1));
        let _ = buffer.insert(Pos::new(0, 1), "b");
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 2)));
        let existing = history.current();

        assert_eq!(history.restore(&mut buffer, parent), Some(Pos::new(0, 1)));
        assert_eq!(buffer.to_string(), "a");

        let checkpoint = history.checkpoint(buffer.clone(), Pos::new(0, 1));
        let _ = buffer.insert(Pos::new(0, 1), "b");
        assert!(history.record_if_changed(checkpoint, &buffer, Pos::new(0, 2)));

        assert_eq!(history.current(), existing);
        assert_eq!(history.tree_entries().len(), 3);
        assert_eq!(buffer.to_string(), "ab");
    }
}
