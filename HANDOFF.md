# Argos handoff

Resume on `macos-capture`. Windows is the priority target and must work by Wednesday November 25, 6 PM EST. macOS is developed alongside when it costs Windows nothing. The app now runs the whole path: capture, on-device transcription, and streamed suggestions from Claude or Codex with a user-supplied key. See the [README](README.md) for behavior and the [design](docs/superpowers/specs/2026-11-18-argos-design.md) for decisions.

## What exists

- `capture/` sends frames and 48 kHz mono playback audio to `media.rs`, which keeps one downscaled JPEG per second and a bounded audio queue.
- `speech/` resamples, detects speech, and transcribes each turn with Moonshine Streaming Tiny through `transcribe-cpp`. The model downloads on first use with a pinned checksum.
- `provider/` streams from the Anthropic Messages API and the OpenAI Responses API and sorts errors into permanent and transient.
- `pipeline.rs` joins them, drops a suggestion when a newer turn arrives, retries transient failures three times, and ends the session on a permanent failure.
- `keys.rs` keeps keys in memory and optionally in the OS credential store.

## Checks run on November 24

- **macOS, Apple silicon, macOS 27.0.1.** 38 tests pass and one measurement test is ignored. Clippy with warnings denied is clean. Measured end of speech to transcript at 550 to 690 ms on synthetic speech.
- **Windows x64.** Clippy is clean for `x86_64-pc-windows-gnu` and the release executable cross-links from macOS. Nothing has run on Windows. Frame readback, the software-adapter fallback, the credential store, and speech speed are unverified there.
- **Earlier Mac capture session, before today's changes.** Start opened Apple's picker, Escape returned to Idle, restart worked, both counters rose during capture, and Stop reset them. A meter response to a known tone was never confirmed. The prepared QuickTime clip `Argos Capture Test.mp4` alternates one second of tone with one second of silence.
- **Not run anywhere.** A live session with a real key. The model download through the app. Either GitHub workflow. The new picker exclusion that should explain the earlier Mac session with video and no audio, which looked like Argos capturing its own window.

## Next

1. Publish the repository and get the Windows workflow to produce the `windows-latest` release. The MSVC build of `transcribe-cpp` is untested. If it fails, upload the mingw cross-build instead.
2. Run the [Windows checks](README.md#try-the-windows-build) on real hardware and fix what they show. Record the OS build, CPU, playback device, and measured latency here.
3. Run one live session per assistant. Confirm the default model names `claude-haiku-5-5` and `gpt-6.1-terra` are accepted. Override with `ARGOS_CLAUDE_MODEL` or `ARGOS_CODEX_MODEL` if not.
4. Run the same checks on the Mac.

Known limits: a Windows default-device change applies on the next Start. On macOS a shared window carries only its own app's audio. A CPU without AVX2 cannot run the Windows build. Sign in with ChatGPT, code signing, an updater, and Linux are not started.
