use crate::ui::syntax::leading_indent;
use redox_core::{
    Pos, Selection, SelectionIncrement, TextObjectSpec, VisualModeKind, VisualSelectionEditPlan,
    motion::{Motion, apply_motion_for_operator, apply_motion_n},
};

use super::{EditorMode, EditorState, RegisterKind};
use crate::input::{OperatorTarget, TextObjectOperator};
use crate::ui::language_for_path;
use crate::ui::syntax::{comment_delimiters_for_path, desired_indent_for_line};

struct OperatorTargetPlan {
    delete_ranges: Vec<(Pos, Pos)>,
    text: String,
    register_kind: RegisterKind,
    preserve_blank_line_on_change: bool,
    yank_highlight: Option<(Selection, VisualModeKind)>,
}

impl OperatorTargetPlan {
    /// Backward motions store their ranges cursor-first, so order them here.
    fn start(&self) -> Option<Pos> {
        self.delete_ranges.iter().map(|&(a, b)| a.min(b)).min()
    }

    /// Lines touched by the target. A charwise range ending at a later line's
    /// start excludes that line, matching Vim's exclusive-motion rule.
    fn line_range(&self) -> (usize, usize) {
        if let Some((selection, VisualModeKind::Line)) = self.yank_highlight {
            return selection.line_range();
        }
        let start = self.start().unwrap_or_else(Pos::zero);
        let end = self
            .delete_ranges
            .iter()
            .map(|&(a, b)| a.max(b))
            .max()
            .unwrap_or(start);
        let end_line = if end.line > start.line && end.col == 0 {
            end.line - 1
        } else {
            end.line
        };
        (start.line, end_line)
    }
}

impl EditorState {
    pub(super) fn join_line_below(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let active_id = self.session.active_id();
        let original_cursor = self.views.entry(active_id).or_default().cursor.cursor;
        let (line, line_end, next_content_start, separator) = {
            let buffer = self.session.active_buffer();
            let line = buffer.clamp_line(original_cursor.line);
            if line + 1 >= buffer.len_lines() {
                return;
            }

            let line_text = buffer.line_string(line);
            let next_line = line + 1;
            let next_text = buffer.line_string(next_line);
            if next_line + 1 == buffer.len_lines() && next_text.is_empty() {
                return;
            }
            let separator = if !line_text.is_empty()
                && !line_text.chars().last().is_some_and(char::is_whitespace)
                && !next_text.trim().is_empty()
            {
                " "
            } else {
                ""
            };
            (
                line,
                buffer.line_len_chars(line),
                buffer.line_first_non_whitespace_col(next_line),
                separator,
            )
        };

        let before = self.capture_active_undo_checkpoint();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let join_at = Pos::new(line, line_end);
            let _ = buffer.delete_range(join_at, Pos::new(line + 1, next_content_start));
            if !separator.is_empty() {
                let _ = buffer.insert(join_at, separator);
            }
            view.cursor.cursor = original_cursor;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.clear_status();
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn register_kind_from_visual_mode(mode: VisualModeKind) -> RegisterKind {
        match mode {
            VisualModeKind::Char => RegisterKind::Char,
            VisualModeKind::Line => RegisterKind::Line,
            VisualModeKind::Block => RegisterKind::Block,
        }
    }

    pub(super) fn active_visual_selection_edit_plan(&self) -> Option<VisualSelectionEditPlan> {
        let (selection, mode) = self.active_visual_selection()?;
        let buffer = self.session.active_buffer();
        Some(buffer.visual_selection_edit_plan(selection, mode))
    }

    pub(super) fn select_text_object_in_visual_mode(&mut self, spec: TextObjectSpec) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let buffer = self.session.active_buffer();
        let cursor = self.active_cursor_pos();
        let Some((selection, mode)) = buffer.text_object_selection(cursor, spec) else {
            return;
        };

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        view.visual_anchor = Some(selection.anchor);
        view.cursor.cursor = selection.cursor;
        self.mode = match mode {
            VisualModeKind::Char => EditorMode::Visual,
            VisualModeKind::Line => EditorMode::VisualLine,
            VisualModeKind::Block => EditorMode::VisualBlock,
        };
        self.clear_status();
    }

    pub(super) fn delete_active_visual_selection_to_private_register(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let Some(plan) = self.active_visual_selection_edit_plan() else {
            return;
        };
        let before = self.capture_active_undo_checkpoint();

        self.private_register = plan.text.clone();
        self.private_register_kind = Self::register_kind_from_visual_mode(plan.mode);

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let mut new_pos = view.cursor.cursor;
            for (start, end) in plan.delete_ranges.iter().rev().copied() {
                new_pos = buffer.delete_range(start, end);
            }
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
    }

    pub(super) fn delete_active_visual_selection_without_yank(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let Some(plan) = self.active_visual_selection_edit_plan() else {
            return;
        };
        let before = self.capture_active_undo_checkpoint();

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let mut new_pos = view.cursor.cursor;
            for (start, end) in plan.delete_ranges.iter().rev().copied() {
                new_pos = buffer.delete_range(start, end);
            }
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
    }

    pub(super) fn change_active_visual_selection_to_private_register(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let Some(plan) = self.active_visual_selection_edit_plan() else {
            return;
        };
        let before = self.capture_active_undo_checkpoint();

        self.private_register = plan.text.clone();
        self.private_register_kind = Self::register_kind_from_visual_mode(plan.mode);

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let mut new_pos = view.cursor.cursor;
            for (start, end) in plan.delete_ranges.iter().rev().copied() {
                new_pos = buffer.delete_range(start, end);
            }
            if plan.mode == VisualModeKind::Line {
                let _ = buffer.insert(new_pos, "\n");
            }
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.finish_active_visual_selection_edit(before, EditorMode::Insert, None);
    }

    fn operator_target_plan(
        &self,
        operator: TextObjectOperator,
        target: &OperatorTarget,
    ) -> Option<OperatorTargetPlan> {
        let buffer = self.session.active_buffer();
        let cursor = self.active_cursor_pos();

        match target {
            OperatorTarget::Motion { motion, count } => {
                let count = (*count).max(1);
                if let Motion::MarkLine(name) | Motion::MarkExact(name) = motion {
                    buffer.mark(*name)?;
                }
                if matches!(
                    motion,
                    Motion::Up
                        | Motion::Down
                        | Motion::FileStart
                        | Motion::FileEnd
                        | Motion::LineNumber
                        | Motion::MarkLine(_)
                        | Motion::MarkExact(_)
                        | Motion::ParagraphBefore
                        | Motion::ParagraphAfter
                ) {
                    let target = apply_motion_for_operator(buffer, cursor, *motion, count);
                    if matches!(motion, Motion::Up | Motion::Down) && target.line == cursor.line {
                        return None;
                    }
                    let exclusive_line_boundary = matches!(
                        motion,
                        Motion::ParagraphBefore | Motion::ParagraphAfter | Motion::MarkExact(_)
                    );
                    let (start, end) = Selection::new(cursor, target).ordered();
                    if !exclusive_line_boundary
                        || (start.line < end.line
                            && end.col == 0
                            && start.col <= buffer.line_first_non_whitespace_col(start.line))
                    {
                        let start_line = start.line;
                        let end_line = end.line - usize::from(exclusive_line_boundary);
                        let (start, end) = buffer.line_span_pos_range(start_line, end_line);
                        return Some(OperatorTargetPlan {
                            delete_ranges: vec![(start, end)],
                            text: buffer.line_span_text_linewise_register(start_line, end_line),
                            register_kind: RegisterKind::Line,
                            preserve_blank_line_on_change: true,
                            yank_highlight: Some((
                                Selection::new(Pos::new(start_line, 0), Pos::new(end_line, 0)),
                                VisualModeKind::Line,
                            )),
                        });
                    }
                }
                if matches!(
                    motion,
                    Motion::MatchDelimiter | Motion::WordEndBefore | Motion::BigWordEndBefore
                ) {
                    let target = if *motion == Motion::MatchDelimiter {
                        buffer.matching_delimiter(cursor)?
                    } else {
                        apply_motion_n(buffer, cursor, *motion, count)
                    };
                    if target == cursor {
                        return None;
                    }
                    let (start, inclusive_end) = if cursor <= target {
                        (cursor, target)
                    } else {
                        (target, cursor)
                    };
                    let end = buffer.move_right(inclusive_end);
                    return Some(OperatorTargetPlan {
                        delete_ranges: vec![(start, end)],
                        text: buffer.slice_pos_range(start, end),
                        register_kind: RegisterKind::Char,
                        preserve_blank_line_on_change: false,
                        yank_highlight: Some((
                            Selection::new(start, inclusive_end),
                            VisualModeKind::Char,
                        )),
                    });
                }

                let end = match (*motion, count) {
                    (Motion::WordStartAfter | Motion::BigWordStartAfter, _)
                        if operator == TextObjectOperator::Change
                            && buffer
                                .char_at(cursor)
                                .is_some_and(|character| !character.is_whitespace()) =>
                    {
                        let (first_end, end_motion) = if *motion == Motion::BigWordStartAfter {
                            (
                                buffer.big_word_end_at_or_after(cursor),
                                Motion::BigWordEndAfter,
                            )
                        } else {
                            (buffer.word_end_at_or_after(cursor), Motion::WordEndAfter)
                        };
                        buffer.move_right(apply_motion_n(buffer, first_end, end_motion, count - 1))
                    }
                    (Motion::LineEnd, n) if n > 1 => {
                        let target_line = buffer
                            .clamp_line(cursor.line)
                            .saturating_add(n - 1)
                            .min(buffer.len_lines().saturating_sub(1));
                        Pos::new(target_line, buffer.line_len_chars(target_line))
                    }
                    (Motion::LineFirstNonWhitespace, n) if n > 1 => {
                        let target_line = buffer
                            .clamp_line(cursor.line)
                            .saturating_add(n - 1)
                            .min(buffer.len_lines().saturating_sub(1));
                        Pos::new(
                            target_line,
                            buffer.line_first_non_whitespace_col(target_line),
                        )
                    }
                    _ => apply_motion_for_operator(buffer, cursor, *motion, count),
                };
                let selection = Selection::new(cursor, end);
                (!selection.is_empty()).then(|| OperatorTargetPlan {
                    delete_ranges: vec![(cursor, end)],
                    text: buffer.slice_pos_range(cursor, end),
                    register_kind: RegisterKind::Char,
                    preserve_blank_line_on_change: false,
                    yank_highlight: {
                        let (start, end) = selection.ordered();
                        Some((
                            Selection::new(start, buffer.move_left(end)),
                            VisualModeKind::Char,
                        ))
                    },
                })
            }
            OperatorTarget::SearchMatch { forward } => {
                let (start, end) = self.search_match_for_gn(cursor, *forward)?;
                (start != end).then(|| OperatorTargetPlan {
                    delete_ranges: vec![(start, end)],
                    text: buffer.slice_pos_range(start, end),
                    register_kind: RegisterKind::Char,
                    preserve_blank_line_on_change: false,
                    yank_highlight: Some((
                        Selection::new(
                            start,
                            super::search::inclusive_match_end(buffer, start, end),
                        ),
                        VisualModeKind::Char,
                    )),
                })
            }
            OperatorTarget::TextObject(spec) => {
                let plan = buffer.text_object_edit_plan(cursor, *spec)?;
                let yank_highlight = buffer.text_object_selection(cursor, *spec);
                Some(OperatorTargetPlan {
                    delete_ranges: plan.delete_ranges,
                    text: plan.text,
                    register_kind: Self::register_kind_from_visual_mode(plan.mode),
                    preserve_blank_line_on_change: plan.mode == VisualModeKind::Line,
                    yank_highlight,
                })
            }
        }
    }

    fn apply_delete_ranges(
        &mut self,
        delete_ranges: &[(Pos, Pos)],
        viewport_width_cells: usize,
        text_vh: usize,
        preserve_blank_line: bool,
    ) {
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer_mut();

        let mut new_pos = view.cursor.cursor;
        for (start, end) in delete_ranges.iter().rev().copied() {
            new_pos = buffer.delete_range(start, end);
        }
        if preserve_blank_line {
            let _ = buffer.insert(new_pos, "\n");
        }
        view.cursor.cursor = new_pos;
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
    }

    pub(super) fn apply_operator_target(
        &mut self,
        operator: TextObjectOperator,
        target: &OperatorTarget,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        if operator == TextObjectOperator::Select {
            if let OperatorTarget::TextObject(spec) = target
                && matches!(
                    self.mode,
                    EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock
                )
            {
                self.select_text_object_in_visual_mode(*spec);
            }
            return;
        }

        if self.mode != EditorMode::Normal {
            return;
        }

        if matches!(target, OperatorTarget::SearchMatch { .. }) {
            self.ensure_search_state_current();
        }
        let Some(plan) = self.operator_target_plan(operator, target) else {
            if matches!(target, OperatorTarget::SearchMatch { .. }) {
                self.set_status("pattern not found");
            }
            return;
        };

        match operator {
            TextObjectOperator::Delete => {
                let before = self.capture_active_undo_checkpoint();
                self.private_register = plan.text;
                self.private_register_kind = plan.register_kind;
                self.apply_delete_ranges(&plan.delete_ranges, viewport_width_cells, text_vh, false);
                self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
            }
            TextObjectOperator::Change => {
                let before = self.capture_active_undo_checkpoint();
                self.private_register = plan.text;
                self.private_register_kind = plan.register_kind;
                self.apply_delete_ranges(
                    &plan.delete_ranges,
                    viewport_width_cells,
                    text_vh,
                    plan.preserve_blank_line_on_change,
                );
                self.finish_active_visual_selection_edit(before, EditorMode::Insert, None);
            }
            TextObjectOperator::Yank => {
                self.private_register = plan.text;
                self.private_register_kind = plan.register_kind;
                if let Some((selection, mode)) = plan.yank_highlight {
                    self.set_one_shot_highlight(selection, mode, super::HighlightKind::Yank);
                }
                self.confirm_yank("yanked");
            }
            TextObjectOperator::Lowercase | TextObjectOperator::Uppercase => {
                let before = self.capture_active_undo_checkpoint();
                let cursor = self.active_cursor_pos();
                let start = plan.start().unwrap_or(cursor);
                // Linewise targets keep the column, like Vim's `gUU` and `gUj`.
                let new_cursor = match plan.register_kind {
                    RegisterKind::Line if start.line == cursor.line => cursor,
                    RegisterKind::Line => Pos::new(
                        start.line,
                        self.session
                            .active_buffer()
                            .line_first_non_whitespace_col(start.line),
                    ),
                    RegisterKind::Char | RegisterKind::Block => start,
                };
                let case = if operator == TextObjectOperator::Uppercase {
                    CaseChange::Upper
                } else {
                    CaseChange::Lower
                };
                self.replace_ranges_with_case(&plan.delete_ranges, case, new_cursor);
                self.reconcile_active_cursor(viewport_width_cells, text_vh);
                self.invalidate_active_render_caches();
                let _ = self.record_active_undo_if_changed(before);
                let _ = self.session.recompute_active_dirty();
            }
            TextObjectOperator::Indent | TextObjectOperator::Outdent => {
                let (start_line, end_line) = plan.line_range();
                self.shift_line_span(
                    start_line,
                    end_line,
                    operator == TextObjectOperator::Outdent,
                    viewport_width_cells,
                    text_vh,
                );
            }
            TextObjectOperator::Select => {}
        }
    }

    /// Shift whole lines by one indent level and leave the cursor on the first
    /// line's first non-blank character, as Vim's `>` and `<` operators do.
    pub(super) fn shift_line_span(
        &mut self,
        start_line: usize,
        end_line: usize,
        outdent: bool,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let before = self.capture_active_undo_checkpoint();
        let indent_width = self.active_indent_width();
        let buffer = self.session.active_buffer_mut();
        if outdent {
            buffer.outdent_line_span(start_line, end_line, 1, indent_width);
        } else {
            // Like Vim, leave empty lines alone instead of adding trailing whitespace.
            let indent = " ".repeat(indent_width);
            for line in start_line..=end_line {
                if buffer.line_len_chars(line) > 0 {
                    buffer.indent_line_span(line, line, 1, &indent);
                }
            }
        }
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        view.cursor.cursor = Pos::new(start_line, buffer.line_first_non_whitespace_col(start_line));
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn active_visual_line_range(&self) -> Option<(usize, usize)> {
        let (selection, _) = self.active_visual_selection()?;
        Some(selection.line_range())
    }

    pub(super) fn delete_current_line_to_private_register(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let before = self.capture_active_undo_checkpoint();

        let active_id = self.session.active_id();
        let (start_pos, end_pos, mut cut_text) = {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let start_line = buffer.clamp_line(view.cursor.cursor.line);
            let end_line = start_line
                .saturating_add(count.saturating_sub(1))
                .min(buffer.len_lines() - 1);
            let (start_pos, end_pos) = buffer.line_span_pos_range(start_line, end_line);
            let text = buffer.line_span_text_linewise_register(start_line, end_line);
            (start_pos, end_pos, text)
        };

        self.private_register = std::mem::take(&mut cut_text);
        self.private_register_kind = RegisterKind::Line;

        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.delete_range(start_pos, end_pos);
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.clear_status();
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn yank_current_line_to_private_register(&mut self, count: usize) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let active_id = self.session.active_id();
        let (start_line, end_line, text) = {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let start_line = buffer.clamp_line(view.cursor.cursor.line);
            let end_line = start_line
                .saturating_add(count.saturating_sub(1))
                .min(buffer.len_lines() - 1);
            let text = buffer.line_span_text_linewise_register(start_line, end_line);
            (start_line, end_line, text)
        };

        self.private_register = text;
        self.private_register_kind = RegisterKind::Line;
        self.set_one_shot_highlight(
            Selection::new(Pos::new(start_line, 0), Pos::new(end_line, 0)),
            VisualModeKind::Line,
            super::HighlightKind::Yank,
        );
        self.confirm_yank(if start_line == end_line {
            "yanked line"
        } else {
            "yanked lines"
        });
    }

    pub(super) fn change_current_line_to_private_register(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let before = self.capture_active_undo_checkpoint();

        let active_id = self.session.active_id();
        let (start_pos, end_pos, mut cut_text, indent, line_ending) = {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let start_line = buffer.clamp_line(view.cursor.cursor.line);
            let end_line = start_line
                .saturating_add(count.saturating_sub(1))
                .min(buffer.len_lines() - 1);
            let (start_pos, end_pos) = buffer.line_span_pos_range(start_line, end_line);
            let exact_text = buffer.line_span_text(start_line, end_line);
            let text = buffer.line_span_text_linewise_register(start_line, end_line);
            let indent = leading_indent(&buffer.line_string(start_line)).to_string();
            let line_ending = trailing_line_ending(&exact_text).to_string();
            (start_pos, end_pos, text, indent, line_ending)
        };

        self.private_register = std::mem::take(&mut cut_text);
        self.private_register_kind = RegisterKind::Line;

        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.delete_range(start_pos, end_pos);
            let replacement = format!("{indent}{line_ending}");
            let _ = buffer.insert(new_pos, &replacement);
            view.cursor.cursor = Pos::new(new_pos.line, indent.chars().count());
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.mode = EditorMode::Insert;
        self.clear_status();
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn delete_char_under_cursor_without_yank(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        // Like Vim, `x` never deletes a line break, so it does nothing on an empty line.
        let cursor = self.active_cursor_pos();
        if cursor.col >= self.session.active_buffer().line_len_chars(cursor.line) {
            return;
        }
        let before = self.capture_active_undo_checkpoint();

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let selection = Selection::empty(view.cursor.cursor);
        {
            let buffer = self.session.active_buffer_mut();
            let selection = buffer.delete(selection);
            view.cursor.cursor = selection.cursor;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.clear_status();
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn replace_under_cursor_or_selection(
        &mut self,
        replacement: char,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        match self.mode {
            EditorMode::Normal => {
                self.replace_char_under_cursor(replacement, viewport_width_cells, text_vh);
            }
            EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock => {
                self.replace_active_visual_selection(replacement, viewport_width_cells, text_vh);
            }
            EditorMode::Insert
            | EditorMode::Command
            | EditorMode::Search
            | EditorMode::Finder
            | EditorMode::PinSelect
            | EditorMode::LspMarketplace
            | EditorMode::CodeActions
            | EditorMode::SymbolInfo
            | EditorMode::DiagnosticsList => {}
        }
    }

    pub(super) fn toggle_case_under_cursor_or_selection(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        match self.mode {
            EditorMode::Normal => {
                self.toggle_case_under_cursor(count, viewport_width_cells, text_vh);
            }
            EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock => {
                self.change_case_active_visual_selection(
                    CaseChange::Toggle,
                    viewport_width_cells,
                    text_vh,
                );
            }
            EditorMode::Insert
            | EditorMode::Command
            | EditorMode::Search
            | EditorMode::Finder
            | EditorMode::PinSelect
            | EditorMode::LspMarketplace
            | EditorMode::CodeActions
            | EditorMode::SymbolInfo
            | EditorMode::DiagnosticsList => {}
        }
    }

    fn toggle_case_under_cursor(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let active_id = self.session.active_id();
        let mut cursor = self.views.entry(active_id).or_default().cursor.cursor;
        if self.session.active_buffer().char_at(cursor).is_none() {
            return;
        }

        let before = self.capture_active_undo_checkpoint();
        let mut changed = false;
        {
            let buffer = self.session.active_buffer_mut();
            for _ in 0..count {
                let Some(ch) = buffer.char_at(cursor) else {
                    break;
                };
                let end = buffer.move_right(cursor);
                let replacement = toggled_case_text(ch);
                if replacement != ch.to_string() {
                    // Case toggling can expand to multiple codepoints (for example, `ß` -> `SS`).
                    // We intentionally advance to the end of the replacement so repeated `~`
                    // steps move past the expanded text, which matches Vim-like behaviour.
                    let selection =
                        buffer.replace_selection(Selection::new(cursor, end), &replacement);
                    cursor = selection.cursor;
                    changed = true;
                } else {
                    cursor = end;
                }
            }

            let view = self.views.entry(active_id).or_default();
            view.cursor.cursor = cursor;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.clear_status();
        if changed {
            self.invalidate_active_render_caches();
            let _ = self.record_active_undo_if_changed(before);
            let _ = self.session.recompute_active_dirty();
        }
    }

    pub(super) fn change_case_active_visual_selection(
        &mut self,
        case: CaseChange,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let Some(plan) = self.active_visual_selection_edit_plan() else {
            return;
        };
        let Some(&(new_cursor, _)) = plan.delete_ranges.first() else {
            return;
        };

        let before = self.capture_active_undo_checkpoint();
        self.replace_ranges_with_case(&plan.delete_ranges, case, new_cursor);
        self.reconcile_active_cursor(viewport_width_cells, text_vh);
        self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
    }

    /// Rewrite each range in the requested case and leave the cursor at `cursor`.
    fn replace_ranges_with_case(&mut self, ranges: &[(Pos, Pos)], case: CaseChange, cursor: Pos) {
        let replacements = {
            let buffer = self.session.active_buffer();
            ranges
                .iter()
                .map(|&(start, end)| {
                    let source = buffer.slice_pos_range(start, end);
                    (start, end, change_case_text(&source, case))
                })
                .collect::<Vec<_>>()
        };
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer_mut();
        for (start, end, text) in replacements.iter().rev() {
            let _ = buffer.replace_selection(Selection::new(*start, *end), text);
        }
        view.cursor.cursor = buffer.clamp_pos(cursor);
    }

    fn reconcile_active_cursor(&mut self, viewport_width_cells: usize, text_vh: usize) {
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        view.cursor.reconcile_after_edit(
            self.session.active_buffer(),
            viewport_width_cells,
            text_vh,
        );
    }

    /// `ctrl+a`-style increments: the number at the cursor in normal mode, or the
    /// selection's numbers (with smart sequences) in visual modes.
    pub(super) fn increment_numbers(
        &mut self,
        count: usize,
        decrement: bool,
        progressive: bool,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let magnitude = count.max(1) as i128;
        let delta = if decrement { -magnitude } else { magnitude };
        let before = self.capture_active_undo_checkpoint();
        if let Some((selection, mode)) = self.active_visual_selection() {
            let kind = if progressive {
                SelectionIncrement::Progressive
            } else {
                SelectionIncrement::Smart
            };
            // Like Vim, leave the cursor at the start of the selection.
            let (start, _) = selection.ordered();
            let start = match mode {
                VisualModeKind::Char => start,
                VisualModeKind::Line => Pos::new(start.line, 0),
                VisualModeKind::Block => {
                    Pos::new(start.line, selection.anchor.col.min(selection.cursor.col))
                }
            };
            let buffer = self.session.active_buffer_mut();
            buffer.increment_numbers_in_selection(selection, mode, delta, kind);
            let start = buffer.clamp_pos(start);
            let active_id = self.session.active_id();
            self.views.entry(active_id).or_default().cursor.cursor = start;
            self.reconcile_active_cursor(viewport_width_cells, text_vh);
            self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
        } else if self.mode == EditorMode::Normal {
            let cursor = self.active_cursor_pos();
            let Some(landed) = self
                .session
                .active_buffer_mut()
                .increment_number_at(cursor, delta)
            else {
                return;
            };
            let active_id = self.session.active_id();
            self.views.entry(active_id).or_default().cursor.cursor = landed;
            self.reconcile_active_cursor(viewport_width_cells, text_vh);
            self.invalidate_active_render_caches();
            let _ = self.record_active_undo_if_changed(before);
            let _ = self.session.recompute_active_dirty();
        }
    }

    /// Apply a linewise operator to `count` lines from the cursor, as `>>` or `guu` do.
    pub(super) fn operate_current_lines(
        &mut self,
        operator: TextObjectOperator,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        let cursor = self.active_cursor_pos();
        let buffer = self.session.active_buffer();
        let start_line = cursor.line;
        let end_line = start_line
            .saturating_add(count.saturating_sub(1))
            .min(buffer.len_lines().saturating_sub(1));
        let case = match operator {
            TextObjectOperator::Indent | TextObjectOperator::Outdent => {
                self.shift_line_span(
                    start_line,
                    end_line,
                    operator == TextObjectOperator::Outdent,
                    viewport_width_cells,
                    text_vh,
                );
                return;
            }
            TextObjectOperator::Lowercase => CaseChange::Lower,
            TextObjectOperator::Uppercase => CaseChange::Upper,
            TextObjectOperator::Delete
            | TextObjectOperator::Change
            | TextObjectOperator::Yank
            | TextObjectOperator::Select => return,
        };
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let end_col = self.session.active_buffer().line_len_chars(end_line);
        let range = (Pos::new(start_line, 0), Pos::new(end_line, end_col));
        let before = self.capture_active_undo_checkpoint();
        self.replace_ranges_with_case(&[range], case, cursor);
        self.reconcile_active_cursor(viewport_width_cells, text_vh);
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    /// Dispatch an action, remembering the selection whenever it ends visual mode for `gv`.
    pub(super) fn apply_input_remembering_visual(
        &mut self,
        action: crate::input::InputAction,
        width: usize,
        height: usize,
    ) {
        let active_id = self.session.active_id();
        let visual = self.active_visual_selection();
        self.apply_input_inner(action, width, height);
        if let Some((selection, mode)) = visual
            && (self.session.active_id() != active_id || self.active_visual_selection().is_none())
        {
            self.remember_visual_selection(active_id, selection, mode);
        }
    }

    fn remember_visual_selection(
        &mut self,
        buffer_id: redox_core::BufferId,
        selection: Selection,
        mode: VisualModeKind,
    ) {
        if let Some(buffer) = self.session.buffer_mut(buffer_id) {
            buffer.set_last_visual_selection(selection.anchor, selection.cursor);
            self.views.entry(buffer_id).or_default().last_visual_mode = Some(mode);
        }
    }

    /// `gv`: reselect the previous selection. In visual mode, swap it with the current one.
    pub(super) fn restore_visual_selection(&mut self, viewport_width_cells: usize, text_vh: usize) {
        let active_id = self.session.active_id();
        let previous = self.session.active_buffer().last_visual_selection().zip(
            self.views
                .get(&active_id)
                .and_then(|view| view.last_visual_mode),
        );
        let Some(((anchor, cursor), mode)) = previous else {
            self.set_status("no previous selection");
            return;
        };
        if !matches!(
            self.mode,
            EditorMode::Normal
                | EditorMode::Visual
                | EditorMode::VisualLine
                | EditorMode::VisualBlock
        ) {
            return;
        }
        if let Some((selection, current_mode)) = self.active_visual_selection() {
            self.remember_visual_selection(active_id, selection, current_mode);
        }
        self.enter_visual_selection(
            Selection::new(anchor, cursor),
            mode,
            viewport_width_cells,
            text_vh,
        );
    }

    pub(super) fn enter_visual_selection(
        &mut self,
        selection: Selection,
        mode: VisualModeKind,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        view.visual_anchor = Some(buffer.clamp_pos(selection.anchor));
        view.cursor.cursor = buffer.clamp_pos(selection.cursor);
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        self.mode = match mode {
            VisualModeKind::Char => EditorMode::Visual,
            VisualModeKind::Line => EditorMode::VisualLine,
            VisualModeKind::Block => EditorMode::VisualBlock,
        };
        self.clear_status();
    }

    /// Visual `o`: move the cursor to the other end of the selection.
    pub(super) fn swap_visual_selection_ends(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if let Some((selection, mode)) = self.active_visual_selection() {
            self.enter_visual_selection(
                Selection::new(selection.cursor, selection.anchor),
                mode,
                viewport_width_cells,
                text_vh,
            );
        }
    }

    pub(super) fn wrap_active_visual_selection(
        &mut self,
        opening: char,
        closing: char,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        let Some((selection, mode)) = self.active_visual_selection() else {
            return;
        };
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let buffer = self.session.active_buffer();
        let ranges = match mode {
            VisualModeKind::Char => vec![buffer.visual_charwise_pos_range(selection)],
            VisualModeKind::Line => {
                let (start_line, end_line) = selection.line_range();
                vec![(
                    Pos::new(start_line, 0),
                    Pos::new(end_line, buffer.line_len_chars(end_line)),
                )]
            }
            VisualModeKind::Block => buffer.visual_blockwise_pos_ranges(selection),
        };
        let Some(&(new_cursor, _)) = ranges.first() else {
            return;
        };

        let before = self.capture_active_undo_checkpoint();
        let opening = opening.to_string();
        let closing = closing.to_string();
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            for (start, end) in ranges.into_iter().rev() {
                let _ = buffer.insert(end, &closing);
                let _ = buffer.insert(start, &opening);
            }
            view.cursor.cursor = new_cursor;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
    }

    pub(super) fn toggle_active_comments(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if !matches!(
            self.mode,
            EditorMode::Normal
                | EditorMode::Visual
                | EditorMode::VisualLine
                | EditorMode::VisualBlock
        ) || !self.ensure_active_fully_loaded_for_edit_or_save()
        {
            return;
        }
        let Some((opening, closing)) =
            comment_delimiters_for_path(self.session.active_meta().path.as_deref())
        else {
            self.set_status("comment syntax unavailable for this file");
            return;
        };
        let (start_line, end_line) = if self.mode == EditorMode::Normal {
            let line = self
                .session
                .active_buffer()
                .clamp_line(self.active_cursor_pos().line);
            (line, line)
        } else {
            let Some(range) = self.active_visual_line_range() else {
                return;
            };
            range
        };
        let buffer = self.session.active_buffer();
        let lines = (start_line..=end_line)
            .map(|line| (line, buffer.line_string(line)))
            .filter(|(_, text)| !text.trim().is_empty())
            .collect::<Vec<_>>();
        let Some((first_line, first_text)) = lines.first() else {
            return;
        };
        let replacements = if closing.is_empty() {
            let uncomment = lines
                .iter()
                .all(|(_, text)| text[leading_indent(text).len()..].starts_with(opening));
            lines
                .iter()
                .map(|(line, text)| {
                    let indent = leading_indent(text);
                    let content = &text[indent.len()..];
                    let replacement = if uncomment {
                        let content = &content[opening.len()..];
                        format!("{indent}{}", content.strip_prefix(' ').unwrap_or(content))
                    } else {
                        format!("{indent}{opening} {content}")
                    };
                    (
                        Pos::new(*line, 0),
                        Pos::new(*line, text.chars().count()),
                        replacement,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            let (last_line, last_text) = lines.last().expect("nonempty selection");
            let start = Pos::new(*first_line, leading_indent(first_text).chars().count());
            let end = Pos::new(*last_line, last_text.trim_end().chars().count());
            let source = buffer.slice_pos_range(start, end);
            if let Some(content) = uncomment_paired_content(&source, opening, closing) {
                vec![(start, end, content.to_string())]
            } else if let Some(replacements) = lines
                .iter()
                .map(|(line, text)| {
                    let indent = leading_indent(text);
                    let content = uncomment_paired_content(
                        text[indent.len()..].trim_end(),
                        opening,
                        closing,
                    )?;
                    Some((
                        Pos::new(*line, indent.chars().count()),
                        Pos::new(*line, text.trim_end().chars().count()),
                        content.to_string(),
                    ))
                })
                .collect::<Option<Vec<_>>>()
            {
                replacements
            } else if source.contains(opening) || source.contains(closing) {
                self.set_status(
                    "cannot toggle comments safely: selection contains comment delimiters",
                );
                return;
            } else {
                vec![(start, end, format!("{opening} {source} {closing}"))]
            }
        };
        let new_cursor = if self.mode == EditorMode::Normal {
            self.active_cursor_pos()
        } else {
            Pos::new(*first_line, 0)
        };
        let before = self.capture_active_undo_checkpoint();
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            for (start, end, text) in replacements.into_iter().rev() {
                let _ = buffer.replace_selection(Selection::new(start, end), &text);
            }
            view.cursor.cursor = new_cursor;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
        self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
    }

    fn replace_char_under_cursor(
        &mut self,
        replacement: char,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let active_id = self.session.active_id();
        let cursor = self.views.entry(active_id).or_default().cursor.cursor;
        if self.session.active_buffer().char_at(cursor).is_none() {
            return;
        }

        let before = self.capture_active_undo_checkpoint();
        let replacement_text = replacement.to_string();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let end = buffer.move_right(cursor);
            let _ = buffer.replace_selection(Selection::new(cursor, end), &replacement_text);
            view.cursor.cursor = cursor;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.clear_status();
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn replace_active_visual_selection(
        &mut self,
        replacement: char,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let Some(plan) = self.active_visual_selection_edit_plan() else {
            return;
        };
        if plan.delete_ranges.is_empty() {
            return;
        }

        let before = self.capture_active_undo_checkpoint();
        let replacements = {
            let buffer = self.session.active_buffer();
            plan.delete_ranges
                .iter()
                .copied()
                .map(|(start, end)| {
                    let source = buffer.slice_pos_range(start, end);
                    let text = replacement_text_for_range(&source, replacement);
                    (start, end, text)
                })
                .collect::<Vec<_>>()
        };

        let new_cursor = replacements
            .first()
            .map(|(start, _, _)| *start)
            .unwrap_or_else(Pos::zero);
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            for (start, end, text) in replacements.iter().rev() {
                let _ = buffer.replace_selection(Selection::new(*start, *end), text);
            }
            view.cursor.cursor = new_cursor;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
    }

    pub(super) fn paste_private_register_before(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if self.private_register.is_empty() {
            return;
        }
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let before = self.capture_active_undo_checkpoint();

        let text = self.private_register.clone();
        let kind = self.private_register_kind;
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();

        {
            let buffer = self.session.active_buffer_mut();
            view.cursor.cursor = match kind {
                RegisterKind::Block => buffer.paste_block(view.cursor.cursor, &text, false),
                RegisterKind::Char | RegisterKind::Line => {
                    buffer.paste_before(view.cursor.cursor, &text, kind == RegisterKind::Line)
                }
            };
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.refresh_active_indentation();
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn finish_active_visual_selection_edit(
        &mut self,
        before: super::UndoCheckpoint,
        mode: EditorMode,
        status: Option<&str>,
    ) {
        self.mode = mode;
        self.clear_active_visual_anchor();
        match status {
            Some(message) => self.set_status(message),
            None => self.clear_status(),
        }
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn move_visual_selection_lines_up(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        if self.active_visual_selection().is_none() {
            return;
        }
        let before = self.capture_active_undo_checkpoint();

        let Some((start_line, end_line)) = self.active_visual_line_range() else {
            return;
        };
        let active_id = self.session.active_id();
        let moved = {
            let buffer = self.session.active_buffer_mut();
            buffer.move_line_range_up(start_line, end_line, count)
        };
        if let Some((new_start, _)) = moved {
            let delta = start_line.saturating_sub(new_start);
            let new_end = end_line.saturating_sub(delta);
            {
                let view = self.views.entry(active_id).or_default();
                if let Some(anchor) = view.visual_anchor.as_mut() {
                    anchor.line = anchor.line.saturating_sub(delta);
                }
                view.cursor.cursor.line = view.cursor.cursor.line.saturating_sub(delta);
            }
            self.reindent_active_line_span(new_start, new_end);
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn move_visual_selection_lines_down(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        if self.active_visual_selection().is_none() {
            return;
        }
        let before = self.capture_active_undo_checkpoint();

        let Some((start_line, end_line)) = self.active_visual_line_range() else {
            return;
        };
        let active_id = self.session.active_id();
        let moved = {
            let buffer = self.session.active_buffer_mut();
            buffer.move_line_range_down(start_line, end_line, count)
        };
        if let Some((new_start, _)) = moved {
            let delta = new_start.saturating_sub(start_line);
            let new_end = end_line.saturating_add(delta);
            {
                let view = self.views.entry(active_id).or_default();
                if let Some(anchor) = view.visual_anchor.as_mut() {
                    anchor.line = anchor.line.saturating_add(delta);
                }
                view.cursor.cursor.line = view.cursor.cursor.line.saturating_add(delta);
            }
            self.reindent_active_line_span(new_start, new_end);
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn reindent_active_line_span(&mut self, start_line: usize, end_line: usize) {
        let language = language_for_path(self.session.active_meta().path.as_deref());
        if language.is_none() {
            return;
        }

        let indent_size = self.active_indent_width();
        self.seed_indent_parser();
        for line in start_line..=end_line {
            let Some(indent) = desired_indent_for_line(
                &mut self.indent_parser,
                self.session.active_buffer(),
                language,
                line,
                indent_size,
            ) else {
                continue;
            };
            let Some((removed, added)) = self
                .session
                .active_buffer_mut()
                .replace_line_indent(line, &indent)
            else {
                continue;
            };
            self.adjust_active_visual_columns_after_indent_change(line, removed, added);
        }
    }

    fn adjust_active_visual_columns_after_indent_change(
        &mut self,
        line: usize,
        removed: usize,
        added: usize,
    ) {
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        if let Some(anchor) = view.visual_anchor.as_mut()
            && anchor.line == line
        {
            anchor.col = adjust_col_after_indent_change(anchor.col, removed, added);
        }
        if view.cursor.cursor.line == line {
            view.cursor.cursor.col =
                adjust_col_after_indent_change(view.cursor.cursor.col, removed, added);
        }
    }

    pub(super) fn indent_visual_selection(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let Some((start_line, end_line)) = self.active_visual_line_range() else {
            return;
        };
        let before = self.capture_active_undo_checkpoint();

        let indent = " ".repeat(self.active_indent_width());
        let added_by_line = self
            .session
            .active_buffer_mut()
            .indent_line_span(start_line, end_line, count, &indent);

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        if let Some(anchor) = view.visual_anchor.as_mut()
            && let Some((_, added)) = added_by_line.iter().find(|(line, _)| *line == anchor.line)
        {
            anchor.col = anchor.col.saturating_add(*added);
        }
        if let Some((_, added)) = added_by_line
            .iter()
            .find(|(line, _)| *line == view.cursor.cursor.line)
        {
            view.cursor.cursor.col = view.cursor.cursor.col.saturating_add(*added);
        }
        let buffer = self.session.active_buffer();
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn outdent_visual_selection(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let Some((start_line, end_line)) = self.active_visual_line_range() else {
            return;
        };
        let before = self.capture_active_undo_checkpoint();

        let indent_size = self.active_indent_width();
        let removed_by_line = self.session.active_buffer_mut().outdent_line_span(
            start_line,
            end_line,
            count,
            indent_size,
        );

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        if let Some(anchor) = view.visual_anchor.as_mut()
            && let Some((_, removed)) = removed_by_line
                .iter()
                .find(|(line, _)| *line == anchor.line)
        {
            anchor.col = anchor.col.saturating_sub(*removed);
        }
        if let Some((_, removed)) = removed_by_line
            .iter()
            .find(|(line, _)| *line == view.cursor.cursor.line)
        {
            view.cursor.cursor.col = view.cursor.cursor.col.saturating_sub(*removed);
        }
        let buffer = self.session.active_buffer();
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn paste_private_register(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if self.private_register.is_empty() {
            return;
        }
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let before = self.capture_active_undo_checkpoint();

        let text = self.private_register.clone();
        let kind = self.private_register_kind;
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();

        {
            let buffer = self.session.active_buffer_mut();
            view.cursor.cursor = match kind {
                RegisterKind::Block => buffer.paste_block(view.cursor.cursor, &text, true),
                RegisterKind::Char | RegisterKind::Line => {
                    buffer.paste_after(view.cursor.cursor, &text, kind == RegisterKind::Line)
                }
            };
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.refresh_active_indentation();
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn paste_system_clipboard_text(
        &mut self,
        text: &str,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        let text = normalize_clipboard_text(text);
        if text.is_empty() {
            return;
        }
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let before = self.capture_active_undo_checkpoint();

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let linewise = text.ends_with('\n');

        {
            let buffer = self.session.active_buffer_mut();
            view.cursor.cursor = buffer.paste_after(view.cursor.cursor, &text, linewise);
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.refresh_active_indentation();
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }
}

fn normalize_clipboard_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|&ch| ch == '\n' || ch == '\t' || !ch.is_control())
        .collect()
}

fn uncomment_paired_content<'source>(
    source: &'source str,
    opening: &str,
    closing: &str,
) -> Option<&'source str> {
    let content = source.strip_prefix(opening)?.strip_suffix(closing)?;
    if content.contains(opening) || content.contains(closing) {
        return None;
    }
    let content = content.strip_prefix(' ').unwrap_or(content);
    Some(content.strip_suffix(' ').unwrap_or(content))
}

fn trailing_line_ending(text: &str) -> &str {
    if text.ends_with("\r\n") {
        "\r\n"
    } else if text.ends_with('\n') {
        "\n"
    } else if text.ends_with('\r') {
        "\r"
    } else {
        ""
    }
}

fn adjust_col_after_indent_change(col: usize, removed: usize, added: usize) -> usize {
    if col <= removed {
        added
    } else {
        col.saturating_sub(removed).saturating_add(added)
    }
}

fn replacement_text_for_range(source: &str, replacement: char) -> String {
    source
        .chars()
        .map(|ch| if ch == '\n' { '\n' } else { replacement })
        .collect()
}

fn toggled_case_text(ch: char) -> String {
    if ch.is_lowercase() {
        ch.to_uppercase().collect()
    } else if ch.is_uppercase() {
        ch.to_lowercase().collect()
    } else {
        ch.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CaseChange {
    Toggle,
    Lower,
    Upper,
}

fn change_case_text(source: &str, case: CaseChange) -> String {
    match case {
        CaseChange::Toggle => source.chars().map(toggled_case_text).collect(),
        CaseChange::Lower => source.to_lowercase(),
        CaseChange::Upper => source.to_uppercase(),
    }
}
