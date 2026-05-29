// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// Opens a URL in the OS default browser. The CEF runtime renders `target="_blank"`
// links inside the embedded Chromium webview, so the frontend routes external links
// here instead. Only http(s) URLs are allowed.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("refusing to open non-http(s) URL: {url}"));
    }

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(&url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("cmd");
        // The empty "" is the window title arg `start` expects before the URL.
        c.args(["/C", "start", "", &url]);
        c
    };

    command.spawn().map_err(|e| e.to_string())?;
    Ok(())
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
