# Argos handoff

The owner approved the [Tauri/Rust design](docs/superpowers/specs/2026-11-18-argos-design.md). Windows comes first, including the first Gemini Live session with a user-supplied key. macOS and Linux follow later.

The [Windows capture plan](docs/superpowers/plans/2026-11-18-windows-capture.md) is ready for implementation review. Start with the session lifecycle tests, then the Windows screen picker and loopback audio. Application code has not yet been scaffolded.

On the Apple Silicon development machine, Rust 1.95 successfully type-checked windows-capture 2.0.1 and wasapi 0.27.0 together for x86_64-pc-windows-msvc in a temporary Cargo project. This did not produce a Windows executable or exercise a display, speaker endpoint, or provider. Native Windows build and hardware checks are outstanding.
