use redox_core::{BufferId, Pos};

use super::{EditorMode, EditorState};

const ABOUT_REPO_URL: &str = "   github.com/JackDerksen/redox";
const ABOUT_CRATES_URL: &str = "crates.io/crates/redox-editor";
const ABOUT_MESSAGE: &str = "Thank you for using Redox.\nAn editor made by Jack Derksen.";

#[derive(Debug, Clone)]
pub struct AboutPopup {
    pub title: String,
    pub version: String,
    pub message: String,
    pub repo_url: String,
    pub crates_url: String,
}

#[derive(Debug, Clone)]
pub(super) struct AboutState {
    pub(super) buffer_id: BufferId,
    pub(super) return_to_buffer_id: BufferId,
}

impl EditorState {
    pub fn about_popup(&self) -> Option<AboutPopup> {
        let about = self.about.as_ref()?;
        if about.buffer_id != self.session.active_id() {
            return None;
        }

        Some(AboutPopup {
            title: "about".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            message: ABOUT_MESSAGE.to_string(),
            repo_url: ABOUT_REPO_URL.to_string(),
            crates_url: ABOUT_CRATES_URL.to_string(),
        })
    }

    pub fn about_background_buffer_id(&self) -> Option<BufferId> {
        let about = self.about.as_ref()?;
        if about.buffer_id != self.session.active_id() {
            return None;
        }
        self.session
            .buffer(about.return_to_buffer_id)
            .map(|_| about.return_to_buffer_id)
    }

    pub(super) fn about_is_active(&self) -> bool {
        self.about
            .as_ref()
            .is_some_and(|about| about.buffer_id == self.session.active_id())
    }

    pub(super) fn command_open_about(&mut self) {
        if self.about_is_active() {
            let _ = self.close_active_surface_buffer();
            self.mode = EditorMode::Normal;
            self.clear_status();
            return;
        }

        if self.active_buffer_is_surface() {
            let _ = self.close_active_surface_buffer();
        }

        self.open_about_buffer();
        self.mode = EditorMode::Normal;
        self.clear_status();
    }

    fn open_about_buffer(&mut self) {
        let return_to = self.session.active_id();
        let about_text = format!(
            "Redox {}\n{}\n{}\n{}",
            env!("CARGO_PKG_VERSION"),
            ABOUT_MESSAGE,
            ABOUT_REPO_URL,
            ABOUT_CRATES_URL
        );
        let about_id = self
            .session
            .open_ui_buffer("[about] Redox (:q to close)", &about_text);
        self.session.mark_active_clean();

        let view = self.views.entry(about_id).or_default();
        view.cursor.cursor = Pos::zero();
        view.cursor.follow.top_margin_rows = 0;
        view.cursor.follow.bottom_margin_rows = 0;
        view.invalidate_render_caches();

        self.about = Some(AboutState {
            buffer_id: about_id,
            return_to_buffer_id: return_to,
        });
    }
}
