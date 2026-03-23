use minui::ColorPair;

use super::EditorState;
use crate::ui::{TextViewport, UiStyle, language_for_path, snapshot_lines_wrapped_cached};

impl EditorState {
    pub(super) fn command_rain(&mut self) {
        if self.active_buffer_is_surface() {
            self.set_status("rain is only available in text buffers");
            return;
        }

        self.rain_animation = None;
        self.rain_pending_start = true;
        self.set_status("making it rain");
    }

    pub fn rain_is_active(&self) -> bool {
        self.rain_pending_start || self.rain_animation.is_some()
    }

    pub fn active_rain_animation(&self) -> Option<&crate::ui::RainAnimation> {
        self.rain_animation.as_ref()
    }

    pub fn ensure_rain_animation(
        &mut self,
        text_width: u16,
        text_height: u16,
        default_colors: ColorPair,
        style: UiStyle,
    ) {
        if !self.rain_pending_start
            || self.rain_animation.is_some()
            || text_width == 0
            || text_height == 0
        {
            return;
        }

        let syntax_language = language_for_path(self.session.active_meta().path.as_deref());
        let animation = self.with_active_buffer_view_mut(|buffer, view| {
            let (scroll_x, scroll_y) = view.cursor.viewport_scroll();
            let viewport = TextViewport {
                scroll_x,
                scroll_y,
                width: text_width,
                height: text_height,
            };
            let snapshot =
                snapshot_lines_wrapped_cached(buffer, &viewport, &mut view.grapheme_cache);
            let syntax_spans = view.syntax_highlighter.visible_line_spans(
                buffer,
                syntax_language,
                snapshot.first_line,
                snapshot.lines.len(),
            );

            crate::ui::RainAnimation::capture(
                buffer,
                &mut view.grapheme_cache,
                snapshot.first_line,
                scroll_x,
                text_width as usize,
                text_height as usize,
                default_colors,
                style,
                syntax_spans.as_deref(),
                None,
            )
        });

        self.rain_animation = Some(animation);
        self.rain_pending_start = false;
    }

    pub fn advance_rain_animation(&mut self) {
        if let Some(animation) = self.rain_animation.as_mut() {
            let _ = animation.update();
        }
    }

    pub fn stop_rain_animation(&mut self) {
        self.rain_animation = None;
        self.rain_pending_start = false;
        self.clear_status();
    }
}
