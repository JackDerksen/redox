# editor_tui

MinUI wrapper for the Redox. 

This crate is just responsible for:
- running the event loop and handling input
- rendering the editor state to the terminal

Depends on `editor_core` for the underlying text/buffer and editor logic.
