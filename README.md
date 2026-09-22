# alpm_cleaner

> ⚠️ This project (and README.md) is **vibecoded** — written in an hour almost entirely by an AI coding
> agent (OpenCode + Muse Spark), driven by a human. 
> It works on the author's machine, has tests, and has been
> reviewed — but you should still read the apply command before hitting `y`,
> especially since this tool runs `pacman -R` as root.

A TUI package manager for Arch Linux. It shows installed packages, lets you
check/uncheck them, previews exactly what else would get uninstalled (like a
dry-run of `pacman -Rns` / `-Rnsc`), and applies the removal when you're done.

## Why

`pacman -Rs` tells you the removal set only *after* you commit, and `-c`
(cascade) can drag half your system with it. This tool shows the removal set
**live, before you commit**, computed by asking pacman itself
(`pacman -Rs[c] --print`), so the preview is authoritative, not a guess.

## Run

```sh
cargo run
```

Needs `pacman` on `PATH` (i.e. run it on Arch). No root needed for browsing —
privileges are only escalated for the final apply step.

## Layout

- **Installed** (left): all installed packages with `[x]` checkboxes, version,
  size, and badges — `E`xplicit/`D`ep, `+` for foreign/AUR.
- **Would remove** (right): `+` red = you checked it, `↳` yellow = pacman
  pulls it in (cascade/orphans), `!` red = blocks `-Rns` (needs `-c`).
- **Package info** (bottom): `-Qi`-style details for the hovered package on
  either pane — description, dates, deps, optional deps, required-by.
- **Command / keybinds / prompt** (very bottom): the exact command that will
  run, the keybind cheatsheet, and the `:` search prompt / status line.

## Keys

| Key | Action |
| --- | ------ |
| `space`/`x` | check/uncheck |
| `s` | sort by name ↔ size |
| `o` | filter source: all → repo → AUR |
| `e` | filter reason: all → explicit → dep |
| `/` or `:` | search (bottom prompt) |
| `c` | toggle cascade (`-Rns` ↔ `-Rnsc`, default `-Rns`) |
| `Tab` | switch panel (each has its own cursor) |
| `j/k`, `PgUp/PgDn`, `g/G` | move |
| `a` then `y` | apply |
| `?` | help, `q` quit, `Esc` clear search/close |

## Privileges & scope

- Already root → runs `pacman` directly. Otherwise honors
  `$ALPM_CLEANER_PRIV` / `$SUDO`, then auto-detects `sudo` → `doas` → `run0`.
- AUR/foreign packages are marked via one `pacman -Qm` call at startup and
  resolve through the same preview.
- Note: `--print` can't combine with `-n`, but `-n` (nosave) doesn't change
  the package set — only backup-file handling — so the preview covers `-Rns`
  exactly.

## Dev

The crate is split into a lib (`src/lib.rs`) plus a thin TUI binary
(`src/main.rs`), so integration tests can exercise the real logic.

```sh
cargo build
cargo clippy --all-targets
cargo test                                   # unit tests (fixed fixtures)
cargo test --test preview_e2e -- --ignored   # live tests: real run_pacman_preview
                                             # against real pacman (needs Arch; uses bash)
```
