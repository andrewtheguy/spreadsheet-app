// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
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
        .invoke_handler(tauri::generate_handler![greet, open_external])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
