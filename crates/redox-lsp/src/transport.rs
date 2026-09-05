use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread;

use serde_json::{Value, json};
use thiserror::Error;

use crate::protocol::Range;
use crate::workspace::file_uri;

pub const INITIALIZE_REQUEST_ID: i64 = 1;
const FIRST_DYNAMIC_REQUEST_ID: i64 = 2;
const DEFAULT_MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;
const DEFAULT_EVENT_QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCommand {
    pub label: String,
    pub executable: String,
    pub args: Vec<String>,
}

impl ServerCommand {
    #[must_use]
    pub fn new(label: impl Into<String>, executable: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            executable: executable.into(),
            args: Vec::new(),
        }
    }

    #[must_use]
    pub fn args(mut self, args: &[&str]) -> Self {
        self.args = args
            .iter()
            .map(|argument| (*argument).to_string())
            .collect();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientInfo<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

/// Resource limits for the language-server transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportOptions {
    pub max_message_size: usize,
    pub event_queue_capacity: usize,
}

impl Default for TransportOptions {
    fn default() -> Self {
        Self {
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
            event_queue_capacity: DEFAULT_EVENT_QUEUE_CAPACITY,
        }
    }
}

/// An error while framing or decoding an incoming JSON-RPC message.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("failed to read language-server output: {0}")]
    Io(#[from] io::Error),
    #[error("language-server header is malformed: {0}")]
    MalformedHeader(String),
    #[error("language-server message is missing Content-Length")]
    MissingContentLength,
    #[error("language-server Content-Length is invalid: {0}")]
    InvalidContentLength(String),
    #[error("language-server message is {content_length} bytes; limit is {max_message_size} bytes")]
    MessageTooLarge {
        content_length: usize,
        max_message_size: usize,
    },
    #[error("could not reserve memory for a {content_length}-byte language-server message")]
    AllocationFailed { content_length: usize },
    #[error("language-server message is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

#[derive(Debug)]
pub enum ClientEvent {
    Message(Value),
    Initialized { result: Value },
    InitializationFailed { message: String },
    Terminated { error: Option<TransportError> },
}

fn matches_initialize_request_id(message: &Value) -> bool {
    message.get("method").is_none()
        && message
            .get("id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id == INITIALIZE_REQUEST_ID)
}

/// Builds the initialization parameters used by Redox frontends.
pub fn default_initialize_params(root: &Path, client_info: ClientInfo<'_>) -> io::Result<Value> {
    let root_uri = file_uri(root)?;
    let root_path = root.to_string_lossy().to_string();
    let workspace_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    Ok(json!({
        "processId": std::process::id(),
        "rootPath": root_path,
        "rootUri": root_uri,
        "workspaceFolders": [{
            "uri": root_uri,
            "name": workspace_name
        }],
        "capabilities": {
            "workspace": { "applyEdit": true },
            "textDocument": {
                "publishDiagnostics": {
                    "relatedInformation": true,
                    "versionSupport": true
                },
                "hover": { "contentFormat": ["markdown", "plaintext"] },
                "completion": {
                    "completionItem": { "snippetSupport": true }
                },
                "codeAction": {
                    "codeActionLiteralSupport": {
                        "codeActionKind": {
                            "valueSet": [
                                "quickfix",
                                "refactor",
                                "refactor.extract",
                                "refactor.inline",
                                "refactor.rewrite",
                                "source",
                                "source.organizeImports"
                            ]
                        }
                    }
                }
            }
        },
        "clientInfo": {
            "name": client_info.name,
            "version": client_info.version
        }
    }))
}

/// A stdio JSON-RPC client for one language-server process.
///
/// Sends enqueue messages for a dedicated writer; they never wait for the server
/// to read. A full write queue fails the session instead of losing document
/// updates. Poll `try_recv` for asynchronous write errors and termination.
pub struct Client {
    child: Child,
    outgoing: SyncSender<String>,
    events: Receiver<ReaderEvent>,
    write_error: Option<io::Error>,
    terminated: bool,
    initialized: bool,
    next_request_id: i64,
}

#[derive(Debug)]
enum ReaderEvent {
    Message(Value),
    Terminated(Option<TransportError>),
}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Client")
            .field("initialized", &self.initialized)
            .finish_non_exhaustive()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Client {
    /// Starts a server and sends its `initialize` request.
    pub fn spawn(
        command: &ServerCommand,
        root: &Path,
        client_info: ClientInfo<'_>,
    ) -> io::Result<Self> {
        let initialize_params = default_initialize_params(root, client_info)?;
        Self::spawn_with_initialize_params_and_options(
            command,
            root,
            initialize_params,
            TransportOptions::default(),
        )
    }

    /// Starts a server with caller-supplied resource limits.
    pub fn spawn_with_options(
        command: &ServerCommand,
        root: &Path,
        client_info: ClientInfo<'_>,
        options: TransportOptions,
    ) -> io::Result<Self> {
        let initialize_params = default_initialize_params(root, client_info)?;
        Self::spawn_with_initialize_params_and_options(command, root, initialize_params, options)
    }

    /// Starts a server with caller-supplied LSP initialization parameters.
    pub fn spawn_with_initialize_params(
        command: &ServerCommand,
        root: &Path,
        initialize_params: Value,
    ) -> io::Result<Self> {
        Self::spawn_with_initialize_params_and_options(
            command,
            root,
            initialize_params,
            TransportOptions::default(),
        )
    }

    /// Starts a server with caller-supplied initialization parameters and
    /// transport resource limits.
    pub fn spawn_with_initialize_params_and_options(
        command: &ServerCommand,
        root: &Path,
        initialize_params: Value,
        options: TransportOptions,
    ) -> io::Result<Self> {
        let mut child = Command::new(&command.executable)
            .args(&command.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(root)
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("failed to open LSP stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("failed to open LSP stdout"))?;
        let (event_sender, events) = mpsc::sync_channel(options.event_queue_capacity);
        let (outgoing, writes) = mpsc::sync_channel::<String>(DEFAULT_EVENT_QUEUE_CAPACITY);
        let writer_events = event_sender.clone();
        let writer_thread = thread::Builder::new()
            .name(format!("redox-lsp-write-{}", command.label))
            .spawn(move || {
                for payload in writes {
                    if let Err(error) = write_message(&mut stdin, &payload) {
                        let _ = writer_events.send(ReaderEvent::Terminated(Some(error.into())));
                        break;
                    }
                }
            });
        if let Err(error) = writer_thread {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let reader_thread = thread::Builder::new()
            .name(format!("redox-lsp-{}", command.label))
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_message(&mut reader, options.max_message_size) {
                        Ok(Some(message)) => {
                            if event_sender.send(ReaderEvent::Message(message)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => {
                            let _ = event_sender.send(ReaderEvent::Terminated(None));
                            return;
                        }
                        Err(error) => {
                            let _ = event_sender.send(ReaderEvent::Terminated(Some(error)));
                            return;
                        }
                    }
                }
            });
        if let Err(error) = reader_thread {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }

        let mut client = Self {
            child,
            outgoing,
            events,
            write_error: None,
            terminated: false,
            initialized: false,
            next_request_id: FIRST_DYNAMIC_REQUEST_ID,
        };
        client.send_initialize(initialize_params)?;
        Ok(client)
    }

    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub fn try_recv(&mut self) -> Option<ClientEvent> {
        if self.terminated {
            return None;
        }
        let event = if let Some(error) = self.write_error.take() {
            ReaderEvent::Terminated(Some(error.into()))
        } else {
            match self.events.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => ReaderEvent::Terminated(None),
            }
        };
        match event {
            ReaderEvent::Message(message)
                if !self.initialized && matches_initialize_request_id(&message) =>
            {
                if let Some(error) = message.get("error") {
                    return Some(ClientEvent::InitializationFailed {
                        message: json_rpc_error_message(error),
                    });
                }
                let Some(result) = message.get("result").cloned() else {
                    return Some(ClientEvent::InitializationFailed {
                        message: "initialize response did not contain a result".to_string(),
                    });
                };
                if let Err(error) = self.send_notification("initialized", json!({})) {
                    return Some(ClientEvent::InitializationFailed {
                        message: format!("failed to acknowledge initialization: {error}"),
                    });
                }
                self.initialized = true;
                Some(ClientEvent::Initialized { result })
            }
            ReaderEvent::Message(message) => Some(ClientEvent::Message(message)),
            ReaderEvent::Terminated(error) => {
                self.terminated = true;
                self.initialized = false;
                Some(ClientEvent::Terminated { error })
            }
        }
    }

    pub fn send_response(&mut self, id: Value, result: Value) -> io::Result<()> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        }))
    }

    pub fn send_method_not_found(&mut self, id: Value, method: &str) -> io::Result<()> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("unsupported request: {method}")
            }
        }))
    }

    pub fn send_notification(&mut self, method: &str, params: Value) -> io::Result<()> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
    }

    pub fn send_request(&mut self, method: &str, params: Value) -> io::Result<i64> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params
        }))?;
        Ok(request_id)
    }

    pub fn send_did_open(
        &mut self,
        path: &Path,
        language_id: &str,
        version: i32,
        text: &str,
    ) -> io::Result<()> {
        let uri = file_uri(path)?;
        self.send_notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": version,
                    "text": text,
                }
            }),
        )
    }

    pub fn send_did_change(&mut self, path: &Path, version: i32, text: &str) -> io::Result<()> {
        let uri = file_uri(path)?;
        self.send_notification(
            "textDocument/didChange",
            json!({
                "textDocument": {
                    "uri": uri,
                    "version": version,
                },
                "contentChanges": [{ "text": text }]
            }),
        )
    }

    pub fn send_did_save(&mut self, path: &Path) -> io::Result<()> {
        let uri = file_uri(path)?;
        self.send_notification(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": uri } }),
        )
    }

    pub fn send_did_close(&mut self, path: &Path) -> io::Result<()> {
        let uri = file_uri(path)?;
        self.send_notification(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
    }

    pub fn send_cancel_request(&mut self, id: i64) -> io::Result<()> {
        self.send_notification("$/cancelRequest", json!({ "id": id }))
    }

    pub fn send_goto_definition(
        &mut self,
        path: &Path,
        line: usize,
        character: u32,
    ) -> io::Result<i64> {
        self.send_position_request("textDocument/definition", path, line, character)
    }

    pub fn send_hover(&mut self, path: &Path, line: usize, character: u32) -> io::Result<i64> {
        self.send_position_request("textDocument/hover", path, line, character)
    }

    pub fn send_completion(&mut self, path: &Path, line: usize, character: u32) -> io::Result<i64> {
        let uri = file_uri(path)?;
        self.send_request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "triggerKind": 1 }
            }),
        )
    }

    pub fn send_code_actions(
        &mut self,
        path: &Path,
        range: &Range,
        diagnostics: &[Value],
    ) -> io::Result<i64> {
        let uri = file_uri(path)?;
        self.send_request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri },
                "range": range,
                "context": {
                    "diagnostics": diagnostics,
                    "only": ["quickfix"]
                }
            }),
        )
    }

    pub fn send_execute_command(&mut self, command: &str, arguments: &[Value]) -> io::Result<i64> {
        self.send_request(
            "workspace/executeCommand",
            json!({ "command": command, "arguments": arguments }),
        )
    }

    fn send_initialize(&mut self, params: Value) -> io::Result<()> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": INITIALIZE_REQUEST_ID,
            "method": "initialize",
            "params": params
        }))
    }

    fn send_position_request(
        &mut self,
        method: &str,
        path: &Path,
        line: usize,
        character: u32,
    ) -> io::Result<i64> {
        let uri = file_uri(path)?;
        self.send_request(
            method,
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }),
        )
    }

    fn write(&mut self, message: &Value) -> io::Result<()> {
        if self.terminated || self.write_error.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "language-server transport stopped",
            ));
        }
        let error = match self.outgoing.try_send(message.to_string()) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Full(_)) => io::Error::new(
                io::ErrorKind::WouldBlock,
                "language-server write queue is full",
            ),
            Err(TrySendError::Disconnected(_)) => {
                io::Error::new(io::ErrorKind::BrokenPipe, "language-server writer stopped")
            }
        };
        // A rejected message would desynchronise documents. Fail the session
        // instead of dropping a notification or blocking the frontend.
        self.write_error = Some(io::Error::new(error.kind(), error.to_string()));
        Err(error)
    }
}

fn write_message(stdin: &mut ChildStdin, payload: &str) -> io::Result<()> {
    write!(stdin, "Content-Length: {}\r\n\r\n{payload}", payload.len())?;
    stdin.flush()
}

fn json_rpc_error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("language server rejected initialization")
        .to_string()
}

fn read_message(
    reader: &mut impl BufRead,
    max_message_size: usize,
) -> Result<Option<Value>, TransportError> {
    let mut content_length = None;
    let mut read_header = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            if read_header {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            return Ok(None);
        }
        read_header = true;
        if line == "\r\n" || line == "\n" {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| TransportError::MalformedHeader(line.trim_end().to_string()))?;
        if name.eq_ignore_ascii_case("Content-Length") {
            let value = value.trim();
            content_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| TransportError::InvalidContentLength(value.to_string()))?,
            );
        }
    }

    let content_length = content_length.ok_or(TransportError::MissingContentLength)?;
    if content_length > max_message_size {
        return Err(TransportError::MessageTooLarge {
            content_length,
            max_message_size,
        });
    }
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(content_length)
        .map_err(|_| TransportError::AllocationFailed { content_length })?;
    payload.resize(content_length, 0);
    reader.read_exact(&mut payload)?;
    Ok(Some(serde_json::from_slice(&payload)?))
}
