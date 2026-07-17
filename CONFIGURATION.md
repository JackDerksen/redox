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
modified-key bindings. It also updates Nerd Font icon rendering immediately.

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
| `scrolloff` | non-negative integer | `5` | Keeps this many rows visible above and below the cursor while scrolling. |
| `background_dimming` | number | `0.301` | Popup background dimming from `0.0` (none) through `1.0` (maximum). |
| `undo_tree_history_size` | positive integer | unlimited | Maximum undo records retained per buffer. When full, Redox starts a fresh bounded segment while keeping the latest edit undoable. |
| `color_column` | non-negative integer | `79` | Zero-based text column at which the colour-column background is drawn. |
| `leader` | one-character string | `" "` | Character substituted for `<leader>` in keybindings and built-in leader sequences. |

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
the same mode are rejected.

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
| Files and tools | `open_explorer`, `open_finder`, `toggle_undo_tree`, `toggle_diagnostics`, `code_actions`, `goto_definition`, `symbol_info`, `completion` |
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
