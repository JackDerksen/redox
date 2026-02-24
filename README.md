<p align="center">
  <img width="250" height="130" alt="Redox Logo 6" src="https://github.com/user-attachments/assets/a0bea6c3-b40e-4f56-b904-9da08b13b2ee" />
</p>

<h1 align="center">
  A terminal-based text editor, built with MinUI
</h1>

<p align="center">
  Redox is a terminal-based, Vim-like text editor written in Rust for my final capstone project. 
</p>

<p align="center">
  <img width="917" height="648" alt="CleanShot 2026-02-24 at 13 55 24@2x" src="https://github.com/user-attachments/assets/c66ae6ad-8e39-4d37-a115-a6dfa0d2443b" />
</p>

## General project structure

The code is structured as a Cargo workspace with a small, testable core logic library and a TUI front-end wrapper (built with MinUI).

The intent is to keep the editor's behaviour and data structures (buffer, indexing, edit operations, motions) independent of any particular UI, so the core logic is testable and so I can make changes to MinUI without massively breaking the editor.

```text
redox/
├── Cargo.toml                  # Workspace manifest
└── crates/
    ├── editor_core/            # UI-agnostic editing primitives and session model
    │   └── src/
    │       ├── buffer/         # Rope-backed text buffer, editing, positions
    │       ├── motion.rs       # Vim-style motion logic
    │       ├── io.rs           # File read/write helpers
    │       └── session/        # Multi-buffer session management
    └── editor_tui/             # MinUI front-end application
        └── src/
            ├── app/            # Editor app state + command handling
            ├── input/          # Key/event mapping + cursor controller
            └── ui/             # Rendering and interface widgets such as the statusline
```

## Installation and usage

### Requirements

- Rust toolchain (`cargo` + `rustc`)
- A terminal that supports basic ANSI features and raw mode (and ideally full colour support)

### Build

```bash
cargo build --release -p editor_tui
```

### Install CLI

```bash
cargo install --path crates/editor_tui --force
```

This installs the `redox` binary into `~/.cargo/bin` by default.

If needed, add that location to your `PATH` (example for zsh):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

### Run

```bash
redox <file_path>
```

Example:

```bash
redox ./editor_tui/readme.md
```

### Command mode quick reference

| Command | Behaviour |
| ------- | --------- |
| `:w` | Write current buffer |
| `:q` | Quit Redox (if all buffers are clean) |
| `:q!` | Force quit |
| `:wq` | Write current buffer, then quit if all buffers are clean |
| `:e <path>` | Open/switch buffer for a specified file path |
| `:bn` / `:bnext` | Switch to next buffer (MRU order) |
| `:bp` / `:bprev` | Switch to previous buffer (MRU order) |
| `:ls` | Show summary of open buffers |
| `:ex` / `:explorer` | Toggle file explorer |


## Currently working Vim motions

| Keys | Behaviour |
| --- | --- |
| `h` / `j` / `k` / `l` | Left / down / up / right cursor motion |
| `w` | Move to start of next word |
| `b` | Move to start of previous word |
| `e` | Move to end of current/next word |
| `gg` | Jump to start of file |
| `G` | Jump to end of file |
| `i` | Insert before cursor |
| `I` | Insert at the start of the line |
| `a` | Insert after cursor |
| `A` | Insert at the end of the line |
| `o` | Insert below cursor |
| `O` | Insert above cursor |
| `<space>e` | Toggle file explorer |

Notes:
- Count prefixes are supported for motion keys (for example: `3w`, `5j`, `2G`).
- Arrow keys are also mapped for basic directional motion.
- Current motions are aligned with Vim's standard, but future ones are subject to change based on my opinionated preferences.

## Roadmap (current progress)

- [x] Rope-backed text buffer core (`editor_core`)
- [x] TUI rendering with statusline + cursor projection
- [x] Text insertion, newline insertion, and backspace editing
- [x] Vim-style mode system (Normal / Insert / Command)
- [x] Core motion model with reusable UI-agnostic logic
- [x] Unit test coverage across core and TUI state logic
- [x] Per-buffer cursor/viewport state preservation
- [x] File open and write flows
- [x] Multi-buffer session architecture in core
- [x] Buffer switching commands (`:e`, `:bn`, `:bp`, `:ls`)
- [x] Intelligent dirty tracking (dirty clears when content returns to saved/original state)
- [x] File explorer/picker widget
- [x] Modify style module to use more absolute colours (RGB or something)
- [ ] Visual mode and visual line mode (with my custom line movement keybinds of shift+j/k)
- [ ] More Vim motions
- [ ] More extendable leader key system with "whichkey" functionality
- [ ] Local search (`/`, `f`, `F`)
- [ ] `:about` "About Redox" screen with version info, checkhealth functionality, etc.
- [ ] The ability to open redox into the entire current working directory with `$ redox .`
- [ ] A dashboard screen with similar functionality to nvim dashboards

## License

Redox is under the terms of the MIT License.
