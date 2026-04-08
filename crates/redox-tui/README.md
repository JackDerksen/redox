# redox-tui

Terminal UI frontend for Redox, built on MinUI.

This crate is responsible for:
- running the event loop and translating terminal input into editor actions
- rendering editor state, overlays, and status UI in the terminal
- handling frontend-only concerns like viewport state, popups, and terminal interaction

It depends on `redox-core` for the underlying text buffer, motions, and editor logic.
