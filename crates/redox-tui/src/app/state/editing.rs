use redox_core::{Pos, Selection, TextBuffer};

use super::{EditorMode, EditorState, RegisterKind};

impl EditorState {
    pub(super) fn active_visual_selection_delete_bounds(&self) -> Option<(Pos, Pos, RegisterKind)> {
        let (selection, line_mode) = self.active_visual_selection()?;
        let buffer = self.session.active_buffer();
        let (start, end_inclusive) = selection.ordered();

        if line_mode {
            let start_pos = Pos::new(start.line, 0);
            let end_pos = buffer.clamp_pos(Pos::new(end_inclusive.line.saturating_add(1), 0));
            Some((start_pos, end_pos, RegisterKind::LineWise))
        } else {
            let end_char = buffer.pos_to_char(end_inclusive);
            let end_exclusive = if end_char < buffer.len_chars() {
                buffer.char_to_pos(end_char + 1)
            } else {
                end_inclusive
            };
            Some((start, end_exclusive, RegisterKind::CharWise))
        }
    }

    pub(super) fn capture_active_visual_selection_text(&self) -> Option<(String, RegisterKind)> {
        let (selection, line_mode) = self.active_visual_selection()?;
        let buffer = self.session.active_buffer();
        if line_mode {
            let (start, end) = selection.ordered();
            let mut out = String::new();
            for line in start.line..=end.line {
                out.push_str(&buffer.line_string(line));
                out.push('\n');
            }
            Some((out, RegisterKind::LineWise))
        } else {
            let (start, end_inclusive) = selection.ordered();
            let end_char = buffer.pos_to_char(end_inclusive);
            let end_exclusive = if end_char < buffer.len_chars() {
                buffer.char_to_pos(end_char + 1)
            } else {
                end_inclusive
            };
            Some((
                buffer.slice_pos_range(start, end_exclusive),
                RegisterKind::CharWise,
            ))
        }
    }

    pub(super) fn delete_active_visual_selection_to_private_register(
        &mut self,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let Some((delete_start, delete_end, kind)) = self.active_visual_selection_delete_bounds()
        else {
            return;
        };
        let Some((text, _)) = self.capture_active_visual_selection_text() else {
            return;
        };

        self.private_register = text;
        self.private_register_kind = kind;

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.delete_range(delete_start, delete_end);
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.mode = EditorMode::Normal;
        self.clear_active_visual_anchor();
        self.set_status("deleted");
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

        let Some((delete_start, delete_end, _)) = self.active_visual_selection_delete_bounds()
        else {
            return;
        };

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.delete_range(delete_start, delete_end);
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.mode = EditorMode::Normal;
        self.clear_active_visual_anchor();
        self.set_status("deleted");
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn line_end_char_exclusive(buffer: &TextBuffer, line: usize) -> usize {
        buffer.pos_to_char(Pos::new(line, buffer.line_len_chars(line)))
    }

    pub(super) fn line_span_char_range(
        buffer: &TextBuffer,
        start_line: usize,
        end_line_inclusive: usize,
    ) -> (usize, usize) {
        let start_line = buffer.clamp_line(start_line);
        let end_line_inclusive = buffer.clamp_line(end_line_inclusive.max(start_line));
        let start_char = buffer.line_to_char(start_line);
        let end_char = if end_line_inclusive + 1 < buffer.len_lines() {
            buffer.line_to_char(end_line_inclusive + 1)
        } else {
            Self::line_end_char_exclusive(buffer, end_line_inclusive)
        };
        (start_char, end_char)
    }

    pub(super) fn active_visual_line_range(&self) -> Option<(usize, usize)> {
        let (selection, _) = self.active_visual_selection()?;
        let (start, end) = selection.ordered();
        Some((start.line, end.line))
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

        let active_id = self.session.active_id();
        let (start_char, end_char, mut cut_text) = {
            let view = self.views.entry(active_id).or_default();
            let buffer = self.session.active_buffer();
            let start_line = buffer.clamp_line(view.cursor.cursor.line);
            let end_line = (start_line + count.saturating_sub(1)).min(buffer.len_lines() - 1);
            let (start_char, end_char) = Self::line_span_char_range(buffer, start_line, end_line);
            let mut text = buffer.slice_chars(start_char, end_char);
            if !text.ends_with('\n') {
                text.push('\n');
            }
            (start_char, end_char, text)
        };

        self.private_register = std::mem::take(&mut cut_text);
        self.private_register_kind = RegisterKind::LineWise;

        let start_pos;
        let end_pos;
        {
            let buffer = self.session.active_buffer();
            start_pos = buffer.char_to_pos(start_char);
            end_pos = buffer.char_to_pos(end_char);
        }

        let view = self.views.entry(active_id).or_default();
        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.delete_range(start_pos, end_pos);
            view.cursor.cursor = new_pos;
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        self.set_status("deleted");
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

        let text = self.private_register.clone();
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();

        let insert_pos = {
            let buffer = self.session.active_buffer();
            match self.private_register_kind {
                RegisterKind::CharWise => buffer.clamp_pos(view.cursor.cursor),
                RegisterKind::LineWise => {
                    let line = buffer.clamp_line(view.cursor.cursor.line);
                    buffer.clamp_pos(Pos::new(line, 0))
                }
            }
        };

        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.insert(insert_pos, &text);
            view.cursor.cursor = match self.private_register_kind {
                RegisterKind::CharWise => new_pos,
                RegisterKind::LineWise => insert_pos,
            };
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

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

        let active_id = self.session.active_id();
        for _ in 0..count {
            let Some((start_line, end_line)) = self.active_visual_line_range() else {
                break;
            };
            if start_line == 0 {
                break;
            }
            let (replace_start_char, replace_end_char, replacement) = {
                let buffer = self.session.active_buffer();
                let (sel_start_char, sel_end_char) =
                    Self::line_span_char_range(buffer, start_line, end_line);
                let (above_start_char, above_end_char) =
                    Self::line_span_char_range(buffer, start_line - 1, start_line - 1);
                let selected = buffer.slice_chars(sel_start_char, sel_end_char);
                let above = buffer.slice_chars(above_start_char, above_end_char);
                (above_start_char, sel_end_char, format!("{selected}{above}"))
            };

            let view = self.views.entry(active_id).or_default();
            {
                let buffer = self.session.active_buffer_mut();
                let replace_start = buffer.char_to_pos(replace_start_char);
                let replace_end = buffer.char_to_pos(replace_end_char);
                let _ = buffer.delete_range(replace_start, replace_end);
                let _ = buffer.insert(replace_start, &replacement);
            }
            if let Some(anchor) = view.visual_anchor.as_mut() {
                anchor.line = anchor.line.saturating_sub(1);
            }
            view.cursor.cursor.line = view.cursor.cursor.line.saturating_sub(1);
            let buffer = self.session.active_buffer();
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
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

        let active_id = self.session.active_id();
        for _ in 0..count {
            let Some((start_line, end_line)) = self.active_visual_line_range() else {
                break;
            };
            let can_move_down = {
                let buffer = self.session.active_buffer();
                end_line + 1 < buffer.len_lines()
            };
            if !can_move_down {
                break;
            }

            let (replace_start_char, replace_end_char, replacement) = {
                let buffer = self.session.active_buffer();
                let (sel_start_char, sel_end_char) =
                    Self::line_span_char_range(buffer, start_line, end_line);
                let (below_start_char, below_end_char) =
                    Self::line_span_char_range(buffer, end_line + 1, end_line + 1);
                let selected = buffer.slice_chars(sel_start_char, sel_end_char);
                let below = buffer.slice_chars(below_start_char, below_end_char);
                (sel_start_char, below_end_char, format!("{below}{selected}"))
            };

            let view = self.views.entry(active_id).or_default();
            {
                let buffer = self.session.active_buffer_mut();
                let replace_start = buffer.char_to_pos(replace_start_char);
                let replace_end = buffer.char_to_pos(replace_end_char);
                let _ = buffer.delete_range(replace_start, replace_end);
                let _ = buffer.insert(replace_start, &replacement);
            }
            if let Some(anchor) = view.visual_anchor.as_mut() {
                anchor.line = anchor.line.saturating_add(1);
            }
            view.cursor.cursor.line = view.cursor.cursor.line.saturating_add(1);
            let buffer = self.session.active_buffer();
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }
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

        let indent = "\t".repeat(count);
        {
            let buffer = self.session.active_buffer_mut();
            for line in start_line..=end_line {
                let line = buffer.clamp_line(line);
                let _ = buffer.insert(Pos::new(line, 0), &indent);
            }
        }

        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();
        if let Some(anchor) = view.visual_anchor.as_mut() {
            if anchor.line >= start_line && anchor.line <= end_line {
                anchor.col = anchor.col.saturating_add(count);
            }
        }
        if view.cursor.cursor.line >= start_line && view.cursor.cursor.line <= end_line {
            view.cursor.cursor.col = view.cursor.cursor.col.saturating_add(count);
        }
        let buffer = self.session.active_buffer();
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn outdent_visual_selection(
        &mut self,
        count: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        const TAB_STOP: usize = 4;

        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let Some((start_line, end_line)) = self.active_visual_line_range() else {
            return;
        };

        let mut removed_by_line: Vec<(usize, usize)> = Vec::new();
        {
            let buffer = self.session.active_buffer_mut();
            for line in start_line..=end_line {
                let line = buffer.clamp_line(line);
                let text = buffer.line_string(line);
                let chars: Vec<char> = text.chars().collect();
                let mut idx = 0usize;
                let mut levels_left = count;
                while levels_left > 0 && idx < chars.len() {
                    if chars[idx] == '\t' {
                        idx += 1;
                        levels_left -= 1;
                        continue;
                    }
                    let mut spaces = 0usize;
                    while idx + spaces < chars.len()
                        && chars[idx + spaces] == ' '
                        && spaces < TAB_STOP
                    {
                        spaces += 1;
                    }
                    if spaces == 0 {
                        break;
                    }
                    idx += spaces;
                    levels_left -= 1;
                }
                let remove_chars = idx;
                if remove_chars > 0 {
                    let _ = buffer.delete_range(Pos::new(line, 0), Pos::new(line, remove_chars));
                }
                removed_by_line.push((line, remove_chars));
            }
        }

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
        let _ = self.session.recompute_active_dirty();
    }

    pub(super) fn paste_private_register(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if self.private_register.is_empty() {
            return;
        }
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }

        let text = self.private_register.clone();
        let active_id = self.session.active_id();
        let view = self.views.entry(active_id).or_default();

        let insert_pos = {
            let buffer = self.session.active_buffer();
            match self.private_register_kind {
                RegisterKind::CharWise => {
                    let line = buffer.clamp_line(view.cursor.cursor.line);
                    let line_len = buffer.line_len_chars(line);
                    let col = if view.cursor.cursor.col < line_len {
                        view.cursor.cursor.col.saturating_add(1)
                    } else {
                        line_len
                    };
                    Pos::new(line, col)
                }
                RegisterKind::LineWise => {
                    let line = buffer.clamp_line(view.cursor.cursor.line);
                    let target_line = (line + 1).min(buffer.len_lines());
                    buffer.clamp_pos(Pos::new(target_line, 0))
                }
            }
        };

        {
            let buffer = self.session.active_buffer_mut();
            let new_pos = buffer.insert(insert_pos, &text);
            view.cursor.cursor = match self.private_register_kind {
                RegisterKind::CharWise => new_pos,
                RegisterKind::LineWise => insert_pos,
            };
            view.cursor
                .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        }

        let _ = self.session.recompute_active_dirty();
    }
}
