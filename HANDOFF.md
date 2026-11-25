# Argos handoff

Work on `main`, the only branch. Windows is the priority target and must work by Wednesday November 25, 6 PM EST. macOS is developed alongside when it costs Windows nothing. The app now runs the whole path: capture, on-device transcription, and streamed suggestions from Claude or Codex with a user-supplied key. See the [README](README.md) for behavior and the [design](docs/superpowers/specs/2026-11-18-argos-design.md) for decisions.

## What exists

- `capture/` sends frames and 48 kHz mono playback audio to `media.rs`, which keeps one downscaled JPEG per second and a bounded audio queue.
- `speech/` resamples, detects speech, and transcribes each turn with Moonshine Streaming Tiny through `transcribe-cpp`. The model downloads on first use with a pinned checksum.
- `provider/` streams from the Anthropic Messages API and the OpenAI Responses API and sorts errors into permanent and transient.
- `pipeline.rs` joins them, drops a suggestion when a newer turn arrives, retries transient failures three times, and ends the session on a permanent failure.
- Solve screen answers what is on screen without waiting for speech. About you is stored in the window's local storage and appended to the instructions. Float on top makes the window frameless, translucent, and always on top. It stays visible to screen sharing by decision.
- `keys.rs` keeps keys in memory and optionally in the OS credential store.

## Checks run

- **November 25, Windows PC, user-run on the CI build.** Capture passes for displays and windows. A window delivers frames only when its content changes, so its count is lower. Key save, refusal of a bad key, remember, and forget pass. The `windows-latest` executable imports only DLLs that ship with Windows.
- **November 25, after adding the coding prompt, Solve screen, About you, and Float on top.** macOS: 38 tests pass, clippy clean. Windows: clippy clean for `x86_64-pc-windows-gnu`. None of the four additions has been run in the app. The transparent frameless window is untested on Windows.
- **November 24, macOS.** End of speech to transcript measured at 550 to 690 ms on synthetic speech.
- **Not run anywhere.** A full live session result has not been reported for either assistant. macOS CI has never been assigned a runner.

## Next

1. On the Windows PC: Float on top (transparency, dragging by the Heard heading, resizing), Solve screen on a coding problem, About you, then one live call per assistant. Record latency here.
2. Confirm the model names `claude-haiku-5-5` and `gpt-6.1-terra` are accepted. Override with `ARGOS_CLAUDE_MODEL` or `ARGOS_CODEX_MODEL` if answers to coding problems are too weak.
3. Publish only files built by the workflow, because local builds embed the builder's file paths.

Known limits: a Windows default-device change applies on the next Start. On macOS a shared window carries only its own app's audio. A CPU without AVX2 cannot run the Windows build. Sign in with ChatGPT, code signing, an updater, and Linux are not started.
