# Argos handoff

Work on `main`, the only branch. Windows is the priority target and must work by Wednesday November 25, 6 PM EST. macOS is developed alongside when it costs Windows nothing. The app now runs the whole path: capture, on-device transcription, and streamed suggestions from Claude or Codex with a user-supplied key. See the [README](README.md) for behavior and the [design](docs/superpowers/specs/2026-11-18-argos-design.md) for decisions.

## What exists

- `capture/` sends frames and 48 kHz mono playback audio to `media.rs`, which keeps one downscaled JPEG per second and a bounded audio queue.
- `speech/` resamples, detects speech, and transcribes each turn with Moonshine Streaming Tiny through `transcribe-cpp`. The model downloads on first use with a pinned checksum.
- `provider/` streams from the Anthropic Messages API and the OpenAI Responses API and sorts errors into permanent and transient.
- `pipeline.rs` joins them, drops a suggestion when a newer turn arrives, makes up to three attempts per turn, and ends the session on a permanent failure.
- Solve screen waits for a captured image. Call turns and screen requests share recent context. The prompt distinguishes coding, debugging, and design questions.
- Your background accepts pasted notes and local PDF, TXT, and Markdown imports. The editable text is saved locally and included in requests when the user starts a session.
- The same window floats over the editor. Windows keeps content hidden until native capture exclusion succeeds at startup and after float transitions. A failure keeps the window contents hidden and requests a session stop. On modern macOS, share the editor window or tab to keep Argos out of the meeting share.
- `keys.rs` keeps keys in memory and optionally in the OS credential store.

## Checks run

- **November 25, Windows PC, user-run on the CI build.** Capture passes for displays and windows. A window delivers frames only when its content changes, so its count is lower. Key save, refusal of a bad key, remember, and forget pass. The `windows-latest` executable imports only DLLs that ship with Windows.
- **November 25, after adding the coding prompt, Solve screen, About you, and Float on top.** macOS: 38 tests pass, clippy clean. Windows: clippy clean for `x86_64-pc-windows-gnu`. The transparent frameless window is untested on Windows.
- **November 24, macOS.** End of speech to transcript measured at 550 to 690 ms on synthetic speech.
- **November 25, local changes after `aca7eb8`.** Mac tests passed (40, one speech benchmark ignored). Mac and Windows GNU clippy passed, as did rustfmt and the JS syntax check. Built the Mac app and cross-built the Windows x64 release executable. On the packaged Mac app, pressing Float on top and Exit float changed the native window layer from 0 to 5 and back to 0. A Chromium UI check with simulated native commands exercised import, oversized input, session notes, floating controls, and streamed output.
- **Hosted macOS build.** The [run for `c901d50`](https://github.com/argos-desktop/argos/actions/runs/45546539567) reached the test compile step. The Swift bridge in `apple-metal` failed to compile because the runner SDK lacked Metal APIs including `MTLTensorUsage`. Check the SDK selected by the workflow.
- **Windows privacy follow-up.** The UI regression passed with simulated Windows success, failure, and retry. Windows GNU release build and clippy passed. The executable imports both display-affinity APIs. The local privacy test ZIP includes the required WebView2 loader. A real meeting viewer test is pending.

## Next

1. Test the workflow-built `Argos-Windows-x64.zip` from the `windows-latest` release containing this privacy change. Confirm from a second meeting participant that setup and floating mode stay out of a full-display share. Check transparency, dragging, resizing, résumé import, Solve screen, and one live call per assistant. Record latency here.
2. Confirm the model names `claude-haiku-5-5` and `gpt-6.1-terra` are accepted. Override with `ARGOS_CLAUDE_MODEL` or `ARGOS_CODEX_MODEL` if answers to coding problems are too weak.
3. Publish only files built by the workflow, because local builds embed the builder's file paths.

Known limits: a Windows default-device change applies on the next Start. On macOS a shared window carries only its own app's audio. A CPU without AVX2 cannot run the Windows build. Sign in with ChatGPT, code signing, an updater, and Linux are not started.
