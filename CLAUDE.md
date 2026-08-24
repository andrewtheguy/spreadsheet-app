- no backward compatibility
- use project root tmp/ dir for test data
- run cargo clippy after changes
- run cargo test after rust code changes
- no cargo fmt
- desktop mac and windows only, no mobile or linux support
- implement backend as much as possible in Rust, so that the backend is more ui agnostic and can be reused in other contexts

## Slint UI

- The UI is Slint (`ui/*.slint`), compiled by `build.rs` and pulled in with
  `slint::include_modules!()`. There is no webview and no Node toolchain.
- Layering: `sheet-core` (pure spreadsheet logic) → `src/state.rs` (controller,
  no Slint types) → `src/view.rs` (display models, no Slint types) →
  `src/main.rs` + `ui/` (Slint glue). Put new behaviour in the lowest layer that
  can hold it.
- Widgets that write their own property (`ComboBox.current-index`,
  `CheckBox.checked`) need a two-way `<=>` binding all the way up to the
  `MainWindow` property. A one-way binding is silently replaced the first time
  the user interacts, and the Rust side stops being able to push values back.
- Every action is reachable from the `MenuBar` in `ui/app.slint`, which is also
  where its keyboard shortcut lives. Add new actions there too — on macOS it
  becomes the system menu bar, which is what UI automation drives during QA.
- `cargo run` is enough to launch the app.
- Slint's default renderer needs an OpenGL 3 driver, which RDP sessions, VMs and
  Windows Server don't have. `main.rs` relaunches itself once with
  `SLINT_BACKEND=winit-software` when startup fails — don't "simplify" that away.
  Verified on the `windows-ci-build` ssh host, which has no desktop session and
  so always takes the fallback path.
- That host sets `CARGO_TARGET_DIR=C:\ci-cache\target` globally, so the binary
  is not under the repo's `target/`. It has no git; ship source with
  `COPYFILE_DISABLE=1 tar czf` + `scp` (plain `tar` adds `._*` AppleDouble
  files).
