//! Buffer-local marks use character offsets so every edit can keep them attached.

use crate::{Pos, TextBuffer};

impl TextBuffer {
    /// Set a lowercase named mark at a clamped position. Return whether the name is valid.
    pub fn set_mark(&mut self, name: char, position: Pos) -> bool {
        if !name.is_ascii_lowercase() {
            return false;
        }
        self.marks[name as usize - 'a' as usize] = Some(self.pos_to_char(position));
        true
    }

    /// Look up the exact position of a lowercase named mark.
    pub fn mark(&self, name: char) -> Option<Pos> {
        if !name.is_ascii_lowercase() {
            return None;
        }
        self.marks[name as usize - 'a' as usize].map(|offset| self.char_to_pos(offset))
    }

    /// Clear a lowercase named mark. Return whether it was set.
    pub fn delete_mark(&mut self, name: char) -> bool {
        name.is_ascii_lowercase() && self.marks[name as usize - 'a' as usize].take().is_some()
    }

    /// Iterate over set marks in alphabetical order.
    pub fn marks(&self) -> impl Iterator<Item = (char, Pos)> + '_ {
        self.marks
            .iter()
            .zip('a'..='z')
            .filter_map(|(offset, name)| offset.map(|offset| (name, self.char_to_pos(offset))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edit, Selection, UndoHistory};

    #[test]
    fn marks_follow_unicode_edits_line_moves_and_undo_without_changing_text_identity() {
        let mut buffer = TextBuffer::from_text("top\n  猫abc\nend");
        let original = buffer.clone();
        assert!(buffer.set_mark('a', Pos::new(1, 3)));
        assert!(!buffer.set_mark('A', Pos::zero()));
        assert_eq!(buffer, original);
        let mut history = UndoHistory::default();
        let checkpoint = history.checkpoint(buffer.clone(), Pos::zero());
        buffer.insert(Pos::zero(), "é\n");
        assert_eq!(buffer.mark('a'), Some(Pos::new(2, 3)));
        buffer.backspace(Selection::empty(Pos::new(2, 1)));
        buffer.delete(Selection::empty(Pos::new(2, 1)));
        assert_eq!(buffer.mark('a'), Some(Pos::new(2, 1)));
        buffer.move_line_range_up_once(2, 2);
        assert_eq!(buffer.mark('a'), Some(Pos::new(1, 1)));
        buffer.move_line_range_down_once(1, 1);
        assert_eq!(buffer.mark('a'), Some(Pos::new(2, 1)));
        buffer.apply_edits(&[Edit::replace(0..2, "")]);
        assert_eq!(buffer.mark('a'), Some(Pos::new(1, 1)));
        buffer.delete_range(Pos::new(1, 0), Pos::new(2, 0));
        assert_eq!(buffer.mark('a'), Some(Pos::new(1, 0)));
        history.record_if_changed(checkpoint, &buffer, Pos::zero());
        history.undo(&mut buffer);
        assert_eq!(buffer, original);
        assert_eq!(buffer.mark('a'), Some(Pos::new(1, 3)));
        history.redo(&mut buffer);
        assert_eq!(buffer.mark('a'), Some(Pos::new(1, 0)));
        buffer.set_mark('a', Pos::zero());
        buffer.set_mark('z', Pos::new(1, 2));
        assert_eq!(
            buffer.marks().collect::<Vec<_>>(),
            vec![('a', Pos::zero()), ('z', Pos::new(1, 2))]
        );
        history.undo(&mut buffer);
        assert_eq!(buffer.mark('a'), Some(Pos::zero()));
        assert!(buffer.delete_mark('a'));
        assert!(!buffer.delete_mark('a'));
        assert!(!buffer.delete_mark('A'));
        assert_eq!(
            buffer.marks().map(|(name, _)| name).collect::<String>(),
            "z"
        );
    }
}
