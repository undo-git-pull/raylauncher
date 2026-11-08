# raylauncher

raylauncher is a keyboard launcher for macOS. Press a hotkey, type a few letters, and 
press Enter to open an app, run a shell command, trigger a system action such as Sleep or 
Lock Screen, or copy the answer to a math expression.

It is a single native binary of under 1 MB. It draws its window with AppKit, matches with
[nucleo-matcher](https://crates.io/crates/nucleo-matcher) (a Rust implementation of fzf's
algorithm), and evaluates math in-process. While idle it uses no CPU; on an Apple silicon
Mac running macOS 26 its physical memory footprint measured about 11 MB.

All configuration is plain TOML in `~/.config/raylauncher`, and saving a file applies the
change at once. The folder also contains an `AGENTS.md` that documents every key, so a
coding agent can read and edit the config as reliably as a person can.

## Install

raylauncher needs macOS 11 or later, on Apple silicon or Intel.

### Install script

```sh
curl -fsSL https://github.com/undo-git-pull/raylauncher/releases/latest/download/install.sh | sh
```

The script downloads the latest release, verifies its SHA-256 checksum, copies the binary
to `~/.local/bin/raylauncher`, and registers a LaunchAgent so it starts at login. Set
`RAYLAUNCHER_VERSION=v0.1.0` to install a specific release.

### Homebrew

```sh
brew install undo-git-pull/tap/raylauncher
raylauncher install
```

`raylauncher install` registers the LaunchAgent. After `brew upgrade raylauncher`, run
`raylauncher install` again to restart on the new version.

### Manual download

Download `raylauncher-macos-universal.tar.gz` from the
[releases page](https://github.com/undo-git-pull/raylauncher/releases), extract it, and run
`./raylauncher install`. The binary is not notarized by Apple, so macOS blocks a copy
downloaded through a browser. Clear the quarantine flag first:

```sh
xattr -d com.apple.quarantine raylauncher
```

The install script and Homebrew don't need this step.

### From source

```sh
cargo install --git https://github.com/undo-git-pull/raylauncher
raylauncher install
```

## Use

Press **Option+Space** to open the window.

| Key | Action |
|---|---|
| Up / Down, Tab / Shift-Tab, Ctrl-P / Ctrl-N | Move the selection |
| Enter | Open the app, run the command, or copy the calculator result |
| Cmd-Enter | Reveal the selected app in Finder |
| Esc | Close the window |

Search accepts fzf syntax: `'exact` for an exact match, `^prefix`, `suffix$`, and `!term`
to exclude. Matching ignores case unless the query contains an uppercase letter.

A query that contains an operator and evaluates to a number shows its result as the first
row, for example `12*7+sqrt(16)` or `0xff + 2^10`. Starting the query with `=` forces
calculator mode, so `=pi` works as well. The calculator supports the usual operators,
factorials, implicit multiplication (`2pi`), and common functions such as `sin`, `ln`,
`log`, `round`, `min`, and `max`.

Commands marked `confirm = true`, such as Restart and Shut Down, need a second Enter.

## Configure

The first run creates these files in `~/.config/raylauncher`:

| File | Contents |
|---|---|
| `raylauncher.toml` | Hotkey, shell, editor, window size and theme, search, calculator |
| `commands.toml` | Your own commands |
| `system.toml` | Lock Screen, Sleep, Restart, Shut Down, Log Out, Empty Trash, Toggle Dark Mode, System Settings panes such as Mouse and Displays, and raylauncher's own actions |
| `AGENTS.md` | Reference for every file and key, rewritten on each start |

Every key in `raylauncher.toml` is optional. A missing key falls back to its default, and an
unknown key or wrong type is reported with the key's path and the valid alternatives. If a
saved file is invalid, raylauncher keeps the last valid settings and shows the error as the
first result.

To change the hotkey:

```toml
hotkey = "cmd+space"   # turn off Spotlight's shortcut in System Settings first
```

To add a command:

```toml
# commands.toml
[[command]]
name = "Open Projects"
run = "open ~/dev"
keywords = "code repos"
icon = "sf:folder"
```

`run` is passed to the shell set in `raylauncher.toml` (`/bin/zsh -lc` by default). `icon`
takes an SF Symbol name with an `sf:` prefix, or the path to a file or app whose icon to use.

Launch history is off by default. With `search.history = true`, raylauncher counts launches
in `~/.local/state/raylauncher/history.tsv` and ranks frequently used items higher.

The first time you run an action that scripts another app, such as Empty Trash or Toggle
Dark Mode, macOS asks for permission to control Finder or System Events. raylauncher never
modifies or deletes applications.

## Command line

```
raylauncher              Run in the foreground
raylauncher toggle       Show or hide the window of the running instance
raylauncher check        Validate the config files; exit 1 on error
raylauncher query TEXT   Print the results the window would show for TEXT
raylauncher paths        Print config, state, log, and LaunchAgent locations
raylauncher install      Start at login (LaunchAgent)
raylauncher uninstall    Remove the LaunchAgent and stop raylauncher
raylauncher --version    Print the version
```

`toggle` can be bound in other tools such as skhd or Hammerspoon. `check` and `query` let
scripts and agents verify a config change without opening the window.

If you stop raylauncher with its "Raylauncher: Quit" action, start it again with:

```sh
launchctl kickstart gui/$(id -u)/local.raylauncher
```

## Uninstall

```sh
raylauncher uninstall
rm ~/.local/bin/raylauncher              # or: brew uninstall raylauncher
rm -r ~/.config/raylauncher ~/.local/state/raylauncher   # optional: config and state
```

## Build

```sh
cargo build --release
cargo test
target/release/raylauncher
```

`RAYLAUNCHER_CONFIG_DIR` and `RAYLAUNCHER_STATE_DIR` point a test instance at scratch
folders, so it doesn't touch your real config. A second instance using the same state
folder exits at startup.

### Releasing

1. Set `version` in `Cargo.toml` and commit.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.

   The [release workflow](.github/workflows/release.yml) checks that the tag matches the
   version, runs the tests, builds a universal binary, and publishes a GitHub Release with
   the archive, its checksum, and `install.sh`. It uses no secrets.

3. Update the Homebrew tap by hand, from a local clone of `undo-git-pull/homebrew-tap`:

   ```sh
   scripts/update-formula.sh v0.1.0 ../homebrew-tap
   cd ../homebrew-tap
   git diff
   git add Formula/raylauncher.rb && git commit -m "raylauncher 0.1.0" && git push
   ```

   The script downloads the release archive, checks that it matches the published checksum,
   and writes `Formula/raylauncher.rb` from [the formula template](packaging/homebrew/raylauncher.rb.in).
   It doesn't commit or push. Homebrew users get the new version on their next
   `brew upgrade`.

## License

[MIT](LICENSE)
