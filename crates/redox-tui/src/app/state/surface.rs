use redox_core::BufferKind;

use super::{EditorMode, EditorState};

impl EditorState {
    pub(super) fn active_buffer_is_surface(&self) -> bool {
        self.session.active_meta().kind == BufferKind::Ui
    }

    pub(crate) fn handle_normal_mode_q_on_surface(&mut self) -> bool {
        if self.mode != EditorMode::Normal
            || !self.active_buffer_is_surface()
            || self.explorer_is_active()
        {
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

        if (is_explorer || is_about)
            && let Some(target) = return_to
        {
            let _ = self.session.activate(target);
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
}
