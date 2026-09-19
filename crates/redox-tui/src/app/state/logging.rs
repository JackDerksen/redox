//! Optional local logging history, stored in bounded files for individual editor sessions.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use minui::Event;
use serde_json::{Value, json};

use super::{EditorMode, EditorState};
use crate::config::LoggingConfig;
use crate::input::{InputAction, macro_key_label};

#[derive(Debug)]
pub(super) struct EventLog {
    root: PathBuf,
    max_events: usize,
    // A separate lock survives atomic replacement of the session's log file.
    session_lock: tempfile::NamedTempFile,
    error: Option<String>,
    failed: bool,
}

impl EventLog {
    fn open(root: PathBuf, max_events: usize) -> io::Result<Self> {
        if max_events == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "max_events must be positive",
            ));
        }
        private_directory(&root.join("recent"))?;
        let session_lock = tempfile::Builder::new()
            .prefix(&format!("session-{}-{}-", timestamp(), std::process::id()))
            .suffix(".lock")
            .tempfile_in(root.join("recent"))?;
        session_lock.as_file().lock()?;
        let log = Self {
            root,
            max_events,
            session_lock,
            error: None,
            failed: false,
        };
        let _lock = log.lock()?;
        log.trim(max_events)?;
        Ok(log)
    }

    fn session_path(&self) -> PathBuf {
        self.session_lock.path().with_extension("jsonl")
    }

    // Open a fresh lock handle per operation so dropping it releases the lock on every path.
    fn lock(&self) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(self.root.join(".lock"))?;
        lock.lock()?;
        Ok(lock)
    }

    fn trim(&self, limit: usize) -> io::Result<()> {
        let current = self.session_path();
        let mut sessions = Vec::new();
        for entry in fs::read_dir(self.root.join("recent"))? {
            let path = entry?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            let contents = fs::read(&path)?;
            let complete = recent_events(&contents, usize::MAX);
            let mut count = complete.split_inclusive(|byte| *byte == b'\n').count();
            if path == current {
                let retained = recent_events(&contents, limit);
                if retained.len() != contents.len() {
                    write_history(&path, retained)?;
                    count = retained.split_inclusive(|byte| *byte == b'\n').count();
                }
            }
            sessions.push((path, count));
        }
        sessions.sort_by(|(left, _), (right, _)| left.cmp(right));
        let mut total: usize = sessions.iter().map(|(_, count)| count).sum();
        for (path, count) in sessions {
            if total <= limit {
                break;
            }
            if path == current {
                continue;
            }
            let lock_path = path.with_extension("lock");
            // A crashed process leaves an unlocked file; a running editor keeps its lock.
            let session_lock = match OpenOptions::new().read(true).write(true).open(&lock_path) {
                Ok(lock) => match lock.try_lock() {
                    Ok(()) => Some(lock),
                    Err(std::fs::TryLockError::WouldBlock) => continue,
                    Err(std::fs::TryLockError::Error(error)) => return Err(error),
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            fs::remove_file(path)?;
            total -= count;
            drop(session_lock);
            match fs::remove_file(lock_path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn append(&self, event: &Value) -> io::Result<()> {
        let _lock = self.lock()?;
        let path = self.session_path();
        let contents = match fs::read(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        let complete = recent_events(&contents, usize::MAX);
        let entry = serde_json::to_string(event)? + "\n";
        let count = complete.split_inclusive(|byte| *byte == b'\n').count();
        if count >= self.max_events || complete.len() != contents.len() {
            let mut updated = complete.to_vec();
            updated.extend_from_slice(entry.as_bytes());
            write_history(&path, recent_events(&updated, self.max_events))?;
        } else {
            let mut options = OpenOptions::new();
            options.append(true).create(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(path)?.write_all(entry.as_bytes())?;
        }
        self.trim(self.max_events)
    }

    fn preserve(&self, note: &str, context: Value) -> io::Result<PathBuf> {
        let _lock = self.lock()?;
        self.trim(self.max_events)?;
        let reports = self.root.join("reports");
        private_directory(&reports)?;
        // NamedTempFile creates a new private file exclusively, even for simultaneous reports.
        let mut report = tempfile::Builder::new()
            .prefix(&format!("report-{}-", timestamp()))
            .suffix(".jsonl")
            .tempfile_in(reports)?;
        serde_json::to_writer(
            &mut report,
            &json!({
                "note": note, "time_ms": timestamp(), "format": 1,
                "version": env!("CARGO_PKG_VERSION"), "context": context,
                "session": self.session_path().file_name().map(|name| name.to_string_lossy()),
            }),
        )?;
        report.write_all(b"\n")?;
        match File::open(self.session_path()) {
            Ok(mut session) => {
                io::copy(&mut session, &mut report)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        report.flush()?;
        report.as_file().sync_all()?;
        let (_, path) = report.keep().map_err(|error| error.error)?;
        Ok(path)
    }
}

fn private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

// Keep complete events only, including after a failed append or interrupted write.
fn recent_events(contents: &[u8], limit: usize) -> &[u8] {
    let end = contents
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let start = contents[..end]
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, byte)| **byte == b'\n')
        .nth(limit)
        .map_or(0, |(index, _)| index + 1);
    &contents[start..end]
}

fn write_history(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    temporary.write_all(contents)?;
    temporary.flush()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

impl EditorState {
    pub(crate) fn configure_logging(&mut self, config: LoggingConfig) -> io::Result<()> {
        if !config.enabled {
            self.event_log = None;
        } else if let Some(log) = &mut self.event_log {
            let _lock = log.lock()?;
            log.trim(config.max_events)?;
            log.max_events = config.max_events;
        } else {
            self.event_log = Some(EventLog::open(
                crate::storage::state_root().join("logs"),
                config.max_events,
            )?);
            self.log_event(
                "session",
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
                    "directory": self.session.launch_dir(),
                }),
            );
        }
        Ok(())
    }

    fn log_context(&self) -> Value {
        let cursor = self.active_cursor_pos();
        json!({
            "mode": format!("{:?}", self.mode),
            "buffer": self.session.active_id().get(),
            "path": self.session.active_meta().path,
            "cursor": [cursor.line + 1, cursor.col + 1],
            "pane": self.active_pane.0, "panes": self.panes.len(),
        })
    }

    pub(crate) fn log_event(&mut self, kind: &str, details: Value) {
        if self.event_log.is_none() {
            return;
        }
        let context = self.log_context();
        let log = self.event_log.as_mut().unwrap();
        let mut event = json!({
            "time_ms": timestamp(),
            "event": kind, "context": context,
        });
        if !details.is_null() {
            event["details"] = details;
        }
        if let Some(key) = &self.log_key {
            event["key"] = json!(key);
        }
        match log.append(&event) {
            Ok(()) => log.failed = false,
            Err(error) => {
                if !log.failed {
                    log.error = Some(format!("logging failed: {error}"));
                }
                log.failed = true;
            }
        }
    }

    pub(crate) fn begin_log_input(&mut self, event: &Event) {
        if self.event_log.is_none() {
            return;
        }
        self.log_key = match event {
            Event::Paste(_) => Some("<Paste>".into()),
            _ => Some(macro_key_label(event)).filter(|key| !key.is_empty()),
        };
    }

    pub(crate) fn finish_log_input(&mut self) {
        self.log_key = None;
        self.report_logging_error();
    }

    pub(crate) fn report_logging_error(&mut self) {
        if let Some(error) = self.event_log.as_mut().and_then(|log| log.error.take()) {
            self.set_status(error);
        }
    }

    pub(super) fn log_action(&mut self, action: &InputAction) {
        if self.event_log.is_none() {
            return;
        }
        match action {
            InputAction::InsertChar(_)
            | InputAction::Backspace
            | InputAction::Enter
            | InputAction::CommandChar(_)
            | InputAction::CommandBackspace
            | InputAction::SearchChar(_)
            | InputAction::SearchBackspace
            | InputAction::FinderChar(_)
            | InputAction::FinderBackspace => return,
            InputAction::CompletionAccept if !self.has_visible_completion_popup() => return,
            InputAction::SnippetNext if !self.has_active_snippet() => return,
            InputAction::None
                if !matches!(
                    self.mode,
                    EditorMode::Normal
                        | EditorMode::Visual
                        | EditorMode::VisualLine
                        | EditorMode::VisualBlock
                ) || self.log_key.is_none() =>
            {
                return;
            }
            InputAction::Paste(_)
                if matches!(
                    self.mode,
                    EditorMode::Command | EditorMode::Search | EditorMode::Finder
                ) =>
            {
                return;
            }
            _ => {}
        }
        let description = match action {
            InputAction::Paste(_) => "Paste (text omitted)".to_string(),
            InputAction::PasteSystemClipboardText(_) => {
                "PasteSystemClipboard (text omitted)".to_string()
            }
            InputAction::ApplyRecordedInsert { .. } => {
                "ApplyRecordedInsert (text omitted)".to_string()
            }
            // Command text is recorded only when the command is actually executed.
            InputAction::RunCommand(_) => "RunCommand".to_string(),
            InputAction::ReplaySequence(_) => "ReplaySequence".to_string(),
            _ => format!("{action:?}"),
        };
        self.log_event("action", json!(description));
    }

    pub(super) fn command_log(&mut self, argument: &str) {
        let Some(log) = &self.event_log else {
            self.set_status("logging is disabled; set [logging] enabled = true in your config");
            return;
        };
        let note = if argument.starts_with('"') {
            match serde_json::from_str::<String>(argument) {
                Ok(note) => note,
                Err(_) => {
                    self.set_status("usage: log \"Describe what happened\"");
                    return;
                }
            }
        } else {
            argument.to_string()
        };
        if note.trim().is_empty() {
            self.set_status("usage: log \"Describe what happened\"");
            return;
        }
        match log.preserve(&note, self.log_context()) {
            Ok(path) => self.set_status(format!("log saved: {}", path.display())),
            Err(error) => self.set_status(format!("could not preserve log: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_core::EditorSession;

    fn recent(log: &EventLog) -> Vec<Value> {
        let _lock = log.lock().unwrap();
        fs::read_to_string(log.session_path())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn session_files_are_bounded_and_active_sessions_and_reports_survive_pruning() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("logs");
        let first = EventLog::open(root.clone(), 129).unwrap();
        let first_path = first.session_path();
        for index in 0..260 {
            first
                .append(&json!({"index": index, "source": "first"}))
                .unwrap();
        }
        let history = recent(&first);
        assert_eq!(history.len(), 129);
        assert_eq!(history.first().unwrap()["index"], 131);
        assert_eq!(history.last().unwrap()["index"], 259);
        let second = EventLog::open(root.clone(), 129).unwrap();
        assert_ne!(first_path, second.session_path());
        assert!(first_path.exists());
        std::thread::scope(|scope| {
            for (log, source) in [(&first, "first"), (&second, "second")] {
                scope.spawn(move || {
                    for index in 260..400 {
                        log.append(&json!({"index": index, "source": source}))
                            .unwrap();
                    }
                });
            }
        });
        for (log, source) in [(&first, "first"), (&second, "second")] {
            let history = recent(log);
            assert_eq!(history.len(), 129);
            assert!(history.iter().all(|event| event["source"] == source));
        }
        let note = "Unexpected split\nwith \"quotes\" and café";
        let report = second.preserve(note, Value::Null).unwrap();
        let preserved = fs::read(&report).unwrap();
        let report_lines = String::from_utf8(preserved.clone()).unwrap();
        let header: Value = serde_json::from_str(report_lines.lines().next().unwrap()).unwrap();
        assert_eq!(header["note"], note);
        assert_eq!(report_lines.lines().count(), 130);
        for event in report_lines.lines().skip(1) {
            assert_eq!(
                serde_json::from_str::<Value>(event).unwrap()["source"],
                "second"
            );
        }
        assert_ne!(report, second.preserve(note, Value::Null).unwrap());

        drop(first);
        second.append(&json!({"last": true})).unwrap();
        assert!(
            !first_path.exists(),
            "closed sessions are pruned as whole files"
        );
        assert_eq!(fs::read(&report).unwrap(), preserved);
        let second_path = second.session_path();
        drop(second);
        let smaller = EventLog::open(root.clone(), 1).unwrap();
        assert!(!second_path.exists());
        smaller.append(&json!({"last": true})).unwrap();
        smaller.append(&json!({"last": false})).unwrap();
        assert_eq!(recent(&smaller), vec![json!({"last": false})]);
        OpenOptions::new()
            .append(true)
            .open(smaller.session_path())
            .unwrap()
            .write_all(b"{\"incomplete\":\"\xff")
            .unwrap();
        smaller.append(&json!({"recovered": true})).unwrap();
        assert_eq!(recent(&smaller), vec![json!({"recovered": true})]);
        assert_eq!(fs::read(report).unwrap(), preserved);
        // A crashed session's leftover lock is unlocked and must not prevent pruning.
        let stale = root.join("recent/session-0000-crashed.jsonl");
        fs::write(&stale, b"{}\n\xff").unwrap();
        fs::write(stale.with_extension("lock"), "").unwrap();
        smaller.append(&json!({"last": true})).unwrap();
        assert!(!stale.exists());
        assert!(!stale.with_extension("lock").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(smaller.session_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn editor_logging_omits_typing_and_drafts_but_keeps_actions_commands_and_notes() {
        let _guard = super::super::global_test_state_lock().lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut state = EditorState::new(EditorSession::open_initial_unnamed().unwrap());
        let mut clipboard = None;
        assert!(state.event_log.is_none());
        state.command_log("disabled");
        assert!(state.status_msg.as_ref().unwrap().contains("disabled"));
        state.event_log = Some(EventLog::open(directory.path().join("logs"), 5_000).unwrap());
        for character in "iprivate-buffer-text".chars() {
            crate::handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        crate::handle_editor_event(&mut state, &mut clipboard, Event::Enter);
        crate::handle_editor_event(&mut state, &mut clipboard, Event::Escape);
        for character in ":private-command-draft".chars() {
            crate::handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        crate::handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::Paste("private-paste-draft".into()),
        );
        crate::handle_editor_event(&mut state, &mut clipboard, Event::Escape);
        for character in "/private-search-draft".chars() {
            crate::handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        crate::handle_editor_event(&mut state, &mut clipboard, Event::Escape);
        for character in "/submitted-search".chars() {
            crate::handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        crate::handle_editor_event(&mut state, &mut clipboard, Event::Enter);
        for character in "lh:zen".chars() {
            crate::handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        crate::handle_editor_event(&mut state, &mut clipboard, Event::Enter);
        state.apply_input(InputAction::SplitVertical, 80, 24);
        for character in ":log \"This bug just happened\"".chars() {
            crate::handle_editor_event(&mut state, &mut clipboard, Event::Character(character));
        }
        crate::handle_editor_event(&mut state, &mut clipboard, Event::Enter);
        assert!(state.status_msg.as_ref().unwrap().starts_with("log saved:"));
        let history = recent(state.event_log.as_ref().unwrap());
        let text = serde_json::to_string(&history).unwrap();
        assert!(!text.contains("private-"));
        assert!(!text.contains("InsertChar"));
        assert!(!text.contains("CommandChar"));
        assert!(history.iter().any(|event| event["event"] == "search"
            && event["details"] == "submitted-search"
            && event["key"] == "<Enter>"));
        assert!(!history.iter().any(|event| event["event"] == "action"
            && event["key"] == "<Enter>"
            && event["context"]["mode"] == "Insert"));
        assert!(history.iter().any(|event| event["event"] == "command"
            && event["details"] == "zen"
            && event["key"] == "<Enter>"));
        assert!(history.iter().any(|event| {
            event["key"] == "h"
                && event["details"]
                    .as_str()
                    .is_some_and(|action| action.starts_with("Motion"))
        }));
        assert!(
            history
                .iter()
                .any(|event| event["event"] == "split_created")
        );
        let root = directory.path().join("logs");
        fs::rename(root.join("recent"), root.join("unavailable-history")).unwrap();
        fs::write(root.join("recent"), "not a directory").unwrap();
        assert!(crate::handle_editor_event(
            &mut state,
            &mut clipboard,
            Event::Character('h')
        ));
        assert!(
            state
                .status_msg
                .as_ref()
                .unwrap()
                .starts_with("logging failed:")
        );
        state.command_log("Could not write a log");
        assert!(
            state
                .status_msg
                .as_ref()
                .unwrap()
                .starts_with("could not preserve log:")
        );
        state.configure_logging(LoggingConfig::default()).unwrap();
        assert!(state.event_log.is_none());
    }
}
