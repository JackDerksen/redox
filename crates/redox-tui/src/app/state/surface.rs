use redox_core::{BufferId, BufferKind};

use super::{EditorMode, EditorState};

impl EditorState {
    pub(super) fn active_buffer_is_surface(&self) -> bool {
        self.session.active_meta().kind == BufferKind::Ui
    }

    pub(crate) fn handle_normal_mode_escape_on_surface(&mut self) -> bool {
        if self.mode != EditorMode::Normal || !self.active_buffer_is_surface() {
            return false;
        }

        if self.close_active_surface_buffer() {
            self.mode = EditorMode::Normal;
            self.clear_status();
            return true;
        }

        false
    }

    pub(super) fn close_active_surface_buffer(&mut self) -> bool {
        self.close_active_surface_buffer_inner(false)
    }

    pub(super) fn close_active_surface_buffer_without_quit(&mut self) -> bool {
        self.close_active_surface_buffer_inner(true)
    }

    pub(super) fn close_active_surfaces_for_command(&mut self) -> bool {
        while self.active_buffer_is_surface() {
            if !self.close_active_surface_buffer_without_quit() {
                return false;
            }
        }
        true
    }

    fn close_active_surface_buffer_inner(&mut self, suppress_quit_after_close: bool) -> bool {
        let active_id = self.session.active_id();
        let is_explorer = self
            .explorer
            .as_ref()
            .is_some_and(|explorer| explorer.buffer_id == active_id);
        let is_about = self
            .about
            .as_ref()
            .is_some_and(|about| about.buffer_id == active_id);
        let is_undo_tree = self
            .undo_tree
            .as_ref()
            .is_some_and(|tree| tree.buffer_id == active_id || tree.diff_buffer_id == active_id);
        if is_undo_tree && let Some(tree) = self.undo_tree.clone() {
            return self.close_undo_tree_panel(tree);
        }

        let return_to = self
            .explorer
            .as_ref()
            .and_then(|explorer| {
                (explorer.buffer_id == active_id).then_some(explorer.return_to_buffer_id)
            })
            .or_else(|| {
                self.about.as_ref().and_then(|about| {
                    (about.buffer_id == active_id).then_some(about.return_to_buffer_id)
                })
            });
        let should_quit_after_close = (is_explorer || is_about)
            && !suppress_quit_after_close
            && return_to.is_some_and(|id| self.is_empty_unnamed_startup_buffer(id));

        if !self.session.close_active_buffer() {
            return false;
        }
        self.views.remove(&active_id);

        if is_explorer {
            self.explorer = None;
        }

        if is_about {
            self.about = None;
        }

        /*
        if is_undo_tree {
            self.undo_tree = None;
        }
        */

        if (is_explorer || is_about)
            && let Some(target) = return_to
        {
            let _ = self.session.activate(target);
            self.ensure_buffer_analysis(target);
        }

        if should_quit_after_close {
            self.should_quit = true;
        }

        true
    }

    pub(super) fn is_empty_unnamed_startup_buffer(&self, id: redox_core::BufferId) -> bool {
        let Some(meta) = self.session.meta(id) else {
            return false;
        };
        if meta.kind != BufferKind::File {
            return false;
        }
        if meta.path.is_some() || meta.dirty || !meta.is_new_file {
            return false;
        }

        self.session
            .buffer(id)
            .is_some_and(|buffer| buffer.to_string().is_empty())
    }

    pub(super) fn close_inactive_empty_unnamed_startup_buffer(&mut self, id: BufferId) -> bool {
        if self.session.active_id() == id || !self.is_empty_unnamed_startup_buffer(id) {
            return false;
        }

        if self.session.close_buffer(id) {
            self.views.remove(&id);
            true
        } else {
            false
        }
    }
}
