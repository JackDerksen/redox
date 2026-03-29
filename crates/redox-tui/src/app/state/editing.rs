use redox_core::{Selection, VisualModeKind, VisualSelectionEditPlan};

use super::{EditorMode, EditorState, RegisterKind};

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

        self.mode = EditorMode::Normal;
        self.clear_active_visual_anchor();
        self.set_status("deleted");
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
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

        self.mode = EditorMode::Normal;
        self.clear_active_visual_anchor();
        self.set_status("deleted");
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
        let before = self.capture_active_undo_snapshot();

        let active_id = self.session.active_id();
        let (start_pos, end_pos, mut cut_text) = {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let start_line = buffer.clamp_line(view.cursor.cursor.line);
            let end_line = (start_line + count.saturating_sub(1)).min(buffer.len_lines() - 1);
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

        self.set_status("deleted");
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

        self.set_status("deleted");
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
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
            let view = self.views.entry(active_id).or_default();
            if let Some(anchor) = view.visual_anchor.as_mut() {
                anchor.line = anchor.line.saturating_sub(delta);
            }
            view.cursor.cursor.line = view.cursor.cursor.line.saturating_sub(delta);
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
            let view = self.views.entry(active_id).or_default();
            if let Some(anchor) = view.visual_anchor.as_mut() {
                anchor.line = anchor.line.saturating_add(delta);
            }
            view.cursor.cursor.line = view.cursor.cursor.line.saturating_add(delta);
            let buffer = self.session.active_buffer();
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
        self.invalidate_active_render_caches();
        let _ = self.record_active_undo_if_changed(before);
        let _ = self.session.recompute_active_dirty();
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
}
