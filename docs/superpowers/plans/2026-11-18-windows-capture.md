# Windows capture implementation plan

> For agentic workers: implement the checked-in tasks in order. Use superpowers:executing-plans for inline execution or superpowers:subagent-driven-development for delegated execution when available.

**Goal:** Start and stop a real, local Windows Graphics Capture and WASAPI loopback session from Argos and show whether both inputs are working.

**Architecture:** Tauri 2 loads a bundled frontend. Rust owns a single capture session, Windows handles, and worker shutdown. The frontend receives status, counters, and errors. Raw media stays in the backend.

**Tech stack:** Rust, Tauri 2, Windows Graphics Capture, WASAPI shared loopback, HTML and JavaScript. First validation target is Windows 11 x64 in an interactive desktop session.

**Spec:** [Approved Argos design](../specs/2026-11-18-argos-design.md)

## Global constraints

- Windows first. Defer macOS and Linux implementations, packaging, and release claims.
- Keep captures in memory. No media, source titles, credentials, or transcripts in diagnostics.
- Exercise capture locally before the separate Gemini Live milestone. A provider key is unnecessary for this milestone.

## Review focus

Canceling the picker, partial startup, silent playback, device removal, and Stop during startup each need a distinct check. An automated Windows build cannot verify these interactions with a desktop and audio endpoint.

## 1. Session lifecycle

- [ ] In `src-tauri/src/session.rs`, write lifecycle tests for duplicate starts, picker cancellation, failed audio initialization after screen startup, and a stale callback following Stop and restart. Run the tests before and after implementing the controller.
- [ ] Implement Idle, Starting, Running, and Stopping states. Give each start a generation, release partially acquired resources on failure, and join workers before reporting Idle. Never hold the controller lock while waiting for a picker or joining a worker.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml --locked` on Windows. Record the result in the handoff.

## 2. Native capture

- [ ] In `src-tauri/src/capture/windows.rs`, adapt the Windows system picker and frame callback. Honor cancellation. Report observed frame dimensions and count without copying frames to the frontend. Bound any retained frames to the latest one.
- [ ] Open the default render endpoint in shared loopback mode on a COM-initialized audio worker. Read and discard packets after computing a peak level and frame count. Silence is valid. A device error ends the session with a visible reason. Poll or wait with a finite timeout so silence cannot block Stop.
- [ ] Verify startup failure unwinds both workers with controlled fake sources. On Windows, play a test tone through the selected endpoint with the mic muted and confirm that the meter responds. Change the selected display's content and confirm that the frame count advances. Cancel the picker, remove the endpoint, and stop during startup. Restart after each case.

## 3. Desktop and distribution checks

- [ ] In `src-tauri/src/lib.rs`, expose start, stop, and status only to the bundled main window. Configure a restrictive content security policy and omit shell, filesystem, and remote-content permissions. Closing the window must terminate capture.
- [ ] In `ui/`, show Start, Stop, device status, frame count, and audio level. Display “Local capture” as the destination. Keep Stop available during startup. Use text-only error rendering.
- [ ] Add a Windows CI job to run formatting, locked tests, and a release build. Launch the packaged app in a real interactive Windows session. Confirm that Stop releases capture and that the app makes no provider requests. Record OS version, playback device type, results, and outstanding failures without retaining recordings.

## After Windows capture

Add the Gemini Live connection in Rust, Windows Credential Manager with a session-only fallback, bounded streaming, turn detection, and text suggestions. Verify the provider boundary with a fake service before a real BYOK session. Prioritize the Windows end-to-end experience before starting either additional platform.
