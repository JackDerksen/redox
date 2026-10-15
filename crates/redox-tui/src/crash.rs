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
    let lines = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state
            .session
            .summaries()
            .into_iter()
            .filter(|summary| summary.dirty)
            .filter_map(|summary| {
                let buffer = state.session.buffer(summary.id)?;
                Some(
                    match storage::write_recovery_file(recovery_dir, &summary.display_name, buffer)
                    {
                        Ok(path) => format!("  {} -> {}", summary.display_name, path.display()),
                        Err(error) => {
                            format!("  {} could not be saved: {error}", summary.display_name)
                        }
                    },
                )
            })
            .collect::<Vec<_>>()
    }));
    match lines {
        Ok(lines) if lines.is_empty() => {
            anyhow::anyhow!("redox crashed; no unsaved changes were lost")
        }
        Ok(lines) => anyhow::anyhow!(
            "redox crashed; unsaved buffers were written to:\n{}",
            lines.join("\n")
        ),
        Err(_) => anyhow::anyhow!("redox crashed and could not save unsaved buffers"),
    }
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

        let clean = EditorState::new(EditorSession::open_initial_file(&path).unwrap());
        let report = crash_report(&clean, &directory.path().join("unused")).to_string();
        assert_eq!(report, "redox crashed; no unsaved changes were lost");
    }
}
