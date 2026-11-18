#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    argos::run();
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("Argos capture currently supports macOS and Windows.");
    std::process::exit(1);
}
