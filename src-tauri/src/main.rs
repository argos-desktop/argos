#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    argos::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Argos capture currently requires Windows.");
    std::process::exit(1);
}
