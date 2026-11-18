# Argos

Argos is an independent Windows desktop assistant in development. The first milestone shows whether Windows can capture a selected screen or window and the default playback device. This version processes media in memory and displays counters and an audio level. It has no provider connection or recording output.

## Try the Windows build

Use an interactive Windows 11 x64 desktop with the Microsoft Edge WebView2 Runtime installed. If using a portable ZIP, extract the entire folder before running `argos.exe`. Keep the included DLLs beside the executable. Development builds are unsigned.

1. Select **Start capture**, then choose a display or window in the Windows picker.
2. Change something in the chosen source. Check that the dimensions appear and the frame count advances. Play sound through the default Windows output and check the audio meter. The microphone can stay muted.
3. Select **Stop**. The app should return to Idle, reset the counters, and release the Windows capture indicator. Start again and cancel the picker to check cancellation.
4. Repeat with a removable playback device. Disconnect it during capture and check that Argos reports the error and lets you restart. Switching the default output while the old device is still connected takes effect on the next Start. Close Argos both during capture and with the picker open, and check that the process exits.

## Build from source

On Windows install Rust with the stable MSVC toolchain, Visual Studio C++ build tools with a Windows SDK, and the WebView2 Runtime. From the repository root:

```powershell
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo run --release --locked --manifest-path src-tauri/Cargo.toml
```

The UI assets are embedded at build time. The Windows workflow builds the release executable and uploads it as an artifact. See [HANDOFF.md](HANDOFF.md) for validation status and the next milestone.
