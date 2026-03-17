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
  
https://github.com/user-attachments/assets/264b8c6c-3d2d-433a-ab66-4db58bd25b6e
  
</p>

## General project structure

The code is structured as a Cargo workspace with a small, testable core logic library and a TUI front-end wrapper (built with MinUI).

The intent is to keep the editor's behaviour and data structures (buffer, indexing, edit operations, motions) independent of any particular UI, so the core logic is testable and so I can make changes to MinUI without massively breaking the editor.

```text
redox/
├── Cargo.toml                  # Workspace manifest
└── crates/
    ├── redox-core/             # UI-agnostic editing primitives and session model
    │   └── src/
    │       ├── buffer/         # Rope-backed text buffer, editing, positions
    │       ├── motion.rs       # Vim-style motion logic
    │       ├── io.rs           # File read/write helpers
    │       └── session/        # Multi-buffer session management
    └── redox-tui/              # MinUI front-end application
        └── src/
            ├── app/            # Editor app state + command handling
            ├── input/          # Key/event mapping + cursor controller
            └── ui/             # Rendering and interface widgets such as the statusline
```

## Installation and usage

### Requirements

- Rust toolchain (`cargo` + `rustc`)
- A terminal that supports basic ANSI features and raw mode (and ideally full colour support)


### Install via CLI

The easiest way to install the editor is to just install the binary from Crates.io:
```
cargo install redox-editor
```

### Build from source

Build from source after cloning the repository:
```bash
cargo build --release -p redox-editor
```

Then install the created binary:
```
cargo install --path .
```

This installs the `redox` binary into `~/.cargo/bin` by default.

If needed, add that location to your `PATH` (example for zsh):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

### Run Redox

```bash
redox <file_path>
```

Example:

```bash
redox ./README.md
```

Open straight into the explorer for any specified directory (including `.`):

```bash
redox src
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
| `:about` | Toggle the "about" popup |


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
| `v` | Visual mode |
| `V` | Visual line mode |
| `x` | Delete character under cursor |
| (visual mode) `x` | Delete selection without copying to register |
| `y` | Yank selection to private register |
| `<space>y` | Yank selection to system clipboard |
| `p` | Paste from private register |
| `P` | Paste above cursor |
| (visual mode) `J` | Move selection up |
| (visual mode) `K` | Move selection down |
| (visual mode) `tab` | Indent selection |
| (visual mode) `shift+tab` | Un-ident selection (outdent?) |
| `<space>e` | Toggle file explorer |
| `u` | Undo edit |
| `ctrl+r` | Redo edit |
| `ctrl+d` | Scroll down by one viewport |
| `ctrl+u` | Scroll up by one viewport |
| `zz` | Center cursor line in the viewport |

Notes:
- Count prefixes are supported for motion keys (for example: `3w`, `5j`, `2G`).
- Arrow keys are also mapped for basic directional motion.
- This is an opinionated editor, so the motions are subject to change based on my personal preferences.

## Roadmap (current progress)

- [x] Rope-backed text buffer core (`redox-core`)
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
- [x] `:about` "About Redox" screen with version info and stuff
- [x] Relative line numbers (no standard line numbers because those are objectively worse)
- [x] The ability to open redox into the entire current working directory with `$ redox .`
- [x] Visual mode and visual line mode (with my custom line movement keybinds of shift+j/k)
- [x] Basic session-bound undo/redo 
- [ ] More Vim motions
- [ ] Undo tree with stored history
- [ ] More extendable leader key system with "whichkey" functionality
- [ ] Local search (`/`, `f`, `F`)
- [ ] A dashboard screen with similar functionality to nvim dashboards

## License

Redox is under the terms of the MIT License.
