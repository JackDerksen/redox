# editor_core

Core library crate for the Redox editor.

This crate contains editor primitives and logic that are independent of any UI:
- Rope-based text buffer (via `ropey`)
- Text indexing utilities (line/column, char indices, selections)
- Small, composable editing operations suitable for building Vim-like behavior

The UI system should depend on `editor_core` and keep rendering, input, and platform concerns outside of this crate.
