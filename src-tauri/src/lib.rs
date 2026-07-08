use std::fs::File;

use sheet_core::{ComparisonPreview, ComparisonResult, CsvTable, FilterOptions, TablePreview};

mod store;
use store::{LatestSlot, TableStore};

// The backend store of derived results: at most one filter result and one compare result, each
// the latest computed. Sort/export reference them by id so the full data never crosses IPC.
type FilteredStore = LatestSlot<CsvTable>;
type ComparisonStore = LatestSlot<ComparisonResult>;

// A loaded spreadsheet handed back to the frontend: a bounded `TablePreview` for display paired
// with the source path (shown in the panel title) and the `TableStore` id. The frontend passes
// `id` back to `filter_csv` / `compare_csv` / `common_columns` / `sort_csv` instead of
// re-shipping the whole table; the full table stays in the store.
#[derive(serde::Serialize)]
struct LoadedCsv {
    id: u64,
    table: TablePreview,
    path: String,
}

// The result of `filter_csv`: a bounded preview for display plus the `FilteredStore` id the
// frontend passes back to `sort_filtered` / `export_filtered` to act on the full filtered data.
#[derive(serde::Serialize)]
struct FilteredCsv {
    id: u64,
    table: TablePreview,
}

// The result of `compare_csv`: a bounded preview plus the `ComparisonStore` id the frontend
// passes back to `sort_comparison` / `export_comparison`.
#[derive(serde::Serialize)]
struct ComparedCsv {
    id: u64,
    result: ComparisonPreview,
}

// A sort to apply to the full dataset before export, mirroring the frontend's on-screen sort
// (`None` exports in the original computed order). Fields are single words, so Tauri's arg
// case-conversion needs no `serde(rename_all)`.
#[derive(serde::Deserialize)]
struct SortSpec {
    column: usize,
    ascending: bool,
}

// Opens a native file picker for a CSV/Excel file, parses it (via `sheet-core`), and returns
// its contents along with the chosen path. Returns `Ok(None)` when the user cancels the
// dialog so the frontend can leave the current table untouched.
#[tauri::command]
async fn load_csv<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    store: tauri::State<'_, TableStore>,
    replace: Option<u64>,
) -> Result<Option<LoadedCsv>, String> {
    // The native dialog must run on the main (UI) thread on macOS. `run_on_main_thread`
    // only schedules the closure, so hand the chosen path back over a channel. `pick_file`
    // blocks while the modal is open, which is fine — it spins its own run loop.
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let path = rfd::FileDialog::new()
            .add_filter("CSV and Excel", &["csv", "xlsx", "xls"])
            .pick_file();
        // Ignore send errors: the only way `rx` is gone is if this command was dropped.
        let _ = tx.send(path);
    })
    .map_err(|e| e.to_string())?;

    let Some(path) = rx.recv().map_err(|e| e.to_string())? else {
        return Ok(None);
    };

    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let table = match extension.as_deref() {
        Some("csv") => {
            let file =
                File::open(&path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
            sheet_core::parse_csv(file)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?
        }
        Some("xlsx" | "xls") => {
            let file =
                File::open(&path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
            sheet_core::parse_excel(file)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?
        }
        _ => {
            return Err(format!(
                "unsupported file type for {}; expected .csv, .xlsx, or .xls",
                path.display()
            ));
        }
    };
    // Hold the full table in the backend store (evicting the side's previous table) and hand its
    // id back alongside a bounded preview the frontend renders.
    let preview = TablePreview::from_table(&table);
    let id = store.insert(table, replace);
    Ok(Some(LoadedCsv {
        id,
        table: preview,
        path: path.display().to_string(),
    }))
}

// Opens a native ".csv" save dialog (on the UI thread, as macOS requires) and writes `table` to
// the chosen path via `sheet-core`. `default_filename` pre-fills the dialog so each caller can
// suggest a context-specific name. Returns `Ok(false)` when the user cancels so the caller can
// no-op. Shared by `export_filtered` / `export_comparison`, which write the *full* dataset.
async fn save_table<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    table: &CsvTable,
    default_filename: &str,
) -> Result<bool, String> {
    // Same main-thread dialog dance as `load_csv`: schedule the modal on the UI thread and hand
    // the chosen path back over a channel.
    let default_filename = default_filename.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let path = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(default_filename)
            .save_file();
        let _ = tx.send(path);
    })
    .map_err(|e| e.to_string())?;

    let Some(path) = rx.recv().map_err(|e| e.to_string())? else {
        return Ok(false);
    };

    let file =
        File::create(&path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    sheet_core::write_csv(file, table)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(true)
}

// Filters `left`'s rows by whether their value in `column` appears in `right`'s same-named
// column. Both tables are referenced by their `TableStore` id rather than passed by value, so a
// recompute ships two ids instead of two full tables. The full filtered table is held in the
// `FilteredStore` (replacing the previous result) and only a bounded preview is returned, paired
// with the store id that `sort_filtered` / `export_filtered` use. Filtering lives in `sheet-core`.
#[tauri::command]
fn filter_csv(
    store: tauri::State<'_, TableStore>,
    filtered: tauri::State<'_, FilteredStore>,
    left_id: u64,
    right_id: u64,
    column: String,
    options: FilterOptions,
) -> Result<FilteredCsv, String> {
    let left = store.get(left_id)?;
    let right = store.get(right_id)?;
    let result = sheet_core::filter_rows(&left, &right, &column, &options);
    let preview = TablePreview::from_table(&result);
    let id = filtered.set(result);
    Ok(FilteredCsv {
        id,
        table: preview,
    })
}

// Header names present in both tables — the candidate key/value columns for `compare_csv`.
#[tauri::command]
fn common_columns(
    store: tauri::State<'_, TableStore>,
    left_id: u64,
    right_id: u64,
) -> Result<Vec<String>, String> {
    let left = store.get(left_id)?;
    let right = store.get(right_id)?;
    Ok(sheet_core::common_columns(&left, &right))
}

// VLOOKUP-style diff of `left` vs `right` by `key_column`, comparing `value_column`. Both
// tables are referenced by their `TableStore` id. The full result is held in the
// `ComparisonStore` (replacing the previous one) and only a bounded preview is returned, paired
// with the store id that `sort_comparison` / `export_comparison` use. Logic lives in `sheet-core`.
#[tauri::command]
fn compare_csv(
    store: tauri::State<'_, TableStore>,
    comparison: tauri::State<'_, ComparisonStore>,
    left_id: u64,
    right_id: u64,
    key_column: String,
    value_column: String,
    case_insensitive: bool,
) -> Result<ComparedCsv, String> {
    let left = store.get(left_id)?;
    let right = store.get(right_id)?;
    let result = sheet_core::compare(&left, &right, &key_column, &value_column, case_insensitive);
    let preview = ComparisonPreview::from_result(&result);
    let id = comparison.set(result);
    Ok(ComparedCsv {
        id,
        result: preview,
    })
}

// Returns a stored source table (Left/Right panel, by `TableStore` id) reordered by the `column`
// header index, as a bounded preview. The full table is sorted server-side so the preview's
// first rows reflect the *global* order; the store copy is left untouched, so filter/compare
// keep their original ordering.
#[tauri::command]
fn sort_csv(
    store: tauri::State<'_, TableStore>,
    id: u64,
    column: usize,
    ascending: bool,
) -> Result<TablePreview, String> {
    let table = store.get(id)?;
    let sorted = sheet_core::sort_rows(&table, column, ascending);
    Ok(TablePreview::from_table(&sorted))
}

// Reorders the stored filter result (by `FilteredStore` id) by `column`, returning a bounded
// preview of the globally-sorted full data.
#[tauri::command]
fn sort_filtered(
    filtered: tauri::State<'_, FilteredStore>,
    id: u64,
    column: usize,
    ascending: bool,
) -> Result<TablePreview, String> {
    let table = filtered.get(id)?;
    let sorted = sheet_core::sort_rows(&table, column, ascending);
    Ok(TablePreview::from_table(&sorted))
}

// Reorders the stored comparison result (by `ComparisonStore` id) by `column` (0=key, 1=left,
// 2=right, 3=status), returning a bounded preview of the globally-sorted full data.
#[tauri::command]
fn sort_comparison(
    comparison: tauri::State<'_, ComparisonStore>,
    id: u64,
    column: usize,
    ascending: bool,
) -> Result<ComparisonPreview, String> {
    let result = comparison.get(id)?;
    let sorted = sheet_core::sort_comparison(&result, column, ascending);
    Ok(ComparisonPreview::from_result(&sorted))
}

// Exports the full stored filter result (by `FilteredStore` id) as CSV, applying `sort` first so
// the file matches the on-screen order. Opens a native save dialog; returns `Ok(false)` on
// cancel. Unlike the previews, this writes *every* row.
#[tauri::command]
async fn export_filtered<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    filtered: tauri::State<'_, FilteredStore>,
    id: u64,
    sort: Option<SortSpec>,
) -> Result<bool, String> {
    let table = filtered.get(id)?;
    // Sorting produces an owned table; the unsorted path borrows the stored one directly to
    // avoid cloning the full (potentially huge) dataset just to pass a reference.
    match sort {
        Some(SortSpec { column, ascending }) => {
            save_table(&app, &sheet_core::sort_rows(&table, column, ascending), "filtered.csv").await
        }
        None => save_table(&app, &table, "filtered.csv").await,
    }
}

// Exports the full stored comparison result (by `ComparisonStore` id) as a four-column CSV,
// applying `sort` first to match the on-screen order. Opens a native save dialog; returns
// `Ok(false)` on cancel. Writes every row.
#[tauri::command]
async fn export_comparison<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    comparison: tauri::State<'_, ComparisonStore>,
    id: u64,
    sort: Option<SortSpec>,
) -> Result<bool, String> {
    let result = comparison.get(id)?;
    let result = match sort {
        Some(SortSpec { column, ascending }) => {
            sheet_core::sort_comparison(&result, column, ascending)
        }
        None => (*result).clone(),
    };
    let table = sheet_core::comparison_to_table(&result);
    save_table(&app, &table, "comparison.csv").await
}

// Exports a stored source table (by `TableStore` id) as CSV, used by the Convert use case to
// turn a loaded Excel file into CSV. `default_name` pre-fills the save dialog (the frontend
// derives it from the source filename, e.g. `sales.xlsx` → `sales.csv`). Returns `Ok(false)`
// when the user cancels. The Excel→CSV conversion is just `parse_excel` (done at load time) +
// `write_csv` (inside `save_table`), so no new logic is needed.
#[tauri::command]
async fn export_csv<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    store: tauri::State<'_, TableStore>,
    id: u64,
    default_name: String,
) -> Result<bool, String> {
    let table = store.get(id)?;
    save_table(&app, &table, &default_name).await
}

// Opens a URL in the OS default browser. The CEF runtime renders `target="_blank"`
// links inside the embedded Chromium webview, so the frontend routes external links
// here instead.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    // The OS opener (ShellExecuteExW on Windows, `open(1)` on macOS) will launch *any*
    // registered scheme, so parse and restrict to http(s) and reject embedded
    // credentials / hostless URLs. `Url::parse` also rejects/percent-encodes control
    // characters, so they never reach the opener.
    let parsed = url::Url::parse(&url).map_err(|e| format!("invalid URL {url:?}: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("refusing to open non-http(s) URL: {url:?}"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!("refusing to open URL with embedded credentials: {url:?}"));
    }
    if parsed.host().is_none() {
        return Err(format!("refusing to open URL without a host: {url:?}"));
    }

    // `that_detached` never invokes a shell on any platform, so a query string like
    // `?a=1&b=2` can't be reinterpreted as a command.
    open::that_detached(parsed.as_str()).map_err(|e| e.to_string())
}

// `Builder::default()` resolves to the CEF runtime because the `cef` feature is enabled
// and the default `wry` feature is disabled in Cargo.toml.
pub fn run() {
    tauri::Builder::default()
        // Holds loaded source tables so filter/compare/common-columns/sort can reference them by
        // id instead of re-receiving full table payloads on every recompute.
        .manage(TableStore::default())
        // Hold the latest filter and compare results server-side, so sort/export operate on the
        // full data while the UI only ever receives a bounded preview.
        .manage(FilteredStore::default())
        .manage(ComparisonStore::default())
        // Chromium's password manager (OSCrypt) stores its "Safe Storage" key in the
        // macOS Keychain. Each rebuild changes the app binary's identity, so macOS
        // re-prompts for Keychain access on every launch. We don't use Chromium's
        // password manager, so point it at a mock keychain / basic store to stop the
        // prompt entirely. Both switches are no-ops on platforms without a keychain.
        //
        // This app never opens peer connections. Keep WebRTC from probing local
        // interfaces; do not pass `disable-features` here because CEF reapplies this
        // list to child processes and Chromium can crash during window creation.
        .command_line_args([
            ("--use-mock-keychain", None::<&str>),
            ("password-store", Some("basic")),
            ("--disable-webrtc", None::<&str>),
            (
                "force-webrtc-ip-handling-policy",
                Some("disable_non_proxied_udp"),
            ),
        ])
        .invoke_handler(tauri::generate_handler![
            open_external,
            load_csv,
            filter_csv,
            common_columns,
            compare_csv,
            sort_csv,
            sort_filtered,
            sort_comparison,
            export_filtered,
            export_comparison,
            export_csv
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
