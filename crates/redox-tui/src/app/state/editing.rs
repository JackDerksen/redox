use redox_core::{
    Pos, Selection, TextObjectSpec, VisualModeKind, VisualSelectionEditPlan,
    motion::{Motion, apply_motion_for_operator},
};

use super::{EditorMode, EditorState, RegisterKind};
use crate::input::{OperatorTarget, TextObjectOperator};
use crate::ui::language_for_path;
use crate::ui::syntax::desired_indent_for_line;

struct OperatorTargetPlan {
    delete_ranges: Vec<(Pos, Pos)>,
    text: String,
    register_kind: RegisterKind,
    preserve_blank_line_on_change: bool,
    yank_highlight: Option<(Selection, VisualModeKind)>,
}

impl EditorState {
    pub(super) fn register_kind_from_visual_mode(mode: VisualModeKind) -> RegisterKind {
        match mode {
            VisualModeKind::Line => RegisterKind::LineWise,
            VisualModeKind::Char | VisualModeKind::Block => RegisterKind::CharWise,
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
        let before = self.capture_active_undo_snapshot();

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
        let before = self.capture_active_undo_snapshot();

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
        let before = self.capture_active_undo_snapshot();

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

    fn operator_target_plan(&self, target: &OperatorTarget) -> Option<OperatorTargetPlan> {
        let buffer = self.session.active_buffer();
        let cursor = self.active_cursor_pos();

        match target {
            OperatorTarget::Motion { motion, count } => {
                let count = (*count).max(1);
                if *motion == Motion::MatchDelimiter {
                    let target = buffer.matching_delimiter(cursor)?;
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
                        register_kind: RegisterKind::CharWise,
                        preserve_blank_line_on_change: false,
                        yank_highlight: None,
                    });
                }

                let end = match (*motion, count) {
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
                    register_kind: RegisterKind::CharWise,
                    preserve_blank_line_on_change: false,
                    yank_highlight: None,
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

        let Some(plan) = self.operator_target_plan(target) else {
            return;
        };

        match operator {
            TextObjectOperator::Delete => {
                let before = self.capture_active_undo_snapshot();
                self.private_register = plan.text;
                self.private_register_kind = plan.register_kind;
                self.apply_delete_ranges(&plan.delete_ranges, viewport_width_cells, text_vh, false);
                self.finish_active_visual_selection_edit(before, EditorMode::Normal, None);
            }
            TextObjectOperator::Change => {
                let before = self.capture_active_undo_snapshot();
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
                    self.set_one_shot_highlight(selection, mode);
                }
                self.set_status("yanked");
            }
            TextObjectOperator::Select => {}
        }
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
        let before = self.capture_active_undo_snapshot();

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
        self.private_register_kind = RegisterKind::LineWise;

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
        self.private_register_kind = RegisterKind::LineWise;
        self.set_one_shot_highlight(
            Selection::new(Pos::new(start_line, 0), Pos::new(end_line, 0)),
            VisualModeKind::Line,
        );
        self.set_status(if start_line == end_line {
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
        let before = self.capture_active_undo_snapshot();

        let active_id = self.session.active_id();
        let (start_pos, end_pos, mut cut_text, indent) = {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let start_line = buffer.clamp_line(view.cursor.cursor.line);
            let end_line = start_line
                .saturating_add(count.saturating_sub(1))
                .min(buffer.len_lines() - 1);
            let (start_pos, end_pos) = buffer.line_span_pos_range(start_line, end_line);
            let text = buffer.line_span_text_linewise_register(start_line, end_line);
            let indent = leading_line_indent(&buffer.line_string(start_line)).to_string();
            (start_pos, end_pos, text, indent)
        };

        self.private_register = std::mem::take(&mut cut_text);
        self.private_register_kind = RegisterKind::LineWise;

        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.delete_range(start_pos, end_pos);
            let replacement = format!("{indent}\n");
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
        let before = self.capture_active_undo_snapshot();

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let sel = Selection::empty(view.cursor.cursor);
        {
            let buffer = self.session.active_buffer_mut();
            let sel = buffer.delete(sel);
            view.cursor.cursor = sel.cursor;
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
            EditorMode::Insert | EditorMode::Command | EditorMode::Search => {}
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
                self.toggle_case_active_visual_selection(viewport_width_cells, text_vh);
            }
            EditorMode::Insert | EditorMode::Command | EditorMode::Search => {}
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

        let before = self.capture_active_undo_snapshot();
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
                    // steps move past the expanded text, which matches Vim-like behavior.
                    let sel = buffer.replace_selection(Selection::new(cursor, end), &replacement);
                    cursor = sel.cursor;
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

    fn toggle_case_active_visual_selection(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let Some(plan) = self.active_visual_selection_edit_plan() else {
            return;
        };
        if plan.delete_ranges.is_empty() {
            return;
        }

        let before = self.capture_active_undo_snapshot();
        let replacements = {
            let buffer = self.session.active_buffer();
            plan.delete_ranges
                .iter()
                .copied()
                .map(|(start, end)| {
                    let source = buffer.slice_pos_range(start, end);
                    let text = toggle_case_text_for_range(&source);
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

        let before = self.capture_active_undo_snapshot();
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

        let before = self.capture_active_undo_snapshot();
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
        let before = self.capture_active_undo_snapshot();

        let text = self.private_register.clone();
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let linewise = matches!(self.private_register_kind, RegisterKind::LineWise);

        {
            let buffer = self.session.active_buffer_mut();
            view.cursor.cursor = buffer.paste_before(view.cursor.cursor, &text, linewise);
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
    }

    fn finish_active_visual_selection_edit(
        &mut self,
        before: super::UndoSnapshot,
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
        let before = self.capture_active_undo_snapshot();

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
        let before = self.capture_active_undo_snapshot();

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

        for line in start_line..=end_line {
            let Some(indent) =
                desired_indent_for_line(self.session.active_buffer(), language, line)
            else {
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
        let before = self.capture_active_undo_snapshot();

        let added_by_line = self
            .session
            .active_buffer_mut()
            .indent_line_span(start_line, end_line, count);

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        if let Some(anchor) = view.visual_anchor.as_mut() {
            if let Some((_, added)) = added_by_line.iter().find(|(line, _)| *line == anchor.line) {
                anchor.col = anchor.col.saturating_add(*added);
            }
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
        let before = self.capture_active_undo_snapshot();

        let removed_by_line = self
            .session
            .active_buffer_mut()
            .outdent_line_span(start_line, end_line, count);

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        if let Some(anchor) = view.visual_anchor.as_mut() {
            if let Some((_, removed)) = removed_by_line
                .iter()
                .find(|(line, _)| *line == anchor.line)
            {
                anchor.col = anchor.col.saturating_sub(*removed);
            }
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
        let before = self.capture_active_undo_snapshot();

        let text = self.private_register.clone();
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let linewise = matches!(self.private_register_kind, RegisterKind::LineWise);

        {
            let buffer = self.session.active_buffer_mut();
            view.cursor.cursor = buffer.paste_after(view.cursor.cursor, &text, linewise);
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

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
        let before = self.capture_active_undo_snapshot();

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        let linewise = text.ends_with('\n');

        {
            let buffer = self.session.active_buffer_mut();
            view.cursor.cursor = buffer.paste_after(view.cursor.cursor, &text, linewise);
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

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

fn leading_line_indent(text: &str) -> &str {
    let end = text
        .char_indices()
        .find_map(|(idx, ch)| (!matches!(ch, ' ' | '\t')).then_some(idx))
        .unwrap_or(text.len());
    &text[..end]
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

fn toggle_case_text_for_range(source: &str) -> String {
    let mut toggled = String::new();
    for ch in source.chars() {
        toggled.push_str(&toggled_case_text(ch));
    }
    toggled
}
