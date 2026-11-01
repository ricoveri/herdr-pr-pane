# herdr-pr-pane

A [herdr](https://herdr.dev) plugin that opens a side pane showing the GitHub
pull request for the branch you're working on: status, checks and comments.

- **Header:** PR number, state, title, base branch and last update.
- **Checks:** collapsible list with pass/fail/pending/skipped state.
- **Comments:** conversation feed with `All` / `Humans` / `Bots` filters,
  folded previews you can expand.
- Shows the PR for the pane that was focused when it opened.
- Refreshes every 45 seconds, or on demand.

## Requirements

- herdr >= 0.9.0 (macOS or Linux)
- [GitHub CLI](https://cli.github.com) (`gh`), authenticated (`gh auth login`)
- Rust toolchain, to build

## Install

```sh
git clone https://github.com/ricoveri/herdr-pr-pane
cd herdr-pr-pane
cargo build --release
herdr plugin link .
```

`herdr plugin link` never runs build steps, so build first and rebuild after
every change. Running panes keep the old binary: close and reopen them.

## Usage

Run the **Toggle PR pane** plugin action. It opens a popup over your layout,
leaving your panes untouched. It is modal: close it with `q` / `Esc` to get
back to your panes.

herdr plugins can't dock a full-height sidebar yet, which is why this is a popup.

| Key             | Action                         |
|-----------------|--------------------------------|
| `j` / `k`, ↓/↑  | Move selection                 |
| `Enter` / `o`   | Open selected item in browser  |
| `Space` / `Tab` | Fold / unfold comment          |
| `c`             | Show / hide checks             |
| `1` `2` `3`     | Filter: all / humans / bots    |
| `r`             | Refresh                        |
| `q` / `Esc`     | Quit                           |

## Configuration

The popup size is set by `width` and `height` on the `pr` pane in
`herdr-plugin.toml` (cells or percentages, e.g. `"80%"`).

## License

[MIT](LICENSE)
