# Herdr Yank

A Herdr plugin that copies visible terminal tokens or opens visible URLs and files with short keyboard hints, inspired by `tmux-fingers`.

Invoke an action while a pane is focused, then type the hint shown over the item you want. `yank` copies to the clipboard, and `open-url` opens an `http` or `https` URL in the default browser, or a local file in the default application of its type. Escape or Ctrl-C cancels.

![Herdr Yank picker](docs/images/yank-picker.png)

> This is a fork of [rmarganti/herdr-pluck](https://github.com/rmarganti/herdr-pluck). See [Differences from upstream](#differences-from-upstream).

## Requirements

- Herdr 0.7.4 or newer
- For release installs: `curl` or `wget`, plus `shasum` or `sha256sum`
- Rust 1.85 or newer, only for source builds
- Optional clipboard command (`pbcopy`, `wl-copy`, `xclip`, or `xsel`), used only when an OSC 52 copy is not possible
- `open` (macOS) or `xdg-open` (Linux) for `open-url`

## Install

```bash
herdr plugin install pgilad/herdr-yank
```

Releases include prebuilt binaries for `aarch64-apple-darwin` and `x86_64-unknown-linux-musl`. On other systems, or when no asset matches the version in `herdr-plugin.toml`, the install builds from source with Cargo.

From a checkout, install the binary before you link the plugin, because `herdr plugin link` does not run build commands:

```bash
./scripts/install-binary.sh                                  # downloads the prebuilt binary
HERDR_YANK_BUILD_FROM_SOURCE=1 ./scripts/install-binary.sh   # builds the checked-out source
herdr plugin link .
```

Run the install script again after each update of the checkout.

Releases from the public repository carry signed build provenance. To verify a downloaded archive:

```bash
gh attestation verify herdr-yank-v1.0.0-aarch64-apple-darwin.tar.gz --repo pgilad/herdr-yank
```

## Keybinding

Add bindings to your Herdr config, then run `herdr server reload-config`:

```toml
[[keys.command]]
key = "prefix+q"
type = "plugin_action"
command = "pgilad.herdr-yank.yank"
description = "yank visible token"

[[keys.command]]
key = "prefix+o"
type = "plugin_action"
command = "pgilad.herdr-yank.open-url"
description = "open visible URL or file"
```

## Usage

Invoke the action to open a picker tab that mirrors the source layout, then type the hint for the token you want. Hints are one character for 26 or fewer matches, two up to 676, and three beyond that. After the first character of a longer hint, only the hints that start with it stay visible. Backspace clears a mistyped prefix. Identical text always gets the same hint.

Press `Tab` in the yank picker for multi-select. Hints then toggle instead of copy, and `Tab` again or `Enter` copies all selections joined by newlines, in the order you picked them. The status line uses the lowest row that shows no match, so it does not hide a hint. Multi-select is not available for `open-url`.

`open-url` also gives a hint to an absolute path, such as `/tmp/report.html`, or a `~/` path, such as `~/repos/app/coverage/index.html`, when the path is an existing file of type `html`, `htm`, `pdf`, `svg`, `png`, `jpg`, `jpeg`, `gif`, or `webp`. Other file types do not get a hint, because `open` and `xdg-open` give a file to the default application of its type, and for a script or an app that application runs it. A relative path does not get a hint, because the picker does not know the working directory of the pane. For a symlink, the type of the real file applies, and `open` gets the path of the real file.

If nothing matches, Herdr Yank shows a notification and does not open a picker tab. If a copy or URL launch fails, Herdr Yank shows the error as a notification. When Herdr does not show notifications, the error stays in the picker tab until you press a key.

You can also invoke the actions directly:

```bash
herdr plugin action invoke pgilad.herdr-yank.yank
herdr plugin action invoke pgilad.herdr-yank.open-url
```

## Configuration

```bash
$EDITOR "$(herdr plugin config-dir pgilad.herdr-yank)/config.toml"
```

Herdr Yank shows a notification when the config has a problem, such as an unknown key, a value of the wrong type, an unknown color, or an invalid regex. It ignores only that key or entry, and the other settings still apply. `herdr plugin log` lists every problem.

### Patterns

Built-in patterns, in priority order: `url`, `git-status`, `git-status-branch`, `diff`, `kubernetes`, `path`, `uuid`, `kubernetes-pod`, `git-sha`, `hex`, `ipv4`, `number`. Omit `enabled_builtin_patterns` to keep all of them, or list the ones you want:

```toml
enabled_builtin_patterns = ["url", "path", "git-sha", "git-status", "diff"]
```

Dropping `number` and `path` is the quickest way to cut hint noise on busy panes.

`url` stops at quotes, backticks, and angle brackets, and drops trailing punctuation and Markdown emphasis, so a URL in Markdown output copies without the markup. Balanced parentheses stay in a URL, and a Markdown link `[text](target)` gives its target as a separate match. `kubernetes` matches `kind/name` references such as `pod/web-1`; a plain word such as `node` or a path segment such as `src/service/api.rs` does not match.

Add your own patterns with Rust regex syntax. Lower `priority` wins overlaps and defaults to `25`. A named capture called `match` limits the copy to that capture — the example below copies only `abc123` from `trace_id=abc123`:

```toml
[[patterns]]
name = "trace-id"
regex = "trace_id=(?<match>[A-Za-z0-9_-]+)"
priority = 25
```

Project-local `.herdr-yank.toml` files, searched from the pane working directory up to the Git root, are **off by default** in this fork:

```toml
[project]
patterns = true
pattern_files = [".herdr-yank.toml"]
```

A project file holds only `[[patterns]]`, and Herdr Yank reports any other key in it as a problem. Only enable this for repositories you trust. A `.herdr-yank.toml` is attacker-controlled whenever the checkout is, and a `priority = 1` pattern in it outranks every built-in, so it can change which substring a hint copies. Nothing in the file is executed. For equal priorities, precedence is project-local, then global, then built-in.

### Clipboard

```toml
clipboard = "auto"  # "auto" (default) | "tool" | "osc52"
```

`auto` copies with an OSC 52 escape. Herdr sends it to the clipboard of the attached client, so the text reaches your computer also when Herdr runs on a remote host. If OSC 52 is not possible, `auto` falls back to a clipboard command (`pbcopy`, `wl-copy`, `xclip`, or `xsel`) on the computer that runs the Herdr server. `tool` uses only a clipboard command, and `osc52` uses only OSC 52.

An OSC 52 copy holds at most 196,602 bytes, the largest text that every supported Herdr version forwards. In `osc52` mode a longer selection is an error, not a partial copy. When the Herdr client itself runs over SSH, the outer terminal can have a lower limit.

### Colors

Omitted slots keep their default. Values accept the sixteen standard terminal color names, `#RRGGBB` literals, and `ansi:N` for a 256-color palette entry.

```toml
[style]
unmatched_fg = "dark-grey"
match_fg = "yellow"
match_bg = "#202020"
hint_fg = "black"
hint_bg = "cyan"
selected_match_fg = "white"
selected_match_bg = "dark-magenta"
selected_hint_fg = "white"
selected_hint_bg = "magenta"
status_fg = "black"
status_bg = "grey"
```

## Differences from upstream

- Project-local `.herdr-yank.toml` patterns are opt-in instead of on by default, and a project file holds only `[[patterns]]`.
- `Tab` multi-select copies several matches at once, joined by newlines.
- Backspace clears a mistyped hint prefix, and a typed prefix hides the hints that do not start with it.
- Hints widen to three characters past 676 matches instead of dropping the extra ones.
- An empty result shows a notification instead of a dead-end picker tab, and a failed copy or launch shows a notification instead of a message that disappears with the picker tab.
- The clipboard prefers OSC 52, so `pbcopy`/`xclip`/`wl-copy` are optional and a copy reaches your computer also with remote attach. Clipboard commands run detached from the picker pane, so `wl-copy` and `xclip` keep the selection after the pane closes.
- `enabled_builtin_patterns`, `clipboard`, and `[style]` are new keys. A config problem shows a notification, and the valid keys still apply.
- `git-sha` matches 7–128 hex characters, so SHA-256 and SHA-512 digests select as one token.
- `url` handles Markdown and parentheses, and `kubernetes` needs a `kind/name` reference.
- `open-url` opens only `http` and `https` URLs, and absolute or `~/` paths to existing web pages, PDFs, and images. `open` and `xdg-open` give a file to the default application of its type, which can start a program or a script, thus other file types and `file://` URLs stay refused. Use `yank` to copy them.
- The picker gets the content rows of the pane from Herdr and fits its output to its real terminal size, so non-default `ui.pane_borders`, `ui.pane_outer_borders`, `ui.pane_gaps`, `ui.pane_scrollbars`, and `ui.hide_tab_bar_when_single_tab` settings do not break it.
- Hardening: the temp pane snapshot uses mode `0600` and `O_EXCL` and is deleted at read time, the URL launcher re-checks the scheme allow-list and the file type of the real file before it spawns `open`/`xdg-open`, and a copy or launch failure reports the size of the selection rather than its text, so a yanked secret never reaches a notification or `herdr plugin log`.
- `install-binary.sh` checks the published `.sha256` before it extracts the archive, and releases carry signed build provenance for `gh attestation verify`.

## Troubleshooting

- The action does not appear: run `herdr plugin link .`, then `herdr plugin action list --plugin pgilad.herdr-yank`.
- The action runs an old version: run `./scripts/install-binary.sh` again, because `herdr plugin link` does not build.
- Checksum mismatch: the install refuses the prebuilt binary on purpose and builds from source. Do not work around it.
- Copy fails: the notification shows the reason. `clipboard = "tool"` needs a clipboard command, and `auto` needs a usable `/dev/tty` or a clipboard command.
- Config problems: the notification shows the first problem, and `herdr plugin log` lists all of them.
- The launcher writes its errors and suppressed notifications to `herdr plugin log`. Herdr does not log the picker tab, thus picker errors show as notifications.
