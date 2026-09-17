<p align="center">
    <img width="385" height="230" alt="Redox Logo" src="assets/redox-logo.png" />
</p>

<h1 align="center">
    A terminal-based text editor that's tasteful by default.
</h1>

<p align="center">
    <a href="https://github.com/JackDerksen/redox/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/JackDerksen/redox/actions/workflows/ci.yml/badge.svg" /></a>
    <a href="https://deepwiki.com/JackDerksen/redox"><img alt="Ask DeepWiki" src="https://deepwiki.com/badge.svg" /></a>
</p>

<p align="center">
    Redox is a terminal-based, Vim-like text editor written in Rust. It was originally made for my university capstone project, but development is ongoing!
    <br><br>
    <strong>PLEASE NOTE</strong>: This editor is in no way associated with
    <a href="https://www.redox-os.org/">Redox OS</a>.
</p>

<p align="center">
    <img width="2419" height="1520" alt="Redox Demo" src="assets/redox-demo.png" />
</p>

## Project structure

Redox is a Cargo workspace with an editor core, an LSP library, and a MinUI frontend. The core crate owns editor logic that should stay UI-agnostic. The LSP crate owns protocol and process mechanisms. The TUI crate owns input mapping, app state, rendering, popups, syntax highlighting, and terminal interaction.

```text
redox/
├── Cargo.toml                  # Workspace manifest and published redox binary wrapper
├── src/main.rs                 # Thin entrypoint that calls redox-tui
└── crates/
    ├── redox-core/
    │   └── src/
    │       ├── buffer/         # Rope-backed text buffer, selections, edits, text objects
    │       ├── logic/          # Shared editor logic helpers
    │       ├── session/        # Multi-buffer session model and background loading
    │       ├── text/           # Shared text indexing and clamp helpers
    │       ├── fuzzy.rs        # Fuzzy matching and path ranking helpers
    │       ├── io.rs           # File read/write helpers
    │       ├── motion.rs       # Vim-style motion logic
    │       └── lib.rs          # Shared editor library helpers
    ├── redox-lsp/
    │   └── src/
    │       ├── transport.rs    # JSON-RPC framing and language-server processes
    │       ├── protocol.rs     # Shared LSP positions, ranges, and definition parsing
    │       ├── provider.rs     # Built-in provider catalogue and installers
    │       ├── lint.rs         # Linter processes and output parsing
    │       └── snippet.rs      # LSP snippet expansion
    └── redox-tui/
        └── src/
            ├── app/            # Main editor state and mode model
            ├── input/          # Key/event mapping, counts, operators, cursor controller
            └── ui/             # UI rendering, widgets, animations, styling
```

This split keeps buffer operations, indexing, motions, fuzzy scoring, session behaviour, and LSP mechanics testable without a terminal. The frontend can evolve the interface without pulling UI details into `redox-core` or `redox-lsp`.

**The subcrates can be found here**:

- [redox-core](https://crates.io/crates/redox-core)
- [redox-lsp](https://crates.io/crates/redox-lsp)
- [redox-tui](https://crates.io/crates/redox-tui)

## Getting Started

### Requirements

- Rust toolchain (`cargo` + `rustc`) for Cargo installs and source builds. Homebrew installs build tools automatically.
- A terminal that supports basic ANSI features and raw mode (and ideally full colour support). I'd **highly** recommend [Ghostty](https://ghostty.org/) for the best experience!
- Optional Go linting: golangci-lint v2.0.0 or newer. v1 is unsupported; see [Language tools](#language-tools) for setup.


### Install with Homebrew

On macOS or Linux:

```sh
brew install jackderksen/tap/redox
```

To update:

```sh
brew update
brew upgrade redox
```

The [personal tap](https://github.com/JackDerksen/homebrew-tap) builds from a pinned
release source archive. It checks for new stable releases hourly and tests formula
updates on macOS and Linux before publishing them.

### Install with Cargo

Install the binary from Crates.io:
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

### Updates

Redox checks GitHub's latest stable release in the background on startup and shows a
toast when a newer version is available. Successful checks are cached for 24 hours
under the Redox state directory. **Note**: Checks require `curl`; missing `curl`,
offline connections, and other startup check failures stay quiet.

Run `:check-update` to check immediately, bypassing the cache, and see the result or
any connection error. Set `check_updates = false` in your configuration to disable
automatic checks (manual checks remain available).

For Homebrew installs, run `brew update && brew upgrade redox`. For Cargo installs,
run `cargo install redox-editor --locked` in your terminal to install the newest
published crate. For release binaries, download the matching archive from
[GitHub releases](https://github.com/JackDerksen/redox/releases/latest) and replace
your installed `redox` binary after closing the editor. GitHub releases may appear
before the corresponding crate is published. Redox only notifies you; it does not
download or install updates itself.


## Usage guide

### Configuration

Redox was designed from the ground-up to be pleasant without the need for configuration, but
it is highly configurable, should you choose to modify the default behaviour or appearance.

It looks for configuration at `$REDOX_CONFIG`, `$XDG_CONFIG_HOME/redox/config.toml`, or
`~/.config/redox/config.toml` (in that order). A different file can be selected with
`redox --config /path/to/config.toml`.

Configuration supports features like named themes, the complete base palette, every syntax role,
UI colour pairs, optional Nerd Font icons, background dimming, popup dimensions, colour-column
position, zen mode, undo-tree history size, the leader character, and mode-specific keybindings. See the
[`config.example.toml`](config.example.toml) starter file and the complete
[`CONFIGURATION.md`](CONFIGURATION.md) reference. Unspecified values always use the built-in
defaults.

Editor-managed data (such as undo history and LSP metadata) lives separately under
`$XDG_STATE_HOME/redox/` (or `~/.local/state/redox/`), leaving the configuration directory for
`config.toml` alone. Existing legacy state is migrated automatically.

<details>
<summary>Command, navigation, editing, and search reference</summary>

### Run Redox
```bash
redox <file_path>
```

Use `redox --help` for launch options and `redox --version` to print the installed version.

Run `redox` without a path to open the startup dashboard. Press a shortcut directly,
or move with `j`/`k` and press `Enter`:

| Key | Action |
| --- | --- |
| `r` | Restore the previous session for the current directory. |
| `f` | Open Finder. |
| `e` | Open the file explorer. |
| `n` | Create an empty, unnamed file. |
| `c` | Open the configuration file. |
| `q` | Quit. |

Use `:dashboard` to reopen it while editing. Open buffers and unsaved edits are
preserved; `Escape` returns to the previous buffer.

Sessions remember saved files, their cursor positions, and the active file when Redox
exits. They live under the Redox state directory in `sessions/`. Exiting an empty
dashboard leaves the previous session intact. Unsaved contents and split layouts
are not restored.

Name a new buffer on its first write, for example `:w file.rs`. The extension selects
syntax highlighting and the existing formatting tools. A failed write keeps the
buffer's previous name and contents, and a different existing file is never overwritten.

Example:
```bash
redox ./README.md
```

Open straight into the explorer for any specified directory (including `.`):
```bash
redox src
```

### Command mode

Enter command mode with `:`.

| Command | Behaviour |
| ------- | --------- |
| `:w [path]` | Write the current buffer, optionally saving under a new name. Explorer buffers apply pending filesystem edits. |
| `:q` / `:quit` | Quit when all buffers are clean, or close the active surface buffer. |
| `:q!` | Force quit. |
| `:wq [path]` | Write the current buffer, optionally saving under a new name, then quit when all buffers are clean. |
| `:e <path>` | Open or switch to a file buffer. |
| `:e!` / `:reload` | Reload the active file from disk. |
| `:config` | Open the active configuration file, creating its parent directory when needed. |
| `:config reload` | Reload configuration, themes, and keybindings without restarting. |
| `:dashboard` | Open the dashboard, keeping existing buffers and unsaved edits. |
| `:convert <value> <source> to <target>` | Preview a base, unit or colour conversion. Press `Enter` to insert the result. |
| `:check-update` | Check GitHub for a newer stable release and show update instructions. |
| `:colorscheme <name>` | Apply a named theme for the current session. Bare `:colorscheme` shows the active theme. |
| `:bn` / `:bnext` | Switch to the next buffer in MRU order. |
| `:bp` / `:bprev` | Switch to the previous buffer in MRU order. |
| `:ls` | Show a compact summary of open buffers. |
| `:macros` | List the current session's macro registers and recorded key sequences. |
| `:ex` / `:explorer` | Toggle the file explorer. |
| `:about` | Toggle the about popup. |
| `:rain` | Toggle rain mode. |
| `:zen` | Toggle zen mode. |
| `:perf` / `:perf popup` | Toggle the performance metrics popup. |
| `:undo-tree` | Toggle the undo tree pane. |
| `:lsp list` | Open the language tools marketplace. |
| `:lsp status` | Show the active buffer's detected language tools. |

Command and subcommand completions appear as ghost text at the end of the input, regardless of cursor position. Press `Tab` from anywhere in the input to accept and move the cursor to the end, `ctrl+n` / `ctrl+p` to cycle suggestions, or `Shift+Tab` to cycle backwards. `Enter` runs only the text you have typed or accepted. Use `Up` / `Down` for command history, `Left` / `Right` to move within the command line, and `Escape` / `ctrl+c` to cancel.

The command line calculator supports arithmetic, base and unit conversions, and RGB/hex colours, with live result previews. `Enter` pastes the result at the buffer cursor as one undoable edit.

<details>
<summary>Calculator usage</summary>

Type an arithmetic expression after `:`, or use `:convert` for base, unit and colour conversions. The answer appears as ghost text and updates while you edit anywhere in the input. `Enter` inserts the result at the buffer cursor; `Escape` leaves the buffer untouched. Number and unit conversions insert just the resulting number. Colour conversions insert `#rrggbb` or `rgb(r, g, b)`.

| Input | Result |
| ----- | ------ |
| `5+(2x3)` | `11` |
| `convert 100 binary to decimal` | `4` |
| `convert 255 decimal to hex` | `ff` |
| `convert FF hex to decimal` | `255` |
| `convert 5 kg to lbs` | `11.0231131092439` |
| `convert (5+2) kg to g` | `7000` |
| `convert 12 in to ft` | `1` |
| `convert 1 acre to m2` | `4046.8564224` |
| `convert 1 usgal to L` | `3.785411784` |
| `convert 100 km/h to mph` | `62.1371192237334` |
| `convert 90 min to h` | `1.5` |
| `convert 90 minutes to seconds` | `5400` |
| `convert 5 kilograms to pounds` | `11.0231131092439` |
| `convert 180 deg to rad` | `3.14159265358979` |
| `convert 0 C to F` | `32` |
| `convert 273.15 K to C` | `0` |
| `convert 1 MiB to B` | `1048576` |
| `convert 8 Mb to MB` | `1` |
| `convert rgb(255, 128, 0) to hex` | `#ff8000` |
| `convert 255,128,0 rgb to hex` | `#ff8000` |
| `convert #ff8000 to rgb` | `rgb(255, 128, 0)` |
| `convert FF8000 hex to rgb` | `rgb(255, 128, 0)` |
| `convert #abc to rgb` | `rgb(170, 187, 204)` |

Arithmetic supports `+`, `-`, `*` or `x`, `/`, `%`, `^`, parentheses, decimals and scientific notation.

Conversions use `convert value source to target`, such as `convert 100 km/h to mph`. All supported units accept spaces. Underscores remain accepted between the value and source, and before the target, within the `convert` command. Unit conversions can take an arithmetic expression as their value, such as `convert (5 + 2) kg to g`.

Base conversions take signed integers. Use `binary` / `bin`, `octal` / `oct`, `decimal` / `dec`, or `hexadecimal` / `hex`. Any base from 2 through 36 can also be written as a number, such as `convert z 36 to decimal`. Hexadecimal numbers and colours share the name `hex`; the other format makes the conversion clear, as in `convert FF hex to decimal` and `convert FF8000 hex to rgb`.

Base conversions retain exact values within the signed 128-bit range, including integers beyond the arithmetic calculator's safe literal range of ±9,007,199,254,740,991. Unit conversions use the calculator's floating-point precision.

Unit symbols are case-sensitive and must measure the same quantity:

| Quantity | Units and aliases |
| -------- | ----------------- |
| Distance | `mm`, `cm`, `m`, `km`, `in`, `ft`, `yd`, `mi`, `nmi` |
| Mass | `mg`, `g`, `kg`, `t`, `oz`, `lb` / `lbs`, `st` |
| Area | `mm2`, `cm2`, `m2`, `km2`, `in2`, `ft2`, `ha`, `acre` / `acres` |
| Volume | `ml` / `mL` / `cm3`, `l` / `L`, `m3`, `usgal`, `impgal`, `uscup`, `usfloz`, `impfloz` |
| Speed | `m/s`, `km/h` / `kph`, `mph`, `kn` / `knots` |
| Time | `ns`, `us`, `ms`, `s` / `sec`, `min`, `h` / `hr`, `d` / `day`, `wk` |
| Temperature | `C`, `F`, `K` |
| Angles | `deg`, `rad`, `turn` |
| Data sizes | `b` / `bit`, `B` / `byte`, `kb`, `Mb`, `Gb`, `Tb`, `kB` / `KB`, `MB`, `GB`, `TB`, `KiB`, `MiB`, `GiB`, `TiB` |

Full names accept singular and plural forms, including irregular plurals such as `feet` and `inches`. They ignore case and accept Canadian/European and American spellings, so `metres` / `meters` and `litres` / `liters` both work. This applies on either side of `to`, and full names can be mixed with symbols.

| Quantity | Full name examples |
| -------- | ------------------ |
| Distance | `millimetres`, `centimetres`, `metres`, `kilometres`, `inches`, `feet`, `yards`, `miles`, `nautical miles` |
| Mass | `milligrams`, `grams`, `kilograms`, `tonnes`, `metric tons`, `ounces`, `pounds`, `stones` |
| Area | `square millimetres`, `square centimetres`, `square metres`, `square kilometres`, `square inches`, `square feet`, `hectares`, `acres` |
| Volume | `millilitres`, `litres`, `cubic centimetres`, `cubic metres`, `US gallons`, `Imperial gallons`, `US cups`, `US fluid ounces`, `Imperial fluid ounces` |
| Speed | `metres per second`, `kilometres per hour`, `miles per hour`, `knots` |
| Time | `nanoseconds`, `microseconds`, `milliseconds`, `seconds`, `minutes`, `hours`, `days`, `weeks` |
| Temperature | `Celsius`, `degrees Celsius`, `Fahrenheit`, `degrees Fahrenheit`, `kelvins` |
| Angles | `degrees`, `radians`, `turns` |
| Data sizes | `bits`, `bytes`, `kilobits`, `megabits`, `gigabits`, `terabits`, `kilobytes`, `megabytes`, `gigabytes`, `terabytes`, `kibibytes`, `mebibytes`, `gibibytes`, `tebibytes` |

For example, `convert 100 kilometres per hour to miles per hour` and `convert (5 + 2) square metres to square feet` work directly. `secs`, `mins`, `hrs`, and `wks` are also accepted. Regional measures still need their qualifier: use `US gallons`, `Imperial gallons`, or `metric tons` rather than bare `gallons` or `tons`.

`t` is a metric tonne, `st` is a stone, and `oz` is an ounce of mass. Volume names beginning with `us` use US customary measures; `imp` means Imperial. A day is 24 hours and a week is seven days. Months, years and currencies are excluded because their conversions depend on context. Temperature conversions use absolute temperatures, with `C` for Celsius, `F` for Fahrenheit and `K` for kelvin.

`MB` is decimal megabytes; `MiB` is binary mebibytes. Lowercase `b` denotes bits and uppercase `B` denotes bytes. These distinctions follow the [NIST binary prefix definitions](https://physics.nist.gov/cuu/Units/binary.html). Physical units follow standard definitions documented in the [NIST conversion reference](https://www.nist.gov/pml/special-publication-811/nist-guide-si-appendix-b-conversion-factors/nist-guide-si-appendix-b9).

RGB inputs require three integers from 0 to 255. Hex colours accept three or six digits, with an optional `#` when specifying `hex` as the source. Alpha channels are not supported.

</details>

`:colorscheme ` also completes theme names from your configuration, plus the built-in `default`. Commands from `[[bind]]` entries with a `command` field are included too. Suggestions refresh after `:config reload`.

### File navigation

| Keys | Behaviour |
| ---- | --------- |
| `<space><space>` | Open the fuzzy file finder for the launch directory. |
| `<space>e` | Toggle the file explorer. |
| `<space>z` | Toggle zen mode, with a centred viewport and syntax colour in the current scope. |
| `<space>u` | Toggle the undo tree pane. |
| `<space>x` | Toggle the diagnostics list for the current file. |
| `<space>ca` | Show quick fixes for the diagnostic under the cursor. |
| `gd` | Go to symbol definition using the active LSP. |
| `ctrl+shift+k` | Request LSP completions in insert mode. |
| `Enter` | Open the selected explorer/finder entry. |
| `-` | Navigate to the parent directory in the explorer. |
| `ctrl+shift+p` | Open the pinboard for the current file or selected finder entry. |
| `ctrl+1` ... `ctrl+5` | Open a pinned file slot. |

Undo tree controls:

| Keys | Behaviour |
| ---- | --------- |
| `<space>u` or `:undo-tree` | Toggle the undo tree pane. |
| `j` / `k` or `Down` / `Up` | Move between undo states. |
| `h` / `l` or `Left` / `Right` | Move to the parent state / newest child state. |
| `Enter` | Restore the selected undo state. |
| `Escape` / `ctrl+c` | Close the undo tree pane. |

Finder controls:

| Keys | Behaviour |
| ---- | --------- |
| Type text | Filter files with fuzzy matching. |
| `Backspace` | Delete the previous query character. |
| `Up` / `Down` | Move through results. |
| `ctrl+p` / `ctrl+n` | Move through results. |
| `Enter` | Open the selected file. |
| `ctrl+shift+p` | Pin the selected finder entry. |
| `ctrl+1` ... `ctrl+5` | Open a pinned file slot. |
| `Escape` / `ctrl+c` | Close the finder. |

Pinboard controls:

| Keys | Behaviour |
| ---- | --------- |
| `j` / `k` or `Down` / `Up` | Move between pin slots. |
| `ctrl+n` / `ctrl+p` | Move between pin slots. |
| `p` or `shift+Enter` | Assign the current file to the selected slot. |
| `ctrl+1` ... `ctrl+5` | Assign directly to a slot while the pinboard is open. |
| `Enter` | Open the selected pinned file. |
| `shift+j` / `shift+k` | Reorder the selected pin down/up. |
| `d` | Delete the selected pin. |
| `Escape` / `ctrl+c` | Close the pinboard. |

### Language tools

Redox can start installed language servers for supported file types and display diagnostics inline, in the status bar, and in a diagnostics popup.

Open the language tools marketplace with `:lsp list`.

Go linting requires golangci-lint v2.0.0 or newer because Redox uses the v2-only
`--output.json.path stdout` and `--output.text.path stderr` flags. Check
`golangci-lint --version` before enabling it. If it reports v1, upgrade using the
[golangci-lint installation guide](https://golangci-lint.run/docs/welcome/install/local/)
and ensure the v2 binary is first on `PATH`. Existing v1 configurations also need
the [v2 migration](https://golangci-lint.run/docs/product/migration-guide/).

Completion controls:

| Keys | Behaviour |
| ---- | --------- |
| Typing code | Request completions automatically after a short pause. |
| `ctrl+i` | Show type, signature, or documentation for the symbol under the cursor. |
| `ctrl+shift+k` | Request completions in insert mode. |
| `ctrl+n` / `ctrl+p` or `Down` / `Up` | Move through completion results. |
| `Enter` | Accept the selected completion. |
| `tab` | Jump to the next snippet placeholder. |
| `ctrl+e` | Close completions. |
| `Escape` | Leave insert mode. |

Marketplace controls:

| Keys | Behaviour |
| ---- | --------- |
| `j` / `k` or `Down` / `Up` | Move through tools. |
| `i` | Enable or install the selected tool. If the executable is already on `PATH`, Redox just enables it. |
| `u` | Disable or uninstall the selected tool when Redox knows how it was installed. |
| `Escape` / `ctrl+c` | Close the marketplace. |

Diagnostics controls:

| Keys | Behaviour |
| ---- | --------- |
| `<space>x` | Toggle the diagnostics list for the current file. |
| `j` / `k` or `Down` / `Up` | Move through diagnostics. |
| `Enter` | Jump to the selected diagnostic. |
| `a` | Open quick fixes for the selected diagnostic in a lower split. |
| `Escape` / `ctrl+c` | Close the lower quick-fix split, or close the diagnostics list when no split is open. |

Code action controls:

| Keys | Behaviour |
| ---- | --------- |
| `<space>ca` | Open quick fixes for the diagnostic under the cursor. |
| `j` / `k` or `Down` / `Up` | Move through available actions. |
| `Enter` | Apply the selected quick fix. |
| `Escape` / `ctrl+c` | Close the quick-fix popup. |

Symbol info controls:

| Keys | Behaviour |
| ---- | --------- |
| `ctrl+i` | Show symbol info in normal or insert mode. |
| `ctrl+tab` | Show symbol info in insert mode. |
| `j` / `k` or `Down` / `Up` | Scroll the symbol info popup. |
| `Escape` / `ctrl+c` | Close the symbol info popup. |

Other language tool commands:

| Command / keys | Behaviour |
| -------------- | --------- |
| `:lsp status` | Show the detected language, LSP, and linter for the current file. |
| `gd` | Go to symbol definition. |
| `ctrl+i` | Show symbol info. |
| Typing code or `ctrl+shift+k` | Request completions. |

### Editing and motion

| Keys | Behaviour |
| ---- | --------- |
| `h` / `j` / `k` / `l` | Move left / down / up / right. |
| Arrow keys | Basic directional motion. |
| `w` / `b` / `e` | Move by word starts and word ends. |
| `0` / `_` / `$` | Move to line start, first non-whitespace, or line end. |
| `gg` / `G` | Jump to the start or end of the file. |
| `%` | Jump to the matching delimiter under or near the cursor. |
| `f` / `t` / `F` / `T` | Find/till a character forward or backward on the current line. |
| `i` / `I` | Insert before the cursor / at first non-whitespace. |
| `a` / `A` | Insert after the cursor / at line end. |
| `o` / `O` | Open a line below / above. |
| `J` | Join the line below onto the current line. |
| `x` | Delete the character under the cursor without touching the private register. |
| `r` | Replace the character under the cursor. |
| `dd` / `cc` / `yy` | Delete, change, or yank the current line. |
| `D` | Delete from the cursor to the end of the line. |
| `p` / `P` | Paste after/before from the private register. |
| `<space>p` | Paste from the system clipboard. |
| `u` / `ctrl+r` | Undo / redo. |
| `.` | Repeat the last edit at the cursor. A count replaces the edit's previous count. |
| `ctrl+d` / `ctrl+u` | Scroll down/up by one viewport. |
| `zz` | Centre the cursor line in the viewport. |
| `~` | Toggle character case, or the whole visual selection. |

### Splits

| Keys | Behaviour |
| ---- | --------- |
| `ctrl+-` | Split the active pane horizontally. |
| <code>ctrl+\</code> | Split the active pane vertically. |
| `ctrl+h` / `ctrl+j` / `ctrl+k` / `ctrl+l` | Focus the split to the left / down / up / right. |
| `ctrl+x` | Close the active split. |

Inactive editor panes show their filename centred in a muted strip along the top.

### Repeating edits and macros

`.` repeats the last edit, including its inserted text or visual selection dimensions.
Moving, searching, yanking, and undoing leave that edit available to repeat.

| Keys | Behaviour |
| ---- | --------- |
| `Qa` ... `Q` | Record a macro into register `a`, then stop. |
| `QA` ... `Q` | Append to the macro in register `a`. |
| `@a` / `3@a` | Play macro `a` once / three times. |
| `Q3` ... `Q` / `@3` | Record / play numeric register `3`. |
| `Q!` ... `Q` / `@!` | Record / play punctuation register `!`. |
| `@@` | Play the last-used macro again. |

Registers accept letters, digits, punctuation, Space, Tab, Enter, Backspace, arrow keys,
and supported Ctrl/Alt combinations. Function keys are excluded. Escape cancels register
selection, and `@` is reserved for `@@`. Uppercase letters append to their lowercase register.
Counts go before `@`: `3@a` plays register `a` three times, while `@3` plays register `3`.

A persistent toast shows the recording register. Stopping displays
the recorded key sequence. Lowercase `q` is unused; `Q` is ordinary text in insert mode.
Macros last for the current editor session and replay recorded actions, including captured
clipboard text. `:macros` lists saved registers and their sequences in a toast, like `:ls`.
Playback groups the entire invocation into one undo step per buffer, including every repetition,
nested macro call, and any undo/redo commands in the macro. Recursive or excessively long playback
stops at a limit.

### Visual modes

| Keys | Behaviour |
| ---- | --------- |
| `v` / `V` / `ctrl+v` | Enter visual, visual line, or visual block mode. |
| `y` / `d` / `c` | Yank, delete, or change the active selection. |
| `x` | Delete the selection without copying to the private register. |
| `r` | Replace the active selection. |
| `<space>y` | Yank the selection to the system clipboard. |
| `tab` / `shift+tab` | Indent / outdent the selection. |
| `J` / `K` | Move selected lines down/up. |
| `<leader>[` / `<leader>]` | Wrap the selection in `[...]`. |
| `<leader>{` / `<leader>}` | Wrap the selection in `{...}`. |
| `<leader>(` / `<leader>)` | Wrap the selection in `(...)`. |
| `<leader><` / `<leader>>` | Wrap the selection in `<...>`. |
| `<leader>"` / `<leader>'` / <code>&lt;leader&gt;`</code> | Wrap the selection in double quotes, single quotes, or backticks. |

The leader defaults to `Space`. Wrapping returns to normal mode and can be undone in one step.
Visual line mode wraps the selected lines together, before the final newline. Visual block mode
wraps each selected row separately, skipping rows with no selected text.

### Search and text objects

`/` opens a compact search popup in the top-right corner. Matches highlight as you type, with the current result and total count shown in the title. `Down` / `Up` or `ctrl+n` / `ctrl+p` cycle through results and centre the active match where the file position allows. `Enter` keeps the result; `Escape` / `ctrl+c` restore the previous cursor, viewport, and search.

The active result uses a distinct highlight while the popup is open. Searches accept regular expressions, such as `\bword\b`, `name\d+`, `^fn`, or `(?i)text` for case-insensitive matches. Escape punctuation to match it literally, for example `\.`. Anchors apply per line; `\n` and `(?s)` allow matches across lines. Invalid expressions show an error in the popup. Look-around and backreferences are unsupported. Character searches with `f` / `t` remain literal.

| Keys | Behaviour |
| ---- | --------- |
| `/` | Search in the current buffer. |
| `ctrl+n` / `ctrl+p` | Repeat the cached search forward/backward. |
| `d$`, `c$`, `y$` | Apply an operator through a motion. |
| `daw`, `ci"`, `yi(` | Apply operators to text objects. |

Notes:
- Count prefixes are supported for motions and many operators, for example `3w`, `5j`, `2G`, and `2ci]`.
- Text objects include words, big words, paragraphs, parentheses, brackets, braces, single quotes, double quotes, and backticks.
- Compound motions are functional, such as `dap`, `ci"`, `d$`, `dt,`, and `ygg`.
- Redox is intentionally opinionated, so keybindings may still move around as the editor settles.

</details>

## Roadmap

<details>
<summary>Current progress and planned work</summary>

These have roughly been categorized, and so aren't necessarily in chronological order.

- [x] Rope-backed text buffer core (`redox-core`)
- [x] TUI rendering with statusline + cursor projection
- [x] Text insertion, newline insertion, and backspace editing
- [x] Vim-style mode system (Normal / Insert / Command)
- [x] Core motion model with reusable UI-agnostic logic
- [x] Unit test coverage across core and TUI state logic
- [x] Per-buffer cursor/viewport state preservation
- [x] File open and write flows
- [x] Multi-buffer session architecture in core
- [x] Incremental loading for large files
- [x] Buffer switching commands (`:e`, `:bn`, `:bp`, `:ls`)
- [x] Intelligent dirty tracking (dirty clears when content returns to saved/original state)
- [x] Editable file explorer/picker widget
- [x] Fuzzy file finder for project directory search
- [x] Global file pinning and pinboard popup
- [x] Style module with explicit RGB-ish theme colours
- [x] `:about` screen with version info
- [x] Relative line numbers (no standard line numbers because those are objectively worse)
- [x] Directory launch mode with `redox .`
- [x] Visual mode and visual line mode
- [x] Visual block mode
- [x] Basic session-bound undo/redo
- [x] Tree-sitter syntax highlighting for
    - [x] Rust
    - [x] Markdown
    - [x] C/C++
    - [x] Go
    - [x] Lua
    - [x] Python
    - [x] HTML
    - [x] CSS
    - [x] JS/TS
    - [x] JSON
    - [x] TOML
    - [x] YAML
- [x] Smart indenting with Tree-sitter
- [x] Subtle colour column at col=80
- [x] Scope indicator lines and delimiter pair highlighting
- [x] Compound motions (like `daw` or `ci"`)
- [x] Basic local search (`/`, `f`, `F`)
- [x] Performance popup for frame timing
- [x] Toast/status popups for longer messages
- [x] LSP diagnostics, go-to-definition, completion, snippets, and language tool management
- [ ] LSP symbol renaming (project-wide)
- [x] LSP document formatting
- [x] LSP code actions and quick fixes
- [x] Basic git integration (diff stats)
- [x] In-editor splits
- [x] Undo-tree UI
- [x] Custom configuration support
- [x] More extendable leader key system with "whichkey" functionality
- [ ] Grep-based finder for searching text patterns across files
- [ ] A dashboard screen with similar functionality to nvim dashboards
- [ ] More persistent project/session state
- [ ] Broader fuzzy finder indexing and async refresh for very large repositories
- [ ] Editor command palette, making use of the fuzzy matching logic
- [ ] More Vim motions (ongoing, not marking this complete until I've caught them all!)

</details>

## License

Redox is under the terms of the MIT License.
