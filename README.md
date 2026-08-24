# CSV / Excel Filter & Compare

A desktop app for filtering and comparing two spreadsheets (CSV or Excel) by a
shared column. Built with [Slint](https://slint.dev) — a native UI, no webview —
with all spreadsheet logic implemented in Rust so the backend stays UI-agnostic
and reusable.

Desktop only — macOS and Windows. No mobile or Linux support.

## Features

- **Load** CSV, `.xlsx`, or `.xls` files into a left/right panel via a native
  file dialog. Large tables stay in the Rust controller; the table view receives
  a bounded preview (first 1,000 rows).
- **Filter** one table against another: keep (Include) or drop (Exclude) rows
  whose value in a chosen column appears in the other table, with optional
  case-insensitive matching.
- **Compare** two tables by a key column and a value column, tagging each row as
  Matched / Diff / Only Left / Only Right, with summary counts and per-row
  colour coding.
- **Sort** any column, and **export** the filtered or compared result back to
  CSV. Sorting and exporting always run over the *full* dataset, not the
  visible preview.
- **Keyboard-first**: every action has a shortcut, listed in the menu bar and in
  the in-app **Shortcuts** popover (Ctrl+/).

### Keyboard shortcuts

`Ctrl` is `⌘` on macOS and `Ctrl` on Windows; `Alt` is `⌥`.

| Shortcut | Action |
| --- | --- |
| `Ctrl+O` / `Ctrl+Shift+O` | Load into the Left / Right panel |
| `Ctrl+T` | Swap Left & Right |
| `Ctrl+E` | Export the result |
| `Ctrl+1` / `Ctrl+2` | Filter / Compare mode |
| `Ctrl+D` | Toggle Exclude / Include |
| `Ctrl+I` | Toggle case-insensitive matching |
| `Ctrl+]` / `Ctrl+[` | Next / previous filter (or compare key) column |
| `Ctrl+}` / `Ctrl+{` | Next / previous compare value column |
| `Ctrl+Backspace` | Clear the column selection |
| `Ctrl+→` / `Ctrl+←` | Result: sort by next / previous column |
| `Ctrl+↓` | Result: ascending → descending → unsorted |
| `Alt+→` / `Alt+←` / `Alt+↓` | Left panel: next / previous column, cycle direction |
| `Alt+Shift+→` / `←` / `↓` | Right panel: next / previous column, cycle direction |
| `Ctrl+Shift+U` | Clear every sort |
| `Ctrl+K` | Excel notes |
| `Ctrl+/` | Shortcut list |
| `Esc` | Dismiss the status message |

### Spreadsheet handling notes

- **Encoding:** non-UTF-8 CSVs (e.g. Windows-1252 Excel exports) load instead of
  failing — invalid bytes in a cell are dropped (ASCII kept), mirroring Ruby's
  `encode("US-ASCII", invalid: :replace, undef: :replace, replace: "")`.
- **Excel numbers:** Excel stores full-precision doubles plus a separate display
  format, so a 20%-off price can be stored as `30.400000000000002`. The backend
  reads the cell's number-format code and reapplies the common cases (fixed
  decimals, thousands separators, leading currency symbol, percent); anything
  outside that subset falls back to the value rounded to Excel's 15 significant
  digits. See the **Excel notes** popover in the app header for the full list of
  limitations (single-sheet only, formulas show last-saved value, dates
  normalized to ISO 8601, etc.).

## Architecture

Four components, each with no knowledge of the one above it:

- `sheet-core/` — pure Rust crate with all CSV/Excel parsing, serializing,
  filtering, comparing, and sorting logic. No UI dependencies, so it builds and
  unit-tests in isolation (`cargo test -p sheet-core`).
- `src/state.rs` — the application controller: loaded tables, the current mode
  and column selections, derived filter/compare results, and sorting. Pure Rust
  with no Slint types, so it's unit-testable and would survive another UI swap.
- `src/view.rs` — display models (`TableView`, `RowView`, status lines, path
  truncation). Still toolkit-free; it decides *what* to show, not how to paint
  it.
- `src/main.rs` + `ui/*.slint` — the Slint shell. `main.rs` is glue: it maps
  widget events onto `state` calls and `view` models onto Slint models.

`ui/data-table.slint` is a hand-rolled table (modelled on Slint's
`StandardTableView`) because the stock widget can't tint rows by comparison
status.

## Development

Prerequisites: [Rust](https://www.rust-lang.org/tools/install). Nothing else —
there's no Node toolchain and no system webview to install.

```sh
cargo run              # run the app
cargo run --release    # optimized build
```

### Checks

```sh
cargo test --workspace     # Rust tests (run after Rust changes)
cargo clippy --workspace --all-targets   # lints (run after changes)
```

### Packaging

`cargo build` produces a plain binary; the release artifacts are assembled by
`scripts/bundle-macos.sh`, which lipos one or more binaries into a universal
executable and wraps it in `spreadsheet-app.app` (ad-hoc signed, so it launches
on Apple silicon):

```sh
cargo build --release
MAKE_DMG=1 ./scripts/bundle-macos.sh target/macos target/release/spreadsheet-app
```

Windows gets an MSI from `scripts/bundle-windows.ps1`, which downloads a pinned
WiX v3 and compiles `packaging/windows/main.wxs`:

```powershell
cargo build --release
pwsh scripts/bundle-windows.ps1 -ExePath target/release/spreadsheet-app.exe
```

The installer is per-machine, puts the app in `%ProgramFiles%\spreadsheet-app`
with a Start Menu shortcut, and upgrades in place. **Never change the
`UpgradeCode` GUID in `main.wxs`** — it's what ties builds together as upgrades
rather than parallel installs.

The `release` workflow runs both, uploading a `.dmg` plus a zipped `.app`, and an
`.msi` plus a zipped `.exe`. The macOS bundle is ad-hoc signed, which is only
enough to let it launch on Apple silicon — there's no trusted developer
certificate on either platform and nothing is notarized, so both still show an
unidentified-developer warning.

### Rendering fallback

Slint's default renderer needs an OpenGL 3 driver. Some real Windows sessions
don't have one (RDP, VMs, Windows Server), where it dies with "Failed to
initialize OpenGL driver". Because Slint's platform is process-global by the time
that surfaces, `main.rs` handles it by relaunching itself once with
`SLINT_BACKEND=winit-software`. On the fallback path you'll see two processes:
a thin parent waiting on the child that owns the window.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) +
  [Slint](https://marketplace.visualstudio.com/items?itemName=Slint.slint) +
  [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
