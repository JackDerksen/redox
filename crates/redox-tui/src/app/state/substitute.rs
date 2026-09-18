use std::sync::mpsc::{self, Receiver, TryRecvError};

use redox_core::{BufferId, BufferKind, Edit, Pos, Selection, TextBuffer, VisualModeKind};
use regex::{Captures, Regex, RegexBuilder};

use super::{BufferViewState, EditorMode, EditorState, SearchMatch};
use crate::ui::STATUS_BAR_HEIGHT_ROWS;
use crate::ui::syntax::{SyntaxHighlighter, SyntaxLanguage, SyntaxParser, language_for_path};

const SUBSTITUTE_WORKER_MIN_BYTES: usize = 64 * 1024;

#[derive(Debug, Default)]
pub(super) struct SubstituteState {
    scope: Option<CommandScope>,
    preview: Option<SubstitutePreview>,
    worker: Option<Receiver<SubstitutePreview>>,
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
                    let language = language_for_path(self.session.active_meta().path.as_deref());
                    if preview.replacing
                        && self.session.active_buffer().len_bytes() >= SUBSTITUTE_WORKER_MIN_BYTES
                    {
                        let mut pending = SubstitutePreview::new(
                            buffer_id,
                            preview.version,
                            preview.command.clone(),
                        );
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
        self.push_command_history(std::mem::take(&mut preview.command));
        self.mode = EditorMode::Normal;
        self.command_line.clear();
        self.command_line_cursor = 0;
        self.reset_command_history_navigation();
        self.substitution = SubstituteState::default();
        self.set_status(format!(
            "{count} substitution{}",
            if count == 1 { "" } else { "s" }
        ));
        true
    }
}

fn substitute_body(command: &str) -> Option<(char, &str)> {
    let command = command.trim_start();
    let command = command.strip_prefix('%').unwrap_or(command);
    let body = command
        .strip_prefix("substitute")
        .or_else(|| command.strip_prefix('s'))?;
    let delimiter = body.chars().next()?;
    (delimiter.is_ascii_punctuation() && delimiter != '\\')
        .then_some((delimiter, &body[delimiter.len_utf8()..]))
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
        let (delimiter, body) = substitute_body(command).ok_or("usage: s/pattern/replacement/g")?;
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
        let mut ignore_case = false;
        for flag in flags.trim().chars() {
            match flag {
                'g' => global = true,
                'i' => ignore_case = true,
                'I' => ignore_case = false,
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
        })
    }

    fn plan(
        &self,
        buffer: &TextBuffer,
        selection: Option<(Selection, VisualModeKind)>,
        language: Option<SyntaxLanguage>,
        preview: &mut SubstitutePreview,
    ) {
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
                    if replacement != matched.as_str() {
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
            ("foo FOO", "s/foo/bar/giI", "bar FOO"),
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
    }
}
