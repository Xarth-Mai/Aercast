# Development contract

This document owns Aercast's product behavior, settings, security boundary,
engineering decisions, active work, acceptance, and non-goals. The
[README](../README.md) is the product homepage, the
[verification record](verification.md) owns evidence, and
[UI design](ui-design.md) owns visual rules.

## Release state

Aercast v0.1.1 is an early x86-64 Linux release distributed through AUR,
distribution packages, and a prebuilt program archive. Availability is not
evidence of support until an artifact passes a real install and launch check.

The recorded niri baseline completed the original product phases. Changes after
that qualification have automated coverage but have not repeated the real
Portal, PipeWire, Zen, and Chromium workflow. Phase 6 is active to make the
existing Viewer compatible across browser engines and qualify available real
platforms. See the [current verification gap](verification.md#current-qualification).

## Product boundary

- Aercast is a lightweight native Linux/Wayland GUI for gamers. niri is the
  formal validation environment; other Wayland desktops remain best-effort
  until separately verified.
- Viewer compatibility targets representative current stable browsers from the
  Chromium family (Chrome and Edge), Gecko family (Firefox and Zen), and WebKit
  family (Safari on macOS and iOS 17.1 or newer). Zen and Chromium remain the
  local Linux test vehicles. A platform becomes qualified only after its exact
  browser, operating system, device, quality, and network path pass a recorded
  real check; untested derivatives and codec-incomplete builds are best-effort.
- The Host selects exactly one monitor or window through the non-persistent XDG
  ScreenCast Portal. Capture never starts without explicit approval.
- One synchronized H.264/AAC-LC fMP4 stream is encoded once and sent directly
  over HTTP to every Viewer. Video and audio are not split into separate public
  streams.
- System audio can exclude specified applications. Exclusions survive restart
  and never change what the Host hears. Microphone and
  selected-applications-only modes are out of scope.
- Keep the product focused: do not add accounts, chat, calls, cameras, WebRTC
  infrastructure, recording, cloud relays, remote control, file transfer, or
  built-in NAT traversal.

## Host behavior

### Desktop lifecycle

- Aercast is one GUI process. Its `920×520` logical-pixel window is fixed-size,
  uses system decorations, and has no Host video preview.
- Its sidebar switches among **Share**, **Viewers**, and **Settings**.
- Closing either the content window or compositor window hides it without
  stopping a share or exiting. `iced::daemon` keeps the process alive.
- Only one process instance may run. A later desktop or command-line launch and
  tray activation bring the existing window in front and focus it once; the
  window does not remain always on top. Unsupported compositor activation is
  best-effort.
- One state-independent `ksni` tray item always opens the window on primary
  activation. Its displayed title and tooltip title are **Aercast**. Its menu
  exposes status, Show, the applicable Start/Copy/Stop action, and Quit; while
  sharing with online Viewers, status is `Status: Sharing: N` using the online
  count only.
- Quitting while sharing requires confirmation. Confirmed Quit revokes the
  token, disconnects Viewers, closes Portal and media state, removes desktop
  integrations, and exits.

### Sharing and links

- **Start Sharing** opens the Portal picker. The same primary action becomes
  **Cancel** while selecting and **Stop Sharing** while active.
- Stop closes media and the Portal session but keeps the link valid. Existing
  Viewer pages wait, and a later Start reuses the link.
- Refresh Link works while waiting or sharing. With connected Viewers it
  requires confirmation; it rotates the token without restarting capture,
  disconnects old streams, clears Viewer history, and makes all old token routes
  return the same `404`.
- Copy briefly changes to a check mark. Process exit is the only other token
  invalidation boundary.

### Viewer management and playback

- The Viewers page shows online/total count and rows containing IP, accumulated
  connection duration, RTT, playback lag, and Disconnect. Online rows precede
  offline history.
- Displayed IP prefers `X-Real-IP`, then the first `X-Forwarded-For` value, then
  the TCP peer. A trusted reverse proxy must replace client-supplied headers.
  Sequential indices distinguish Viewers sharing an IP.
- Telemetry reports the previous request RTT and buffered-end playback lag every
  two seconds; offline or six-second-old values are unavailable.
- Each token keeps at most 100 in-memory Viewer records. Viewer IDs, IPs, and
  telemetry are neither persisted nor written to ordinary logs.
- A random browser-scoped Viewer ID merges reconnects. A second tab takes over
  the active session. Host Disconnect permanently blocks that Viewer until
  Refresh Link or process exit; there is no manual retry.
- Connection duration accumulates from the Host's monotonic clock and pauses
  while offline.
- The Viewer is one square-cornered, viewport-filling `contain` video using only
  native controls. It tries unmuted autoplay, retries muted when policy blocks
  it, and remembers only a user-selected muted state. Diagnostics stay in the
  console and document state attributes.
- The Viewer selects `ManagedMediaSource` when available and otherwise uses
  `MediaSource`; managed playback disables remote playback so Safari opens the
  source without requiring an AirPlay alternative. Missing media-source support
  fails before requesting the stream. Starting playback never blocks continued
  fragment download and append.
- Manual seeking returns to the live edge. Lag from 350 ms through 2 s catches
  up gradually at up to 1.15×; larger lag seeks to 150 ms behind the buffered
  end once playback has started, without issuing another seek while one is in
  progress. Automatic reconnect continues until the share ends or the Host
  blocks the Viewer.

## Settings contract

| Area | Behavior |
| --- | --- |
| Presets | `720p60 / 6 Mbps`, `1080p60 / 12 Mbps`, `1440p60 / 24 Mbps` |
| Custom quality | Encoder-supported width and height; preserve aspect ratio with centered black padding |
| Frame rate | `30 / 60 / 120 FPS` |
| Bitrate | Preset value, encoder default for untouched Custom, or manual `1–500 Mbps` |
| Encoder | Auto prefers detected hardware and falls back to software; detected implementations may be selected; codec is not exposed |
| System audio | Enabled by default; no microphone |
| Audio exclusions | Permanent toggleable Communication rule; toggleable/deletable Discord, Vesktop, and Steam Voice defaults; add active applications |
| Network | `127.0.0.1:8877` by default; configurable unicast address, port, and optional `scheme://host:port` public base URL |
| Notifications | Enabled by default |

Unsupported saved quality blocks Start instead of being repaired or downgraded.
Saved quality changes apply to the next Start; an active share and its media-only
recoveries keep the snapshot taken before Portal selection. Audio edits apply
to an active share only through **Apply to Current Share**, which coordinates
one media restart. Network rebinding is stopped-only and warns that waiting
pages using the old address may not recover.

Notifications are sent only while the Host window is hidden for the first
transition into sharing, return to waiting, first online Viewer connection,
last online Viewer disconnection, and Host or network failure. They never
contain a link or token. Pointer behavior and autostart have no setting.
Settings are replace-written internal state at
`$XDG_CONFIG_HOME/aercast/settings.json`, not a public hand-edited interface.

## Security and network

- The link token is 32 bytes of operating-system CSPRNG output and is a bearer
  credential, not transport security.
- Page, media, telemetry, and controls are same-origin. Invalid, expired, and
  superseded routes all return `404` without revealing prior state.
- Tokens, Viewer IDs, IP addresses, and telemetry must not enter ordinary logs
  or third-party requests. Reverse proxies must redact tokenized paths.
- Loopback is the safe default. Trusted-LAN binding is explicit. Untrusted or
  public access requires an external HTTPS reverse proxy; Aercast owns no
  domains, certificates, tunnels, port forwarding, or NAT traversal.

## Engineering decisions

- One Rust process owns one Portal source and one GStreamer capture, encode, and
  mux pipeline per share regardless of Viewer count. Completed fMP4 fragments
  fan out through Axum; bounded per-Viewer delivery drops lagging readers.
- Video uses the Portal's restricted PipeWire remote. Audio observes the regular
  PipeWire graph, taps only safe allowed stereo playback streams, and preserves
  their existing sink routes. Audio-off and no-source states keep a silent track
  so the MSE schema remains stable.
- Audio identity is `application.id`, then process binary, then application
  name. PID is diagnostic only. Communication is excluded by the permanent
  default rule; unknown, mono, surround, duplicate, or unsafe layouts stay
  silent.
- On PipeWire 1.6.8, the Aercast capture node uses `node.passive=in`; exact links
  do not request `link.passive`. Missing-resource `ENOENT`, including from core
  object `0`, triggers graph rebuild rather than ending the share. No daemon-wide
  passive-link override is allowed.
- GStreamer owns media conversion, clocks, H.264, AAC-LC, and fMP4. Keep
  `mp4mux` while real MSE and late-join checks pass. Hardware encoding and
  DMA-BUF optimization require measurements; do not add factories before a
  second verified implementation exists.
- HTTP state owns the token separately from replaceable media state. Viewer
  telemetry and Host controls use ordinary HTTP, not WebSocket.
- Desktop integration uses iced Wayland/wgpu, `ksni`, direct `zbus`, ashpd, and
  serde. Prefer current dependencies and platform APIs; do not add speculative
  abstraction or cross-platform layers.
- Pull requests run the canonical static checks on `ubuntu-latest`. A pushed
  stable `vX.Y.Z` tag must match the Cargo package version before the same
  checks build and publish x86-64 `.deb`, binary tarball, and checksum assets.
  Workflow actions use their latest major release; Arch remains source-built.

## Completed product work

**Phases 1–5 — 2026-08-25 to 2026-08-27:** Portal capture, browser fMP4
playback, per-application audio exclusions, same-link recovery, desktop
lifecycle, settings, and one-encoder three-Viewer workflows completed the
recorded niri baseline.
[Evidence](verification.md#qualified-baseline)

Git history retains the old phase checklists.

## Active product work

**Phase 6 — Cross-platform Viewer compatibility**

- Keep one capability-detected fMP4 Viewer implementation with no browser-name
  branches and no changes to the public HTTP interface or media format. Host
  changes are limited to the tray naming, online count, and one-shot window
  activation polish specified in Desktop lifecycle.
- Qualify 720p60 at 6 Mbps and 1080p60 at 12 Mbps first on the available real
  Linux Zen, Linux Chromium, and iOS Safari devices. Playback must advance for
  two minutes without repeated stalls, reconnects, or cancellation errors;
  audio, telemetry, same-link recovery, Disconnect, and Refresh Link must work.
- Windows Chrome, Edge, and Firefox; macOS Safari; and Android Chrome and Firefox
  remain explicit qualification gaps until the same scenarios run on real
  platforms. Passing one engine representative does not qualify another OS.
- Mobile 1440p, 120 FPS, AirPlay, background playback, adaptive bitrate, and new
  transport or media protocols are non-goals for this Phase.
