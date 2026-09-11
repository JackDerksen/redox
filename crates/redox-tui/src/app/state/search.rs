use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use redox_core::Pos;
use redox_core::motion::{Motion, apply_motion_n};

use super::{
    EditorMode, EditorState, SearchLanding, SearchLineHighlights, SearchMatch, SearchOrigin,
    SearchQuery, SearchSource, SearchState,
};

const SEARCH_PREVIEW_DELAY: Duration = Duration::from_millis(100);

impl SearchQuery {
    fn landing_pos(&self, start: Pos) -> Pos {
        match self.landing {
            SearchLanding::OnMatch => start,
            SearchLanding::BeforeMatch => {
                if start.col > 0 {
                    Pos::new(start.line, start.col - 1)
                } else {
                    start
                }
            }
            SearchLanding::AfterMatch => Pos::new(start.line, start.col.saturating_add(1)),
        }
    }
}

impl EditorState {
    pub(crate) fn active_search_highlight_ranges(
        &mut self,
        first_line: usize,
        line_count: usize,
    ) -> BTreeMap<usize, SearchLineHighlights> {
        self.ensure_search_state_current();

        let mut ranges = BTreeMap::new();
        let last_line = first_line.saturating_add(line_count);
        let Some(search) = self.search_state.as_ref() else {
            return ranges;
        };
        if !search.visible {
            return ranges;
        }

        for (index, matched) in search.matches.iter().enumerate() {
            for line in matched.start.line.max(first_line)
                ..=matched.end.line.min(last_line.saturating_sub(1))
            {
                if line >= last_line
                    || (line > matched.start.line
                        && line == matched.end.line
                        && matched.end.col == 0)
                {
                    continue;
                }
                let start = if line == matched.start.line {
                    matched.start.col
                } else {
                    0
                };
                let end = if line == matched.end.line {
                    matched.end.col
                } else {
                    self.session.active_buffer().line_len_chars(line)
                };
                let highlights = ranges
                    .entry(line)
                    .or_insert_with(SearchLineHighlights::default);
                highlights.ranges.push(start..end);
                if self.mode == EditorMode::Search && search.active_match == Some(index) {
                    highlights.active = Some(start..end);
                }
            }
        }

        ranges
    }

    pub(super) fn remember_motion_search(&mut self, motion: Motion, count: usize) {
        let Some(query) = search_query_from_motion(motion) else {
            return;
        };

        let active_id = self.session.active_id();
        let cursor = self.active_cursor_pos();
        let (matches, error) = self.search_matches(&query);
        let buffer = self.session.active_buffer();
        let landing = apply_motion_n(buffer, cursor, motion, count.max(1));
        let active_match = motion_search_target_start(buffer, cursor, motion, count.max(1))
            .and_then(|target_start| {
                matches
                    .iter()
                    .position(|matched| matched.start == target_start)
            })
            .or_else(|| {
                matches
                    .iter()
                    .position(|matched| query.landing_pos(matched.start) == landing)
            });

        self.search_state = Some(SearchState {
            query,
            buffer_id: active_id,
            matches,
            active_match,
            visible: true,
            dirty: false,
            error,
        });
    }

    pub(super) fn enter_search_mode(&mut self) {
        if !self.ensure_active_fully_loaded_for_edit_or_save() {
            return;
        }
        let buffer_id = self.session.active_id();
        self.search_origin = Some(SearchOrigin {
            buffer_id,
            cursor: self.views.entry(buffer_id).or_default().cursor.clone(),
            search: self.search_state.take(),
        });
        self.mode = EditorMode::Search;
        self.search_preview_due = None;
        self.command_line.clear();
        self.command_line_cursor = 0;
        self.clear_status();
        self.input.reset_prefixes();
    }

    pub(crate) fn search_match_position(&self) -> (usize, usize) {
        self.search_state.as_ref().map_or((0, 0), |search| {
            (
                search.active_match.map_or(0, |index| index + 1),
                search.matches.len(),
            )
        })
    }

    pub(crate) fn search_error(&self) -> Option<&str> {
        self.search_state.as_ref()?.error.as_deref()
    }

    fn restore_search_cursor(&mut self) {
        if let Some(origin) = &self.search_origin
            && origin.buffer_id == self.session.active_id()
        {
            self.views.entry(origin.buffer_id).or_default().cursor = origin.cursor.clone();
        }
    }

    pub(super) fn cancel_search(&mut self) {
        if self.mode != EditorMode::Search {
            return;
        }
        self.restore_search_cursor();
        if let Some(origin) = self.search_origin.take() {
            self.search_state = origin.search;
        }
        self.mode = EditorMode::Normal;
        self.search_preview_due = None;
        self.command_line.clear();
        self.command_line_cursor = 0;
        self.clear_status();
        self.input.reset_prefixes();
    }

    pub(super) fn schedule_search_preview(&mut self, now: Instant) {
        self.clear_status();
        if self.command_line.is_empty() {
            self.search_preview_due = None;
            self.search_state = None;
            self.restore_search_cursor();
        } else {
            self.search_preview_due = Some(now + SEARCH_PREVIEW_DELAY);
        }
    }

    pub(crate) fn poll_search_preview(&mut self, now: Instant) {
        if self.mode != EditorMode::Search {
            self.search_preview_due = None;
            return;
        }
        if self.search_preview_due.is_some_and(|due| now >= due) {
            let (width, height) = self.viewport_size();
            self.update_search_preview(
                width,
                height.saturating_sub(crate::ui::STATUS_BAR_HEIGHT_ROWS),
            );
        }
    }

    fn update_search_preview(&mut self, viewport_width_cells: usize, text_vh: usize) {
        self.search_preview_due = None;
        if self.mode != EditorMode::Search {
            return;
        }
        self.clear_status();
        if self.command_line.is_empty() {
            self.search_state = None;
            self.restore_search_cursor();
            return;
        }
        let query = SearchQuery {
            term: self.command_line.clone(),
            regex: true,
            landing: SearchLanding::OnMatch,
        };
        let buffer_id = self.session.active_id();
        let cursor = self
            .search_origin
            .as_ref()
            .filter(|origin| origin.buffer_id == buffer_id)
            .map_or_else(|| self.active_cursor_pos(), |origin| origin.cursor.cursor);
        let (matches, error) = self.search_matches(&query);
        let buffer = self.session.active_buffer();
        let active_match = next_match_index_from_cursor(buffer, &matches, cursor, true)
            .or_else(|| (!matches.is_empty()).then_some(0));
        self.search_state = Some(SearchState {
            query,
            buffer_id,
            matches,
            active_match,
            visible: true,
            dirty: false,
            error,
        });
        if let Some(index) = active_match {
            self.move_cursor_to_search_match(index, viewport_width_cells, text_vh);
        } else {
            self.restore_search_cursor();
        }
    }

    pub(super) fn execute_search_line(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if self.mode != EditorMode::Search {
            return;
        }
        if self.command_line.is_empty() {
            self.cancel_search();
            return;
        }
        if self.search_preview_due.is_some()
            || self
                .search_state
                .as_ref()
                .is_none_or(|search| search.query.term != self.command_line)
        {
            self.update_search_preview(viewport_width_cells, text_vh);
        }
        if self.search_error().is_some() {
            return;
        }
        let term = std::mem::take(&mut self.command_line);
        self.command_line_cursor = 0;
        self.mode = EditorMode::Normal;
        self.search_origin = None;
        let (_, count) = self.search_match_position();
        if count == 0 {
            self.set_status("pattern not found");
        } else {
            self.set_status(format_search_match_count(&term, count));
        }
    }

    pub(super) fn repeat_search(
        &mut self,
        forward: bool,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        if self.mode == EditorMode::Search && self.search_preview_due.is_some() {
            self.update_search_preview(viewport_width_cells, text_vh);
        }
        self.ensure_search_state_current();

        let next_index = {
            let buffer = self.session.active_buffer();
            let cursor = self.active_cursor_pos();
            let Some(search) = self.search_state.as_ref() else {
                return;
            };

            if search.matches.is_empty() {
                None
            } else if search.matches.len() == 1 && search.active_match.is_some() {
                None
            } else if let Some(active) = search.active_match {
                Some(if forward {
                    (active + 1) % search.matches.len()
                } else {
                    active.checked_sub(1).unwrap_or(search.matches.len() - 1)
                })
            } else {
                next_match_index_from_cursor(buffer, &search.matches, cursor, forward).or_else(
                    || {
                        (!search.matches.is_empty()).then_some(if forward {
                            0
                        } else {
                            search.matches.len() - 1
                        })
                    },
                )
            }
        };

        let Some(next_index) = next_index else {
            self.set_status("no other pattern instances");
            return;
        };

        if let Some(search) = self.search_state.as_mut() {
            search.active_match = Some(next_index);
            search.visible = true;
        }
        self.move_cursor_to_search_match(next_index, viewport_width_cells, text_vh);
        self.clear_status();
    }

    pub(super) fn clear_search_highlights(&mut self) {
        if let Some(search) = self.search_state.as_mut() {
            search.visible = false;
            search.active_match = None;
        }
    }

    fn ensure_search_state_current(&mut self) {
        let Some(search) = self.search_state.as_ref() else {
            return;
        };
        if search.buffer_id == self.session.active_id() && !search.dirty {
            return;
        }

        let query = search.query.clone();
        let previous_buffer_id = search.buffer_id;
        let buffer_id = self.session.active_id();
        let previous_start = (previous_buffer_id == buffer_id)
            .then(|| {
                search
                    .active_match
                    .and_then(|idx| search.matches.get(idx))
                    .map(|matched| matched.start)
            })
            .flatten();
        let visible = search.visible;
        let (matches, error) = self.search_matches(&query);
        let active_match = previous_start
            .and_then(|start| matches.iter().position(|matched| matched.start == start));

        self.search_state = Some(SearchState {
            query,
            buffer_id,
            matches,
            active_match,
            visible,
            dirty: false,
            error,
        });
    }

    fn move_cursor_to_search_match(
        &mut self,
        index: usize,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
        let Some(search) = self.search_state.as_ref() else {
            return;
        };
        let Some(matched) = search.matches.get(index).copied() else {
            return;
        };

        let active_id = self.session.active_id();
        let landing = search.query.landing_pos(matched.start);
        let view = self.views.entry(active_id).or_default();
        let buffer = self.session.active_buffer();
        view.cursor.cursor = buffer.clamp_pos(landing);
        view.cursor
            .reconcile_after_edit(buffer, viewport_width_cells, text_vh);
        self.center_active_cursor_line(text_vh);
    }
}

fn format_search_match_count(term: &str, count: usize) -> String {
    let label = if count == 1 { "instance" } else { "instances" };
    format!("{count} {label} of '{term}'")
}

fn search_query_from_motion(motion: Motion) -> Option<SearchQuery> {
    match motion {
        Motion::FindChar(ch) => Some(SearchQuery {
            term: ch.to_string(),
            regex: false,
            landing: SearchLanding::OnMatch,
        }),
        Motion::TillChar(ch) => Some(SearchQuery {
            term: ch.to_string(),
            regex: false,
            landing: SearchLanding::BeforeMatch,
        }),
        Motion::FindCharBefore(ch) => Some(SearchQuery {
            term: ch.to_string(),
            regex: false,
            landing: SearchLanding::OnMatch,
        }),
        Motion::TillCharBefore(ch) => Some(SearchQuery {
            term: ch.to_string(),
            regex: false,
            landing: SearchLanding::AfterMatch,
        }),
        _ => None,
    }
}

fn motion_search_target_start(
    buffer: &redox_core::TextBuffer,
    cursor: Pos,
    motion: Motion,
    count: usize,
) -> Option<Pos> {
    let count = count.max(1);
    let mut current = buffer.clamp_pos(cursor);
    let mut target = None;

    match motion {
        Motion::FindChar(needle) => {
            for _ in 0..count {
                let found = buffer.find_char_after_on_line(current, needle)?;
                target = Some(found);
                current = found;
            }
        }
        Motion::TillChar(needle) => {
            for _ in 0..count {
                let found = buffer.find_char_after_on_line(current, needle)?;
                target = Some(found);
                let next = if found.col > 0 {
                    Pos::new(found.line, found.col - 1)
                } else {
                    found
                };
                if next == current {
                    break;
                }
                current = next;
            }
        }
        Motion::FindCharBefore(needle) | Motion::TillCharBefore(needle) => {
            for _ in 0..count {
                let found = buffer.find_char_before_on_line(current, needle)?;
                target = Some(found);
                current = found;
            }
        }
        _ => return None,
    }

    target
}

impl EditorState {
    fn search_matches(&mut self, query: &SearchQuery) -> (Vec<SearchMatch>, Option<String>) {
        let buffer_id = self.session.active_id();
        let buffer = self.session.active_buffer();
        if !query.regex {
            return (
                buffer
                    .find_matches(&query.term)
                    .into_iter()
                    .map(|(start, end)| SearchMatch { start, end })
                    .collect(),
                None,
            );
        }
        let pattern = match regex::RegexBuilder::new(&query.term)
            .multi_line(true)
            .build()
        {
            Ok(pattern) => pattern,
            Err(error) => {
                return (
                    Vec::new(),
                    Some(
                        error
                            .to_string()
                            .lines()
                            .last()
                            .unwrap_or("invalid regex")
                            .trim_start_matches("error: ")
                            .to_string(),
                    ),
                );
            }
        };
        let version = self
            .views
            .get(&buffer_id)
            .map_or(0, |view| view.analysis_version);
        let source = self.search_source.get_or_insert_with(|| SearchSource {
            buffer_id,
            version,
            text: buffer.to_string(),
        });
        if source.buffer_id != buffer_id || source.version != version {
            *source = SearchSource {
                buffer_id,
                version,
                text: buffer.to_string(),
            };
        }
        // ponytail: scans run after the debounce; use a worker if one scan exceeds the frame budget.
        let matches = pattern
            .find_iter(&source.text)
            .filter_map(|matched| {
                Some(SearchMatch {
                    start: buffer.char_to_pos(buffer.byte_to_char(matched.start())?),
                    end: buffer.char_to_pos(buffer.byte_to_char(matched.end())?),
                })
            })
            .collect();
        (matches, None)
    }
}

fn next_match_index_from_cursor(
    buffer: &redox_core::TextBuffer,
    matches: &[SearchMatch],
    cursor: Pos,
    forward: bool,
) -> Option<usize> {
    let cursor_char = buffer.pos_to_char(cursor);
    if forward {
        matches
            .iter()
            .position(|matched| buffer.pos_to_char(matched.start) >= cursor_char)
    } else {
        matches
            .iter()
            .rposition(|matched| buffer.pos_to_char(matched.start) <= cursor_char)
    }
}
