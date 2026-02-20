<p align="center">
  <img width="250" height="130" alt="Redox Logo 6" src="https://github.com/user-attachments/assets/a0bea6c3-b40e-4f56-b904-9da08b13b2ee" />
</p>

<h1 align="center">
  A terminal-based text editor, built with MinUI
</h1>

Redox is a terminal-based, Vim-like text editor built in Rust for my final capstone project. The code is structured as a cargo workspace with a small, testable core logic library and a TUI front-end wrapper (MinUI).

The intent is to keep the editor’s behavior and data structures (buffer, indexing, edit operations, motions) independent of any particular UI, so the core logic is testable and so I can make changes to MinUI without massively breaking the editor.

## Workspace crates

```
crates
├── editor_core  # Logic
└── editor_tui   # UI
```

- `crates/editor_core`  
  Editor core library. Owns the text buffer implementation (Ropey-backed), text/indexing utilities, and core editing primitives intended to be UI-independent.

- `crates/editor_tui`  
  MinUI TUI application. Hosts the event loop, input, rendering, and integrates `editor_core` to provide an interactive editor experience.

## Development notes

At the moment, you can run the program with `cargo run -p editor_tui -- ./<file_path>`. This will be cleaned up soon!

### What currently works:
- File input and parsing into a rope buffer (will be optimized futher)
- Drawing that rope buffer to the screen in a MinUI viewport (will be optimized further)
- Cursor drawing and movement
- Viewport scrolling (follows cursor)
- Basic vim navigation motions (`h`, `j`, `k`, `l`, `gg`, `G`, `w`, `e`, `b`, `i`, `a`, `I`, `A`, `:`)
- Normal/insert/command modes, along with basic functionality for each 
    - Editing and writing to files **does** work now!
- Statusline with current mode (colour coded) and cursor position

## License

TBD!
