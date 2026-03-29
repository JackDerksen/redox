//! Selection and line-span helpers for `TextBuffer`.
//!
//! These methods are editor-logic primitives that are independent of input.

use crate::buffer::{Pos, Selection, TextBuffer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualModeKind {
    Char,
    Line,
    Block,
}

/// A precomputed plan for visual selection operations.
///
/// This bundles mode-aware text capture and deletion bounds so callers can
/// perform yank/delete flows without duplicating selection maths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualSelectionEditPlan {
    pub delete_ranges: Vec<(Pos, Pos)>,
    pub text: String,
    pub mode: VisualModeKind,
}

impl TextBuffer {
    /// Return a char range spanning full lines from `start_line..=end_line_inclusive`.
    ///
    /// The end of the range includes a trailing newline when one exists.
    pub fn line_span_char_range(
        &self,
        start_line: usize,
        end_line_inclusive: usize,
    ) -> std::ops::Range<usize> {
        let start_line = self.clamp_line(start_line);
        let end_line = self.clamp_line(end_line_inclusive.max(start_line));
        let start = self.line_to_char(start_line);
        let end = self.line_full_end_char(end_line);
        start..end
    }

    /// Return a position range spanning full lines from `start_line..=end_line_inclusive`.
    pub fn line_span_pos_range(&self, start_line: usize, end_line_inclusive: usize) -> (Pos, Pos) {
        let range = self.line_span_char_range(start_line, end_line_inclusive);
        (self.char_to_pos(range.start), self.char_to_pos(range.end))
    }

    /// Return text spanning full lines from `start_line..=end_line_inclusive`.
    ///
    /// This preserves the underlying buffer content exactly.
    pub fn line_span_text(&self, start_line: usize, end_line_inclusive: usize) -> String {
        let range = self.line_span_char_range(start_line, end_line_inclusive);
        self.slice_chars(range.start, range.end)
    }

    /// Return line-span text suitable for a line-wise register.
    ///
    /// Ensures the returned text ends with `'\n'`.
    pub fn line_span_text_linewise_register(
        &self,
        start_line: usize,
        end_line_inclusive: usize,
    ) -> String {
        let mut text = self.line_span_text(start_line, end_line_inclusive);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    /// Return the charwise visual selection as an order-normalized deletion range.
    ///
    /// Visual charwise mode is inclusive of the cursor endpoint.
    pub fn visual_charwise_pos_range(&self, selection: Selection) -> (Pos, Pos) {
        let (start, end_inclusive) = selection.ordered();
        let end_char = self.pos_to_char(end_inclusive);
        let end_exclusive = if end_char < self.len_chars() {
            self.char_to_pos(end_char + 1)
        } else {
            end_inclusive
        };
        (start, end_exclusive)
    }

    /// Return the linewise visual selection as a full-line deletion range.
    pub fn visual_linewise_pos_range(&self, selection: Selection) -> (Pos, Pos) {
        let (start, end_inclusive) = selection.ordered();
        self.line_span_pos_range(start.line, end_inclusive.line)
    }

    /// Return visual charwise selection text (inclusive endpoint semantics).
    pub fn visual_charwise_text(&self, selection: Selection) -> String {
        let (start, end_exclusive) = self.visual_charwise_pos_range(selection);
        self.slice_pos_range(start, end_exclusive)
    }

    /// Return visual linewise selection text for line-wise register semantics.
    pub fn visual_linewise_text(&self, selection: Selection) -> String {
        let (start, end) = selection.ordered();
        self.line_span_text_linewise_register(start.line, end.line)
    }

    /// Return visual block selection text/deletion slices without line collapsing.
    pub fn visual_blockwise_pos_ranges(&self, selection: Selection) -> Vec<(Pos, Pos)> {
        let (start, end) = selection.ordered();
        let left = start.col.min(end.col);
        let right_exclusive = start.col.max(end.col).saturating_add(1);
        let mut ranges = Vec::new();

        for line_idx in start.line..=end.line {
            let line_len = self.line_len_chars(line_idx);
            let range_start = left.min(line_len);
            let range_end = right_exclusive.min(line_len);
            if range_start < range_end {
                ranges.push((Pos::new(line_idx, range_start), Pos::new(line_idx, range_end)));
            }
        }

        ranges
    }

    /// Return deletion ranges for visual block mode.
    ///
    /// When the block fully covers a line's content, delete the whole logical
    /// line so trailing text is pulled upward instead of leaving empty rows.
    pub fn visual_blockwise_delete_ranges(&self, selection: Selection) -> Vec<(Pos, Pos)> {
        let (start, end) = selection.ordered();
        let left = start.col.min(end.col);
        let right_exclusive = start.col.max(end.col).saturating_add(1);
        let mut ranges = Vec::new();

        for line_idx in start.line..=end.line {
            let line_len = self.line_len_chars(line_idx);
            if left == 0 && right_exclusive >= line_len {
                let full_line = self.line_span_pos_range(line_idx, line_idx);
                if full_line.0 != full_line.1 {
                    ranges.push(full_line);
                }
                continue;
            }

            let range_start = left.min(line_len);
            let range_end = right_exclusive.min(line_len);
            if range_start < range_end {
                ranges.push((Pos::new(line_idx, range_start), Pos::new(line_idx, range_end)));
            }
        }

        ranges
    }

    /// Return visual selection text for the current visual mode.
    pub fn visual_blockwise_text(&self, selection: Selection) -> String {
        self.visual_blockwise_pos_ranges(selection)
            .into_iter()
            .map(|(start, end)| self.slice_pos_range(start, end))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Return visual selection deletion bounds for the current visual mode.
    pub fn visual_selection_pos_ranges(
        &self,
        selection: Selection,
        mode: VisualModeKind,
    ) -> Vec<(Pos, Pos)> {
        match mode {
            VisualModeKind::Char => vec![self.visual_charwise_pos_range(selection)],
            VisualModeKind::Line => vec![self.visual_linewise_pos_range(selection)],
            VisualModeKind::Block => self.visual_blockwise_delete_ranges(selection),
        }
    }

    /// Return visual selection text for the current visual mode.
    pub fn visual_selection_text(&self, selection: Selection, mode: VisualModeKind) -> String {
        match mode {
            VisualModeKind::Char => self.visual_charwise_text(selection),
            VisualModeKind::Line => self.visual_linewise_text(selection),
            VisualModeKind::Block => self.visual_blockwise_text(selection),
        }
    }

    /// Build a mode-aware visual selection edit plan.
    pub fn visual_selection_edit_plan(
        &self,
        selection: Selection,
        mode: VisualModeKind,
    ) -> VisualSelectionEditPlan {
        let delete_ranges = self.visual_selection_pos_ranges(selection, mode);
        let text = self.visual_selection_text(selection, mode);
        VisualSelectionEditPlan {
            delete_ranges,
            text,
            mode,
        }
    }

    /// Return selection char bounds for a specific `line_idx`.
    ///
    /// The returned range is half-open (`start..end`) in char-column units for
    /// that line and follows visual-mode inclusive endpoint behaviour.
    pub fn visual_selection_char_range_on_line(
        &self,
        selection: Selection,
        mode: VisualModeKind,
        line_idx: usize,
    ) -> Option<std::ops::Range<usize>> {
        let line_len = self.line_len_chars(line_idx);
        match mode {
            VisualModeKind::Line => {
                let (start_line, end_line) = selection.line_range();
                if line_idx < start_line || line_idx > end_line {
                    return None;
                }
                Some(0..line_len)
            }
            VisualModeKind::Block => {
                let (start, end) = selection.ordered();
                if line_idx < start.line || line_idx > end.line {
                    return None;
                }
                let left = start.col.min(end.col).min(line_len);
                let right = start.col.max(end.col).saturating_add(1).min(line_len);
                if left < right {
                    Some(left..right)
                } else {
                    None
                }
            }
            VisualModeKind::Char => {
                let (start, end) = selection.ordered();
                if line_idx < start.line || line_idx > end.line {
                    return None;
                }
                if line_len == 0 {
                    return None;
                }

                let max_char = line_len.saturating_sub(1);
                let sel_start = if line_idx == start.line {
                    start.col.min(max_char)
                } else {
                    0
                };
                let sel_end_inclusive = if line_idx == end.line {
                    end.col.min(max_char)
                } else {
                    max_char
                };
                if sel_start > sel_end_inclusive {
                    return None;
                }

                Some(sel_start..sel_end_inclusive.saturating_add(1))
            }
        }
    }
}
