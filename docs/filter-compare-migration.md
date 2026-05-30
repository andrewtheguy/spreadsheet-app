# Filter / Compare migration roadmap

Migrate the **Filter** and **Compare** operations from
[`csv-filter`](../../csv-filter) (Electron + MUI) into this app
(Tauri + React + Mantine), implementing the logic in the Rust `sheet-core`
crate. This is a living roadmap executed phase by phase; update it as phases
land.

## Status — ✅ Complete (2026-05-29)

All three phases shipped. The app's two operations (Filter, Compare) are
implemented in `sheet-core` and exposed through thin Tauri commands; the old
merge scaffolding is gone.

| Phase | Status | Outcome |
| ----- | ------ | ------- |
| 0 — Remove merge scaffolding | ✅ Done | `merge`/`matching_rows`/`merge_csv` + Merged panel deleted |
| 1 — Filtering | ✅ Done | `filter_rows`/`filter_csv`; column Select + Exclude/Include + case-insensitive |
| 2 — Compare | ✅ Done | `compare`/`common_columns`/`comparison_to_table`; Filter\|Compare switcher, status-tinted table, summary badges |

App title is now **"CSV Filter & Compare"**. Verification: `cargo test -p
sheet-core` (20 passing), `cargo clippy` clean, `bunx tsc --noEmit` clean, app
launches and renders. The interactive load→filter/compare path uses a native
file dialog + CEF webview (no AX), so it's validated by a human, not automation;
fixtures live in `tmp/` (`left.csv`/`right.csv` for filter, `cmp-left.csv`/
`cmp-right.csv` for compare's four statuses).

Two gotchas worth remembering for future work on these structs:
- Tauri auto-converts only **top-level** command arg names camelCase↔snake_case;
  **nested struct fields need `#[serde(rename_all = "camelCase")]`** (applied to
  `FilterOptions` and all `Comparison*` types).
- `ComparisonStatus` serializes kebab-case (`only-left`/`only-right`) — the
  frontend's union type and `STATUS_BG`/`STATUS_LABEL` maps key off those exact
  strings.

The sections below are the original phase-by-phase plan, retained as a record of
what was built.

## Context

The app's current core operation — `merge` / `matching_rows` in `sheet-core`,
the `merge_csv` command, and the "Merged" UI panel — was **throwaway test
scaffolding**. It is being **deleted** and replaced with the two real operations
from `csv-filter`:

- **Filter** — keep or drop LEFT rows whose value in a chosen column appears in
  RIGHT's same-named column. Modes: `exclude` (default) / `include`; optional
  case-insensitive matching.
  Source: `filterCsvData` — `csv-filter/src/renderer/src/utils/csvFilterUtils.ts:229`.
- **Compare** — VLOOKUP-style key/value diff across the two CSVs, classifying
  every key as `matched` / `diff` / `only left` / `only right`, with summary
  counts.
  Source: `compareCSVData` — same file, line 303.

### Rust-first mandate (CLAUDE.md)

> implement backend as much as possible in Rust, so that the backend is more ui
> agnostic and can be reused in other contexts.

All logic — filter, compare, **and** the supporting derivations (column
resolution by header name, common-column computation, comparison→table
rendering) — lives in `sheet-core` as pure, unit-tested functions. Tauri
commands are thin wrappers; the React UI only collects inputs, invokes commands,
and renders results. **No filtering/compare logic is reimplemented in
TypeScript.**

### Data-model translation

| csv-filter (source)                    | spreadsheet-app (target)                                       |
| -------------------------------------- | -------------------------------------------------------------- |
| rows = `Record<string,string>`         | `CsvTable { headers: Vec<String>, rows: Vec<Vec<String>> }`    |
| column addressed by **header name**    | resolve header name → **column index** per table (in Rust)     |
| PapaParse (delimiter sniff, BOM strip) | existing Rust `csv` crate `parse_csv` (simpler)                |

Filter/Compare select a column by **header name** (as the source does); the Rust
layer resolves that name to an index in each table independently, so the "same"
column may sit at different positions in left vs right.

### Kept vs removed

- **Keep:** `parse_csv`, `write_csv` (sheet-core); `load_csv`, `save_csv`,
  `open_external` commands; the load panels, file-path display, pagination,
  blank-row/blank-column display handling, swap button, toast errors.
- **Remove:** `merge`, `matching_rows` + their tests (sheet-core); `merge_csv`
  command + its `generate_handler!` entry (lib.rs); the auto-merge `useEffect`,
  the "Merged" panel, `merged`/`mergedPage`/`mergedCount` state (App.tsx).

---

## Phase 0 — Remove merge scaffolding — ✅ Done

Clears the way so the new operations don't sit beside dead code.

- `src-tauri/sheet-core/src/lib.rs`: delete `matching_rows`, `merge`, and their
  `#[cfg(test)]` tests. Keep parse/write + their tests.
- `src-tauri/src/lib.rs`: delete the `merge_csv` command and remove it from
  `generate_handler!`.
- `src/App.tsx`: remove the merge `useEffect`, `merged`/`mergedPage` state,
  `mergedCount`, the "Merged" `Paper` panel, and `exportResult`'s dependence on
  `merged` (export is re-wired per-mode in Phases 1–2). The reusable
  `CsvTableView`, `TablePanel`, `isBlankRow`, `displayHeaders`, `truncatePath`,
  and the two source panels stay.
- **Verify:** `cargo clippy` clean, `cargo test` green, `bunx tsc --noEmit`
  clean.

---

## Phase 1 — Filtering logic (Rust + command + UI) — ✅ Done

### Rust (`sheet-core/src/lib.rs`)

Add pure logic mirroring `filterCsvData`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode { Exclude, Include }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FilterOptions { pub mode: FilterMode, pub case_insensitive: bool }

/// Index of the first header equal to `name`, if any (shared helper, also used by compare).
pub fn column_index(table: &CsvTable, name: &str) -> Option<usize>

/// Returns LEFT's rows kept/dropped by membership of LEFT's `column` value in the
/// set of RIGHT's `column` values. Column resolved by header name in each table.
pub fn filter_rows(left: &CsvTable, right: &CsvTable, column: &str, opts: &FilterOptions) -> CsvTable
```

Semantics to match the source exactly:

- Resolve `column` to an index in each table via `column_index` (first match).
- If RIGHT lacks the column → return LEFT unchanged.
- Build a `HashSet` of RIGHT's column values, normalized (lowercased iff
  `case_insensitive`); a row missing the cell contributes a sentinel/`None`.
- `include` → keep LEFT rows whose (normalized) value is in the set; `exclude` →
  keep those not in the set.
- Preserve original (untrimmed) cells, order, and duplicates.

Tests (follow existing `table()`/`rows()` helpers): exclude default, include
mode, case-insensitive on/off, column-missing-in-right passthrough, column at
different indices in left vs right, duplicate preservation.

### Command (`src-tauri/src/lib.rs`)

```rust
#[tauri::command]
fn filter_csv(left: CsvTable, right: CsvTable, column: String, options: FilterOptions) -> CsvTable
```

Register in `generate_handler!`. Thin wrapper over `sheet_core::filter_rows`.

### Frontend (`src/App.tsx`)

- Add a result panel (reuse `CsvTableView`) replacing the old Merged panel.
- Controls: column `Select` (from RIGHT `displayHeaders`), mode radio
  (Exclude/Include), case-insensitive `Checkbox` — mirror `CsvFilter.tsx:254`.
- Auto-apply `useEffect` (left/right/column/mode/caseInsensitive deps) calling
  `invoke<CsvTable>("filter_csv", {...})` with the cancellation-flag pattern
  already used; toast on error; reset result page on change.
- Export filtered result via existing `save_csv`.

**Verify:** clippy/test/tsc clean; run the app with `tmp/` fixtures —
exclude/include flip the kept set; case-insensitive toggles matching; pagination
+ blank-row hiding still correct.

---

## Phase 2 — Compare logic (Rust + command + UI) — ✅ Done

### Rust (`sheet-core/src/lib.rs`)

Mirror `compareCSVData`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]            // "matched" | "diff" | "only-left" | "only-right"
pub enum ComparisonStatus { Matched, Diff, OnlyLeft, OnlyRight }

pub struct ComparisonRow { pub key: String, pub left_value: Option<String>, pub right_value: Option<String>, pub status: ComparisonStatus }
pub struct ComparisonSummary { pub total: usize, pub matched: usize, pub diff: usize, pub only_left: usize, pub only_right: usize }
pub struct ComparisonResult { pub rows: Vec<ComparisonRow>, pub key_column: String, pub value_column: String, pub summary: ComparisonSummary }

/// Header names present in BOTH tables (used to populate the key/value selectors).
pub fn common_columns(left: &CsvTable, right: &CsvTable) -> Vec<String>

pub fn compare(left: &CsvTable, right: &CsvTable, key_column: &str, value_column: &str, case_insensitive: bool) -> ComparisonResult

/// Render a result as an exportable table (key, value-left, value-right, status)
/// so export reuses `write_csv`/`save_csv`.
pub fn comparison_to_table(result: &ComparisonResult) -> CsvTable
```

Semantics to match the source exactly:

- Build left/right `HashMap<normalized_key, (orig_key, value)>`; last occurrence
  wins. Null/missing key → sentinel so it never collides with empty string.
- Union of keys → per key: only-left / only-right / (matched vs diff by comparing
  values after `String + trim`, so `100` == `"100"`).
- Summary counts. Key/value columns resolved by header name per table via
  `column_index`.

Tests: matched, diff (incl. numeric-vs-string equality), only-left, only-right,
case-insensitive keys, duplicate-key last-wins, summary totals, `common_columns`
intersection, `comparison_to_table` shape.

### Commands (`src-tauri/src/lib.rs`)

```rust
#[tauri::command]
fn common_columns(left: CsvTable, right: CsvTable) -> Vec<String>

#[tauri::command]
fn compare_csv(left: CsvTable, right: CsvTable, key_column: String, value_column: String, case_insensitive: bool) -> ComparisonResult
```

Register both. Export reuses `save_csv` after a small wrapper builds the table
from `comparison_to_table` (keeps the render logic in Rust).

### Frontend (`src/App.tsx`)

- Introduce a **Filter | Compare** mode switcher (Mantine `SegmentedControl` or
  `Tabs`) above the result panel; the active mode owns the result area.
- Populate key/value `Select`s from the `common_columns` command (no
  intersection logic in TS).
- Compare UI (mirror `CsvFilter.tsx:437`): case-insensitive `Checkbox`,
  key-column + value-column `Select`s, summary as Mantine `Badge`s
  (Total/Matched/Diff/Only Left/Only Right), and a color-tinted results table
  (row background by status: diff=red, only-left=orange, only-right=blue,
  matched=default) with pagination.
- Auto-apply `useEffect` calling `compare_csv`; toast on error; export via the
  comparison-table export path.

**Verify:** clippy/test/tsc clean; run the app with `tmp/` fixtures covering all
four statuses and verify summary counts + color coding + export.

---

## Out of scope

- PapaParse-specific parsing (delimiter auto-detection, BOM stripping,
  duplicate-header rejection) — the Rust `csv` crate parser stays as-is.
- Any MUI; UI is Mantine only.
- Multi-condition / regex / range filters (the source supports none of these).

## Cross-cutting conventions (CLAUDE.md)

`no backward compatibility` · test data under project `tmp/` · `cargo clippy`
after Rust changes · no `cargo fmt` · mac + windows only · **all logic in Rust,
UI-agnostic** — TypeScript invokes commands and renders, nothing more.
