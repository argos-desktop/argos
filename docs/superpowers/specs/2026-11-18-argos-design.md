# Argos desktop assistant

Approved November 18, 2026, with Windows as the first development and release target. macOS and Linux follow the Windows milestone. Argos is an independent implementation with fresh source, assets, and Git history. Its first end-to-end goal is to show a timely suggestion based on the selected screen and the other side of a call. The owner prefers a user-supplied provider key. Offline inference is outside the first milestone.

## Desktop and capture

Use Tauri 2 with a small bundled frontend and Rust session management. Electron offers a common Chromium capture API and would be a reasonable alternative. Tauri is preferred because the project can grant a small set of commands per window and keep credentials, media, and provider connections behind that boundary. Either choice requires native capture testing.

Start on Windows with one selected display and system playback audio using Windows Graphics Capture and WASAPI loopback. When macOS development begins, investigate ScreenCaptureKit. When Linux development begins, start with the desktop screencast portal and PipeWire on Wayland, selecting a playback monitor for audio. Treat X11 and individual Linux desktop environments as separate validation targets. Record tested OS versions and desktops before advertising support. Prefer maintained capture libraries when they preserve source selection, permission errors, and reliable shutdown.

## Session and data

The user chooses the inputs, sees the destination provider, and explicitly starts a session. Show recording state and a persistent Stop control. Sample screen frames at a bounded rate. Claude (Anthropic key) and Codex (OpenAI key) are the two primary choices. Route each session only to the selected assistant. A user should be able to use either one with its own key alone. Detect the end of a spoken turn and supply its transcript and relevant screen context for a suggestion. Choose and verify the transcription path and supported model inputs during integration. Disclose any remote processing of audio before capture begins. Measure latency from the end of speech to the first visible response. Render responses as untrusted text. Select a supported model when implementing. Model choice and rate should remain changeable without rebuilding capture.

The Rust backend owns the connection and credential lookup. Restrict commands by window and validate their arguments. An entered key goes directly to the backend, which never returns a saved key to the UI. Use the OS credential store when the user elects to remember it. If that store is unavailable, offer session-only use. Keep session content in memory by default. Logs may contain timings and categorized errors, but must omit secrets, request URLs with credentials, and captured content.

Keep capture and upload queues bounded. Discard stale frames on congestion. Report disconnected audio or a revoked capture source visibly. On Stop, device loss, or a terminal provider failure, cancel pending work and release media handles. Allow bounded retries for transient failures while indicating the interruption. Reject late events from an ended session. Permanent authentication failures require user action.

## Delivery and evidence

First demonstrate source selection, real system playback capture, permission denial, Stop, and restart on Windows. Next exercise Claude and Codex independently end to end, using synthetic fixtures for automated tests and recording time to first suggestion in real sessions. Include network loss, invalid credentials, and device removal. Verify that stopping ends capture and outbound media, and inspect diagnostics for sensitive material. Build and run automated checks for Windows first. Apply the same acceptance criteria when each additional platform enters development. Report hardware checks separately from successful builds.

Both coding tools use the root contributor instructions and current handoff note. Git history records durable changes. Developer chat histories are unnecessary for resuming work.
