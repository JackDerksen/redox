# redox-tui

Terminal UI frontend for Redox, built on MinUI.

`redox-tui` owns the interactive terminal application. It depends on `redox-core` for text buffers, motions, fuzzy matching, and session logic, then layers on input handling, app state, rendering, syntax highlighting, and terminal-specific UI.

Most users should install the top-level `redox-editor` package, which exposes the `redox` binary. This crate is published separately because it is the frontend runtime used by that package.

## What lives here

- The MinUI event loop and terminal window integration
- Input mapping for normal, insert, command, search, finder, pinboard, and visual modes
- Per-buffer cursor and viewport state
- Command handling, undo/redo state, search state, and editor actions
- File explorer and fuzzy finder popups
- Global file pinning and pinboard UI
- Status bar, status toast, command line, about popup, and performance popup widgets
- Tree-sitter syntax highlighting, smart indenting, overlays, delimiter highlights, and scope guides

## Layout

```text
src/
├── app/
│   ├── state.rs    # Main EditorState and mode model
│   └── state/      # Commands, editing, explorer, finder, perf, search, surfaces, tests
├── input/          # Event mapping, counts/operators, cursor projection
├── ui/
│   ├── syntax/     # Tree-sitter language adapters
│   ├── widgets/    # Popup/status/rendered UI components
│   ├── overlays.rs # Scope guides, delimiter highlights, colour column
│   ├── render.rs   # Text snapshot and render helpers
│   └── style.rs    # Theme and colour definitions
├── lib.rs          # Runtime entrypoint and main draw loop
└── main.rs         # Binary entrypoint for this crate
```

## Development

Run this crate's tests from the workspace root:

```bash
cargo test -p redox-tui
```

Run the whole editor locally through the top-level package:

```bash
cargo run -p redox-editor -- ./README.md
```

**Related crates**:
- [redox-core](https://crates.io/crates/redox-core)
- [redox-editor](https://crates.io/crates/redox-editor)
