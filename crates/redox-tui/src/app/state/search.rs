use std::collections::BTreeMap;
use std::ops::Range;

use redox_core::Pos;
use redox_core::motion::{Motion, apply_motion_n};

use super::{EditorMode, EditorState, SearchLanding, SearchMatch, SearchQuery, SearchState};

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
        }
    }
}

impl EditorState {
    pub(crate) fn active_search_highlight_ranges(
        &mut self,
        first_line: usize,
        line_count: usize,
    ) -> BTreeMap<usize, Vec<Range<usize>>> {
        self.ensure_search_state_current();

        let mut ranges = BTreeMap::new();
        let last_line = first_line.saturating_add(line_count);
        let Some(search) = self.search_state.as_ref() else {
            return ranges;
        };
        if !search.visible {
            return ranges;
        }

        for matched in &search.matches {
            if matched.start.line < first_line || matched.start.line >= last_line {
                continue;
            }
            if matched.start.line != matched.end.line {
                continue;
            }
            ranges
                .entry(matched.start.line)
                .or_insert_with(Vec::new)
                .push(matched.start.col..matched.end.col);
        }

        ranges
    }

    pub(super) fn remember_motion_search(&mut self, motion: Motion, count: usize) {
        let Some(query) = search_query_from_motion(motion) else {
            return;
        };

        let active_id = self.session.active_id();
        let buffer = self.session.active_buffer();
        let cursor = self.active_cursor_pos();
        let landing = apply_motion_n(buffer, cursor, motion, count.max(1));
        let matches = search_matches_for_buffer(buffer, &query);
        let active_match = motion_search_target_start(buffer, cursor, motion, count.max(1))
            .and_then(|target_start| matches.iter().position(|matched| matched.start == target_start))
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
        });
    }

    pub(super) fn enter_search_mode(&mut self) {
        self.mode = EditorMode::Search;
        self.command_line.clear();
        self.clear_status();
        self.input.reset_prefixes();
    }

    pub(super) fn execute_search_line(&mut self, viewport_width_cells: usize, text_vh: usize) {
        if self.mode != EditorMode::Search {
            return;
        }

        let term = std::mem::take(&mut self.command_line);
        self.mode = EditorMode::Normal;

        if term.is_empty() {
            self.clear_status();
            return;
        }

        let query = SearchQuery {
            term,
            landing: SearchLanding::OnMatch,
        };
        let active_id = self.session.active_id();
        let cursor = self.active_cursor_pos();
        let matches = {
            let buffer = self.session.active_buffer();
            search_matches_for_buffer(buffer, &query)
        };
        let active_match = {
            let buffer = self.session.active_buffer();
            next_match_index_from_cursor(buffer, &matches, cursor, true)
                .or_else(|| (!matches.is_empty()).then_some(0))
        };

        self.search_state = Some(SearchState {
            query,
            buffer_id: active_id,
            matches,
            active_match,
            visible: true,
            dirty: false,
        });

        if let Some(index) = active_match {
            self.move_cursor_to_search_match(index, viewport_width_cells, text_vh);
            self.clear_status();
        } else {
            self.set_status("pattern not found");
        }
    }

    pub(super) fn repeat_search(
        &mut self,
        forward: bool,
        viewport_width_cells: usize,
        text_vh: usize,
    ) {
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
        let previous_start = search
            .active_match
            .and_then(|idx| search.matches.get(idx))
            .map(|matched| matched.start);
        let buffer_id = self.session.active_id();
        let matches = {
            let buffer = self.session.active_buffer();
            search_matches_for_buffer(buffer, &query)
        };
        let active_match = previous_start
            .and_then(|start| matches.iter().position(|matched| matched.start == start));

        self.search_state = Some(SearchState {
            query,
            buffer_id,
            matches,
            active_match,
            visible: search.visible,
            dirty: false,
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
    }
}

fn search_query_from_motion(motion: Motion) -> Option<SearchQuery> {
    match motion {
        Motion::FindChar(ch) => Some(SearchQuery {
            term: ch.to_string(),
            landing: SearchLanding::OnMatch,
        }),
        Motion::TillChar(ch) => Some(SearchQuery {
            term: ch.to_string(),
            landing: SearchLanding::BeforeMatch,
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
        _ => return None,
    }

    target
}

fn search_matches_for_buffer(
    buffer: &redox_core::TextBuffer,
    query: &SearchQuery,
) -> Vec<SearchMatch> {
    buffer
        .find_matches(&query.term)
        .into_iter()
        .map(|(start, end)| SearchMatch { start, end })
        .collect()
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
