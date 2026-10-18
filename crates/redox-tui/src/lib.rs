//! The Redox terminal editor: launch handling, the event loop, editor state and rendering.

mod app;
mod config;
mod crash;
mod event_loop;
mod indentation;
mod input;
mod launch;
mod mode;
mod storage;
mod terminal;
#[cfg(test)]
mod tests;
mod ui;
mod view;

use std::time::Duration;

use redox_core::EditorSession;

use crate::app::EditorState;
use crate::event_loop::{configured_input, run_terminal_session};
use crate::launch::{LaunchTarget, parse_launch_options};

pub(crate) const SOFT_TAB_WIDTH: usize = 4;

const GUTTER_CONTENT_PADDING: u16 = 1;
const ANIMATION_FRAME_RATE_HZ: u64 = 60;
const ANIMATION_FRAME_INTERVAL: Duration =
    Duration::from_nanos(1_000_000_000 / ANIMATION_FRAME_RATE_HZ);

pub fn run() -> anyhow::Result<()> {
    let Some(options) = parse_launch_options()? else {
        return Ok(());
    };
    if let Err(error) = storage::migrate_legacy_state() {
        eprintln!("warning: could not migrate legacy Redox state: {error}");
    }
    let (mut config, _) = options.config.load()?;
    let input = configured_input(&config)?;
    let mut style = config.style()?;
    let mut active_theme = config.theme.clone();
    let mut theme_override = None;
    let launch = options.target;
    let launch_empty = matches!(&launch, LaunchTarget::Empty);
    let launch_explorer_dir = match &launch {
        LaunchTarget::Explorer(dir) => Some(dir.clone()),
        LaunchTarget::Empty | LaunchTarget::File(_) => None,
    };
    let session = match launch {
        LaunchTarget::Empty | LaunchTarget::Explorer(_) => EditorSession::open_initial_unnamed()?,
        LaunchTarget::File(path) => EditorSession::open_initial_file(path)?,
    };

    let mut state = EditorState::new(session);
    let logging_result = state.configure_logging(config.logging);
    state.zen = config.zen;
    state.configure_animations(config.animations);
    state.icons_enabled = config.icons_enabled;
    state.save = config.save;
    state.configure(
        input,
        config.undo_tree_history_size,
        config.scrolloff,
        config.which_key.enabled,
        Duration::from_millis(config.which_key.delay_ms),
    );
    state.configure_command_completions(config.theme_names());
    if let Some(dir_path) = launch_explorer_dir {
        state.open_explorer_at_path(dir_path)?;
    }
    if launch_empty {
        state.open_dashboard();
    }
    if let Err(error) = logging_result {
        state.set_status(format!("could not enable logging: {error}"));
    }

    crash::install_panic_hook();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_terminal_session(
            &mut state,
            &mut config,
            &mut style,
            &mut active_theme,
            &mut theme_override,
            &options.config,
        )
    }));
    // The terminal is restored by now, so deferred panic messages are readable.
    for message in crash::take_deferred_panics() {
        eprintln!("{message}");
    }
    match outcome {
        Ok(result) => result?,
        Err(_) => {
            return Err(crash::crash_report(
                &state,
                &storage::state_root().join("recovery"),
            ));
        }
    }
    state.log_event("session_end", serde_json::Value::Null);
    state.save_previous_session(&storage::session_path(state.session.launch_dir()))?;
    Ok(())
}
