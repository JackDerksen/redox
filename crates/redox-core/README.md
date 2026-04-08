# redox-core

Core editor library for Redox.

This crate contains the editor primitives and behaviour that should stay independent of any UI:
- Rope-based text buffer (via `ropey`)
- Text indexing and cursor utilities (lines, columns, char indices, selections)
- Motions, text objects, search helpers, and small composable editing operations

Frontend crates should depend on `redox-core` and keep rendering, input mapping, and platform concerns outside this crate.
