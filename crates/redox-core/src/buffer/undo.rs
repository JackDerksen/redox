//! Diff-based undo/redo history for text buffers.
//!
//! Undo history is editor-core behaviour: frontends decide when an edit begins
//! and ends, but this module owns how changed buffer states are represented and
//! replayed.

use crate::buffer::{Edit, Pos, TextBuffer};

#[derive(Debug, Clone)]
pub struct UndoCheckpoint {
    buffer: TextBuffer,
    cursor: Pos,
    coalesced: bool,
    replaces_previous_coalesced_record: bool,
}

impl UndoCheckpoint {
    pub fn new(buffer: TextBuffer, cursor: Pos) -> Self {
        Self {
            buffer,
            cursor,
            coalesced: false,
            replaces_previous_coalesced_record: false,
        }
    }

    fn coalesced(buffer: TextBuffer, cursor: Pos) -> Self {
        Self {
            buffer,
            cursor,
            coalesced: true,
            replaces_previous_coalesced_record: false,
        }
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

        let mut prefix_len = 0;
        while prefix_len < min_len
            && before.rope().char(prefix_len) == after.rope().char(prefix_len)
        {
            prefix_len += 1;
        }

        let mut suffix_len = 0;
        while suffix_len < before_len.saturating_sub(prefix_len)
            && suffix_len < after_len.saturating_sub(prefix_len)
            && before.rope().char(before_len - suffix_len - 1)
                == after.rope().char(after_len - suffix_len - 1)
        {
            suffix_len += 1;
        }

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

#[derive(Debug, Default, Clone)]
pub struct UndoHistory {
    undo_stack: Vec<UndoRecord>,
    redo_stack: Vec<UndoRecord>,
    insert_mode_coalesce_base: Option<UndoCheckpoint>,
}

impl UndoHistory {
    pub fn checkpoint(buffer: TextBuffer, cursor: Pos) -> UndoCheckpoint {
        UndoCheckpoint::new(buffer, cursor)
    }

    pub fn coalesced_checkpoint(&mut self, buffer: TextBuffer, cursor: Pos) -> UndoCheckpoint {
        if let Some(mut existing) = self.insert_mode_coalesce_base.clone() {
            existing.replaces_previous_coalesced_record = true;
            return existing;
        }

        let checkpoint = UndoCheckpoint::coalesced(buffer, cursor);
        self.insert_mode_coalesce_base = Some(checkpoint.clone());
        checkpoint
    }

    pub fn clear_coalesce(&mut self) {
        self.insert_mode_coalesce_base = None;
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn record_if_changed(
        &mut self,
        before: UndoCheckpoint,
        after_buffer: &TextBuffer,
        after_cursor: Pos,
    ) -> bool {
        let Some(record) = UndoRecord::from_checkpoint(before, after_buffer, after_cursor) else {
            return false;
        };

        if record.coalesced
            && record.replaces_previous_coalesced_record
            && self
                .undo_stack
                .last()
                .is_some_and(|last| last.coalesced && last.before_cursor == record.before_cursor)
        {
            let _ = self.undo_stack.pop();
        }

        self.undo_stack.push(record);
        self.redo_stack.clear();
        true
    }

    pub fn undo(&mut self, buffer: &mut TextBuffer) -> Option<Pos> {
        let record = self.undo_stack.pop()?;
        let cursor = record.before_cursor;
        let _ = buffer.apply_edit(record.diff.reverse_edit());
        self.redo_stack.push(record);
        self.clear_coalesce();
        Some(cursor)
    }

    pub fn redo(&mut self, buffer: &mut TextBuffer) -> Option<Pos> {
        let record = self.redo_stack.pop()?;
        let cursor = record.after_cursor;
        let _ = buffer.apply_edit(record.diff.forward_edit());
        self.undo_stack.push(record);
        self.clear_coalesce();
        Some(cursor)
    }

    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn last_undo_record(&self) -> Option<&UndoRecord> {
        self.undo_stack.last()
    }
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
}
