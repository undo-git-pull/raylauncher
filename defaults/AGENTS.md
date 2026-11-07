# Raylauncher config guide for agents

This folder configures `raylauncher`, a keyboard launcher for macOS. It opens apps, runs
shell commands, performs system actions, and evaluates math. Raylauncher rewrites this
file on every start so it matches the running version. Edit the `.toml` files instead.

## Rules

- Do not add commands or features that delete, move, rename, or modify applications.
  Raylauncher only reads app folders and must never need the macOS App Management
  permission. To hide an app, use `search.exclude`. To remove one, the user reveals it
  in Finder (Cmd-Enter) and deletes it there.

## How changes take effect

Raylauncher watches this folder with FSEvents. When `raylauncher.toml`, `commands.toml`, or
`system.toml` is saved, it reloads all three files within about 0.2 seconds. There is no
restart or reload command. Symlinked files work, so these files can live in a dotfiles repo.

If a file is invalid, raylauncher keeps running with the last valid config. The error
appears as the first result in the raylauncher window and in the log. To validate after an
edit, run:

```
raylauncher check            # prints "ok: ..." or "error: <file>: <problem>", exit 1 on error
raylauncher query "<text>"   # prints the results the window would show, one per line
raylauncher paths            # config, state, history, log, and LaunchAgent locations
raylauncher toggle           # show or hide the window of the running raylauncher
```

Error messages name the file, the key path, and the valid alternatives, for example
``unknown key `window.widht`. Valid keys here: font_size, max_results, ...``.

## Files

| File | Purpose | Format |
|---|---|---|
| `raylauncher.toml` | Hotkey, shell, editor, window, search, calculator | Tables of fixed keys |
| `commands.toml` | Commands the user adds | List of `[[command]]` |
| `system.toml` | Lock, sleep, restart, and raylauncher actions | List of `[[command]]` |

### raylauncher.toml

Every key is optional. Missing keys use the built-in defaults, which match the comments in
the file as first written. Unknown keys and wrong types are errors. An integer is accepted
where a float is expected.

| Key | Type | Default | Notes |
|---|---|---|---|
| `hotkey` | string | `"alt+space"` | `mod+...+key`. Modifiers: `cmd`, `alt`/`option`, `ctrl`, `shift`. Keys: `a`-`z`, `0`-`9`, `space`, `return`, `tab`, `escape`, `f1`-`f12`, `minus`, `equal`, `comma`, `period`, `slash`, `semicolon`, `quote`, `grave`, `backslash`, `leftbracket`, `rightbracket`. A modifier is required unless the key is `f1`-`f12`. |
| `shell` | string array | `["/bin/zsh", "-lc"]` | The command text is appended as the last argument. |
| `editor` | string | `"open -t"` | Shell command prefix; the file path is appended, quoted. |
| `window.width` | float | `680` | 200-3000 |
| `window.max_results` | integer | `8` | 1-20 |
| `window.font_size` | float | `22` | 8-72. Row height and result text scale from it. |
| `window.top_offset` | float | `0.22` | 0-0.8, fraction of screen height above the window |
| `window.theme` | string | `"system"` | `"system"`, `"dark"`, `"light"` |
| `window.show_icons` | bool | `true` | |
| `window.placeholder` | string | | Hint text in the empty search field |
| `search.app_dirs` | string array | `/Applications`, `/System/Applications`, `/System/Library/CoreServices/Applications`, `~/Applications` | `~` expands. Missing folders are skipped. When two folders contain an app with the same name, the earlier folder wins. |
| `search.scan_depth` | integer | `2` | Folder levels searched below each app dir |
| `search.extra_apps` | string array | `["/System/Library/CoreServices/Finder.app"]` | Paths to individual `.app` bundles to include. Listed before apps from `app_dirs`, so they win name clashes. Missing paths are skipped. |
| `search.exclude` | string array | `[]` | App names to hide, exact match without `.app` |
| `search.prefer_prefix` | bool | `true` | Rank matches at the start of a name higher |
| `search.history` | bool | `false` | Record launches and use them for ranking. Off means nothing is written. |
| `search.history_weight` | float | `8.0` | With history on: bonus = weight * ln(1 + launch count) |
| `search.show_recent` | bool | `true` | With history on: list the most-launched items when the query is empty |
| `calculator.enabled` | bool | `true` | |
| `calculator.precision` | integer | `10` | 0-15 decimal places |

### commands.toml and system.toml

Both files hold a list of `[[command]]` tables with the same keys.

| Key | Required | Type | Notes |
|---|---|---|---|
| `name` | yes | string | Shown and fuzzy-matched. With history on, also the history key (`cmd:<name>`), so renaming resets its count. |
| `run` | yes | string | Shell command run with `shell`, with `$HOME` as the working directory. Stdout is discarded; stderr goes to the log. |
| `keywords` | no | string | Extra words to match. A keyword match scores 75% of a name match. |
| `subtitle` | no | string | Second line. Defaults to `run`. |
| `icon` | no | string | `"sf:<SF Symbol name>"` such as `"sf:terminal"`, or a path to a file or `.app` whose icon is used |
| `confirm` | no | bool | `true` requires Enter twice. Use it for destructive commands. |
| `enabled` | no | bool | `false` hides the entry without deleting it |

`run` values that start with `@` are built-in actions:

| Built-in | Effect |
|---|---|
| `@edit-config` | Opens `raylauncher.toml` with `editor` |
| `@reindex` | Rescans `search.app_dirs` immediately |
| `@quit` | Exits raylauncher. The LaunchAgent starts it again at next login. |

Example:

```toml
[[command]]
name = "Open Project Folder"
run = "open ~/dev"
keywords = "code repos"
icon = "sf:folder"
```

## Search behaviour

Matching uses nucleo-matcher, a Rust implementation of fzf's algorithm. fzf query syntax
works in the search field: `'exact`, `^prefix`, `suffix$`, `!negate`, and space-separated
terms that must all match. Matching is case-insensitive unless the query contains an
uppercase letter.

The calculator result appears first when the query starts with `=`, or when the query
contains an operator and evaluates to a number. A bare number or constant keeps searching
apps. Supported: `+ - * / % ^ ** !`, parentheses, implicit multiplication (`2pi`),
`0x`/`0b`/`0o` literals, `pi e tau phi`, and `sqrt cbrt abs sin cos tan asin acos atan
sinh cosh tanh ln log log2 exp floor ceil round trunc deg rad min max pow hypot`. Angles
are in radians. Enter copies the result.

## Keys in the window

| Key | Action |
|---|---|
| Up / Down, Tab / Shift-Tab, Ctrl-P / Ctrl-N | Move selection |
| Enter | Open, run, or copy the selected result |
| Cmd-Enter | Reveal the selected app in Finder |
| Esc | Close |

## State outside this folder

`raylauncher paths` prints the locations. By default they are under `~/.local/state/raylauncher/`:

- `history.tsv` has one `count<TAB>id` line per launched item, written only while
  `search.history = true`. Delete it to reset ranking.
- `raylauncher.log` is the LaunchAgent log of startup, reloads, and config errors.

`RAYLAUNCHER_CONFIG_DIR` and `RAYLAUNCHER_STATE_DIR` override both folders, which is useful for
testing a config change against a scratch copy.
