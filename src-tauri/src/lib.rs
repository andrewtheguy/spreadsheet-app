use std::fs::File;

use sheet_core::CsvTable;

// A parsed CSV paired with the path it was loaded from, so the frontend can display the
// source file in the panel title.
#[derive(serde::Serialize)]
struct LoadedCsv {
    table: CsvTable,
    path: String,
}

// Opens a native file picker for a `.csv` file, parses it (via `sheet-core`), and returns
// its contents along with the chosen path. Returns `Ok(None)` when the user cancels the
// dialog so the frontend can leave the current table untouched.
#[tauri::command]
async fn load_csv<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<LoadedCsv>, String> {
    // The native dialog must run on the main (UI) thread on macOS. `run_on_main_thread`
    // only schedules the closure, so hand the chosen path back over a channel. `pick_file`
    // blocks while the modal is open, which is fine — it spins its own run loop.
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let path = rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .pick_file();
        // Ignore send errors: the only way `rx` is gone is if this command was dropped.
        let _ = tx.send(path);
    })
    .map_err(|e| e.to_string())?;

    let Some(path) = rx.recv().map_err(|e| e.to_string())? else {
        return Ok(None);
    };

    let file = File::open(&path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let table =
        sheet_core::parse_csv(file).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    Ok(Some(LoadedCsv {
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
            .set_file_name("merged.csv")
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

// Computes the merged result — `left`'s rows whose first column appears in `right`'s first
// column. The matching logic lives in the `sheet-core` crate.
#[tauri::command]
fn merge_csv(left: CsvTable, right: CsvTable) -> CsvTable {
    sheet_core::merge(&left, &right)
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
            merge_csv
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
