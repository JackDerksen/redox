# Redox configuration reference

Every setting is optional. Omitted values use the built-in defaults; add only
the options you want to change.

## Configuration file location

Redox selects one configuration path in this order:

1. The path passed to `redox --config /path/to/config.toml`.
2. The `REDOX_CONFIG` environment variable.
3. `$XDG_CONFIG_HOME/redox/config.toml`, when `XDG_CONFIG_HOME` is a non-empty
   absolute path.
4. `~/.config/redox/config.toml`.

Paths supplied through `--config` or `REDOX_CONFIG` must exist. If the selected
automatic path is missing, Redox uses built-in defaults; it does not search
lower-priority locations or merge multiple files. Empty or relative
`XDG_CONFIG_HOME` values are ignored. If `HOME` is unavailable, the fallback is
`.config/redox/config.toml` beneath the current directory. Unknown fields and
invalid values are rejected at startup so that spelling mistakes do not fail
silently.

Launch with `redox --default-config` to use only the built-in defaults. This
skips configuration files, including paths supplied by `--config` or
`REDOX_CONFIG`, and also applies to CLI help and version styling. The flag can
be combined with a file or directory, for example
`redox --default-config README.md`.

For a compact starting point, try copying
[`config.example.toml`](config.example.toml).

Other environment settings also affect Redox:

| Variable | Behaviour |
| -------- | --------- |
| `SHELL` | Program used by the integrated terminal, in interactive mode; defaults to `/bin/sh` when unset or empty. Its startup files configure its prompt, aliases, and history. |
| `PATH` | Executable search path used by language tools and editor commands that invoke external programs. The integrated terminal inherits it. |
| `NO_COLOR` | A non-empty value disables styled CLI help and version output. |
| `TERM` | `dumb` disables styled CLI help and version output. Redirected CLI output is also plain. |

## Reloading configuration

Run `:config` to open the active configuration file directly. Redox creates its
parent directory when needed, so this also works before the file exists. Run
`:config reload` to apply saved changes without restarting Redox. A successful
reload updates the active theme, all UI and syntax colours, dimming, popup
sizes, line numbering, the colour column, scrolloff, undo-history limits, the
leader, and both character and modified-key bindings. It also updates which-key
behaviour and Nerd Font icon rendering immediately. Zen settings also update
immediately; reloading preserves the current mode unless `zen.enabled` changed
in the configuration. Logging settings also take effect on reload, including
enabling, disabling, and changing the event-history limit.

Animation settings also reload immediately; changing them finishes any active
visual feedback. Mouse settings and automatic update checks also take effect on
reload.

Reloading is transactional: if the file cannot be read or contains an invalid
option, colour, theme, mode, action, or key combination, Redox displays the
error and keeps the active configuration. This makes the command suitable for
iterating on themes while the editor remains open.

If Redox started without a configuration file and one is later created in an
automatic location, `:config reload` discovers it. If an automatically
discovered file is removed, reloading restores the built-in defaults. Paths
supplied using `--config` or `REDOX_CONFIG` remain authoritative and must
continue to exist.

With `--default-config`, `:config reload` restores the built-in defaults without
reading a configuration file. `:config` still opens the usual configuration
path for editing; restart without the flag to load those changes.

## Managed state

The configuration directory contains user-authored configuration only. Redox
stores data that it manages itself under `$XDG_STATE_HOME/redox/`, or
`~/.local/state/redox/` when `XDG_STATE_HOME` is unset, empty, or relative. As
with the configuration directory, a missing `HOME` makes the fallback relative
to the current directory:

```text
~/.config/redox/
└── config.toml

~/.local/state/redox/
├── installed-tools.json
├── pinned-files.json
├── update-check.json
├── logs/                # Created only when logging is enabled
│   ├── recent/          # One bounded JSON Lines file per editor session
│   └── reports/         # Preserved notes and history, never rotated
├── undo-history/
├── sessions/            # Open files and cursor positions, grouped by launch directory
└── legacy/
    └── lsp.json
```

On startup, Redox moves older `installed_lsps.json`, `pinned_files.txt`,
`undo-tree/`, and `lsp.json` entries out of the configuration directory. Files
use kebab-case consistently, and legacy data is preserved rather than deleted.

Language-tool choices are managed through `:lsp list`: enable or install a tool
with `i`, and disable or uninstall it with `u`. Redox saves those choices in
`installed-tools.json`. See the [language tools guide](README.md#language-tools)
for available controls.

## Top-level options

```toml
theme = "default"
icons_enabled = false
text_formatting = true # Set false for plain, unstyled text throughout Redox
mouse = true
mouse_invert_vertical = false
mouse_invert_horizontal = false
mouse_scroll_step_vertical = 3
mouse_scroll_step_horizontal = 3
check_updates = true
scrolloff = 5
background_dimming = 0.301
# undo_tree_history_size = 1000 # Optional limit; unlimited when omitted
color_column = 79 # Renders on top of cl=80
line_numbers = "relative" # Or "absolute"
leader = " "
```

| Option | Type | Default | Description |
| --- | --- | --- | --- |
| `theme` | string | `"default"` | Active built-in or user-defined theme name. |
| `icons_enabled` | boolean | `false` | Enables built-in Nerd Font icons in status modules, file lists, and popup titles. Requires a Nerd Font in the terminal. |
| `text_formatting` | boolean | `true` | Enables font decorations. Set `false` to suppress bold, italic, dim, reverse, strikethrough, and underlines throughout the UI, syntax highlighting, integrated terminal, and CLI output. Colours are retained. |
| `mouse` | boolean | `true` | Enables scrolling, cursor placement, drag selection, popup interaction, and pane resizing. Takes effect on configuration reload. When disabled, terminal mouse capture is released. |
| `mouse_invert_vertical` | boolean | `false` | Reverses the vertical wheel direction reported by the terminal. |
| `mouse_invert_horizontal` | boolean | `false` | Reverses the horizontal wheel direction reported by the terminal. |
| `mouse_scroll_step_vertical` | integer | `3` | Rows per vertical wheel event, from `1` to `65535`. |
| `mouse_scroll_step_horizontal` | integer | `3` | Columns per horizontal wheel event, from `1` to `65535`. |
| `check_updates` | boolean | `true` | Check GitHub for a newer release on startup. Successful checks are cached for 24 hours and require `curl`. `:check-update` performs a manual check at any time. |
| `scrolloff` | non-negative integer | `5` | Keeps this many rows visible above and below the cursor while scrolling. |
| `background_dimming` | number | `0.301` | Dimming for inactive panes, popup backgrounds, and staged Git gutter markers from `0.0` (none) through `1.0` (maximum). |
| `undo_tree_history_size` | positive integer | unlimited | Maximum undo records retained per buffer. When full, Redox starts a fresh bounded segment while keeping the latest edit undoable. |
| `color_column` | non-negative integer | `79` | Zero-based text column at which the colour-column background is drawn. |
| `line_numbers` | string | `"relative"` | `"relative"` shows the distance from the cursor, with the current line's actual number. `"absolute"` shows actual line numbers on every row. Applies to editor panes and the explorer. |
| `leader` | one-character string | `" "` | Character substituted for `<leader>` in keybindings and built-in leader sequences. |

## Animations

Set these options under `[animations]`; all are optional. See
[`config.example.toml`](config.example.toml) for a complete example.

| Option | Default | Behaviour |
| --- | --- | --- |
| `enabled` | `true` | Set `false` to disable all animations. |
| `yank_highlight_ms` | `150` | Sweep a highlight across copied text. |
| `jump_highlight_ms` | `150` | Fade a highlight from the destination line after a jump. |
| `undo_redo_highlight_ms` | `150` | Fade a faint white highlight from the changed text after undo or redo; removed text leaves a cue at the deletion point. |
| `delimiter_blink_ms` | `150` | Blink the matching opening delimiter. |
| `save_confirmation_ms` | `600` | Total lifetime of the save checkmark. |
| `save_fade_ms` | `150` | Final portion of the checkmark's lifetime; capped at `save_confirmation_ms`. |
| `toast_fade_ms` | `150` | Final portion of a timed notification's five-second lifetime; capped at that lifetime. Sticky messages and active recording or loading indicators stay solid. |
| `focus_fade_ms` | `150` | Shared by pane and terminal focus, pane closing, and popup background dimming. |
| `dashboard_logo_ms` | `500` | Entrance for both dashboard sizes. The About logo stays static. |
| `spinner_frame_ms` | `100` | Interval between loading-spinner frames. |
| `rain_fps` | `60` | Rain speed, from `1` to `60` frames per second; `0` disables `:rain`. |

The previous names `yank_ripple_ms`, `jump_pulse_ms`, and `undo_redo_ms` remain
supported as aliases.

Durations use non-negative integer milliseconds. Set a value to `0` to disable
that effect. A zero `save_fade_ms` or `toast_fade_ms` keeps that feedback solid
until expiry; a zero `spinner_frame_ms` leaves a static indicator while loading
continues. A zero `rain_fps` disables `:rain`. Editing, copying, saving, jump
centring, and focus changes still work normally with animations disabled.

Use `:config reload` to apply changes. Active feedback finishes immediately and
subsequent effects use the new timings. Rain adopts its new speed, or stops if
disabled.

## Mouse support

Mouse support is enabled by default. Scroll vertically or horizontally to move
the pane under the pointer without changing keyboard focus. Left-click to place
the cursor or focus a split pane. Drag with the left button to enter
characterwise visual mode; the selection remains active after release and
accepts the usual visual-mode keys. Dragging beyond the pane's edge scrolls as
drag events arrive.

Mouse scrolling moves the page without changing the cursor's document position,
even beyond the viewport edge or `scrolloff` margin. The cursor is hidden
outside the visible area and reappears when scrolled back into view. Resuming
editing while the cursor is off-screen centres the viewport on that position
before applying the edit. Near the start of the document or line, centring stops
at the content boundary.

Scrolling filters small movements on the other axis. Horizontal scrolling starts
after three wheel events in the same direction; switching back to vertical takes
two. An event on the active axis clears the pending switch, and
opposite-direction events cancel each other. After a quarter-second pause,
vertical scrolling responds immediately again. Filtered events are discarded
rather than replayed as a jump.

The terminal's reported down/right wheel events move the viewport down/right by
default. Reverse either axis independently with `mouse_invert_vertical = true`
or `mouse_invert_horizontal = true`. These settings apply on `:config reload`
and reverse the events received from the terminal, which may already reflect OS
scrolling preferences.

Popups use the same mouse setting and scroll directions. Scroll the region under
the pointer, click an entry to select it, and double-click to open or accept it.
Finder pins stay fixed above the scrolling file list; its preview scrolls
independently. Clicking an entry keeps it at the same screen position for a
second click.

| Popup | Mouse behaviour |
| --- | --- |
| Finder | Scroll results or the preview; click a result to select it, double-click to open it, or click the query to position its cursor. |
| Explorer | Scroll the directory, click to position the text cursor, and double-click a file or directory to open it. |
| Pinboard | Click a slot to select it; double-click an occupied slot to open its file. Pin assignment keeps its existing keyboard controls. |
| Completion, diagnostics, code actions | Scroll and select entries; double-click to accept a completion, jump to a diagnostic, or apply an action. Diagnostic details and the actions list scroll separately. |
| Language tools | Scroll and select tools. Installation and removal keep their existing keyboard controls. |
| Symbol information | Scroll the documentation. |
| Command and search | Click to position the input cursor. Wheel events are ignored. |
| Which-key | Click a concrete key to use it; placeholders still require keyboard input. |
| Undo tree | Scroll the history vertically or its preview in either direction; click a history row to select it, or click the file pane to return to editing. Drag its pane dividers to resize the history and preview. Scrolling does not change keyboard focus or restore a revision. Selecting another revision resets the preview's scroll position. |

Left-click outside a popup to dismiss it. For stacked popups, this closes only
the top one, and the click does not also activate the editor underneath. An
Explorer with unsaved directory edits stays open; use `:w` to save or `Esc` to
discard them. About and performance popups also support outside-click dismissal.

When a popup closes during a scroll gesture, Redox ignores the remaining wheel
events until there has been a quarter-second pause. This prevents trackpad
momentum from scrolling the buffer or another popup after dismissal.

Redox ignores right-clicks and does not display a context menu.

Set `mouse = false` and run `:config reload` to release mouse capture so the
terminal can handle selection and its own context menu. Some terminals translate
wheel gestures into arrow-key input while mouse capture is disabled, which moves
the editor cursor instead of scrolling the viewport independently.

## Optional logging

Logging is entirely optional and disabled by default. Its only purpose is to
help reproduce issues. Logs stay on your machine, Redox never uploads them or
sends them to anyone, and you choose whether to share a saved report when
reporting a bug.

```toml
[logging]
enabled = true
max_events = 5000
```

`enabled` defaults to `false`. `max_events` is an optional positive integer and
defaults to `5000`. Each editor session has its own file, containing at most its
latest `max_events` events. When full, the file drops its oldest 10% of events
to make room for new entries, with a minimum of one event removed. This keeps
ordinary appends from rewriting the full history. At the default limit, a full
session therefore retains between 4,501 and 5,000 events as logging continues.
Redox deletes older, closed session files oldest first when the total recent
history exceeds this budget. Pruning runs at startup, at compaction, when saving
a report or reloading configuration, and every 30 seconds while logging is
enabled. The combined history may exceed the budget between these maintenance
runs. Sessions that are still running are protected, so concurrent editors can
each retain up to their own limit. Changing the limit with `:config reload`
immediately trims the current session and prunes older closed sessions as
needed. Disabling logging stops recording and closes that session's log, making
it eligible for later pruning.

Redox records navigation and action keys, pending key sequences, executed
commands and searches, accepted completions, split and buffer changes, file
writes and external changes, terminal resizing, and language-server event
metadata. Events include timestamps, the current mode, numeric buffer and pane
identifiers, and cursor position. Commands and searches record the event and
associated key without their text, including commands executed through
configured bindings. File events identify buffers by number and omit file names
and paths. Accepted buffer completions include their label and inserted text.
Command-line completion records only the acceptance action.

Ordinary buffer typing, pasted buffer text, command drafts, and unfinished
search or finder queries are omitted. This is an action history for
investigating a bug, not a complete recording of file contents or an automatic
replay script. Notes and accepted buffer completions can contain sensitive
information, so review a report before sharing it.

When you notice a bug occurs, run:

```vim
:log "This bug just happened: the split stopped responding"
```

The message may also be unquoted. Quoted messages support JSON escapes such as
`\"` and `\n`. The command saves the note at the top of a new report, followed
by the current session's recent history, and displays its full path. Reports are
independent files in `~/.local/state/redox/logs/reports/`. Redox never
overwrites, rotates, or automatically deletes them, even after a restart or a
change to `max_events`. Delete reports yourself when you no longer need them.

The rolling history lives in `~/.local/state/redox/logs/recent/`. With
`XDG_STATE_HOME` set to an absolute path, both locations move under
`$XDG_STATE_HOME/redox/logs/`. Files use compact UTF-8 JSON Lines, one event per
line, with no compression tool required to read them. Session filenames include
their start timestamp and process ID, with a unique suffix to prevent
collisions. Redox appends events to the same file and atomically replaces it
when trimming its oldest events. Small `.lock` files protect active sessions
from pruning. A process crash may leave an incomplete last event; an
operating-system crash or power loss can also lose recent writes. Saved reports
include the session filename and are flushed to disk before the command reports
success. On Unix, new log directories and files are private to your user.

Press Escape to leave a text-entry mode or dismiss a popup, then `:` to open the
command line. From rain mode, `:` stops the animation and opens the command line
directly. Logging errors appear as editor messages and do not stop editing; a
failed `:log` command reports the failure instead of claiming that a report was
saved.

## Which-key

Which-key lists the valid continuations whenever normal or visual mode is
waiting for more input. This includes character-, line-, and block-visual modes.
It combines the built-in sequence tree with configured bindings for the active
mode, including movements moved onto multi-key sequences. Single-key actions
execute immediately and therefore never open the popup.

```toml
[which_key]
enabled = true
delay_ms = 3000
```

The delay starts with the first key in a pending sequence. Once visible, the
popup remains visible and updates immediately as deeper keys are entered. Set
`delay_ms = 0` to show it immediately, or set `enabled = false` to disable it.

While a normal- or visual-mode sequence is pending, Backspace removes its most
recent key and Escape cancels the sequence. This editing behaviour belongs to
the input system and remains available when the popup is disabled.

### Custom motion and command bindings

Use `[[bind]]` to give a normal- or visual-mode character sequence a custom
description and make it perform either another motion sequence or an editor
command:

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

Each entry must set exactly one of `sequence` or `command`. A `sequence` is
interpreted by the same motion resolver as typed input, so it supports counts
and can invoke another custom motion binding. It must resolve completely to one
or more motions; cycles, incomplete input, and non-motion actions are rejected
when configuration is loaded. A command uses the existing command-line parser
and may optionally include its leading `:`.

| Field | Values | Meaning |
| --- | --- | --- |
| `mode` | `"normal"`, `"visual"`, `"visual_line"`, `"visual_block"` | Mode in which the binding is active. |
| `keys` | Character sequence | Trigger; supports `<leader>`. An empty string disables the entry. |
| `sequence` | Non-empty motion sequence | Motions to replay; supports counts and `<leader>`. Set this or `command`. |
| `command` | Non-empty command string | Command to execute, with or without the leading `:`. Set this or `sequence`. |
| `desc` | Non-empty string | Description used by which-key. |

`keys` is a character sequence and may contain `<leader>`; modified-key tokens
are not supported in custom bindings. Set `keys = ""` to disable an entry
entirely. A disabled entry is ignored before its other fields are validated, so
`[[bind]]` followed by only `keys = ""` is valid. Multi-key entries in normal
and visual modes appear in which-key using their `desc`. Single-key entries
execute immediately and therefore do not open the popup.

## Zen mode

Press `<leader>z`, or run `:zen`, to toggle a centred editor viewport. The toast
says `zen` or `standard`. Remap the `toggle_zen` action in normal or visual
keybindings as needed.

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
| `min_width` | `80` | Minimum editor width in columns, from `1` to `65535`; capped at the terminal width. |
| `hide_gutter` | `true` | Hide line numbers, their padding, and Git markers. |
| `hide_color_column` | `true` | Hide the configured colour column. |
| `focus_scope` | `true` | Keep syntax colours in the innermost multiline scope, including its header and closing line. Other lines use `zen.ghost` and retain their configured syntax formatting. Uses the current line when no scope is available. |
| `hide_diagnostics` | `true` | Hide inline diagnostic text and highlights. Diagnostics continue updating and remain available through `<leader>x`. |
| `minimal_statusline` | `true` | Show the mode, filename without its path, and all normal modules on the right. Popup labels still identify active tools. |
| `show_toast` | `true` | Show `zen` or `standard` when toggled. |

The centred area contains existing splits, the statusline, and popups. Popup
width percentages and minimums in `[popups.<name>]` apply within that area;
their height settings are unchanged. Selections, search matches, and snippet
placeholders remain visible while scope focus is enabled. Indent guides share
the same scope selection in standard and zen mode. Delimiter highlights stay
within that scope and identify its body when the cursor is on its header. Guides
use the delimiter pair's indentation and run only between its opening and
closing lines.

Margins default to a slightly darker version of the editor background. Zen and
standard ghost text both default to the theme's `dark_gray` palette colour.
Override the zen colours per theme:

```toml
[themes.my_theme.ui]
"zen.margin" = "#111112"
"zen.ghost" = "#606079"
```

## Popup sizes

Use `[popups.<name>]` to override the dimensions listed below. Percentage values
must be from `1` through `100`; minimum dimensions use terminal cells and accept
integers from `0` through `65535`. Actual sizes are capped by the available
area. Omitted fields keep their built-in values. The `command_line` popup also
uses `stacked_padding` (from `0` to `65535`, default `0`), the preferred number
of rows between it and another open popup. On very short terminals, this padding
shrinks as needed to preserve both popup bodies.

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
| `undo_tree` | `32` wide, minimum `32` | Accepts `width_percent` and `min_width`, but currently opens using built-in dimensions. See the resizing note below. |

Because `finder`, `diagnostics`, and `code_actions` share one layout style,
configure only one of those aliases when setting their common dimensions.

`height_percent` and `min_height` have no effect on `command_line` or
`undo_tree`. `stacked_padding` has no effect outside `command_line`.

The undo tree's opening width currently ignores configured size overrides.
Resize it by dragging its dividers with `mouse = true`, or use `Ctrl+Left` /
`Ctrl+Right` for width and `Ctrl+Down` / `Ctrl+Up` for height while the history
pane has focus. Its resized width is kept when toggled off and on during the
same editor session; it is not saved to the configuration file.

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
toggle_comments = "gcc"
repeat_last_change = "."

[keybindings.insert]
completion = "<ctrl-shift-k>"

[keybindings.visual]
toggle_comments = "gc"
```

Plain text represents a character sequence in normal and visual modes; other
modes accept a single character. `<leader>` can appear one or more times inside
such a sequence. A modified key is written as one complete angle-bracket token,
such as `<ctrl-w>`, `<ctrl-shift-k>`, or `<shift-enter>`. Modified keys cannot
be mixed into a multi-character sequence. One action can have one configured
assignment per mode.

Modifiers are `ctrl` (also `control`), `alt`, and `shift`; their order and case
are ignored. `+` is accepted instead of `-`, so `<Control+Shift+K>` is
equivalent to `<ctrl-shift-k>`.

Configured bindings take precedence over built-in bindings that use the same
input. Bindings do not remove unrelated defaults. Duplicate bindings and
ambiguous prefixes such as `g` plus `gg` in the same mode are rejected.
Configured multi-key normal-mode bindings are automatically included in the
which-key tree one key at a time.

`visual` bindings also apply in `visual_line` and `visual_block` mode, including
modified keys and `[[bind]]` sequences. A binding for the same key in a more
specific visual mode takes precedence. Prefix validation and which-key hints
include the inherited bindings.

The `[keybindings.<mode>]` tables remap named built-in actions. Use `[[bind]]`
when a key sequence should replay a motion sequence or execute an editor command
with its own which-key description.

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
| History | `undo`, `redo`, `repeat_last_change` |
| Movement | `move_left`, `move_down`, `move_up`, `move_right`, `word_forward`, `word_backward`, `line_start`, `line_end`, `file_start`, `file_end`, `centre_cursor` (`center_cursor` is also accepted), `viewport_down`, `viewport_up` |
| Editing modes | `insert`, `append`, `insert_line_start`, `append_line_end`, `open_line_below`, `open_line_above`, `command`, `search`, `visual`, `visual_line`, `visual_block` |
| Editing and clipboard | `delete_char`, `toggle_comments`, `yank`, `delete`, `paste`, `paste_before`, `yank_system`, `paste_system` |
| Splits | `split_horizontal`, `split_vertical`, `close_split`, `focus_left`, `focus_down`, `focus_up`, `focus_right` |
| Completion | `completion_next`, `completion_previous`, `completion_accept`, `completion_cancel` |
| Finder | `finder_next`, `finder_previous`, `finder_open`, `finder_cancel` |
| Surfaces | `surface_open`, `surface_parent` |
| Pinboard | `pin_next`, `pin_previous`, `pin_open`, `pin_assign`, `pin_delete` |
| LSP marketplace | `marketplace_next`, `marketplace_previous`, `marketplace_install`, `marketplace_uninstall` |
| Diagnostics | `diagnostic_next`, `diagnostic_previous`, `diagnostic_open` |
| Code actions | `code_action_next`, `code_action_previous`, `code_action_apply` |
| Symbol information | `symbol_info_next`, `symbol_info_previous` |

Actions are mode-aware. For example, `yank` expects an active visual selection,
and completion navigation is useful while completion results are visible.

### Comment toggling

`toggle_comments` uses `gcc` in normal mode to toggle the current line and `gc`
in visual mode to toggle all lines touched by the selection. Character-, line-,
and block-visual selections all operate on whole lines. The action uses comment
syntax for the file type, skips blank lines, and records the change as one undo
step. It can be repeated with `.` (`repeat_last_change`). Files without
recognised comment syntax are left unchanged.

```toml
[keybindings.normal]
toggle_comments = "gcc"

[keybindings.visual]
toggle_comments = "gc"
```

Replace either sequence with your preferred keys, then run `:config reload`. A
visual assignment also applies to line and block selections unless that specific
mode overrides the same keys. Custom assignments add bindings; the built-in `gc`
and `gcc` remain available unless another configured action takes over those
keys. Comment toggling uses the named action tables above; `[[bind]].sequence`
accepts motions only.

## Themes

Define any number of themes beneath `[themes.<name>]` and select one with the
top-level `theme` option. A theme has three optional layers:

1. `palette` changes the base colours from which all default roles are derived.
2. `syntax` overrides individual syntax-highlight roles.
3. `ui` overrides individual interface styles.

Use `:colorscheme <name>` to switch themes for the current session without
editing the file. Bare `:colorscheme` reports the active name. A session
override survives `:config reload` while that theme remains defined, but it is
never written back to `config.toml`; restarting Redox returns to the top-level
`theme` setting.

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

Colours use six-digit hexadecimal notation (`#RRGGBB`, with the `#` optional).
`"transparent"` is also accepted, without regard to case. A plain string sets
the foreground, retaining transparent backgrounds and using the active theme
background for other roles. Existing strings and `{ fg, bg }` tables remain
valid and retain the role's default formatting. Which-key text strings retain
the which-key background.

Syntax and UI text roles also accept partial tables. Omitted properties keep
that role's defaults, so `{ italic = false }` changes only italics, and
`{ fg = "#abcdef", bold = true }` changes only the foreground and bold setting.

| Property | Values | Meaning |
| --- | --- | --- |
| `fg`, `bg` | Colour string | Foreground and background. |
| `bold`, `italic`, `dim` | Boolean | Enable or disable the corresponding font attribute. |
| `reverse` | Boolean | Swap the displayed foreground and background. |
| `strikethrough` | Boolean | Draw a line through the text. |
| `underline` | `"none"`, `"single"`, `"curl"` | No underline, a straight underline, or an undercurl. |
| `underline_color` | Colour string | Underline colour; `"transparent"` follows the text colour. |

```toml
text_formatting = true

[themes.default.syntax]
comment = { italic = false }
keyword = { bold = true }
function = { italic = true, underline = "single" }
markdown_heading = { bold = true, underline = "none" }

[themes.default.ui]
"finder.directory" = { bold = false, italic = true }
"finder.pinned" = { italic = false, strikethrough = true }
"dashboard.version" = { italic = false }
"command_line.ghost" = { italic = true, dim = true }
"diagnostic.error_range" = { underline = "curl", underline_color = "#ff8080" }
```

By default, titles, explorer directories, finder directory portions, dashboard
hotkeys, and compact logos are bold. Pinned paths, comments, Markdown emphasis,
and the dashboard version are italic. Markdown strong text is bold; Markdown
headings are bold and underlined. Search matches have a straight underline, and
diagnostic error ranges have an undercurl. Terminal applications retain the
bold, dim, italic, underline, and reverse attributes supported by the terminal
parser.

Set the top-level `text_formatting = false` to disable all font attributes,
overriding every theme role and terminal application. Reload with
`:config reload` to apply changes immediately. CLI help and version output read
the same configuration, including `--config`; redirected output remains plain.
Invalid configuration does not prevent help from being displayed.

Overlapping roles combine their enabled attributes. For example, emphasis inside
a Markdown heading is both bold and italic. Setting a role's `bold = false`
removes its own bold default; it does not cancel bold contributed by an
enclosing role. Diagnostic range underlines take precedence over search
underlines. Setting `diagnostic.error_range.underline` to `"none"` leaves any
search or syntax underline visible. Terminal support determines how undercurl
and underline colours appear.

`finder.directory`, `finder.pinned`, `popup.section_title`, and
`diagnostic.error_range` decorate existing text: transparent foregrounds/
backgrounds inherit the text beneath them. Search roles set match colours and
add formatting; selections retain their existing colour precedence. Finder
matches preserve directory and pinned attributes. `finder.directory` affects
directory portions of paths, while `explorer.directory` affects directory
entries, including hidden directories.

`zen.margin`, `zen.ghost`, and `which_key.background` use a single colour:
provide a colour string or a table with `fg`. Their `bg` value has no effect,
and they reject font properties. Which-key text roles support full styles; their
background defaults to `which_key.background`, and an explicit `bg` overrides
it.

### Base palette keys

- `background` (`bg` is an alias)
- `color_column`
- `scope`
- `selection_bg`, `selection_fg`
- `white`, `black`
- `red`, `green`, `yellow`, `blue`, `purple`, `orange`
- `light_red`, `light_green`, `light_yellow`, `light_blue`, `light_purple`,
  `light_orange`
- `dark_gray`, `mid_gray`, `light_gray`

### Syntax style keys

- Markdown: `markdown_code`, `markdown_emphasis`, `markdown_frontmatter`,
  `markdown_heading`, `markdown_highlight`, `markdown_link`,
  `markdown_list_marker`, `markdown_strong`
- Variables: `variable_builtin`, `variable_parameter`
- Keywords: `keyword`, `keyword_operator`, `keyword_import`
- Types: `type` (`type_name` is an alias), `type_builtin`, `type_definition`
- Functions: `function`, `function_macro`, `function_method`
- Literals: `string`, `string_escape`, `character`, `number`, `boolean`, `float`
- Constants: `constant`, `constant_builtin`, `constant_macro`
- Other tokens: `comment`, `constructor`, `attribute`, `property`, `operator`,
  `punctuation_delimiter`, `punctuation_bracket`, `punctuation_special`

### UI style keys

- Zen mode: `zen.margin`, `zen.ghost`. These use the foreground value as a
  single colour.
- Editor and pane text: `editor.text`, `editor.snippet`, `pane.title`
- Gutter: `gutter.line_number`, `gutter.current_line_number` (also used for
  lines covered by a visual selection)
- Search: `search.match`, `search.current`
- Additional popup headings: `popup.section_title`
- Completion: `completion.ghost`, `completion.keyword`,
  `completion.match_highlight`
- Dashboard: `dashboard.text`, `dashboard.selected`, `dashboard.hotkey`,
  `dashboard.version`, `dashboard.icon`, `dashboard.logo_red`,
  `dashboard.logo_white`, `dashboard.logo_blue`
- Git: `git.added`, `git.modified`, `git.conflict`, `git.removed`
- Status line: `status.bar`, `status.path`, `status.dirty`, `status.saved`,
  `status.mode_normal`, `status.mode_insert`, `status.mode_command`,
  `status.mode_visual`, `status.metadata_wrapper`, `status.metadata_content`,
  `status.language_icon`, `status.coords_wrapper`, `status.coords_content`,
  `status.minimap_wrapper`, `status.minimap_content`, `status.minimap`,
  `status.minimap_alt`
- About: `about.border`, `about.title`, `about.text`, `about.logo_red`,
  `about.logo_white`, `about.logo_blue`
- Command line: `command_line.border`, `command_line.title`,
  `command_line.text`, `command_line.prompt`, `command_line.ghost`,
  `command_line.error`, `command_line.inactive_title`
- Which-key: `which_key.background`, `which_key.edge`, `which_key.prefix`,
  `which_key.key`, `which_key.arrow`, `which_key.text`
- Inline diagnostics: `diagnostic.error`, `diagnostic.warning`,
  `diagnostic.information`, `diagnostic.hint`, `diagnostic.error_range`
- Explorer: `explorer.border`, `explorer.title`, `explorer.file`,
  `explorer.directory`, `explorer.executable`, `explorer.hidden`
- Finder and shared modal lists: `finder.border`, `finder.title`, `finder.text`,
  `finder.prompt`, `finder.query_title`, `finder.dim`, `finder.match_highlight`,
  `finder.selected`, `finder.pinned_bg`, `finder.pinned_marker`,
  `finder.hotkey`, `finder.preview_title`, `finder.preview_path`,
  `finder.directory`, `finder.pinned`
- Performance popup: `perf.border`, `perf.title`, `perf.text`, `perf.label`,
  `perf.value`, `perf.dim`, `perf.good`, `perf.warn`, `perf.hot`, `perf.bar_bg`
- Undo tree: `undo_tree.title`, `undo_tree.text`, `undo_tree.selected`,
  `undo_tree.selected_indicator`, `undo_tree.node`, `undo_tree.node_label`,
  `undo_tree.redo_marker`, `undo_tree.edge`, `undo_tree.timestamp`,
  `undo_tree.preview_title`, `undo_tree.preview_label`,
  `undo_tree.preview_text`, `undo_tree.preview_dim`,
  `undo_tree.preview_separator`, `undo_tree.preview_deleted`,
  `undo_tree.preview_inserted`

About logo roles also style the compact dashboard and CLI logos. The
`dashboard.logo_*` roles control the large dashboard artwork. Finder roles are
shared by the pinboard and several modal lists; `finder.pinned` styles pinned
paths in both the finder and pinboard. Diagnostic severity roles style messages,
while `diagnostic.error_range` styles the corresponding source range.

For status modules, each `*_content` pair controls the text foreground and the
complete module background. The corresponding `*_wrapper` pair styles internal
separators. Half-cell outer edges are derived automatically from `status.bar`
and the content background, so themes do not need to manually invert edge
foreground/background colours.

Palette changes are applied first, followed by syntax and UI overrides. This
means a small theme can replace only the base palette, while a detailed theme
can control every exposed role.

The [`config.example.toml`](config.example.toml) starter includes a complete
Redox port of the Vague Neovim theme covering every palette entry, syntax role,
and UI role.
