# Redox configuration reference

Redox was designed from the ground-up to be pleasant without the need for configuration, but 
it is highly configurable, should you choose to modify the default behaviour or appearance.

## Configuration file location

Redox loads the first applicable path in this order:

1. The path passed to `redox --config /path/to/config.toml`.
2. The `REDOX_CONFIG` environment variable.
3. `$XDG_CONFIG_HOME/redox/config.toml`.
4. `~/.config/redox/config.toml`.

The automatic locations are optional. A path explicitly supplied with `--config`
must exist. Unknown fields and invalid values are rejected at startup so that
spelling mistakes do not fail silently.

For a compact starting point, try copying [`config.example.toml`](config.example.toml).

## Reloading configuration

Run `:config` to open the active configuration file directly. Redox creates its parent directory
when needed, so this also works before the file exists. Run `:config reload` to apply saved changes
without restarting Redox. A successful reload updates the active theme, all UI and syntax colours,
dimming, popup sizes, the colour column, undo-history limits, the leader, and both character and
modified-key bindings. It also updates which-key behaviour and Nerd Font icon rendering
immediately. Zen settings also update immediately; reloading preserves the current mode unless
`zen.enabled` changed in the configuration.
Logging settings also take effect on reload, including enabling, disabling, and changing the
event-history limit.

Reloading is transactional: if the file cannot be read or contains an invalid option, colour,
theme, mode, action, or key combination, Redox displays the error and keeps the active
configuration. This makes the command suitable for iterating on themes while the editor remains
open.

If Redox started without a configuration file and one is later created in an automatic location,
`:config reload` discovers it. If an automatically discovered file is removed, reloading restores
the built-in defaults. A path supplied using `--config` remains authoritative and must continue to
exist.

## Managed state

The configuration directory contains user-authored configuration only. Redox stores data that it
manages itself under `$XDG_STATE_HOME/redox/`, or `~/.local/state/redox/` when
`XDG_STATE_HOME` is unset:

```text
~/.config/redox/
└── config.toml

~/.local/state/redox/
├── installed-tools.json
├── pinned-files.json
├── logs/                # Created only when logging is enabled
│   ├── recent/          # One bounded JSON Lines file per editor session
│   └── reports/         # Preserved notes and history, never rotated
├── undo-history/
└── legacy/
    └── lsp.json
```

On startup, Redox moves older `installed_lsps.json`, `pinned_files.txt`, `undo-tree/`, and
`lsp.json` entries out of the configuration directory. Files use kebab-case consistently, and
legacy data is preserved rather than deleted.

## Top-level options

```toml
theme = "default"
icons_enabled = false
check_updates = true
scrolloff = 5 
background_dimming = 0.5
undo_tree_history_size = 1000
color_column = 79 # Renders on top of cl=80
leader = " "
```

| Option | Type | Default | Description |
| --- | --- | --- | --- |
| `theme` | string | `"default"` | Active built-in or user-defined theme name. |
| `icons_enabled` | boolean | `false` | Enables built-in Nerd Font icons in status modules, file lists, and popup titles. Requires a Nerd Font in the terminal. |
| `check_updates` | boolean | `true` | Check GitHub for a newer release on startup. Successful checks are cached for 24 hours and require `curl`. `:check-update` performs a manual check at any time. |
| `scrolloff` | non-negative integer | `5` | Keeps this many rows visible above and below the cursor while scrolling. |
| `background_dimming` | number | `0.301` | Popup background dimming from `0.0` (none) through `1.0` (maximum). |
| `undo_tree_history_size` | positive integer | unlimited | Maximum undo records retained per buffer. When full, Redox starts a fresh bounded segment while keeping the latest edit undoable. |
| `color_column` | non-negative integer | `79` | Zero-based text column at which the colour-column background is drawn. |
| `leader` | one-character string | `" "` | Character substituted for `<leader>` in keybindings and built-in leader sequences. |

## Optional logging

Logging is entirely optional and disabled by default. Its only purpose is to help reproduce
issues. Logs stay on your machine, Redox never uploads them or sends them to anyone, and
you choose whether to share a saved report when reporting a bug.

```toml
[logging]
enabled = true
max_events = 5000
```

`enabled` defaults to `false`. `max_events` is an optional positive integer and defaults to
`5000`. Each editor session has its own file, containing at most its latest `max_events` events.
When full, the file drops its oldest 10% of events to make room for new entries, with a minimum
of one event removed. This keeps ordinary appends from rewriting the full history. At the default
limit, a full session therefore retains between 4,501 and 5,000 events as logging continues.
Redox deletes older, closed session files oldest first when the total recent history exceeds this
budget. Pruning runs at startup, at compaction, when saving a report or reloading configuration,
and every 30 seconds while logging is enabled. The combined history may exceed the budget between
these maintenance runs. Sessions that are still running are protected, so concurrent editors can each retain up
to their own limit. Changing the limit with `:config reload` immediately trims the current session
and prunes older closed sessions as needed. Disabling logging stops recording and closes that
session's log, making it eligible for later pruning.

Redox records navigation and action keys, pending key sequences, executed commands and searches, accepted
completions, split and buffer changes, file writes and external changes, terminal resizing, and
language-server event metadata. Events include timestamps, the current mode,
numeric buffer and pane identifiers, and cursor position. Commands and searches record the event
and associated key without their text, including commands executed through configured bindings.
File events identify buffers by number and omit file names and paths. Accepted buffer completions
include their label and inserted text. Command-line completion records only the acceptance action.

Ordinary buffer typing, pasted buffer text, command drafts, and unfinished search or finder
queries are omitted. This is an action history for investigating a bug, not a complete recording of
file contents or an automatic replay script. Notes and accepted buffer completions can contain
sensitive information, so review a report before sharing it.

When you notice a bug occurs, run:

```vim
:log "This bug just happened: the split stopped responding"
```

The message may also be unquoted. Quoted messages support JSON escapes such as `\"` and `\n`.
The command saves the note at the top of a new report, followed by the current session's recent history,
and displays its full path. Reports are independent files in `~/.local/state/redox/logs/reports/`.
Redox never overwrites, rotates, or automatically deletes them, even after a restart or a change
to `max_events`. Delete reports yourself when you no longer need them.

The rolling history lives in `~/.local/state/redox/logs/recent/`. With `XDG_STATE_HOME` set to an
absolute path, both locations move under `$XDG_STATE_HOME/redox/logs/`. Files use compact UTF-8
JSON Lines, one event per line, with no compression tool required to read them. Session filenames
include their start timestamp and process ID, with a unique suffix to prevent collisions. Redox
appends events to the same file and atomically replaces it when trimming its oldest events.
Small `.lock` files protect active sessions from pruning. A process crash may leave an incomplete
last event; an operating-system crash or power loss can also lose recent writes. Saved reports
include the session filename and are flushed to disk before the command reports success.
On Unix, new log directories and files are private to your user.

Press Escape to leave a text-entry mode or dismiss a popup, then `:` to open the command line.
From rain mode, `:` stops the animation and opens the command line directly. Logging errors appear
as editor messages and do not stop editing; a failed `:log` command reports the failure instead of
claiming that a report was saved.

## Which-key

Which-key lists the valid continuations whenever normal or visual mode is waiting for more input.
This includes character-, line-, and block-visual modes. It combines the built-in sequence tree
with configured bindings for the active mode, including movements moved onto multi-key sequences.
Single-key actions execute immediately and therefore never open the popup.

```toml
[which_key]
enabled = true
delay_ms = 3000
```

The delay starts with the first key in a pending sequence. Once visible, the popup remains visible
and updates immediately as deeper keys are entered. Set `delay_ms = 0` to show it immediately, or
set `enabled = false` to disable it.

While a normal- or visual-mode sequence is pending, Backspace removes its most recent key and
Escape cancels the sequence. This editing behaviour belongs to the input system and remains
available when the popup is disabled.

### Custom motion and command bindings

Use `[[bind]]` to give a normal- or visual-mode character sequence a custom description
and make it perform either another motion sequence or an editor command:

```toml
[[bind]]
mode = "normal"
keys = "<leader>j"
sequence = "10j"
desc = "Move down 10 lines"

[[bind]]
mode = "normal"
keys = "<leader>w"
command = "w"
desc = "Write current file"
```

Each entry must set exactly one of `sequence` or `command`. A `sequence` is interpreted by the same
motion resolver as typed input, so it supports counts and can invoke another custom motion binding.
It must resolve completely to one or more motions; cycles, incomplete input, and non-motion actions
are rejected when configuration is loaded. A command uses the existing command-line parser and may
optionally include its leading `:`.

`keys` is a character sequence and may contain `<leader>`; modified-key tokens are not supported in
custom bindings. Set `keys = ""` to disable an entry entirely. A disabled entry is ignored before
its other fields are validated, so `[[bind]]` followed by only `keys = ""` is valid. Multi-key
entries in normal and visual modes appear in which-key using their `desc`. Single-key entries
execute immediately and therefore do not open the popup.

## Zen mode

Press `<leader>z`, or run `:zen`, to toggle a centred editor viewport. The toast says `zen` or
`standard`. Remap the `toggle_zen` action in normal or visual keybindings as needed.

```toml
[zen]
enabled = false
width_percent = 80
min_width = 80
hide_gutter = true
hide_color_column = true
focus_scope = true
hide_diagnostics = true
minimal_statusline = true
show_toast = true
```

| Option | Default | Behaviour |
| --- | --- | --- |
| `enabled` | `false` | Start in zen mode. Toggling during a session does not edit the configuration. |
| `width_percent` | `80` | Percentage of terminal columns used by the centred editor, from 1 to 100. Use 100 to keep full width. |
| `min_width` | `80` | Minimum editor width in columns. Must be positive; capped at the terminal width. |
| `hide_gutter` | `true` | Hide line numbers, their padding, and Git markers. |
| `hide_color_column` | `true` | Hide the configured colour column. |
| `focus_scope` | `true` | Keep syntax colours in the innermost multiline scope, including its header and closing line. Other lines use `zen.ghost`. Uses the current line when no scope is available. |
| `hide_diagnostics` | `true` | Hide inline diagnostic text and highlights. Diagnostics continue updating and remain available through `<leader>x`. |
| `minimal_statusline` | `true` | Show the mode, filename without its path, and all normal modules on the right. Popup labels still identify active tools. |
| `show_toast` | `true` | Show `zen` or `standard` when toggled. |

The centred area contains existing splits, the statusline, and popups. Popup width percentages
and minimums in `[popups.<name>]` apply within that area; their height settings are unchanged.
Selections, search matches, and snippet placeholders remain visible while scope focus is enabled.
Indent guides share the same scope selection in standard and zen mode. Delimiter highlights stay
within that scope and identify its body when the cursor is on its header.
Guides use the delimiter pair's indentation and run only between its opening and closing lines.

Margins default to a slightly darker version of the editor background. Zen and standard ghost text
both default to the theme's `dark_gray` palette colour. Override the zen colours per theme:

```toml
[themes.my_theme.ui]
"zen.margin" = "#111112"
"zen.ghost" = "#606079"
```

## Popup sizes

Popup sections accept `width_percent`, `height_percent`, `min_width`, and `min_height`. Percentage
values must be from `1` through `100`; minimum dimensions use terminal cells. The `command_line`
popup also accepts `stacked_padding`, the preferred number of rows between it and another open
popup. On very short terminals, this padding shrinks as needed to preserve both popup bodies.

```toml
[popups.finder]
width_percent = 75
height_percent = 70
min_width = 60
min_height = 16
```

| Popup name | Built-in size (`width% × height%`, minimum) | Notes |
| --- | --- | --- |
| `about` | `65 × 52`, `52 × 12` | Supports all four size fields. |
| `explorer` | `65 × 60`, `20 × 6` | Supports all four size fields. |
| `finder` | `65 × 60`, `52 × 14` | Also controls the diagnostics and code-actions layouts. |
| `diagnostics` | Finder sizing | Alias for the shared finder-style layout. |
| `code_actions` | Finder sizing | Alias for the shared finder-style layout. |
| `lsp_marketplace` | `65 × 60`, `52 × 12` | Supports all four size fields. |
| `perf` | `44 × 34`, `40 × 12` | Supports all four size fields. |
| `command_line` | `65` wide, minimum `24` | Uses `width_percent` and `min_width`; `stacked_padding` defaults to `0`; its content is always one row high. |
| `undo_tree` | `32` wide, minimum `32` | Uses `width_percent` and `min_width`; it is a pane rather than a modal popup. |

Because `finder`, `diagnostics`, and `code_actions` share one layout style,
configure only one of those aliases when setting their common dimensions.

## Keybindings

Keybindings are grouped by editor mode. Each entry places the action first and
assigns its key or character sequence as the value:

```toml
leader = ","

[keybindings.normal]
open_finder = "<leader><leader>"
open_explorer = "<leader>e"
goto_definition = "gd"
close_split = "<ctrl-w>"

[keybindings.insert]
completion = "<ctrl-shift-k>"
```

Plain text represents a character sequence in normal and visual modes; other modes accept a single
character. `<leader>` can appear one or more times inside such a sequence. A modified key is written
as one complete angle-bracket token, such as `<ctrl-w>`,
`<ctrl-shift-k>`, or `<shift-enter>`. Modified keys cannot be mixed into a multi-character
sequence. One action can have one configured assignment per mode.

Configured bindings take precedence over built-in bindings that use the same input. Bindings do
not remove unrelated defaults. Duplicate bindings and ambiguous prefixes such as `g` plus `gg` in
the same mode are rejected. Configured multi-key normal-mode bindings are automatically included in
the which-key tree one key at a time.

`visual` bindings also apply in `visual_line` and `visual_block` mode, including
modified keys and `[[bind]]` sequences. A binding for the same key in a more specific
visual mode takes precedence. Prefix validation and which-key hints include the
inherited bindings.

The `[keybindings.<mode>]` tables remap named built-in actions. Use `[[bind]]` when a key
sequence should replay a motion sequence or execute an editor command with its own which-key
description.

### Keybinding modes

- `normal`
- `insert`
- `command`
- `search`
- `finder`
- `pin_select`
- `lsp_marketplace`
- `diagnostics`
- `code_actions`
- `symbol_info`
- `visual`
- `visual_line`
- `visual_block`

### Keybinding actions

| Category | Actions |
| --- | --- |
| Files and tools | `open_explorer`, `open_finder`, `toggle_undo_tree`, `toggle_zen`, `toggle_diagnostics`, `code_actions`, `goto_definition`, `symbol_info`, `completion` |
| History | `undo`, `redo` |
| Movement | `move_left`, `move_down`, `move_up`, `move_right`, `word_forward`, `word_backward`, `line_start`, `line_end`, `file_start`, `file_end`, `centre_cursor` (`center_cursor` is also accepted), `viewport_down`, `viewport_up` |
| Editing modes | `insert`, `append`, `insert_line_start`, `append_line_end`, `open_line_below`, `open_line_above`, `command`, `search`, `visual`, `visual_line`, `visual_block` |
| Editing and clipboard | `delete_char`, `yank`, `delete`, `paste`, `paste_before`, `yank_system`, `paste_system` |
| Splits | `split_horizontal`, `split_vertical`, `close_split`, `focus_left`, `focus_down`, `focus_up`, `focus_right` |
| Completion | `completion_next`, `completion_previous`, `completion_accept`, `completion_cancel` |
| Finder | `finder_next`, `finder_previous`, `finder_open`, `finder_cancel` |
| Surfaces | `surface_open`, `surface_parent` |
| Pinboard | `pin_next`, `pin_previous`, `pin_open`, `pin_assign`, `pin_delete` |
| LSP marketplace | `marketplace_next`, `marketplace_previous`, `marketplace_install`, `marketplace_uninstall` |
| Diagnostics | `diagnostic_next`, `diagnostic_previous`, `diagnostic_open` |
| Code actions | `code_action_next`, `code_action_previous`, `code_action_apply` |
| Symbol information | `symbol_info_next`, `symbol_info_previous` |

Actions are mode-aware. For example, `yank` expects an active visual selection, and completion
navigation is useful while completion results are visible.

## Themes

Define any number of themes beneath `[themes.<name>]` and select one with the top-level `theme`
option. A theme has three optional layers:

1. `palette` changes the base colours from which all default roles are derived.
2. `syntax` overrides individual syntax-highlight roles.
3. `ui` overrides individual interface colour pairs.

Use `:colorscheme <name>` to switch themes for the current session without editing the file. Bare
`:colorscheme` reports the active name. A session override survives `:config reload` while that
theme remains defined, but it is never written back to `config.toml`; restarting Redox returns to
the top-level `theme` setting.

```toml
theme = "paper"

[themes.paper.palette]
background = "#faf8f2"
white = "#25211d"
blue = "#356a8a"

[themes.paper.syntax]
keyword = "#a43b3b"
markdown_highlight = { fg = "#25211d", bg = "#d8e8b8" }

[themes.paper.ui]
"finder.selected" = { fg = "#25211d", bg = "#e8e2d7" }
```

Colours use six-digit hexadecimal notation (`#RRGGBB`). `"transparent"` is also accepted. A plain
string sets the foreground and inherits the active theme background. Use `{ fg = ..., bg = ... }`
to control both sides of a syntax or UI role.

Which-key roles are individual colours rather than foreground/background pairs, so plain colour
strings are recommended for all six roles; `which_key.background` controls the popup fill directly.

### Base palette keys

- `background` (`bg` is an alias)
- `color_column`
- `scope`
- `selection_bg`, `selection_fg`
- `white`, `black`
- `red`, `green`, `yellow`, `blue`, `purple`, `orange`
- `light_red`, `light_green`, `light_yellow`, `light_blue`, `light_purple`, `light_orange`
- `dark_gray`, `mid_gray`, `light_gray`

### Syntax colour keys

- Markdown: `markdown_code`, `markdown_emphasis`, `markdown_frontmatter`, `markdown_heading`,
  `markdown_highlight`, `markdown_link`, `markdown_list_marker`, `markdown_strong`
- Variables: `variable_builtin`, `variable_parameter`
- Keywords: `keyword`, `keyword_operator`, `keyword_import`
- Types: `type` (`type_name` is an alias), `type_builtin`, `type_definition`
- Functions: `function`, `function_macro`, `function_method`
- Literals: `string`, `string_escape`, `character`, `number`, `boolean`, `float`
- Constants: `constant`, `constant_builtin`, `constant_macro`
- Other tokens: `comment`, `constructor`, `attribute`, `property`, `operator`,
  `punctuation_delimiter`, `punctuation_bracket`, `punctuation_special`

### UI colour keys

- Zen mode: `zen.margin`, `zen.ghost`. These use the foreground value as a single colour.

- Git: `git.added`, `git.modified`, `git.conflict`, `git.removed`
- Status line: `status.bar`, `status.path`, `status.dirty`, `status.mode_normal`,
  `status.mode_insert`, `status.mode_command`, `status.mode_visual`,
  `status.metadata_wrapper`, `status.metadata_content`, `status.coords_wrapper`,
  `status.coords_content`, `status.minimap_wrapper`, `status.minimap_content`, `status.minimap`,
  `status.minimap_alt`
- About: `about.border`, `about.title`, `about.text`, `about.logo_red`, `about.logo_white`,
  `about.logo_blue`
- Command line: `command_line.border`, `command_line.title`, `command_line.text`,
  `command_line.prompt`
- Which-key: `which_key.background`, `which_key.edge`, `which_key.prefix`, `which_key.key`,
  `which_key.arrow`, `which_key.text`
- Inline diagnostics: `diagnostic.error`, `diagnostic.warning`, `diagnostic.information`,
  `diagnostic.hint`
- Explorer: `explorer.border`, `explorer.title`, `explorer.file`, `explorer.directory`,
  `explorer.executable`, `explorer.hidden`
- Finder and shared modal lists: `finder.border`, `finder.title`, `finder.text`, `finder.prompt`,
  `finder.query_title`, `finder.dim`, `finder.match_highlight`, `finder.selected`,
  `finder.pinned_bg`, `finder.pinned_marker`, `finder.hotkey`, `finder.preview_title`,
  `finder.preview_path`
- Performance popup: `perf.border`, `perf.title`, `perf.text`, `perf.label`, `perf.value`,
  `perf.dim`, `perf.good`, `perf.warn`, `perf.hot`, `perf.bar_bg`
- Undo tree: `undo_tree.title`, `undo_tree.text`, `undo_tree.selected`, `undo_tree.selected_indicator`,
  `undo_tree.node`, `undo_tree.node_label`, `undo_tree.redo_marker`, `undo_tree.edge`,
  `undo_tree.timestamp`, `undo_tree.preview_title`, `undo_tree.preview_label`,
  `undo_tree.preview_text`, `undo_tree.preview_dim`, `undo_tree.preview_separator`,
  `undo_tree.preview_deleted`, `undo_tree.preview_inserted`

For status modules, each `*_content` pair controls the text foreground and the complete module
background. The corresponding `*_wrapper` pair styles internal separators. Half-cell outer edges
are derived automatically from `status.bar` and the content background, so themes do not need to
manually invert edge foreground/background colours.

Palette changes are applied first, followed by syntax and UI overrides. This means a small theme
can replace only the base palette, while a detailed theme can control every exposed role.

The repository also includes [`vague-theme.toml`](vague-theme.toml), a complete Redox port of the
Vague Neovim theme covering every palette entry, syntax role, and UI role.
