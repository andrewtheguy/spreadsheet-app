# Spreadsheet App

A desktop app for working with spreadsheets (CSV or Excel): filter and compare
two files by a shared column, and convert between CSV and Excel. Built with
Tauri + React + Mantine, with all spreadsheet logic implemented in Rust so the
backend stays UI-agnostic and reusable.

Desktop only — macOS and Windows. No mobile or Linux support.

## Features

- **Load** CSV, `.xlsx`, or `.xls` files into a left/right panel via a native
  file dialog. Large tables stay in the Rust backend; the UI receives a bounded
  preview (first 1,000 rows).
- **Filter** one table against another: keep (Include) or drop (Exclude) rows
  whose value in a chosen column appears in the other table, with optional
  case-insensitive matching.
- **Compare** two tables by a key column and a value column, tagging each row as
  Matched / Diff / Only Left / Only Right, with summary counts.
- **Sort** any column, and **export** the filtered or compared result back to
  CSV.
- **Convert** a file between formats: Excel → CSV, or CSV → Excel. The CSV →
  Excel direction writes every cell as text, so numeric-looking values (leading
  zeros, long IDs, date-like strings) keep their exact value instead of being
  auto-converted by Excel on open.

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

- `src/` — React + TypeScript + Mantine frontend (`App.tsx`).
- `src-tauri/` — Tauri app shell; thin `#[tauri::command]` wrappers in
  `src/lib.rs` that delegate to the core crate and manage the in-memory table
  store (`src/store.rs`).
- `src-tauri/sheet-core/` — pure Rust crate with all CSV/Excel parsing,
  serializing, filtering, comparing, and sorting logic. No Tauri/CEF
  dependencies, so it builds and unit-tests in isolation (`cargo test -p
  sheet-core`).

## Development

Prerequisites: [Rust](https://www.rust-lang.org/tools/install),
[Bun](https://bun.sh) (or npm), and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```sh
bun install          # install frontend deps
bun run tauri dev    # run the app in development
bun run tauri build  # produce a release build
```

### Checks

```sh
cargo test            # Rust tests (run after Rust changes)
cargo clippy          # lints (run after changes)
bun run tsc --noEmit  # frontend typecheck
```

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) +
  [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) +
  [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
