# redox-core

Core editor library for Redox.

`redox-core` contains the editor primitives and behaviour that should stay independent of any UI. Frontends should depend on this crate for buffer/session logic and keep rendering, input mapping, and terminal/platform concerns outside it.

## What lives here

- Rope-backed text buffers via `ropey`
- Positions, selections, line/column indexing, and text slicing helpers
- Small composable edit operations and edit summaries
- Vim-style motions and text objects
- Search helpers and delimiter matching
- Fuzzy matching and path-ranking helpers used by the file finder
- Multi-buffer session management, including dirty tracking and background loading
- File I/O helpers for loading and saving buffers

## Layout

```text
src/
├── buffer/         # TextBuffer, positions, editing, selections, text objects
├── fuzzy.rs        # Fuzzy matching and path ranking
├── io.rs           # File read/write helpers
├── logic/          # Shared editor logic helpers
├── motion.rs       # UI-agnostic motion model
├── session/        # Buffer/session model and background loading
└── text/           # Shared text indexing and clamp helpers
```

## Development

Run this crate's tests from the workspace root:

```bash
cargo test -p redox-core
```

Run all workspace tests before merging changes that affect editor behaviour:

```bash
cargo test --workspace
```

**Related crates**:
- [redox-tui](https://crates.io/crates/redox-tui)
- [redox-editor](https://crates.io/crates/redox-editor)
