# Argos

Argos is a desktop assistant in development for Windows and macOS. It watches a screen or window you choose, listens to what plays through your speakers, and after each spoken turn shows a short suggestion from Claude or Codex. Windows is the current priority.

## How a session works

1. Choose **Claude** (Anthropic API key) or **Codex** (OpenAI API key). Paste the key and select **Save key**. Argos checks the key with the provider before keeping it. Tick **Remember in the system keychain** to keep it across restarts. Otherwise it lasts until you quit.
2. Select **Start** and choose a display or window in the system picker.
3. Play or join a call. When the other side pauses for about half a second, **Heard** shows the transcript and **Suggestion** streams the reply with the time from end of speech to first text.
4. Select **Stop** to end capture and cancel any pending request.

Speech is transcribed on your computer. The first assistant session downloads a 50 MB speech model from huggingface.co into the Argos data folder and verifies its checksum. Each spoken turn sends the transcript, recent turns, and one screenshot (at most 1568 pixels on the long edge) to the provider you chose. Nothing is written to disk and no other provider is contacted. **Local capture only** runs the meters without any network use.

A rejected key, exhausted credit, or spending limit stops the session and says why. Argos never switches to another key or provider on its own. Brief provider outages are retried up to three times per turn.

Set `ARGOS_CLAUDE_MODEL` or `ARGOS_CODEX_MODEL` before launching to use a different model.

Tell the people on a call that you are using an assistant where the law or courtesy requires it.

## Try the Windows build

Use an interactive Windows 11 x64 desktop with the Microsoft Edge WebView2 Runtime, which Windows 11 includes. Download `Argos-Windows-x64.zip` from the repository's `windows-latest` release, extract the whole folder, and run `argos.exe`. Keep any included DLL beside the executable. Development builds are unsigned, so SmartScreen asks for confirmation on first run.

1. With **Local capture only**, select **Start** and choose a display or window. Change something in the source and check that the dimensions appear and the frame count advances. Play sound through the default output and check the audio meter. Select **Stop** and check that Argos returns to Idle with zero counters and the capture border disappears.
2. Start again and cancel the picker. Argos should return to Idle with no error.
3. Save a key for an assistant and start a session. Play a video of someone speaking. Check **Heard**, **Suggestion**, and the latency line. Stop while a suggestion is streaming and check that the text stops.
4. Save a wrong key. Argos should refuse it. Turn off networking during a session and check for the retry notice.
5. Disconnect a removable playback device during capture and check that Argos reports the error and lets you restart. Switching the default output while the old device stays connected takes effect on the next Start.
6. Close Argos during capture and with the picker open. The process should exit within five seconds.

## Try the Mac build

Open Argos.app on Apple silicon running macOS 14 or later and follow the same steps. In Apple's sharing picker, a window carries only that app's audio and a display carries all playback audio. Argos does not offer its own window as a source.

## Build from source

Install Rust, CMake, and a C++ compiler. On Windows use the stable MSVC toolchain with the Visual Studio C++ build tools and set `TRANSCRIBE_CMAKE_ARGS` as [the Windows workflow](.github/workflows/windows.yml) does. On macOS install Xcode and Node.js.

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo run --release --locked --manifest-path src-tauri/Cargo.toml
```

macOS app bundle, written to `src-tauri/target/release/bundle/macos/Argos.app`:

```sh
export MACOSX_DEPLOYMENT_TARGET=14.0
npx --yes --package=@tauri-apps/cli@2.12.6 -- tauri build --bundles app -- --locked
```

Windows executable cross-built from macOS with mingw-w64, run from the repository root so `.cargo/config.toml` applies:

```sh
cargo build --release --locked --target x86_64-pc-windows-gnu --manifest-path src-tauri/Cargo.toml
```

See [HANDOFF.md](HANDOFF.md) for validation status and the next step.
