use std::fs::File;

use sheet_core::{ComparisonResult, CsvTable, FilterOptions};

mod store;
use store::TableStore;

// A parsed CSV paired with the path it was loaded from, so the frontend can display the
// source file in the panel title. `id` is the handle into the backend `TableStore`; the
// frontend passes it back to `filter_csv` / `compare_csv` / `common_columns` instead of
// re-shipping the whole table on every recompute.
#[derive(serde::Serialize)]
struct LoadedCsv {
    id: u64,
    table: CsvTable,
    path: String,
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
    let file = File::open(&path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let table = match extension.as_deref() {
        Some("csv") => sheet_core::parse_csv(file)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?,
        Some("xlsx" | "xls") => sheet_core::parse_excel(file)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?,
        _ => {
            return Err(format!(
                "unsupported file type for {}; expected .csv, .xlsx, or .xls",
                path.display()
            ));
        }
    };
    // Hold the table in the backend store (evicting the side's previous table) and hand its
    // id back alongside the data the frontend renders.
    let id = store.insert(table.clone(), replace);
    Ok(Some(LoadedCsv {
        id,
        table,
        path: path.display().to_string(),
    }))
}

// Opens a native save dialog for a `.csv` file and writes `table` to it (via `sheet-core`).
// Returns `Ok(false)` when the user cancels the dialog so the frontend can no-op.
#[tauri::command]
async fn save_csv<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    table: CsvTable,
) -> Result<bool, String> {
    // Same main-thread dialog dance as `load_csv`: the native dialog must run on the UI
    // thread on macOS, so schedule it and hand the chosen path back over a channel.
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let path = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name("filtered.csv")
            .save_file();
        let _ = tx.send(path);
    })
    .map_err(|e| e.to_string())?;

    let Some(path) = rx.recv().map_err(|e| e.to_string())? else {
        return Ok(false);
    };

    let file =
        File::create(&path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    sheet_core::write_csv(file, &table)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(true)
}

// Filters `left`'s rows by whether their value in `column` appears in `right`'s same-named
// column. Both tables are referenced by their `TableStore` id rather than passed by value,
// so a recompute ships two ids instead of two full tables. The filtering logic lives in the
// `sheet-core` crate.
#[tauri::command]
fn filter_csv(
    store: tauri::State<'_, TableStore>,
    left_id: u64,
    right_id: u64,
    column: String,
    options: FilterOptions,
) -> Result<CsvTable, String> {
    let left = store.get(left_id)?;
    let right = store.get(right_id)?;
    Ok(sheet_core::filter_rows(&left, &right, &column, &options))
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
// tables are referenced by their `TableStore` id. The comparison logic lives in the
// `sheet-core` crate.
#[tauri::command]
fn compare_csv(
    store: tauri::State<'_, TableStore>,
    left_id: u64,
    right_id: u64,
    key_column: String,
    value_column: String,
    case_insensitive: bool,
) -> Result<ComparisonResult, String> {
    let left = store.get(left_id)?;
    let right = store.get(right_id)?;
    Ok(sheet_core::compare(
        &left,
        &right,
        &key_column,
        &value_column,
        case_insensitive,
    ))
}

// Renders a comparison result as a four-column table for export via `save_csv`.
#[tauri::command]
fn comparison_to_table(result: ComparisonResult) -> CsvTable {
    sheet_core::comparison_to_table(&result)
}

// Returns a stored table (the Left/Right source panels, referenced by id) reordered by the
// `column` header index for display. The store copy is left untouched, so filter/compare
// operations keep their original ordering.
#[tauri::command]
fn sort_csv(
    store: tauri::State<'_, TableStore>,
    id: u64,
    column: usize,
    ascending: bool,
) -> Result<CsvTable, String> {
    let table = store.get(id)?;
    Ok(sheet_core::sort_rows(&table, column, ascending))
}

// Reorders a table that isn't held in the store — the filter result — by `column` for
// display. The (already-computed) table is passed by value since this is a one-off click.
#[tauri::command]
fn sort_table(table: CsvTable, column: usize, ascending: bool) -> CsvTable {
    sheet_core::sort_rows(&table, column, ascending)
}

// Reorders a comparison result by `column` (0=key, 1=left, 2=right, 3=status) for display.
#[tauri::command]
fn sort_comparison(result: ComparisonResult, column: usize, ascending: bool) -> ComparisonResult {
    sheet_core::sort_comparison(&result, column, ascending)
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
        // Holds loaded tables so filter/compare/common-columns can reference them by id
        // instead of re-receiving full table payloads on every recompute.
        .manage(TableStore::default())
        // Chromium's password manager (OSCrypt) stores its "Safe Storage" key in the
        // macOS Keychain. Each rebuild changes the app binary's identity, so macOS
        // re-prompts for Keychain access on every launch. We don't use Chromium's
        // password manager, so point it at a mock keychain / basic store to stop the
        // prompt entirely. Both switches are no-ops on platforms without a keychain.
        .command_line_args([
            ("--use-mock-keychain", None::<&str>),
            ("password-store", Some("basic")),
        ])
        .invoke_handler(tauri::generate_handler![
            open_external,
            load_csv,
            save_csv,
            filter_csv,
            common_columns,
            compare_csv,
            comparison_to_table,
            sort_csv,
            sort_table,
            sort_comparison
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
