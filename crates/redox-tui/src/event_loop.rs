//! The terminal session: reading input, dispatching it to the editor, and runtime config reloads.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use minui::KeybindAction;
use minui::input::{Clipboard, KeyboardHandler};
use minui::prelude::{
    input::{Event, KeyKind},
    render::{TerminalWindow, Window},
};

use crate::app::{EditorState, FramePerfSample};
use crate::input::{
    ConfiguredBinding, ConfiguredBindingTarget, InputAction, InputState, map_event_with_context,
};
use crate::launch::LaunchConfig;
use crate::ui::UiStyle;
use crate::view::draw_buffer_view;

pub(crate) fn is_cancel_event(event: &Event) -> bool {
    matches!(event, Event::Escape)
        || matches!(
            event,
            Event::KeyWithModifiers(key)
                if matches!(key.key, KeyKind::Escape)
                    && !key.mods.ctrl
                    && !key.mods.alt
                    && !key.mods.super_key
        )
        || matches!(
            event,
            Event::KeyWithModifiers(key)
                if key.mods.ctrl
                    && !key.mods.alt
                    && !key.mods.super_key
                    && matches!(key.key, KeyKind::Char('c') | KeyKind::Char('C'))
        )
}

pub(crate) fn handle_editor_event(
    state: &mut EditorState,
    clipboard: &mut Option<Clipboard>,
    event: Event,
) -> bool {
    if matches!(event, Event::Unknown) {
        return !state.should_quit;
    }
    if state.handle_pane_resize_mouse(&event) {
        return !state.should_quit;
    }
    if let Event::KeyWithModifiers(key) = &event
        && key.mods == minui::KeyModifiers::ctrl()
    {
        let direction = match key.key {
            KeyKind::Left if !state.terminal.is_focused() => {
                Some(crate::app::state::SplitDirection::Left)
            }
            KeyKind::Right if !state.terminal.is_focused() => {
                Some(crate::app::state::SplitDirection::Right)
            }
            KeyKind::Up => Some(crate::app::state::SplitDirection::Up),
            KeyKind::Down => Some(crate::app::state::SplitDirection::Down),
            _ => None,
        };
        if let Some(direction) = direction {
            state.resize_active_pane(direction);
            state.input.reset_prefixes();
            return !state.should_quit;
        }
    }
    if state.terminal.is_visible()
        && let Event::KeyWithModifiers(key) = &event
        && key.mods == minui::KeyModifiers::ctrl()
        && let KeyKind::Char(character @ ('j' | 'k' | 'J' | 'K')) = key.key
    {
        state.focus_split(if character.eq_ignore_ascii_case(&'j') {
            crate::app::state::SplitDirection::Down
        } else {
            crate::app::state::SplitDirection::Up
        });
        state.input.reset_prefixes();
        state.request_redraw();
        return !state.should_quit;
    }
    // Terminal input belongs to the terminal, including passwords and control keys.
    // Keep it outside editor logging, mappings, macros and popup dispatch.
    let scroll_lines = match event {
        Event::MouseScroll { delta, .. } => state.mouse.scroll_distance(false, delta),
        _ => 0,
    };
    match state
        .terminal
        .handle_event(&event, state.session.launch_dir(), scroll_lines)
    {
        Ok(true) => {
            state.input.reset_prefixes();
            state.request_redraw();
            return !state.should_quit;
        }
        Err(error) => {
            state.set_status(format!("terminal: {error:#}"));
            return !state.should_quit;
        }
        Ok(false) => {}
    }
    state.begin_log_input(&event);
    let keep_running = handle_editor_event_inner(state, clipboard, event);
    state.finish_log_input();
    keep_running
}

pub(crate) fn handle_editor_event_inner(
    state: &mut EditorState,
    clipboard: &mut Option<Clipboard>,
    event: Event,
) -> bool {
    if !state.has_substitute_confirmation() && state.handle_mouse_input(&event) {
        if let Some(key) = state.mouse.pending_key.take() {
            return handle_editor_event_inner(state, clipboard, key);
        }
        return !state.should_quit;
    }
    if state.handle_dashboard_event(&event) {
        state.log_event("dashboard_input", serde_json::Value::Null);
        return !state.should_quit;
    }
    if state.rain_is_active() {
        state.log_event("rain_input", serde_json::Value::Null);
        let enter_command = crate::input::macro_key_label(&event) == ":";
        if is_cancel_event(&event) || enter_command {
            state.stop_rain_animation();
        }
        if !enter_command {
            return !state.should_quit;
        }
    }

    if is_cancel_event(&event) && state.handle_normal_mode_escape_on_surface() {
        state.log_event("surface_dismissed", serde_json::Value::Null);
        return !state.should_quit;
    }

    if is_cancel_event(&event) && state.dismiss_perf_popup() {
        state.log_event("perf_dismissed", serde_json::Value::Null);
        return !state.should_quit;
    }

    let confirm_explorer_delete = state.has_pending_explorer_delete_confirmation();
    let action = match &event {
        Event::Paste(text) => InputAction::Paste(text.clone()),
        _ => map_event_with_context(
            &mut state.input,
            state.mode,
            confirm_explorer_delete,
            &event,
        ),
    };

    let (w, h) = state.viewport_size();
    if !matches!(
        action,
        InputAction::ToggleMacroRecording | InputAction::StartMacroRecording { .. }
    ) {
        state.record_macro_key(&crate::input::macro_key_label(&event));
    }
    match action {
        InputAction::PasteSystemClipboard => match clipboard.as_mut() {
            Some(system_clipboard) => match system_clipboard.paste() {
                Ok(text) => state.apply_input(InputAction::PasteSystemClipboardText(text), w, h),
                Err(error) => state.set_status(format!("clipboard paste failed: {error}")),
            },
            None => state.set_status("system clipboard unavailable"),
        },
        action => state.apply_input(action, w, h),
    }
    if let Some(text) = state.take_pending_system_clipboard() {
        match clipboard.as_mut() {
            Some(system_clipboard) => {
                if let Err(error) = system_clipboard.copy(&text) {
                    state.set_status(format!("clipboard copy failed: {error}"));
                } else {
                    state.confirm_yank("yanked to system clipboard");
                }
            }
            None => {
                state.set_status("system clipboard unavailable");
            }
        }
    }

    !state.should_quit
}

pub(crate) fn install_keyboard_bindings(
    keyboard: &mut KeyboardHandler,
    input: &InputState,
) -> minui::Result<()> {
    keyboard.clear_keybinds();

    let symbol_info = KeybindAction::Custom("trigger-symbol-info".to_string());
    keyboard.add_keybind("ctrl-i", symbol_info)?;
    let completion = KeybindAction::Custom("trigger-completion".to_string());
    keyboard.add_keybind("ctrl-shift-k", completion)?;

    for key in input.special_keys() {
        let binding = key
            .strip_prefix('<')
            .and_then(|key| key.strip_suffix('>'))
            .expect("configured special keys are normalized");
        keyboard.add_keybind(binding, KeybindAction::Custom(format!("redox-key:{key}")))?;
    }
    Ok(())
}

pub(crate) fn configured_input(config: &crate::config::Config) -> anyhow::Result<InputState> {
    let mut input = InputState::new();
    input.configure(config.leader(), &config.keybindings)?;
    let custom_bindings = config
        .bind
        .iter()
        .filter(|binding| !binding.keys.is_empty())
        .map(|binding| {
            let target = match (&binding.sequence, &binding.command) {
                (Some(sequence), None) => ConfiguredBindingTarget::Sequence(sequence.clone()),
                (None, Some(command)) => ConfiguredBindingTarget::Command(command.clone()),
                (Some(_), Some(_)) => anyhow::bail!(
                    "custom binding {:?} must set exactly one of sequence or command",
                    binding.keys
                ),
                (None, None) => anyhow::bail!(
                    "custom binding {:?} must set either sequence or command",
                    binding.keys
                ),
            };
            Ok(ConfiguredBinding {
                mode: binding.mode.clone(),
                keys: binding.keys.clone(),
                target,
                description: binding.desc.clone(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    input.configure_custom_bindings(&custom_bindings)?;
    install_keyboard_bindings(&mut KeyboardHandler::new(), &input)?;
    Ok(input)
}

pub(crate) fn open_runtime_config(
    state: &mut EditorState,
    explicit_path: Option<&std::path::Path>,
) {
    if !state.take_config_open_request() {
        return;
    }

    let path = crate::config::Config::path(explicit_path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        state.set_status(format!(
            "cannot create configuration directory {}: {error}",
            parent.display()
        ));
        return;
    }
    state.open_config_file(&path);
}

pub(crate) fn reload_runtime_config(
    state: &mut EditorState,
    keyboard: &mut KeyboardHandler,
    style: &mut UiStyle,
    active_config: &mut crate::config::Config,
    active_theme: &mut String,
    theme_override: &mut Option<String>,
    launch_config: &LaunchConfig,
) {
    if !state.take_config_reload_request() {
        return;
    }

    let result = (|| -> anyhow::Result<Option<PathBuf>> {
        let (candidate, loaded_path) = launch_config.load()?;
        let candidate_theme = theme_override
            .as_ref()
            .filter(|name| candidate.has_theme(name))
            .cloned()
            .unwrap_or_else(|| candidate.theme.clone());
        let candidate_style = candidate.style_for_theme(&candidate_theme)?;
        let candidate_input = configured_input(&candidate)?;
        install_keyboard_bindings(keyboard, &candidate_input)?;
        state.configure(
            candidate_input,
            candidate.undo_tree_history_size,
            candidate.scrolloff,
            candidate.which_key.enabled,
            Duration::from_millis(candidate.which_key.delay_ms),
        );
        let enabled = if candidate.zen.enabled != active_config.zen.enabled {
            candidate.zen.enabled
        } else {
            state.zen.enabled
        };
        state.zen = crate::config::ZenConfig {
            enabled,
            ..candidate.zen
        };
        state.configure_command_completions(candidate.theme_names());
        if candidate.check_updates != active_config.check_updates {
            state.configure_update_checks(candidate.check_updates);
        }
        state.configure_logging(candidate.logging)?;
        state.configure_animations(candidate.animations);
        state.icons_enabled = candidate.icons_enabled;
        state.save = candidate.save;
        *active_config = candidate;
        *style = candidate_style;
        *active_theme = candidate_theme;
        if theme_override
            .as_ref()
            .is_some_and(|name| !active_config.has_theme(name))
        {
            *theme_override = None;
        }
        Ok(loaded_path)
    })();

    match result {
        Ok(loaded_path) => match loaded_path {
            Some(path) => state.set_status(format!("configuration reloaded: {}", path.display())),
            None if launch_config.default_only => {
                state.set_status("configuration reloaded with built-in defaults (--default-config)")
            }
            None => state
                .set_status("configuration reloaded with built-in defaults (no config file found)"),
        },
        Err(error) => state.set_status(format!("configuration reload failed: {error:#}")),
    }
}

pub(crate) fn apply_runtime_colorscheme(
    state: &mut EditorState,
    style: &mut UiStyle,
    config: &crate::config::Config,
    active_theme: &mut String,
    theme_override: &mut Option<String>,
) {
    let Some(name) = state.take_colorscheme_request() else {
        return;
    };
    if name.is_empty() {
        state.set_status(format!("colorscheme: {active_theme}"));
        return;
    }
    match config.style_for_theme(&name) {
        Ok(candidate) => {
            *style = candidate;
            *active_theme = name.clone();
            *theme_override = Some(name);
            state.set_status(format!("colorscheme set to {active_theme}"));
        }
        Err(error) => state.set_status(error.to_string()),
    }
}

pub(crate) struct KeyboardReporting;

impl KeyboardReporting {
    pub(crate) fn enable() -> std::io::Result<Self> {
        use crossterm::event::{
            EnableFocusChange, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        };
        crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            ),
            EnableFocusChange
        )?;
        Ok(Self)
    }
}

impl Drop for KeyboardReporting {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::DisableFocusChange,
            crossterm::event::PopKeyboardEnhancementFlags
        );
    }
}

pub(crate) fn decode_editor_input(
    state: &mut EditorState,
    keyboard: &KeyboardHandler,
    event: crossterm::event::Event,
) -> Event {
    match event {
        crossterm::event::Event::Key(key) => {
            if let Some(key) = state.input.prepare_key_event(key) {
                keyboard.process_key_event(key)
            } else {
                state.request_redraw();
                Event::Unknown
            }
        }
        crossterm::event::Event::FocusLost => {
            state.input.clear_held_modifiers();
            state.request_redraw();
            Event::Unknown
        }
        crossterm::event::Event::Paste(text) => Event::Paste(text),
        crossterm::event::Event::Resize(width, height) => Event::Resize { width, height },
        _ => Event::Unknown,
    }
}

pub(crate) fn read_editor_input(
    state: &mut EditorState,
    window: &mut TerminalWindow,
    timeout: Option<Duration>,
) -> minui::Result<Option<Event>> {
    if state.terminal.is_focused() {
        state.terminal.read_input(window, &mut state.input, timeout)
    } else {
        if let Some(timeout) = timeout
            && !crossterm::event::poll(timeout)?
        {
            return Ok(None);
        }
        let event = crossterm::event::read()?;
        let event = match event {
            crossterm::event::Event::Mouse(mouse) => window.mouse_mut().process_mouse_event(mouse),
            event => decode_editor_input(state, window.keyboard_mut(), event),
        };
        if let Event::Resize { width, height } = event {
            window.handle_resize(width, height);
        }
        Ok(Some(event))
    }
}

/// Own the terminal for one editing session. Its guards restore the terminal when
/// dropped, including while unwinding from a panic.
pub(crate) fn run_terminal_session(
    state: &mut EditorState,
    config: &mut crate::config::Config,
    style: &mut UiStyle,
    active_theme: &mut String,
    theme_override: &mut Option<String>,
    launch_config: &LaunchConfig,
) -> anyhow::Result<()> {
    let _panic_deferral = crate::crash::PanicDeferral::start();
    let mut window = TerminalWindow::new()?;
    let _keyboard_reporting = KeyboardReporting::enable()?;
    window.mouse_mut().set_movement_tracking(false);
    window.mouse_mut().set_scroll_axis_filtering(false);
    // MinUI reports left as positive; Redox uses positive deltas for down/right.
    window.mouse_mut().set_invert_scroll_horizontal(true);
    window.set_mouse_capture(config.mouse)?;
    state.configure_mouse(
        config.mouse,
        config.mouse_invert_vertical,
        config.mouse_invert_horizontal,
        config.mouse_scroll_step_vertical,
        config.mouse_scroll_step_horizontal,
    );
    install_keyboard_bindings(window.keyboard_mut(), &state.input)?;
    window.set_auto_flush(false);
    let mut clipboard = Clipboard::new().ok();
    state.configure_update_checks(config.check_updates);

    const MAX_EVENTS_PER_FRAME: usize = 256;
    const MAX_INPUT_TIME_PER_FRAME: Duration = Duration::from_millis(8);

    let mut pending_wake_event: Option<Event> = None;
    let mut previous_terminal_size = window.get_size();

    'editor: loop {
        let frame_start = Instant::now();
        let mut perf_sample = FramePerfSample::default();
        let input_start = Instant::now();
        let mut event_count = 0usize;
        let mut mouse_frame_pending = false;

        if let Some(event) = pending_wake_event.take() {
            event_count += 1;
            let previous_context = (state.mode, state.session.active_id());
            mouse_frame_pending =
                crate::input::is_mouse_event(&event) || !state.mouse.popups.is_empty();
            if !handle_editor_event(state, &mut clipboard, event) {
                break 'editor;
            }
            mouse_frame_pending |= previous_context != (state.mode, state.session.active_id())
                || state.has_visible_completion_popup();
        }

        for _ in 0..MAX_EVENTS_PER_FRAME {
            // Leave time to draw even when input arrives faster than it can be handled.
            if mouse_frame_pending || input_start.elapsed() >= MAX_INPUT_TIME_PER_FRAME {
                break;
            }
            match read_editor_input(state, &mut window, Some(Duration::ZERO))? {
                Some(Event::Unknown) => continue,
                Some(event) => {
                    event_count += 1;
                    let previous_context = (state.mode, state.session.active_id());
                    mouse_frame_pending =
                        crate::input::is_mouse_event(&event) || !state.mouse.popups.is_empty();
                    if !handle_editor_event(state, &mut clipboard, event) {
                        break 'editor;
                    }
                    mouse_frame_pending |= previous_context
                        != (state.mode, state.session.active_id())
                        || state.has_visible_completion_popup();
                }
                None => break,
            }
        }
        perf_sample.input = input_start.elapsed();
        perf_sample.event_count = event_count;
        open_runtime_config(state, launch_config.path.as_deref());
        reload_runtime_config(
            state,
            window.keyboard_mut(),
            style,
            config,
            active_theme,
            theme_override,
            launch_config,
        );
        if config.mouse != state.mouse.enabled {
            window.set_mouse_capture(config.mouse)?;
        }
        state.configure_mouse(
            config.mouse,
            config.mouse_invert_vertical,
            config.mouse_invert_horizontal,
            config.mouse_scroll_step_vertical,
            config.mouse_scroll_step_horizontal,
        );
        apply_runtime_colorscheme(state, style, config, active_theme, theme_override);
        if event_count > 0 {
            state.request_redraw();
        }
        let (width, height) = window.get_size();
        if (width, height) != previous_terminal_size {
            previous_terminal_size = (width, height);
            state.log_event("resize", serde_json::json!([width, height]));
            state.request_redraw();
        }
        perf_sample.load = state.update_background(Instant::now());
        state.report_logging_error();

        if state.should_quit {
            break;
        }

        if state.take_redraw_request() {
            window.clear_screen()?;
            draw_buffer_view(state, style, &mut window, &mut perf_sample)?;
            let flush_start = Instant::now();
            window.end_frame()?;
            perf_sample.flush = flush_start.elapsed();
            perf_sample.frame = frame_start.elapsed();
            state.record_perf_sample(perf_sample);
        }

        pending_wake_event = match state.next_wake_deadline(Instant::now()) {
            Some(deadline) => read_editor_input(
                state,
                &mut window,
                Some(deadline.saturating_duration_since(Instant::now())),
            )?,
            None => read_editor_input(state, &mut window, None)?,
        }
        .filter(|event| !matches!(event, Event::Unknown));
    }
    Ok(())
}
