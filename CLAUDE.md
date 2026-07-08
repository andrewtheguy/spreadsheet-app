- no backward compatibility
- use project root tmp/ dir for test data
- run cargo clippy after changes
- run cargo test after rust code changes
- no cargo fmt
- desktop mac and windows only, no mobile or linux support
- implement backend as much as possible in Rust, so that the backend is more ui agnostic and can be reused in other contexts

## CEF / running the app

- The CEF framework lives inside the `.app` bundle — launch the bundle, not `src-tauri/target/debug/spreadsheet-app` directly, or CEF won't find its runtime.
- A stale running instance holding `~/Library/Caches/com.andrewtheguy.spreadsheet-app/cef` causes Chromium to print "Opening in existing browser session." and Tauri to surface `Runtime(WebviewRuntimeNotInstalled)` — it is NOT a missing CEF bundle. Fix: `pkill -9 -f '/spreadsheet-app.app/Contents/MacOS/spreadsheet-app|/spreadsheet-app Helper'` then rerun.