//! Keep panics readable and unsaved work recoverable.
//!
//! While the editor owns the terminal, a panic message printed by the default hook
//! lands in the alternate screen and disappears when the terminal is restored. The
//! hook defers those messages until the terminal is back to normal, and a crash
//! writes every unsaved buffer to the recovery directory.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::app::EditorState;
use crate::storage;

static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);
static DEFERRED_PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());
const MAX_DEFERRED_PANICS: usize = 8;

/// Defer panic messages from any thread while a [`PanicDeferral`] is alive.
pub(crate) fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !TERMINAL_ACTIVE.load(Ordering::SeqCst) {
            default_hook(info);
            return;
        }
        let thread = std::thread::current();
        let mut message = format!("thread '{}' {info}", thread.name().unwrap_or("<unnamed>"));
        let backtrace = std::backtrace::Backtrace::capture();
        if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            message.push_str(&format!("\nstack backtrace:\n{backtrace}"));
        }
        if let Ok(mut panics) = DEFERRED_PANICS.lock()
            && panics.len() < MAX_DEFERRED_PANICS
        {
            panics.push(message);
        }
    }));
}

/// Marks the terminal as owned by the editor. Create it before the terminal guards
/// so it is dropped after them, once the terminal has been restored.
pub(crate) struct PanicDeferral;

impl PanicDeferral {
    pub(crate) fn start() -> Self {
        TERMINAL_ACTIVE.store(true, Ordering::SeqCst);
        Self
    }
}

impl Drop for PanicDeferral {
    fn drop(&mut self) {
        TERMINAL_ACTIVE.store(false, Ordering::SeqCst);
    }
}

pub(crate) fn take_deferred_panics() -> Vec<String> {
    DEFERRED_PANICS
        .lock()
        .map(|mut panics| std::mem::take(&mut *panics))
        .unwrap_or_default()
}

/// Write every unsaved buffer to `recovery_dir` and describe what was saved.
pub(crate) fn crash_report(state: &EditorState, recovery_dir: &Path) -> anyhow::Error {
    // The state may be mid-update; never let recovery itself abort the report.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut saved = Vec::new();
        let mut failed = Vec::new();
        for summary in state.session.summaries() {
            let Some(buffer) = state.session.buffer(summary.id).filter(|_| summary.dirty) else {
                continue;
            };
            match storage::write_recovery_file(recovery_dir, &summary.display_name, buffer) {
                Ok(path) => saved.push(format!("  {} -> {}", summary.display_name, path.display())),
                Err(error) => failed.push(format!("  {}: {error}", summary.display_name)),
            }
        }
        (saved, failed)
    }));
    let Ok((saved, failed)) = outcome else {
        return anyhow::anyhow!("redox crashed and could not save unsaved buffers");
    };
    let mut report = String::from("redox crashed; ");
    match (saved.is_empty(), failed.is_empty()) {
        (true, true) => report.push_str("no unsaved changes were lost"),
        (false, _) => {
            report.push_str("unsaved buffers were written to:\n");
            report.push_str(&saved.join("\n"));
        }
        (true, false) => report.push_str("unsaved buffers could not be saved:"),
    }
    if !failed.is_empty() {
        if !saved.is_empty() {
            report.push_str("\nthese could not be saved:");
        }
        report.push('\n');
        report.push_str(&failed.join("\n"));
    }
    anyhow::anyhow!(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_core::EditorSession;

    #[test]
    fn panics_are_deferred_while_the_terminal_is_owned() {
        install_panic_hook();
        let deferral = PanicDeferral::start();
        let worker = std::thread::Builder::new()
            .name("deferred-panic-test".to_string())
            .spawn(|| panic!("worker exploded"))
            .unwrap();
        assert!(worker.join().is_err());
        drop(deferral);

        let messages = take_deferred_panics();
        assert!(
            messages.iter().any(|message| message
                .contains("thread 'deferred-panic-test' panicked at")
                && message.contains("worker exploded")),
            "{messages:?}"
        );
        assert!(!TERMINAL_ACTIVE.load(Ordering::SeqCst));
    }

    #[test]
    fn crash_report_writes_unsaved_buffers_to_the_recovery_directory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("draft.txt");
        std::fs::write(&path, "saved\n").unwrap();
        let mut state = EditorState::new(EditorSession::open_initial_file(&path).unwrap());
        state
            .session
            .active_buffer_mut()
            .insert(redox_core::Pos::zero(), "unsaved ");
        state.session.recompute_active_dirty();

        let recovery = directory.path().join("recovery");
        let report = crash_report(&state, &recovery).to_string();

        assert!(
            report.contains("unsaved buffers were written to"),
            "{report}"
        );
        let written: Vec<_> = std::fs::read_dir(&recovery)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(written.len(), 1);
        assert!(
            report.contains(&written[0].display().to_string()),
            "{report}"
        );
        assert_eq!(
            std::fs::read_to_string(&written[0]).unwrap(),
            "unsaved saved\n"
        );

        // A recovery location that cannot be created must not claim success.
        let blocked = directory.path().join("blocked");
        std::fs::write(&blocked, "").unwrap();
        let report = crash_report(&state, &blocked).to_string();
        assert!(
            report.starts_with("redox crashed; unsaved buffers could not be saved:\n"),
            "{report}"
        );
        assert!(report.contains("draft.txt: "), "{report}");
        assert!(!report.contains("written to"), "{report}");

        let clean = EditorState::new(EditorSession::open_initial_file(&path).unwrap());
        let report = crash_report(&clean, &directory.path().join("unused")).to_string();
        assert_eq!(report, "redox crashed; no unsaved changes were lost");
    }
}
