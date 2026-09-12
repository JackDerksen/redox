//! Background updates and deadlines, independent of drawing.

use std::time::{Duration, Instant};

use super::EditorState;
use crate::ANIMATION_FRAME_INTERVAL;

// MinUI waits on terminal input. Poll only while a background producer may reply.
pub(super) const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_millis(50);
const PERF_REFRESH_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub(super) struct RuntimeState {
    redraw_requested: bool,
    next_animation_frame: Instant,
    next_perf_refresh: Instant,
    which_key_visible: bool,
    loading_toast: Option<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            redraw_requested: true,
            next_animation_frame: Instant::now(),
            next_perf_refresh: Instant::now(),
            which_key_visible: false,
            loading_toast: None,
        }
    }
}

impl EditorState {
    pub(crate) fn request_redraw(&mut self) {
        self.runtime.redraw_requested = true;
    }

    pub(crate) fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.runtime.redraw_requested)
    }

    pub(crate) fn update_background(&mut self, now: Instant) -> Duration {
        self.poll_analysis_results();
        self.poll_lsp();
        self.poll_finder_results();
        self.poll_external_file_changes(now);
        self.expire_status_message(now);
        let load_start = Instant::now();
        self.pump_active_loading(self.viewport_height_rows.saturating_sub(1));
        let load_time = load_start.elapsed();
        self.poll_search_preview(now);

        self.git.remove_closed_buffers(&self.session);
        for buffer_id in self.views.keys().copied() {
            self.git.refresh_for_buffer(&self.session, buffer_id);
        }
        if let Some(explorer) = &self.explorer {
            self.git.refresh_repo_status_for_dir(&explorer.dir_path);
        }
        if self.git.take_changed() {
            self.request_redraw();
        }

        if (self.rain_is_active() || self.one_shot_highlight().is_some())
            && now >= self.runtime.next_animation_frame
        {
            self.advance_rain_animation();
            self.advance_one_shot_highlight();
            self.runtime.next_animation_frame = now + ANIMATION_FRAME_INTERVAL;
            self.request_redraw();
        }
        let which_key_visible = self.which_key_popup(now).is_some();
        let loading_toast = self.active_lsp_loading_toast(now);
        if which_key_visible != self.runtime.which_key_visible
            || loading_toast != self.runtime.loading_toast
        {
            self.runtime.which_key_visible = which_key_visible;
            self.runtime.loading_toast = loading_toast;
            self.request_redraw();
        }
        if self.perf_visible && now >= self.runtime.next_perf_refresh {
            self.runtime.next_perf_refresh = now + PERF_REFRESH_INTERVAL;
            self.request_redraw();
        }
        load_time
    }

    pub(crate) fn next_wake_deadline(&self, now: Instant) -> Option<Instant> {
        if self.runtime.redraw_requested {
            return Some(now);
        }
        let has_file = self.views.keys().any(|id| {
            self.session
                .meta(*id)
                .is_some_and(|meta| meta.path.is_some())
        });
        let loading = self.views.keys().any(|id| {
            self.session
                .buffer_load_status(*id)
                .is_some_and(|status| status.phase == redox_core::BufferLoadPhase::Loading)
        });
        let background_pending = loading
            || self.analysis_worker.is_pending()
            || self.finder_index_worker.is_some()
            || self.git.has_pending_work();
        let animation = self.rain_is_active() || self.one_shot_highlight().is_some();
        let which_key = self
            .which_key_enabled
            .then(|| self.input.which_key_deadline(self.which_key_delay))
            .flatten()
            .filter(|deadline| {
                !self.runtime.which_key_visible && self.which_key_popup(*deadline).is_some()
            });

        [
            has_file.then_some(self.next_external_file_check_at),
            background_pending.then_some(now + BACKGROUND_POLL_INTERVAL),
            self.lsp_poll_deadline(now),
            self.status_msg_expires_at,
            self.search_preview_due,
            which_key,
            animation.then_some(self.runtime.next_animation_frame),
            self.perf_visible.then_some(self.runtime.next_perf_refresh),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{InputAction, InputMode, map_event_with_state};
    use minui::Event;
    use redox_core::EditorSession;

    fn settle(state: &mut EditorState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            state.update_background(Instant::now());
            if !state.analysis_worker.is_pending() && !state.git.has_pending_work() {
                state.take_redraw_request();
                return;
            }
            assert!(Instant::now() < deadline, "background work did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn startup_sleeps_until_input_and_timers_redraw_once() {
        let _guard = super::super::global_test_state_lock().lock().unwrap();
        let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        state.command_open_about();
        settle(&mut state);
        let now = Instant::now();
        state.update_background(now);
        assert!(!state.take_redraw_request());
        assert_eq!(state.next_wake_deadline(now), None);

        state.set_status("temporary");
        assert!(state.take_redraw_request());
        let expiry = state.status_msg_expires_at.unwrap();
        assert_eq!(state.next_wake_deadline(now), Some(expiry));
        state.update_background(expiry);
        assert!(state.take_redraw_request());
        assert!(state.status_msg.is_none());
        assert_eq!(state.next_wake_deadline(expiry), None);

        state.command_open_about();
        settle(&mut state);
        map_event_with_state(&mut state.input, InputMode::Normal, &Event::Character(' '));
        let hint_due = state
            .input
            .which_key_deadline(state.which_key_delay)
            .unwrap();
        assert_eq!(state.next_wake_deadline(now), Some(hint_due));
        // Even if the deadline passes between updating and waiting, it must be serviced.
        assert_eq!(state.next_wake_deadline(hint_due), Some(hint_due));
        state.update_background(hint_due);
        assert!(state.take_redraw_request());
        assert!(state.which_key_popup(hint_due).is_some());
        assert_eq!(state.next_wake_deadline(hint_due), None);
        state.input.reset_prefixes();
        state.update_background(hint_due);
        assert!(state.take_redraw_request());

        state.apply_input(InputAction::YankCurrentLinePrivate { count: 1 }, 80, 24);
        let frame = state.runtime.next_animation_frame;
        state.update_background(frame);
        assert!(state.one_shot_highlight().is_some());
        assert!(state.take_redraw_request());
        state.update_background(frame + ANIMATION_FRAME_INTERVAL);
        assert!(state.one_shot_highlight().is_none());
        assert!(
            state.take_redraw_request(),
            "the final frame must erase the highlight"
        );
    }

    #[test]
    fn explorer_outside_a_repository_does_not_repeat_git_requests() {
        let _guard = super::super::global_test_state_lock().lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        state
            .open_explorer_at_path(directory.path().to_path_buf())
            .unwrap();
        settle(&mut state);
        state.clear_status();
        state.take_redraw_request();
        let now = Instant::now();
        state.update_background(now);
        assert!(!state.take_redraw_request());
        assert_eq!(state.next_wake_deadline(now), None);
    }

    #[test]
    fn file_checks_and_background_results_do_not_require_rendering() {
        let _guard = super::super::global_test_state_lock().lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("idle.txt");
        std::fs::write(&path, "before\n").unwrap();
        let mut state = EditorState::new(EditorSession::open_initial_file(&path).unwrap());
        settle(&mut state);
        let check = state.next_external_file_check_at;
        assert_eq!(state.next_wake_deadline(Instant::now()), Some(check));
        state.update_background(check);
        assert!(
            !state.take_redraw_request(),
            "unchanged file checks must not redraw"
        );
        std::fs::write(&path, "changed on disk\n").unwrap();
        state.update_background(state.next_external_file_check_at);
        assert!(state.take_redraw_request());
        assert_eq!(
            state.session.active_buffer().to_string(),
            "changed on disk\n"
        );
        assert!(state.next_wake_deadline(Instant::now()).is_some());
        settle(&mut state);
        let active_id = state.session.active_id();
        let version = state.views[&active_id].analysis_version();
        state.request_analysis(active_id, version);
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.analysis_worker.is_pending() {
            state.poll_analysis_results();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            state.take_redraw_request(),
            "analysis results must request a frame"
        );
    }
}
