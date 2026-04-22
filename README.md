<p align="center">
  <img width="250" height="130" alt="Redox Logo" src="assets/redox-logo.png" />
</p>

<h1 align="center">
  A terminal-based text editor, built with MinUI
</h1>

<p align="center">
  Redox is a terminal-based, Vim-like text editor written in Rust. It was originally made for my university capstone project, but development is ongoing!
  <br><br>
  <strong>PLEASE NOTE</strong>: This editor is in no way associated with
  <a href="https://www.redox-os.org/">Redox OS</a>.
</p>

<p align="center">
    <img width="1541" height="1027" alt="Redox Demo" src="assets/redox-demo.png" />
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
    │       ├── session/        # Multi-buffer session management
    │       └── text/           # Shared text types and helper functions
    └── redox-tui/              # MinUI front-end application
        └── src/
            ├── app/            # Editor app state + command handling
            ├── input/          # Key/event mapping + cursor controller
            └── ui/             # Rendering, syntax highlights, and interface widgets
```

## Getting Started

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


## Usage Guide
<details>

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
| `:perf` | Toggle the performance metrics popup |

### Currently working Vim motions

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
| `ctrl+v` | Visual block mode |
| `x` | Delete character under cursor |
| (visual mode) `x` | Delete selection without copying to register |
| (visual mode) `y` | Yank selection to private register |
| (visual mode) `c` | Change selection and enter insert mode |
| (visual mode) `<space>y` | Yank selection to system clipboard |
| `p` | Paste from private register |
| `P` | Paste before cursor / above line |
| `<space>p` | Paste from system clipboard |
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
| `dd` | Delete current line |
| `cc` | Change current line |
| `yy` | Yank current line |
| `~` | Invert the capitalisation of the character under the cursor (or a whole selection) |
| `0` | Go to 0th character in the line |
| `_` | Go to first non-whitespace character in the line |
| `$` | Go to end of the line |
| `%` | Jump to the matching delimiter under the cursor |
| `D` | Delete from cursor position to end of the line |
| (normal mode) `r` | Replace under cursor (eg. `r-` replaces the cursor cell with a `-`) |
| (visual mode) `r` | Replace entire selection |
| `/` | Open search to read for instances of a pattern in the current file |
| `f` | Move cursor on top of the closest occurrence of the specified character |
| `t` | Move cursor up to (before) the closest occurrence of the specified character |
| `F` | Move cursor backward onto the closest occurrence of the specified character |
| `T` | Move cursor backward to just after the closest occurrence of the specified character |

Notes:
- Count prefixes are supported for motion keys (for example: `3w`, `5j`, `2G`).
- Compound motions are functional, such as `dap` to delete a full paragraph, `ci"` to change a string, and `d$` (or just `D`) to delete to the end of the line.
- Arrow keys are also mapped for basic directional motion.
- This is an opinionated editor, so the motions are subject to change based on my personal preferences.

</details>

## Roadmap (current progress)
<details>

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
- [x] Syntax highlighting for...
    - [x] Rust
    - [x] Markdown
    - [x] C/C++
    - [x] Go
    - [x] Lua
    - [x] Python
    - [x] Web langs (HTML, CSS, JS/TS)
    - [x] json
    - [x] toml
    - [x] yaml
- [x] Subtle colour column at col=80
- [x] Scope indicator lines and delimiter pair highlighting
- [x] Visual block mode
- [x] Compound motions (like `daw` or `ci"`)
- [x] Basic local search (`/`, `f`, `F`)
- [x] Smart indenting with Tree-sitter
- [ ] Undo tree UI with stored history
- [ ] Fuzzy finder (file name) widget for project directory search (like Telescope)
- [ ] Grep-based fuzzy finder for searching text patterns across files
- [ ] More extendable leader key system with "whichkey" functionality
- [ ] A dashboard screen with similar functionality to nvim dashboards
- [ ] More Vim motions (ongoing, not marking this complete until I've caught them all!)

</details>

## License

Redox is under the terms of the MIT License.
