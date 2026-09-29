//! One editor-owned terminal, kept alive while its bottom panel is hidden.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use anyhow::Context;
use crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use minui::{Color, ColorPair, Event, KeyKind, KeybindAction, Style, TerminalWindow, Window};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};

use crate::ui::UiStyle;
use crate::ui::style::dim_foreground_color;

const SCROLLBACK_LINES: usize = 5_000;
const INPUT_PREFIX: &str = "terminal-input:";

#[derive(Default)]
pub(crate) struct TerminalPanel {
    visible: bool,
    focused: bool,
    session: Option<TerminalSession>,
    origin: u16,
    width: u16,
    window_height: u16,
    preferred_height: Option<u16>,
    minimum_editor_height: u16,
}

impl std::fmt::Debug for TerminalPanel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalPanel")
            .field("visible", &self.visible)
            .field("focused", &self.focused)
            .field("started", &self.session.is_some())
            .finish()
    }
}

impl TerminalPanel {
    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = self.visible && focused;
    }

    pub(crate) fn status_message(&self) -> Option<&str> {
        self.session
            .as_ref()
            .filter(|_| self.visible)?
            .message
            .as_deref()
    }

    pub(crate) fn is_focused(&self) -> bool {
        self.visible && self.focused
    }

    pub(crate) fn height(&self, available: u16) -> u16 {
        if self.visible {
            let minimum = 3.min(available.saturating_sub(1));
            let maximum = available
                .saturating_sub(self.minimum_editor_height.max(4))
                .max(minimum);
            self.preferred_height
                .unwrap_or(available / 3)
                .clamp(minimum, maximum)
        } else {
            0
        }
    }

    pub(crate) fn separator_contains(&self, column: u16, row: u16) -> bool {
        let height = self.height(self.window_height);
        height > 0 && self.contains_column(column) && row == self.window_height - height - 1
    }

    fn contains_column(&self, column: u16) -> bool {
        column >= self.origin && column < self.origin.saturating_add(self.width)
    }

    pub(crate) fn resize_height(&mut self, amount: i16) {
        let requested = i32::from(self.height(self.window_height)) + i32::from(amount);
        self.set_height(requested.clamp(0, i32::from(u16::MAX)) as u16);
    }

    pub(crate) fn move_separator(&mut self, row: u16) {
        self.set_height(self.window_height.saturating_sub(row.saturating_add(1)));
    }

    fn set_height(&mut self, height: u16) {
        if self.visible {
            self.preferred_height = Some(height);
            self.preferred_height = Some(self.height(self.window_height));
        }
    }

    pub(crate) fn toggle(&mut self, directory: &Path) -> anyhow::Result<()> {
        if !self.visible && self.session.as_ref().is_none_or(|session| session.exited) {
            let program = std::env::var_os("SHELL")
                .filter(|program| !program.is_empty())
                .unwrap_or_else(|| "/bin/sh".into());
            self.session = Some(TerminalSession::spawn(
                terminal_command(Path::new(&program), directory),
                self.width.max(1),
                (self.window_height / 3).max(1),
            )?);
        }
        self.visible = !self.visible;
        self.focused = self.visible;
        Ok(())
    }

    pub(crate) fn handle_event(
        &mut self,
        event: &Event,
        directory: &Path,
        scroll_lines: isize,
    ) -> anyhow::Result<bool> {
        if is_toggle(event) {
            self.toggle(directory)?;
            return Ok(true);
        }
        if !self.visible {
            return Ok(false);
        }
        let top = self
            .window_height
            .saturating_sub(self.height(self.window_height));
        let mouse_position = match event {
            Event::MouseClick { x, y, .. }
            | Event::MouseDrag { x, y, .. }
            | Event::MouseRelease { x, y, .. }
            | Event::MouseMove { x, y, .. }
            | Event::MouseScroll { x, y, .. }
            | Event::MouseScrollHorizontal { x, y, .. } => Some((*x, *y)),
            _ => None,
        };
        if let Some((column, row)) = mouse_position {
            if !self.contains_column(column) || row >= self.window_height {
                return Ok(true);
            }
            if matches!(
                event,
                Event::MouseClick {
                    button: minui::MouseButton::Left,
                    ..
                }
            ) {
                self.focused = row >= top;
            }
            if row >= top {
                if matches!(event, Event::MouseScroll { .. }) {
                    self.scroll(-scroll_lines);
                }
                return Ok(true);
            }
            return Ok(false);
        }
        if !self.is_focused() {
            return Ok(false);
        }
        if let Event::Keybind(KeybindAction::Custom(action)) = event {
            match action.as_str() {
                "terminal-page-up" => {
                    self.scroll(self.height(self.window_height) as isize);
                    return Ok(true);
                }
                "terminal-page-down" => {
                    self.scroll(-(self.height(self.window_height) as isize));
                    return Ok(true);
                }
                _ => {}
            }
        }
        let Some(session) = &mut self.session else {
            return Ok(true);
        };
        if session.exited {
            return Ok(true);
        }
        let bytes = encode_event(event, session.parser.screen());
        if !bytes.is_empty() {
            session.parser.screen_mut().set_scrollback(0);
            session.send(bytes)?;
        }
        Ok(true)
    }

    fn scroll(&mut self, lines: isize) {
        if let Some(session) = &mut self.session {
            let screen = session.parser.screen_mut();
            screen.set_scrollback(screen.scrollback().saturating_add_signed(lines));
        }
    }

    pub(crate) fn poll(&mut self) -> bool {
        self.session.as_mut().is_some_and(TerminalSession::poll) && self.visible
    }

    pub(crate) fn needs_polling(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| !session.exited || !session.output_closed)
    }

    pub(crate) fn layout(
        &mut self,
        origin: u16,
        width: u16,
        height: u16,
        minimum_editor_height: u16,
    ) -> anyhow::Result<()> {
        self.origin = origin;
        self.width = width;
        self.window_height = height;
        self.minimum_editor_height = minimum_editor_height;
        let rows = self.height(height);
        if rows > 0
            && width > 0
            && let Some(session) = &mut self.session
        {
            session.resize(width, rows)?;
        }
        Ok(())
    }

    pub(crate) fn draw(
        &self,
        window: &mut dyn Window,
        style: UiStyle,
        dimming: f32,
    ) -> minui::Result<()> {
        let height = self.height(self.window_height);
        if height == 0 || self.width == 0 {
            return Ok(());
        }
        let top = self.window_height - height;
        let Some(session) = &self.session else {
            return Ok(());
        };
        let screen = session.parser.screen();
        let mut text = String::with_capacity(self.width as usize);
        for row in 0..height {
            let mut start_column = 0;
            let mut run_style = None;
            text.clear();
            for column in 0..self.width {
                let Some(cell) = screen.cell(row, column) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut foreground = terminal_color(cell.fgcolor(), style.theme.white);
                let mut background = terminal_color(cell.bgcolor(), style.theme.bg);
                if style.text_formatting && cell.inverse() {
                    std::mem::swap(&mut foreground, &mut background);
                }
                if dimming > 0.0 {
                    foreground = dim_foreground_color(
                        terminal_rgb(foreground),
                        terminal_rgb(background),
                        dimming,
                    );
                }
                let mut cell_style = Style::from(ColorPair::new(foreground, background));
                if style.text_formatting && cell.bold() {
                    cell_style = cell_style.bold();
                }
                if style.text_formatting && cell.dim() {
                    cell_style = cell_style.dim();
                }
                if style.text_formatting && cell.italic() {
                    cell_style = cell_style.italic();
                }
                if style.text_formatting && cell.underline() {
                    cell_style = cell_style.underlined();
                }
                if let Some(previous) = run_style
                    && previous != cell_style
                {
                    window.write_str_styled(top + row, start_column, &text, previous)?;
                    text.clear();
                }
                if text.is_empty() {
                    start_column = column;
                    run_style = Some(cell_style);
                }
                let contents = cell.contents();
                text.push_str(if contents.is_empty() { " " } else { contents });
            }
            if let Some(style) = run_style {
                window.write_str_styled(top + row, start_column, &text, style)?;
            }
        }
        if self.focused {
            let (row, column) = screen.cursor_position();
            window.request_cursor(minui::window::CursorSpec {
                x: column.min(self.width - 1),
                y: top + row.min(height - 1),
                visible: !screen.hide_cursor() && screen.scrollback() == 0 && !session.exited,
            });
        }
        Ok(())
    }

    // MinUI's key model omits Home/End/PageUp/PageDown and resolves editor bindings
    // before exposing keys. Read full crossterm keys only while the terminal has focus.
    pub(crate) fn read_input(
        &self,
        window: &mut TerminalWindow,
        timeout: Option<Duration>,
    ) -> minui::Result<Option<Event>> {
        if let Some(timeout) = timeout
            && !event::poll(timeout)?
        {
            return Ok(None);
        }
        let input = match event::read()? {
            event::Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.modifiers == KeyModifiers::CONTROL
                    && matches!(key.code, KeyCode::Char('`' | ' ' | '\0'))
                {
                    Event::KeyWithModifiers(minui::KeyWithModifiers {
                        key: KeyKind::Char('`'),
                        mods: minui::KeyModifiers::ctrl(),
                    })
                } else if key.modifiers == KeyModifiers::CONTROL
                    && let KeyCode::Char(character @ ('j' | 'k' | 'J' | 'K')) = key.code
                {
                    Event::KeyWithModifiers(minui::KeyWithModifiers {
                        key: KeyKind::Char(character.to_ascii_lowercase()),
                        mods: minui::KeyModifiers::ctrl(),
                    })
                } else if key.modifiers == KeyModifiers::CONTROL
                    && matches!(
                        key.code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                    )
                {
                    minui::input::KeyboardHandler::new().process_key_event(key)
                } else if key.modifiers == KeyModifiers::SHIFT
                    && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
                {
                    Event::Keybind(KeybindAction::Custom(
                        if key.code == KeyCode::PageUp {
                            "terminal-page-up"
                        } else {
                            "terminal-page-down"
                        }
                        .into(),
                    ))
                } else {
                    let application_cursor = self
                        .session
                        .as_ref()
                        .is_some_and(|session| session.parser.screen().application_cursor());
                    Event::Keybind(KeybindAction::Custom(format!(
                        "{INPUT_PREFIX}{}",
                        encode_key(key, application_cursor)
                    )))
                }
            }
            event::Event::Paste(text) => Event::Paste(text),
            event::Event::Mouse(mouse) => window.mouse_mut().process_mouse_event(mouse),
            event::Event::Resize(width, height) => {
                window.handle_resize(width, height);
                Event::Resize { width, height }
            }
            _ => Event::Unknown,
        };
        Ok(Some(input))
    }
}

fn is_toggle(event: &Event) -> bool {
    matches!(event, Event::KeyWithModifiers(key)
        if key.mods.ctrl && !key.mods.alt && !key.mods.shift && !key.mods.super_key
        && matches!(key.key, KeyKind::Char('`' | ' ' | '\0')))
        || matches!(event, Event::Character('\0'))
}

fn terminal_color(color: vt100::Color, default: Color) -> Color {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(index) => Color::AnsiValue(index),
        vt100::Color::Rgb(red, green, blue) => Color::rgb(red, green, blue),
    }
}

fn terminal_rgb(color: Color) -> Color {
    match color {
        Color::AnsiValue(index) => {
            let (red, green, blue) = minui::term::capabilities::ansi256_to_rgb(index);
            Color::rgb(red, green, blue)
        }
        other => other,
    }
}

enum TerminalOutput {
    Bytes(Vec<u8>),
    Error(io::Error),
    Closed,
}

struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    input: Option<SyncSender<Vec<u8>>>,
    pending_input: Vec<u8>,
    output: Receiver<TerminalOutput>,
    parser: vt100::Parser<TerminalReplies>,
    exited: bool,
    output_closed: bool,
    message: Option<String>,
}

impl TerminalSession {
    fn spawn(command: CommandBuilder, columns: u16, rows: u16) -> anyhow::Result<Self> {
        let pair = portable_pty::native_pty_system()
            .openpty(pty_size(columns, rows))
            .context("could not open pseudo-terminal")?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let child = pair
            .slave
            .spawn_command(command)
            .context("could not start terminal")?;
        drop(pair.slave);
        let (output_sender, output) = mpsc::sync_channel(64);
        let (input, input_receiver) = mpsc::sync_channel::<Vec<u8>>(32);
        let session = Self {
            master: pair.master,
            child,
            input: Some(input),
            pending_input: Vec::new(),
            output,
            parser: vt100::Parser::new_with_callbacks(
                rows,
                columns,
                SCROLLBACK_LINES,
                TerminalReplies::default(),
            ),
            exited: false,
            output_closed: false,
            message: None,
        };
        let writer_output = output_sender.clone();
        std::thread::Builder::new()
            .name("redox-terminal-input".into())
            .spawn(move || {
                for bytes in input_receiver {
                    if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                        let _ = writer_output.send(TerminalOutput::Error(error));
                        break;
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("redox-terminal-output".into())
            .spawn(move || {
                let mut buffer = [0; 4096];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(length) => {
                            if output_sender
                                .send(TerminalOutput::Bytes(buffer[..length].to_vec()))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        // Linux PTYs report EIO after the last slave closes.
                        Err(error) if cfg!(unix) && error.raw_os_error() == Some(5) => break,
                        Err(error) => {
                            let _ = output_sender.send(TerminalOutput::Error(error));
                            break;
                        }
                    }
                }
                let _ = output_sender.send(TerminalOutput::Closed);
            })?;
        Ok(session)
    }

    fn send(&mut self, bytes: Vec<u8>) -> anyhow::Result<()> {
        // Keep queued typing intact when the writer is busy. Reject an oversized
        // paste as a whole instead of sending a partial command.
        anyhow::ensure!(
            self.pending_input.len() + bytes.len() <= 1024 * 1024,
            "terminal input exceeds the 1 MiB pending limit"
        );
        self.pending_input.extend(bytes);
        self.flush_input()
    }

    fn flush_input(&mut self) -> anyhow::Result<()> {
        if self.pending_input.is_empty() {
            return Ok(());
        }
        let input = self.input.as_ref().context("terminal input is closed")?;
        match input.try_send(std::mem::take(&mut self.pending_input)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(bytes)) => {
                self.pending_input = bytes;
                Ok(())
            }
            Err(mpsc::TrySendError::Disconnected(_)) => anyhow::bail!("terminal input is closed"),
        }
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        // Bound work per frame so a noisy command cannot starve editor input.
        for message in self.output.try_iter().take(64) {
            changed = true;
            match message {
                TerminalOutput::Bytes(bytes) => self.parser.process(&bytes),
                TerminalOutput::Error(error) => {
                    self.message = Some(format!("terminal I/O failed: {error}"))
                }
                TerminalOutput::Closed => self.output_closed = true,
            }
        }
        if let Err(error) = self.flush_input() {
            self.message = Some(error.to_string());
            self.pending_input.clear();
            changed = true;
        }
        let replies = std::mem::take(&mut self.parser.callbacks_mut().bytes);
        if !replies.is_empty()
            && let Err(error) = self.send(replies)
        {
            self.message = Some(error.to_string());
            changed = true;
        }
        if !self.exited {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.exited = true;
                    self.input = None;
                    self.message = Some(format!("terminal exited: {status}; reopen to restart"));
                    changed = true;
                }
                Ok(None) => {}
                Err(error) => {
                    let message = format!("could not check terminal: {error}");
                    changed |= self.message.as_ref() != Some(&message);
                    self.message = Some(message);
                }
            }
        }
        changed
    }

    fn resize(&mut self, columns: u16, rows: u16) -> anyhow::Result<()> {
        if self.parser.screen().size() != (rows, columns) {
            if !self.exited {
                self.master.resize(pty_size(columns, rows))?;
            }
            self.parser.screen_mut().set_size(rows, columns);
        }
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.input = None;
        // A foreground job has its own process group, separate from the command process.
        #[cfg(unix)]
        if let Some(group) = self
            .master
            .process_group_leader()
            .and_then(rustix::process::Pid::from_raw)
            && Some(group.as_raw_nonzero().get() as u32) != self.child.process_id()
        {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
        if !self.exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn pty_size(columns: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols: columns,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn terminal_command(program: &Path, directory: &Path) -> CommandBuilder {
    let mut command = CommandBuilder::new(program);
    command.arg("-i");
    command.cwd(directory);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command
}

#[derive(Default)]
struct TerminalReplies {
    bytes: Vec<u8>,
}

impl vt100::Callbacks for TerminalReplies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        first: Option<u8>,
        second: Option<u8>,
        parameters: &[&[u16]],
        final_character: char,
    ) {
        if first.is_some() || second.is_some() {
            return;
        }
        let reply = match (final_character, parameters) {
            ('n', [parameter]) if *parameter == [5] => "\x1b[0n".to_owned(),
            ('n', [parameter]) if *parameter == [6] => {
                let (row, column) = screen.cursor_position();
                format!("\x1b[{};{}R", row + 1, column + 1)
            }
            ('c', []) => "\x1b[?1;2c".to_owned(),
            ('c', [parameter]) if *parameter == [0] => "\x1b[?1;2c".to_owned(),
            _ => return,
        };
        self.bytes.extend_from_slice(reply.as_bytes());
    }
}

fn encode_event(event: &Event, screen: &vt100::Screen) -> Vec<u8> {
    match event {
        Event::Keybind(KeybindAction::Custom(action)) => action
            .strip_prefix(INPUT_PREFIX)
            .unwrap_or("")
            .as_bytes()
            .to_vec(),
        Event::Paste(text) => {
            if screen.bracketed_paste() {
                format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes()
            } else {
                text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
            }
        }
        Event::KeyWithModifiers(key) => {
            let code = match key.key {
                KeyKind::Char(character) => KeyCode::Char(character),
                KeyKind::Up => KeyCode::Up,
                KeyKind::Down => KeyCode::Down,
                KeyKind::Left => KeyCode::Left,
                KeyKind::Right => KeyCode::Right,
                KeyKind::Enter => KeyCode::Enter,
                KeyKind::Escape => KeyCode::Esc,
                KeyKind::Tab => KeyCode::Tab,
                KeyKind::Backspace => KeyCode::Backspace,
                KeyKind::Delete => KeyCode::Delete,
                KeyKind::Function(number) => KeyCode::F(number),
                KeyKind::CapsLock => return Vec::new(),
            };
            let mut modifiers = KeyModifiers::empty();
            modifiers.set(KeyModifiers::CONTROL, key.mods.ctrl);
            modifiers.set(KeyModifiers::ALT, key.mods.alt);
            modifiers.set(KeyModifiers::SHIFT, key.mods.shift);
            modifiers.set(KeyModifiers::SUPER, key.mods.super_key);
            encode_key(KeyEvent::new(code, modifiers), screen.application_cursor()).into_bytes()
        }
        Event::Character(character) => character.to_string().into_bytes(),
        Event::Enter => b"\r".to_vec(),
        Event::Tab => b"\t".to_vec(),
        Event::Backspace => b"\x7f".to_vec(),
        Event::Escape => b"\x1b".to_vec(),
        _ => Vec::new(),
    }
}

fn encode_key(key: KeyEvent, application_cursor: bool) -> String {
    let modifiers = key.modifiers;
    if modifiers.intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META) {
        return String::new();
    }
    let parameter = 1
        + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL));
    let cursor_key = match key.code {
        KeyCode::Up => Some('A'),
        KeyCode::Down => Some('B'),
        KeyCode::Right => Some('C'),
        KeyCode::Left => Some('D'),
        KeyCode::Home => Some('H'),
        KeyCode::End => Some('F'),
        _ => None,
    };
    if let Some(final_character) = cursor_key {
        return if parameter > 1 {
            format!("\x1b[1;{parameter}{final_character}")
        } else {
            format!(
                "\x1b{}{final_character}",
                if application_cursor { 'O' } else { '[' }
            )
        };
    }
    let numeric_key = match key.code {
        KeyCode::Insert => Some(2),
        KeyCode::Delete => Some(3),
        KeyCode::PageUp => Some(5),
        KeyCode::PageDown => Some(6),
        KeyCode::F(number @ 5..=12) => {
            Some([15, 17, 18, 19, 20, 21, 23, 24][usize::from(number - 5)])
        }
        _ => None,
    };
    if let Some(number) = numeric_key {
        return if parameter > 1 {
            format!("\x1b[{number};{parameter}~")
        } else {
            format!("\x1b[{number}~")
        };
    }
    if let KeyCode::F(number @ 1..=4) = key.code {
        let final_character = char::from(b'P' + number - 1);
        return if parameter > 1 {
            format!("\x1b[1;{parameter}{final_character}")
        } else {
            format!("\x1bO{final_character}")
        };
    }
    let text = match key.code {
        KeyCode::Char(character) if modifiers.contains(KeyModifiers::CONTROL) => {
            let character = character.to_ascii_uppercase();
            match character {
                '@'..='_' => char::from(character as u8 & 0x1f).to_string(),
                ' ' | '2' | '`' => "\0".into(),
                '3' => "\x1b".into(),
                '4' => "\x1c".into(),
                '5' => "\x1d".into(),
                '6' => "\x1e".into(),
                '7' | '/' => "\x1f".into(),
                '8' | '?' => "\x7f".into(),
                _ => return String::new(),
            }
        }
        KeyCode::Char(character) => {
            #[cfg(not(windows))]
            if !modifiers.contains(KeyModifiers::ALT)
                && modifiers.contains(KeyModifiers::SHIFT)
                    != key.state.contains(event::KeyEventState::CAPS_LOCK)
                && character.is_lowercase()
            {
                let mut uppercase = character.to_uppercase();
                if let Some(character) = uppercase.next()
                    && uppercase.next().is_none()
                {
                    return character.to_string();
                }
            }
            character.to_string()
        }
        KeyCode::Enter => "\r".into(),
        KeyCode::Esc => "\x1b".into(),
        KeyCode::Backspace if modifiers.contains(KeyModifiers::CONTROL) => "\x08".into(),
        KeyCode::Backspace => "\x7f".into(),
        KeyCode::BackTab => "\x1b[Z".into(),
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => "\x1b[Z".into(),
        KeyCode::Tab => "\t".into(),
        _ => return String::new(),
    };
    if modifiers.contains(KeyModifiers::ALT) {
        format!("\x1b{text}")
    } else {
        text
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Instant;

    fn configured_command(program: &str, directory: &Path) -> CommandBuilder {
        // Exercise normal rc loading without running the developer's startup files.
        let startup = r#"PS1='__PROMPT__ '
HISTFILE="$HOME/history"
HISTSIZE=1000
SAVEHIST=1000
alias redox_config_check="printf '__%s__\\n' CONFIG"
"#;
        std::fs::write(directory.join(".bashrc"), startup).unwrap();
        std::fs::write(directory.join(".zshrc"), startup).unwrap();
        let mut command = terminal_command(Path::new(program), directory);
        command.env("HOME", directory);
        command.env("ZDOTDIR", directory);
        command.env("INPUTRC", "/dev/null");
        command.env_remove("ENV");
        command.env_remove("BASH_ENV");
        command
    }

    pub(crate) fn configured_panel(directory: &Path) -> TerminalPanel {
        TerminalPanel {
            session: Some(
                TerminalSession::spawn(configured_command("/bin/bash", directory), 100, 10)
                    .unwrap(),
            ),
            ..TerminalPanel::default()
        }
    }

    fn wait_for(session: &mut TerminalSession, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            session.poll();
            if session.parser.screen().contents().contains(expected) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "missing {expected:?}: {} ({:?})",
                session.parser.screen().contents(),
                session.message
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn terminal_keys_paste_and_replies_follow_modes() {
        for (code, modifiers, expected) in [
            (KeyCode::Char('c'), KeyModifiers::CONTROL, "\x03"),
            (KeyCode::Char('d'), KeyModifiers::CONTROL, "\x04"),
            (KeyCode::Char('r'), KeyModifiers::CONTROL, "\x12"),
            (KeyCode::Char('é'), KeyModifiers::NONE, "é"),
            (KeyCode::Char('a'), KeyModifiers::SHIFT, "A"),
            (KeyCode::Char('b'), KeyModifiers::ALT, "\x1bb"),
            (KeyCode::Home, KeyModifiers::NONE, "\x1b[H"),
            (KeyCode::End, KeyModifiers::NONE, "\x1b[F"),
            (KeyCode::PageUp, KeyModifiers::NONE, "\x1b[5~"),
            (KeyCode::Left, KeyModifiers::CONTROL, "\x1b[1;5D"),
            (KeyCode::BackTab, KeyModifiers::SHIFT, "\x1b[Z"),
            (KeyCode::F(5), KeyModifiers::NONE, "\x1b[15~"),
        ] {
            assert_eq!(encode_key(KeyEvent::new(code, modifiers), false), expected);
        }
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true),
            "\x1bOA"
        );
        let mut parser = vt100::Parser::new_with_callbacks(8, 80, 0, TerminalReplies::default());
        parser.process(b"\x1b[?2004h\x1b[2;3H\x1b[6n");
        assert_eq!(parser.callbacks().bytes, b"\x1b[2;3R");
        assert_eq!(
            encode_event(
                &Event::Paste("first\nsecond\x1b[201~".into()),
                parser.screen()
            ),
            b"\x1b[200~first\nsecond[201~\x1b[201~"
        );
    }

    #[test]
    fn terminal_session_retains_cwd_history_and_output_and_cleans_up() {
        let _guard = crate::app::state::global_test_state_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("nested")).unwrap();
        for program in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(program).exists() {
                continue;
            }
            let command = configured_command(program, directory.path());
            let mut session = TerminalSession::spawn(command.clone(), 100, 12).unwrap();
            wait_for(&mut session, "__PROMPT__");
            session.send(b"redox_config_check\r".to_vec()).unwrap();
            wait_for(&mut session, "__CONFIG__");
            session
                .send(b"printf '\\033[2J\\033[H'; printf '__%s__\\n' READY\r".to_vec())
                .unwrap();
            wait_for(&mut session, "__READY__");
            let (blocked_input, blocked_receiver) = mpsc::sync_channel(1);
            let original_input = session.input.replace(blocked_input).unwrap();
            session.send(b"first".to_vec()).unwrap();
            session.send(b"second".to_vec()).unwrap();
            session.send(b"third".to_vec()).unwrap();
            assert_eq!(blocked_receiver.recv().unwrap(), b"first");
            session.flush_input().unwrap();
            assert_eq!(blocked_receiver.recv().unwrap(), b"secondthird");
            assert!(session.pending_input.is_empty());
            session.input = Some(original_input);
            session
                .send(b"printf '__%s__\\n' HISTORY\r".to_vec())
                .unwrap();
            wait_for(&mut session, "__HISTORY__");
            session.send(b"\x1b[A\r".to_vec()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while session
                .parser
                .screen()
                .contents()
                .matches("__HISTORY__")
                .count()
                < 2
            {
                session.poll();
                assert!(
                    Instant::now() < deadline,
                    "history was not recalled: {}",
                    session.parser.screen().contents()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            session
                .send(
                    b"cd nested; printf '__%s__\\n' MOVED; sleep 0.1; printf '__%s__\\n' HIDDEN\r"
                        .to_vec(),
                )
                .unwrap();
            wait_for(&mut session, "__MOVED__");
            let process = session.child.process_id();
            let mut panel = TerminalPanel {
                visible: true,
                focused: true,
                session: Some(session),
                origin: 0,
                width: 100,
                window_height: 39,
                preferred_height: None,
                minimum_editor_height: 4,
            };
            panel.toggle(directory.path()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(!panel.poll(), "hidden output must not request a redraw");
                if panel
                    .session
                    .as_ref()
                    .unwrap()
                    .parser
                    .screen()
                    .contents()
                    .contains("__HIDDEN__")
                {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            panel.toggle(directory.path()).unwrap();
            assert_eq!(panel.session.as_ref().unwrap().child.process_id(), process);
            panel.layout(0, 90, 30, 4).unwrap();
            let session = panel.session.as_mut().unwrap();
            assert_eq!(session.parser.screen().size(), (10, 90));
            session
                .send(
                    b"printf '__%s__\\n' \"${PWD##*/}\"; stty size; printf '__%s__\\n' RESIZED\r"
                        .to_vec(),
                )
                .unwrap();
            wait_for(session, "__RESIZED__");
            assert!(session.parser.screen().contents().contains("__nested__"));
            assert!(session.parser.screen().contents().contains("10 90"));
            session
                .send(b"printf '__%s__\\n' WAITING; sleep 30\r".to_vec())
                .unwrap();
            wait_for(session, "__WAITING__");
            session.send(b"\x03".to_vec()).unwrap();
            session
                .send(b"printf '__%s__\\n' INTERRUPTED\r".to_vec())
                .unwrap();
            wait_for(session, "__INTERRUPTED__");
            session.send(b"exit\r".to_vec()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !session.exited || !session.output_closed {
                session.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                std::fs::read_to_string(directory.path().join("history"))
                    .unwrap()
                    .contains("INTERRUPTED")
            );
            let mut fresh = TerminalSession::spawn(command, 100, 12).unwrap();
            fresh
                .send(b"printf '\\033[2J\\033[H'; printf '__%s__\\n' FRESH\r".to_vec())
                .unwrap();
            wait_for(&mut fresh, "__FRESH__");
            fresh
                .send(b"history 5; printf '__%s__\\n' HISTORY_END\r".to_vec())
                .unwrap();
            wait_for(&mut fresh, "__HISTORY_END__");
            assert!(fresh.parser.screen().contents().contains("INTERRUPTED"));
            fresh.send(b"exit\r".to_vec()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !fresh.exited || !fresh.output_closed {
                fresh.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(panel);
            #[cfg(unix)]
            if let Some(process) =
                process.and_then(|process| rustix::process::Pid::from_raw(process as i32))
            {
                assert!(
                    rustix::process::test_kill_process(process).is_err(),
                    "terminal must be reaped on drop"
                );
            }
        }
    }
}
