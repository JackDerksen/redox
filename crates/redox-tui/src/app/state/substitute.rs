use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use redox_core::{
    BufferId, BufferKind, Edit, Pos, Selection, TextBuffer, UndoCheckpoint, VisualModeKind,
};
use regex::{Captures, Regex, RegexBuilder};

use super::{BufferViewState, EditorMode, EditorState, SearchMatch};
use crate::input::InputAction;
use crate::ui::STATUS_BAR_HEIGHT_ROWS;
use crate::ui::syntax::{SyntaxHighlighter, SyntaxLanguage, SyntaxParser, language_for_path};

const SUBSTITUTE_WORKER_MIN_BYTES: usize = 64 * 1024;

#[derive(Debug, Default)]
pub(super) struct SubstituteState {
    scope: Option<CommandScope>,
    preview: Option<SubstitutePreview>,
    worker: Option<Receiver<SubstitutePreview>>,
    confirmation: Option<SubstituteConfirmation>,
}

#[derive(Debug)]
struct SubstituteConfirmation {
    edits: VecDeque<Edit>,
    before: UndoCheckpoint,
    removed_chars: usize,
    inserted_chars: usize,
    count: usize,
}

#[derive(Debug)]
struct CommandScope {
    buffer_id: BufferId,
    version: u64,
    visual: Option<(Selection, VisualModeKind)>,
}

#[derive(Debug)]
pub(crate) struct SubstitutePreview {
    buffer_id: BufferId,
    version: u64,
    command: String,
    pub(super) matches: Vec<SearchMatch>,
    edits: Vec<Edit>,
    pub error: Option<String>,
    pub replacing: bool,
    confirm: bool,
    pub pending: bool,
    pub buffer: Option<TextBuffer>,
    pub syntax: SyntaxHighlighter,
}

impl SubstitutePreview {
    fn new(buffer_id: BufferId, version: u64, command: String) -> Self {
        Self {
            buffer_id,
            version,
            command,
            matches: Vec::new(),
            edits: Vec::new(),
            error: None,
            replacing: false,
            confirm: false,
            pending: false,
            buffer: None,
            syntax: SyntaxHighlighter::default(),
        }
    }

    pub(crate) fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub(crate) fn display_position(&self, source: &TextBuffer, position: Pos) -> Pos {
        let Some(buffer) = &self.buffer else {
            return position;
        };
        let original = source.pos_to_char(position);
        let mut mapped = original;
        for edit in &self.edits {
            if edit.range.start >= original {
                break;
            }
            let removed = edit.range.end.min(original) - edit.range.start;
            let inserted = edit.insert.chars().count();
            mapped = mapped - removed
                + if original < edit.range.end {
                    removed.min(inserted)
                } else {
                    inserted
                };
        }
        buffer.char_to_pos(mapped)
    }

    pub(crate) fn highlight_ranges(
        &self,
        buffer: &TextBuffer,
        first_line: usize,
        line_count: usize,
    ) -> std::collections::BTreeMap<usize, super::SearchLineHighlights> {
        super::search::match_highlight_ranges(buffer, &self.matches, None, first_line, line_count)
    }
}

struct Substitution {
    pattern: Regex,
    replacement: Option<Vec<ReplacementPart>>,
    global: bool,
    confirm: bool,
    range: Option<SubstituteRange>,
}

enum SubstituteRange {
    WholeFile,
    Lines(usize, usize),
}

enum ReplacementPart {
    Literal(String),
    Capture(usize),
}

impl EditorState {
    pub(super) fn begin_command(&mut self) {
        let buffer_id = self.session.active_id();
        self.substitution = SubstituteState {
            scope: Some(CommandScope {
                buffer_id,
                version: self
                    .views
                    .get(&buffer_id)
                    .map_or(0, |view| view.analysis_version),
                visual: self.active_visual_selection(),
            }),
            ..SubstituteState::default()
        };
        self.close_completion();
        self.clear_active_visual_anchor();
        self.mode = EditorMode::Command;
        self.command_line.clear();
        self.command_line_cursor = 0;
        self.reset_command_history_navigation();
        self.clear_status();
        self.input.reset_prefixes();
    }

    pub(super) fn refresh_substitute_preview(&mut self) {
        if self.has_substitute_confirmation() {
            return;
        }
        if self.mode != EditorMode::Command {
            self.substitution = SubstituteState::default();
            return;
        }
        if substitute_body(&self.command_line).is_none() {
            self.substitution.preview = None;
            return;
        }
        let buffer_id = self.session.active_id();
        let version = self
            .views
            .get(&buffer_id)
            .map_or(0, |view| view.analysis_version);
        if self
            .substitute_preview()
            .is_some_and(|preview| !preview.pending || self.substitution.worker.is_some())
        {
            return;
        }
        let mut preview = SubstitutePreview::new(buffer_id, version, self.command_line.clone());
        let scope = self.substitution.scope.as_ref();
        if self.session.active_meta().kind != BufferKind::File {
            preview.error = Some("substitution requires a file buffer".into());
        } else if scope.is_some_and(|scope| {
            scope.buffer_id != buffer_id || scope.visual.is_some() && scope.version != version
        }) {
            preview.error = Some("selection changed; cancel and select the text again".into());
        } else if !self.ensure_active_fully_loaded_for_edit_or_save() {
            preview.error = Some("could not load the complete file".into());
        } else {
            // Completing an incremental load can advance the analysis version.
            preview.version = self
                .views
                .get(&buffer_id)
                .map_or(0, |view| view.analysis_version);
            if let Some(scope) = &mut self.substitution.scope {
                scope.version = preview.version;
            }
            let selection = self
                .substitution
                .scope
                .as_ref()
                .and_then(|scope| scope.visual);
            match Substitution::parse(&self.command_line) {
                Ok(substitution) => {
                    preview.replacing = substitution.replacement.is_some();
                    preview.confirm = substitution.confirm;
                    let language = language_for_path(self.session.active_meta().path.as_deref());
                    if preview.replacing
                        && self.session.active_buffer().len_bytes() >= SUBSTITUTE_WORKER_MIN_BYTES
                    {
                        // Keep the displayed text, position mapping, and highlights
                        // together while the next preview is computed.
                        let mut pending = self
                            .substitution
                            .preview
                            .take()
                            .filter(|previous| {
                                previous.buffer_id == buffer_id
                                    && previous.version == preview.version
                                    && previous.buffer.is_some()
                            })
                            .unwrap_or_else(|| {
                                SubstitutePreview::new(
                                    buffer_id,
                                    preview.version,
                                    preview.command.clone(),
                                )
                            });
                        pending.command.clone_from(&preview.command);
                        pending.error = None;
                        pending.replacing = true;
                        pending.pending = true;
                        // A running job finishes first; the next refresh dispatches only
                        // the latest command, without queuing a buffer per keystroke.
                        if self.substitution.worker.is_none() {
                            let buffer = self.session.active_buffer().clone();
                            let (sender, receiver) = mpsc::channel();
                            match std::thread::Builder::new()
                                .name("redox-substitute".into())
                                .spawn(move || {
                                    substitution.plan(&buffer, selection, language, &mut preview);
                                    let _ = sender.send(preview);
                                }) {
                                Ok(_) => self.substitution.worker = Some(receiver),
                                Err(error) => {
                                    pending.pending = false;
                                    pending.error =
                                        Some(format!("could not start preview: {error}"));
                                }
                            }
                        }
                        preview = pending;
                    } else {
                        substitution.plan(
                            self.session.active_buffer(),
                            selection,
                            language,
                            &mut preview,
                        );
                    }
                }
                Err(error) => preview.error = Some(error),
            }
        }
        self.substitution.preview = Some(preview);
        self.request_redraw();
    }

    pub(super) fn substitute_preview_pending(&self) -> bool {
        self.substitution.worker.is_some()
    }

    pub(super) fn poll_substitute_preview(&mut self) {
        self.receive_substitute_preview(false);
    }

    fn receive_substitute_preview(&mut self, wait: bool) {
        let Some(receiver) = &self.substitution.worker else {
            return;
        };
        let result = match if wait {
            receiver.recv().map_err(|_| TryRecvError::Disconnected)
        } else {
            receiver.try_recv()
        } {
            Err(TryRecvError::Empty) => return,
            result => result,
        };
        self.substitution.worker = None;
        let Ok(preview) = result else {
            if let Some(preview) = self
                .substitution
                .preview
                .as_mut()
                .filter(|preview| preview.pending)
            {
                preview.pending = false;
                preview.error = Some("preview worker stopped".into());
                self.request_redraw();
            }
            return;
        };
        if !self.substitute_preview().is_some_and(|current| {
            current.pending
                && current.buffer_id == preview.buffer_id
                && current.version == preview.version
                && current.command == preview.command
        }) {
            return;
        }
        self.substitution.preview = Some(preview);
        self.request_redraw();
    }

    pub(crate) fn substitute_preview(&self) -> Option<&SubstitutePreview> {
        let preview = self.substitution.preview.as_ref()?;
        (self.mode == EditorMode::Command
            && preview.buffer_id == self.session.active_id()
            && preview.command == self.command_line
            && self
                .views
                .get(&preview.buffer_id)
                .map_or(0, |view| view.analysis_version)
                == preview.version)
            .then_some(preview)
    }

    pub(crate) fn with_buffer_display_view_mut<R>(
        &mut self,
        buffer_id: BufferId,
        render: impl FnOnce(&TextBuffer, &mut BufferViewState, Option<&SubstitutePreview>) -> R,
    ) -> Option<R> {
        let preview_active = self
            .substitute_preview()
            .is_some_and(|preview| preview.buffer_id == buffer_id && preview.buffer.is_some());
        let preview = self
            .substitution
            .preview
            .as_ref()
            .filter(|_| preview_active);
        let buffer = preview
            .and_then(|preview| preview.buffer.as_ref())
            .or_else(|| self.session.buffer(buffer_id))?;
        let scrolloff = self.configured_scrolloff_for_buffer(buffer_id);
        let view = self.views.entry(buffer_id).or_default();
        view.cursor.set_scrolloff_rows(scrolloff);
        Some(render(buffer, view, preview))
    }

    pub(super) fn execute_substitute_command(&mut self) -> bool {
        if substitute_body(&self.command_line).is_none() {
            return false;
        }
        self.refresh_substitute_preview();
        // Submission must finish before a macro or mapped sequence runs its next action.
        while self
            .substitute_preview()
            .is_some_and(|preview| preview.pending)
        {
            self.receive_substitute_preview(true);
            self.refresh_substitute_preview();
        }
        let Some(preview) = self.substitution.preview.as_ref() else {
            return true;
        };
        if let Some(error) = &preview.error {
            self.set_status(format!("substitute: {error}"));
            return true;
        }
        if !preview.replacing {
            self.set_status(
                "substitute: finish the pattern with a delimiter and enter a replacement",
            );
            return true;
        }
        let mut preview = self.substitution.preview.take().unwrap();
        let count = preview.matches.len();
        if preview.confirm && !preview.edits.is_empty() {
            self.substitution.confirmation = Some(SubstituteConfirmation {
                edits: std::mem::take(&mut preview.edits).into(),
                before: self.capture_active_undo_checkpoint(),
                removed_chars: 0,
                inserted_chars: 0,
                count: 0,
            });
            preview.buffer = None;
            self.substitution.preview = Some(preview);
            self.show_substitute_confirmation();
            return true;
        }
        if !preview.edits.is_empty() {
            let before = self.capture_active_undo_checkpoint();
            let cursor = preview.edits[0].range.start;
            // Core edit batches use sequential coordinates; replace from the end.
            preview.edits.reverse();
            self.session.active_buffer_mut().apply_edits(&preview.edits);
            let (width, height) = self.viewport_size();
            self.with_active_buffer_view_mut(|buffer, view| {
                view.cursor.cursor = buffer.char_to_pos(cursor);
                view.cursor.reconcile_after_edit(
                    buffer,
                    width,
                    height.saturating_sub(STATUS_BAR_HEIGHT_ROWS),
                );
            });
            self.refresh_active_indentation();
            self.invalidate_active_render_caches();
            self.record_active_undo_if_changed(before);
            self.session.recompute_active_dirty();
        }
        self.finish_substitute_command(count);
        true
    }

    fn finish_substitute_command(&mut self, count: usize) {
        let command = std::mem::take(&mut self.command_line);
        self.push_command_history(command);
        self.mode = EditorMode::Normal;
        self.with_active_buffer_view_mut(|buffer, view| view.cursor.clamp_for_normal_mode(buffer));
        self.command_line_cursor = 0;
        self.reset_command_history_navigation();
        self.input.reset_prefixes();
        self.substitution = SubstituteState::default();
        self.set_status(format!(
            "{count} substitution{}",
            if count == 1 { "" } else { "s" }
        ));
    }

    pub(crate) fn has_substitute_confirmation(&self) -> bool {
        self.substitution.confirmation.is_some()
    }

    pub(crate) fn substitute_confirmation_prompt(&self) -> Option<String> {
        let edit = self.substitution.confirmation.as_ref()?.edits.front()?;
        Some(format!(
            "y=yes n=no a=all l=last q/Esc=quit | Replace with {:?}",
            edit.insert
        ))
    }

    pub(super) fn handle_substitute_confirmation(&mut self, action: &InputAction) -> bool {
        if !self.has_substitute_confirmation() {
            return false;
        }
        let answer = match action {
            InputAction::CommandChar(answer @ ('y' | 'n' | 'a' | 'l' | 'q')) => *answer,
            InputAction::CommandCancel => 'q',
            _ => return true,
        };
        let mut confirmation = self.substitution.confirmation.take().unwrap();
        if matches!(answer, 'y' | 'a' | 'l') {
            while let Some(mut edit) = confirmation.edits.pop_front() {
                edit.range = edit.range.start - confirmation.removed_chars
                    + confirmation.inserted_chars
                    ..edit.range.end - confirmation.removed_chars + confirmation.inserted_chars;
                confirmation.removed_chars += edit.range.len();
                confirmation.inserted_chars += edit.insert.chars().count();
                let cursor = self.session.active_buffer().char_to_pos(edit.range.start);
                self.session.active_buffer_mut().apply_edit(edit);
                self.views
                    .entry(self.session.active_id())
                    .or_default()
                    .cursor
                    .place_cursor(cursor);
                confirmation.count += 1;
                if answer != 'a' {
                    break;
                }
            }
            self.refresh_active_indentation();
            self.invalidate_active_render_caches();
            self.session.recompute_active_dirty();
        } else if answer == 'n' {
            confirmation.edits.pop_front();
        }
        if confirmation.edits.is_empty() || matches!(answer, 'a' | 'l' | 'q') {
            self.record_active_undo_if_changed(confirmation.before);
            self.finish_substitute_command(confirmation.count);
            let (width, height) = self.viewport_size();
            let text_height = height.saturating_sub(STATUS_BAR_HEIGHT_ROWS);
            self.with_active_buffer_view_mut(|buffer, view| {
                view.cursor.reconcile_scroll(buffer, width, text_height);
            });
            self.center_active_cursor_line(text_height);
        } else {
            self.substitution.confirmation = Some(confirmation);
            self.show_substitute_confirmation();
        }
        self.request_redraw();
        true
    }

    fn show_substitute_confirmation(&mut self) {
        let confirmation = self.substitution.confirmation.as_ref().unwrap();
        let edit = confirmation.edits.front().unwrap();
        let buffer = self.session.active_buffer();
        let start = buffer.char_to_pos(
            edit.range.start - confirmation.removed_chars + confirmation.inserted_chars,
        );
        let end = buffer
            .char_to_pos(edit.range.end - confirmation.removed_chars + confirmation.inserted_chars);
        let view = self.views.entry(self.session.active_id()).or_default();
        view.cursor.place_cursor(start);
        let preview = self.substitution.preview.as_mut().unwrap();
        preview.version = view.analysis_version;
        preview.matches = vec![SearchMatch { start, end }];
        let (_, height) = self.viewport_size();
        self.center_active_cursor_line(height.saturating_sub(STATUS_BAR_HEIGHT_ROWS));
        self.request_redraw();
    }
}

fn substitute_body(command: &str) -> Option<(&str, char, &str)> {
    let command = command.trim_start();
    let offset = command.find('s')?;
    let range = command[..offset].trim();
    if !range.chars().all(|character| {
        character.is_ascii_digit()
            || character.is_ascii_whitespace()
            || matches!(character, ',' | '%')
    }) {
        return None;
    }
    let command = &command[offset..];
    let body = command
        .strip_prefix("substitute")
        .or_else(|| command.strip_prefix('s'))?;
    let delimiter = body.chars().next()?;
    (delimiter.is_ascii_punctuation() && delimiter != '\\').then_some((
        range,
        delimiter,
        &body[delimiter.len_utf8()..],
    ))
}

fn delimited_part(text: &str, delimiter: char) -> (&str, Option<&str>) {
    let mut escaped = false;
    for (offset, character) in text.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == delimiter {
            return (
                &text[..offset],
                Some(&text[offset + character.len_utf8()..]),
            );
        }
    }
    (text, None)
}

impl Substitution {
    fn parse(command: &str) -> Result<Self, String> {
        let (range, delimiter, body) =
            substitute_body(command).ok_or("usage: [range]s/pattern/replacement/g")?;
        let range = match range {
            "" => None,
            "%" => Some(SubstituteRange::WholeFile),
            _ => {
                let (first, last) = range.split_once(',').unwrap_or((range, range));
                let first = first
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid line range")?
                    .max(1);
                let last = last
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid line range")?
                    .max(1);
                if first > last {
                    return Err("line range must run from first to last".into());
                }
                Some(SubstituteRange::Lines(first, last))
            }
        };
        let (pattern, tail) = delimited_part(body, delimiter);
        if pattern.is_empty() {
            return Err("enter a search pattern".into());
        }
        let (replacement, flags) = match tail {
            Some(tail) => {
                let (replacement, flags) = delimited_part(tail, delimiter);
                (Some(replacement), flags.unwrap_or(""))
            }
            None => (None, ""),
        };
        let mut global = false;
        let mut confirm = false;
        let mut ignore_case = false;
        for flag in flags.trim().chars() {
            match flag {
                'g' => global = true,
                'c' => confirm = true,
                'i' => ignore_case = true,
                _ => return Err(format!("unsupported substitute flag: {flag}")),
            }
        }
        let pattern = vim_pattern(pattern, delimiter)?;
        let pattern = RegexBuilder::new(&pattern)
            .multi_line(true)
            .case_insensitive(ignore_case)
            .build()
            .map_err(|error| {
                error
                    .to_string()
                    .lines()
                    .last()
                    .unwrap_or("invalid regex")
                    .trim_start_matches("error: ")
                    .to_owned()
            })?;
        let replacement = replacement
            .map(|text| replacement_parts(text, delimiter, pattern.captures_len()))
            .transpose()?;
        Ok(Self {
            pattern,
            replacement,
            global,
            confirm,
            range,
        })
    }

    fn plan(
        &self,
        buffer: &TextBuffer,
        selection: Option<(Selection, VisualModeKind)>,
        language: Option<SyntaxLanguage>,
        preview: &mut SubstitutePreview,
    ) {
        let selection = match self.range {
            Some(SubstituteRange::WholeFile) => None,
            Some(SubstituteRange::Lines(first, last)) => {
                if last > buffer.len_lines() {
                    preview.error = Some("line range is outside the file".into());
                    return;
                }
                Some((
                    Selection::new(Pos::new(first - 1, 0), Pos::new(last - 1, 0)),
                    VisualModeKind::Line,
                ))
            }
            None => selection,
        };
        let scopes = match selection {
            Some((selection, VisualModeKind::Block)) => {
                buffer.visual_blockwise_pos_ranges(selection)
            }
            Some((selection, mode)) => buffer.visual_selection_pos_ranges(selection, mode),
            None => vec![(
                buffer.char_to_pos(0),
                buffer.char_to_pos(buffer.len_chars()),
            )],
        };
        let mut last_line = None;
        let mut replacement_ranges = Vec::new();
        let mut removed_chars = 0;
        let mut inserted_chars = 0;
        for (start, end) in scopes {
            let start_byte = buffer.char_to_byte(buffer.pos_to_char(start));
            let selected_text = buffer.slice_pos_range(start, end);
            for captures in self.pattern.captures_iter(&selected_text) {
                let matched = captures.get(0).unwrap();
                let match_start = start_byte + matched.start();
                if matched.start() == selected_text.len() && selected_text.ends_with('\n') {
                    continue;
                }
                let range = buffer.byte_to_char(match_start).unwrap()
                    ..buffer.byte_to_char(start_byte + matched.end()).unwrap();
                let start = buffer.char_to_pos(range.start);
                if !self.global && self.replacement.is_some() && last_line == Some(start.line) {
                    continue;
                }
                last_line = Some(start.line);
                preview.matches.push(SearchMatch {
                    start,
                    end: buffer.char_to_pos(range.end),
                });
                if let Some(parts) = &self.replacement {
                    let replacement = expand_replacement(parts, &captures);
                    let start = range.start - removed_chars + inserted_chars;
                    let length = replacement.chars().count();
                    replacement_ranges.push(start..start + length);
                    removed_chars += range.len();
                    inserted_chars += length;
                    if self.confirm || replacement != matched.as_str() {
                        preview.edits.push(Edit::replace(range, replacement));
                    }
                }
            }
        }
        if self.replacement.is_some() {
            let mut changed = buffer.clone();
            for edit in preview.edits.iter().rev() {
                changed.apply_edit(edit.clone());
            }
            preview.matches = replacement_ranges
                .into_iter()
                .map(|range| SearchMatch {
                    start: changed.char_to_pos(range.start),
                    end: changed.char_to_pos(range.end),
                })
                .collect();
            preview.buffer = Some(changed);
        }
        if let Some(buffer) = &preview.buffer
            && let Some(language) = language
        {
            preview
                .syntax
                .replace_cache(SyntaxParser::default().compute_cache(buffer, language));
        }
    }
}

// Translate Vim's default magic operators without changing escaped literals or classes.
fn vim_pattern(pattern: &str, delimiter: char) -> Result<String, String> {
    let mut result = String::new();
    let mut characters = pattern.chars();
    let mut in_class = false;
    let mut very_magic = false;
    while let Some(character) = characters.next() {
        if character == '\\' {
            let next = characters.next().ok_or("incomplete regex escape")?;
            if next == delimiter {
                result.push_str(&regex::escape(&next.to_string()));
            } else if in_class {
                result.push('\\');
                result.push(next);
            } else {
                match next {
                    'v' => very_magic = true,
                    'm' => very_magic = false,
                    '(' | ')' | '+' | '?' | '|' if !very_magic => result.push(next),
                    '=' if !very_magic => result.push('?'),
                    '{' if !very_magic => {
                        let mut count = String::new();
                        loop {
                            match characters.next() {
                                Some('}') => break,
                                Some(character) => count.push(character),
                                None => return Err("unclosed repetition count".into()),
                            }
                        }
                        let lazy = count.starts_with('-');
                        let count = count.strip_prefix('-').unwrap_or(&count);
                        if count.is_empty() {
                            result.push('*');
                        } else {
                            result.push('{');
                            if count.starts_with(',') {
                                result.push('0');
                            }
                            result.push_str(count);
                            result.push('}');
                        }
                        if lazy {
                            result.push('?');
                        }
                    }
                    '<' => result.push_str(r"\b{start}"),
                    '>' => result.push_str(r"\b{end}"),
                    '_' => match characters.next() {
                        Some('.') => result.push_str("(?s:.)"),
                        _ => {
                            return Err("only \\_. is supported for newline-inclusive atoms".into());
                        }
                    },
                    '@' | '%' | 'z' | 'M' | 'V' => {
                        return Err(format!("unsupported Vim regex escape: \\{next}"));
                    }
                    _ => {
                        result.push('\\');
                        result.push(next);
                    }
                }
            }
        } else if character == '[' {
            in_class = true;
            result.push(character);
        } else if character == ']' && in_class {
            in_class = false;
            result.push(character);
        } else if !in_class
            && !very_magic
            && matches!(character, '(' | ')' | '+' | '?' | '|' | '{' | '}')
        {
            result.push('\\');
            result.push(character);
        } else {
            result.push(character);
        }
    }
    Ok(result)
}

fn replacement_parts(
    text: &str,
    delimiter: char,
    captures: usize,
) -> Result<Vec<ReplacementPart>, String> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        let capture = match character {
            '&' => Some(0),
            '\\' => {
                let next = characters.next().ok_or("incomplete replacement escape")?;
                if next == delimiter {
                    literal.push(next);
                    None
                } else if let Some(capture) = next.to_digit(10) {
                    Some(capture as usize)
                } else {
                    literal.push(match next {
                        'r' | 'n' => '\n',
                        't' => '\t',
                        '\\' | '&' => next,
                        _ => return Err(format!("unsupported replacement escape: \\{next}")),
                    });
                    None
                }
            }
            _ => {
                literal.push(character);
                None
            }
        };
        if let Some(capture) = capture {
            if capture >= captures {
                return Err(format!("capture \\{capture} does not exist"));
            }
            if !literal.is_empty() {
                parts.push(ReplacementPart::Literal(std::mem::take(&mut literal)));
            }
            parts.push(ReplacementPart::Capture(capture));
        }
    }
    if !literal.is_empty() {
        parts.push(ReplacementPart::Literal(literal));
    }
    Ok(parts)
}

fn expand_replacement(parts: &[ReplacementPart], captures: &Captures<'_>) -> String {
    let mut result = String::new();
    for part in parts {
        match part {
            ReplacementPart::Literal(text) => result.push_str(text),
            ReplacementPart::Capture(index) => {
                if let Some(matched) = captures.get(*index) {
                    result.push_str(matched.as_str());
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::InputAction;
    use redox_core::EditorSession;

    fn state_with_text(text: &str) -> EditorState {
        let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        *state.session.active_buffer_mut() = TextBuffer::from_text(text);
        state
    }

    #[test]
    fn substitution_supports_vim_patterns_captures_and_flags() {
        let _lock = super::super::global_test_state_lock().lock().unwrap();
        for (text, command, expected) in [
            ("foo foo\nfoo foo\n", "s/foo/bar/g", "bar bar\nbar bar\n"),
            ("foo foo\nfoo foo\n", "s/foo/bar/", "bar foo\nbar foo\n"),
            ("one\ntwo\n", r"s/\(.*\)/[\1]/g", "[one]\n[two]\n"),
            (
                "雪42 雪7",
                r"s/雪\(\d\+\)/number=\1/g",
                "number=42 number=7",
            ),
            ("foo BAR Foo", r"s/\(foo\|bar\)/<&>/gi", "<foo> <BAR> <Foo>"),
            ("foo FOO", "s/foo/bar/g", "bar FOO"),
            ("a/b a/b", r"s/a\/b/c\/d/g", "c/d c/d"),
            ("a/b a/b", r"substitute#a/b#c/d#g", "c/d c/d"),
            ("a", r"s/a/\&\\\0 $1/", "&\\a $1"),
            ("a a", "s/a//g", " "),
            ("a", "s/a/  ", "  "),
            ("a\nb\n", "s/^/>/g", ">a\n>b\n"),
            ("abc", "s/./&-/g", "a-b-c-"),
            ("a+b (a)", "s/(a)/x/g", "a+b x"),
            ("aaa aa a", r"s/a\{2,3}/X/g", "X X a"),
            ("ab abb a", r"s/\v(ab+)/[\1]/g", "[ab] [abb] a"),
            ("(+) ?", "s/[()+?]/x/g", "xxx x"),
            ("foo food afoo", r"s/\<foo\>/x/g", "x food afoo"),
            ("a\nb", r"s/a\nb/joined/g", "joined"),
            ("a", r"%s/a/x\ry/g", "x\ny"),
            (
                "foo\nfoo foo\nfoo\nfoo\n",
                "2,3s/foo/bar/g",
                "foo\nbar bar\nbar\nfoo\n",
            ),
            ("foo\nfoo\nfoo", "2s/foo/bar/", "foo\nbar\nfoo"),
            (
                "foo\nfoo\nfoo",
                " 1, 2 substitute#foo#bar#g",
                "bar\nbar\nfoo",
            ),
            ("foo\nfoo", "0s/foo/bar/", "bar\nfoo"),
            ("foo\nfoo", "2s/foo//", "foo\n"),
            ("雪\n雪\n雪", "2,3s/雪/猫/g", "雪\n猫\n猫"),
            ("foo", "s/missing/bar/gc", "foo"),
        ] {
            let mut state = state_with_text(text);
            state.apply_input(InputAction::RunCommand(command.into()), 80, 24);
            assert_eq!(
                state.mode,
                EditorMode::Normal,
                "{command}: {:?}",
                state.status_msg
            );
            assert_eq!(
                state.session.active_buffer().to_string(),
                expected,
                "{command}"
            );
            state.undo_active(80, 23);
            assert_eq!(
                state.session.active_buffer().to_string(),
                text,
                "undo {command}"
            );
        }
        for command in [
            "s/foo/bar/I",
            "s/foo/bar/giI",
            "3,2s/foo/bar/g",
            "1,9s/foo/bar/g",
            "1,,2s/foo/bar/g",
            "999999999999999999999999s/foo/bar/g",
        ] {
            let mut state = state_with_text("foo\nfoo");
            state.apply_input(InputAction::RunCommand(command.into()), 80, 24);
            assert!(
                state.substitute_preview().unwrap().error.is_some(),
                "{command}"
            );
            assert_eq!(state.session.active_buffer().to_string(), "foo\nfoo");
            assert!(!state.session.active_meta().dirty);
        }
        for (command, expected) in [
            ("s/foo/bar/g", "bar\nfoo\nfoo"),
            ("%s/foo/bar/g", "bar\nbar\nbar"),
            ("2s/foo/bar/g", "foo\nbar\nfoo"),
        ] {
            let mut state = state_with_text("foo\nfoo\nfoo");
            state.mode = EditorMode::VisualLine;
            state
                .views
                .entry(state.session.active_id())
                .or_default()
                .visual_anchor = Some(Pos::zero());
            state.apply_input(InputAction::RunCommand(command.into()), 80, 24);
            assert_eq!(
                state.session.active_buffer().to_string(),
                expected,
                "{command}"
            );
        }
    }
}
