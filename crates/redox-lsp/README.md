# redox-lsp

`redox-lsp` contains Redox's editor-independent Language Server Protocol code.
It can also back another frontend that wants the same stdio transport, built-in
server catalogue, response parsing, snippets, workspace discovery, or linters.

In its current form, it's a solid and reusable LSP toolkit for editors with
similar needs to Redox, but I wouldn't advertise it as a general-purpose LSP
framework quite yet. Future work will be done to make it more flexible, but
I feel like it's important to clarify that first.

The main limitations in v0.1.0 are essentially:
- Custom providers still receive the closed Redox `Language` enum. A provider 
  for Zig, C#, Kotlin, or a private language cannot properly participate in
  that interface.
- The response parsers are deliberately lossy. Definitions retain only the first
  target, unknown completion kinds become "item", disabled actions disappear,
  resource operations are rejected, diagnostic metadata is dropped, and hover
  whitespace is normalized. Those are reasonable Redox adapters, but your
  editor may need the original protocol information.
- `Client` is tied directly to spawning one stdio child process. It discards
  stderr and kills the process on drop instead of performing the LSP shutdown/exit
  sequence.
- Incoming traffic remains raw JSON in `ClientEvent::Message`. Consumers must
  rebuild message classification, response correlation, server-request handling,
  capability tracking, and error decoding themselves.

## Usage

The transport accepts an owned command, so callers are not limited to Redox's
built-in providers.

```rust,no_run
use std::path::Path;

use redox_lsp::{Client, ClientEvent, ClientInfo, ServerCommand};

let command = ServerCommand::new("rust-analyzer", "rust-analyzer");
let mut client = Client::spawn(
    &command,
    Path::new("/workspace"),
    ClientInfo {
        name: "my-editor",
        version: "1.0.0",
    },
)?;

if let Some(ClientEvent::Message(message)) = client.try_recv() {
    // The frontend decides how and when to handle this message.
    println!("{message}");
}

# Ok::<(), std::io::Error>(())
```

`built_in_providers()` and `built_in_linters()` supply Redox's default tools.
Callers can construct `ServerCommand` directly for tools outside that catalogue.

## Ownership boundary

The crate owns these mechanisms:
- JSON-RPC framing and language-server process lifetime
- protocol coordinates and response parsing
- file URI conversion and provider-aware workspace discovery
- LSP snippet expansion
- built-in provider and linter metadata
- provider installation and linter process execution

The TUI owns request timing and cancellation policy, buffer synchronisation,
editor modes, cursor and view changes, completion ranking, popup state,
rendering, and diagnostic presentation. Keeping those decisions in the TUI
means this crate does not prescribe an interaction model.

## Design notes

A collection of free transport functions would have exposed request IDs,
framing, process cleanup, and reader-thread coordination to every frontend.
`Client` keeps those details together while still exposing generic request and
notification methods for protocol extensions.

Protocol results are parsed into owned values. They do not depend on
`redox-core`, terminal widgets, or editor state. This keeps the dependency
direction one-way: `redox-tui` depends on `redox-lsp`, and `redox-lsp` remains
usable on its own.
