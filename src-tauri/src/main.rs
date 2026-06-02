// Thin launcher: all app logic lives in the library crate so it stays
// unit-testable and the mobile/desktop entry points share one `run()`.
// The `windows_subsystem` attr suppresses the console window on a release
// Windows GUI build (no effect on debug or other platforms).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    doctree_tauri_lib::run();
}
