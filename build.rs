// Compiles the Slint markup into Rust, which `main.rs` pulls in via `slint::include_modules!()`.
// `ui/app.slint` re-exports everything the Rust side touches, so it's the only entry point.
fn main() {
    slint_build::compile("ui/app.slint").expect("failed to compile ui/app.slint");
}
