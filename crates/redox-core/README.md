# redox-core

Core editor library for the Redox editor.

`redox-core` contains the editor primitives and behaviour that are independent of a terminal UI. Its main job is to translate between Ropey's storage model and Redox's editing model without exposing Ropey to the frontend.

Text is addressed with Unicode scalar-value indices or zero-based `(line, column)` positions. Byte conversion exists for parsers and protocols. Grapheme segmentation, terminal-cell widths, cursors, and viewports remain frontend concerns.

## Using the library

The crate root is the supported public facade. A small prelude contains the
types most editor frontends would need:

```rust
use redox_core::prelude::*;

let mut buffer = TextBuffer::from("hello");
let cursor = buffer.insert(Pos::new(0, 5), " world");

assert_eq!(cursor, Pos::new(0, 11));
assert_eq!(String::from(&buffer), "hello world");
```

`TextBuffer::from_reader` and `TextBuffer::write_to` provide streaming UTF-8
I/O without exposing Ropey or requiring a contiguous intermediate `String`.
Sessions, fuzzy matching, and specialized editing types are intentionally left
as explicit imports rather than making the prelude mirror the entire crate.

## What lives here

- Rope-backed text buffers without Ropey types in the public API
- Checked character/byte/line conversion and non-allocating borrowed slices
- Editing, selections, text objects, search, and diff-based undo
- Vim-style motions and text objects
- Fuzzy matching and path-ranking helpers used by the file finder
- Multi-buffer session management, including dirty tracking and background loading

## Layout

```text
src/
├── buffer/         # TextBuffer, positions, editing, selections, text objects
├── fuzzy.rs        # Fuzzy matching and path ranking
├── motion.rs       # UI-agnostic motion model
└── session/        # Buffer/session model, file I/O, and incremental loading
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
