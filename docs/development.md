# Development contract

This document owns Aercast's product behavior, settings, security boundary,
engineering decisions, acceptance, and non-goals. The
[README](../README.md) is the product homepage, the
[verification record](verification.md) owns evidence, and
[UI design](ui-design.md) owns visual rules.

## Release state

Aercast v0.1.4 is an early x86-64 Linux release distributed through AUR, GitHub
packages, and a prebuilt program archive. Availability is not evidence of
support until an artifact passes a real install and launch check; see the
[current verification gap](verification.md#current-qualification).

The recorded niri baseline remains the latest complete real qualification.
Changes after it have automated coverage but have not repeated the real Portal,
PipeWire, Zen, Chromium, Safari, or constrained-network workflow. See the
[current verification gap](verification.md#current-qualification).

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
- Mobile 1440p, 120 FPS, AirPlay, background playback, adaptive bitrate, split
  audio/video delivery, and new transport or media protocols are out of scope.

## Host behavior

### Desktop lifecycle

- Aercast is one GUI process. Its Host window is an ordinary resizable,
  tileable `normal` window with system decorations, `aercast` as its Wayland
  application ID, an initial `960×640` logical-pixel size hint, and no Host
  video preview. It has no maximum size and does not persist geometry; the
  compositor remains responsible for placement and tiling. Minimum geometry is
  defined in [UI design](ui-design.md#window-and-layout).
- Its icon-and-text sidebar switches among **Overview**, **Viewers**, and
  **Settings**.
- Closing the Host window from the compositor hides it without stopping a
  share, discarding a Settings draft, or exiting. `iced::daemon` keeps the
  process alive. The compositor title bar is the only window-close control.
- Only one process instance may run. A later desktop or command-line launch and
  tray activation bring the existing window in front and focus it once; the
  window does not remain always on top. Unsupported compositor activation is
  best-effort.
- One state-independent `ksni` tray item always opens the window on primary
  activation. Its displayed title and tooltip title are **Aercast**. Its menu
  exposes status, Show, the applicable Start/Copy/Stop action, and Quit; while
  sharing with online Viewers, status is `Status: Sharing: N` using the online
  count only.
- Quitting while sharing or with unsaved Settings changes requires confirmation.
  If both apply, one confirmation covers stopping the share and discarding the
  draft. Confirmed Quit revokes the token, disconnects Viewers, closes Portal
  and media state, removes desktop integrations, discards the draft, and exits.

### Overview

- Overview shows the complete sharing stage, selected source type, and exactly
  one primary Start, Cancel, or Stop action; an in-progress stage replaces that
  action with non-interactive status.
- Its link group exposes the read-only current link, Refresh Link, and Copy.
  Copy shows the visible text **Copied** for 1.5 seconds. A direct loopback link
  is marked **This device only** and links directly to Network settings;
  Aercast does not guess a LAN interface address.
- Its Viewer summary shows online/total count and the worst available online
  RTT and playback lag, with a direct route to Viewers.
- Its Active media summary reports actual resolution, frame rate, bitrate,
  encoder, audio state, and audio-exclusion count. When Saved and Active differ,
  the mismatch is explicit.
- Refresh Link and Quit confirmations remain in place in the page rather than
  opening a modal window.

### Sharing and links

- **Start Sharing** opens the Portal picker. The same primary action becomes
  **Cancel** while selecting and **Stop Sharing** while active.
- Stop closes media and the Portal session but keeps the link valid. Existing
  Viewer pages wait, and a later Start reuses the link.
- Refresh Link works while waiting or sharing. With connected Viewers it
  requires confirmation; it rotates the token without restarting capture,
  disconnects old streams, clears Viewer history, and makes all old token routes
  return the same `404`.
- Process exit is the only other token invalidation boundary.
- After the first complete media fragment, zero online Viewers starts a fixed
  two-second grace period. A Viewer connection cancels it; the transition from
  the last online Viewer to zero starts it again, while telemetry does not
  extend it. Expiry pauses and removes the media pipeline and selective-audio
  links but retains the Portal session, selected source, token, video plan, and
  negotiated capture caps. The Host UI, tray, and notifications remain in the
  Sharing state.
- While media sleeps, only a valid, unblocked `GET /stream` for the current
  token generation requests a wake, and the request still receives the normal
  `425` polling response. Concurrent requests merge into one wake that reopens
  the restricted PipeWire remote and rebuilds the same media pipeline. Page,
  HEAD, telemetry, invalid-token, invalid-identity, blocked-identity, and old
  generation requests do not wake it.
- Applying current-share quality or audio while asleep updates the Active
  snapshot without waking media. Refresh Link rotates the token and remains
  asleep. Stop, Quit, Portal closure, and HTTP server failure remain terminal
  controls. Successful sleep consumes no media-recovery attempt; wake failures
  use the existing bounded recovery policy.

### Viewer management and playback

- The Viewers page shows online/total count in one scrollable list of compact
  two-line rows. Each row shows state, IP, and Block first, then accumulated
  connection duration, RTT, and playback lag. Online rows precede offline
  history.
- For a loopback TCP peer, displayed IP prefers `X-Real-IP`, then the first
  `X-Forwarded-For` value, then the peer. Non-loopback peers ignore both headers
  completely, so a trusted reverse proxy must connect from `127/8` or `::1` and
  replace client-supplied forwarding headers. Sequential indices distinguish
  Viewers sharing an IP.
- Telemetry reports the previous request RTT and buffered-end playback lag every
  two seconds; the Host accepts at most one state-changing report per online
  Viewer per second, and faster valid reports still return `204` without
  changing state or notifying the GUI. Offline or six-second-old values are
  unavailable.
- Each token keeps at most 100 in-memory Viewer records. At capacity, only an
  offline, unblocked record may be evicted; if all records are online or blocked,
  a new identity receives `429`. Viewer IDs, IPs, and telemetry are neither
  persisted nor written to ordinary logs.
- A random browser-scoped Viewer ID merges reconnects. A second tab takes over
  the active session. Host Block is cooperative identity control: that
  presented ID stays blocked across Stop and Start until Refresh Link or process
  exit, but a client can present a different ID. Refresh Link is the only Host
  control that revokes a token already held by a malicious client.
- Block confirms on the same button: the first click changes it to
  **Confirm block**, and a second click within three seconds blocks the Viewer.
  It never opens a modal or an extra confirmation region. Timeout, leaving the
  page, the Viewer going offline, or list replacement cancels confirmation.
- The once-per-second Host UI tick runs only while Overview or Viewers is open
  and at least one Viewer is online. It refreshes health summaries and expires
  pending Block confirmation.
- Connection duration accumulates from the Host's monotonic clock and pauses
  while offline.
- The Viewer is one square-cornered, viewport-filling `contain` video using only
  native controls. It tries unmuted autoplay, retries muted when policy blocks
  it, and remembers only a user-selected muted state. While video is not
  playing, a passive status layer uses the following fixed text; it is absent
  visually and from the accessibility tree while playing. Diagnostics stay in
  the console and document state attributes.

| Viewer state | Visible text |
| --- | --- |
| Connecting | `Connecting…` |
| Waiting / `425` | `Waiting for the Host to start sharing…` |
| Reconnecting | `Connection interrupted. Reconnecting…` |
| Stopped / `404` after page load | `This share has ended or the link is invalid.` |
| Blocked / `409` | `Blocked by the Host.` |
| Autoplay error | `Playback could not start. Press Play to try again.` |
| Unsupported media source | `This browser cannot play the shared media.` |
| Inactive tab | `Playback moved to another tab. Press Play to resume here.` |

An initially invalid token still receives the server's uniform `404` instead
of the Viewer document. The status layer's visual and ARIA rules are defined in
[UI design](ui-design.md#browser-viewer).

- The Viewer selects `ManagedMediaSource` when available and otherwise uses
  `MediaSource`; managed playback disables remote playback so Safari opens the
  source without requiring an AirPlay alternative. Missing media-source support
  fails before requesting the stream. Starting playback never blocks continued
  fragment download and append.
- Playback starts after 1.5 seconds are buffered and stays about one second
  behind the live edge. Lag from 1.5 through 3 seconds catches up gradually at
  up to 1.15×; larger lag and manual seeking return to that live position once
  playback has started, without issuing another seek while one is in progress.
  Normal media replacement reconnects immediately; a failed request waits 500
  ms, and a waiting `425` response polls after 500 ms. Automatic reconnect
  continues until the share ends or the Host blocks the Viewer.
- Bounded per-Viewer delivery may end and replace a lagging response so the
  Viewer jumps forward as one synchronized stream. If sustained throughput is
  below the configured video-plus-audio rate, continuous audio is not
  guaranteed; there is no audio-priority side channel.

## Settings contract

| Area | Behavior |
| --- | --- |
| Presets | `720p60 / 6 Mbps`, `1080p60 / 12 Mbps`, `1440p60 / 24 Mbps` |
| Custom quality | Encoder-supported width and height; preserve aspect ratio with centered black padding |
| Frame rate | `30 / 60 / 120 FPS` |
| Bitrate | Preset value, encoder default for untouched Custom, or manual `1–500 Mbps` |
| Encoder | Auto prefers detected hardware and falls back to software; detected implementations may be selected; codec is not exposed |
| System audio | Enabled by default; no microphone |
| Audio bitrate | AAC-LC, 48 kHz stereo; `96 / 128 / 160 kbps`, default `128 kbps` |
| Audio exclusions | Permanent toggleable Communication rule; toggleable/deletable Discord, Vesktop, and Steam Voice defaults; add active applications |
| Network | `127.0.0.1:8877` by default; configurable unicast address, port, and optional `scheme://host:port` public base URL |
| Notifications | Enabled by default |

Unsupported saved quality blocks Start instead of being repaired or downgraded.
Quality, Audio, Network, and Notifications remain fully expanded; professional
options are not hidden behind Basic/Advanced modes or accordions.

Settings maintains three explicit layers:

- **Saved** is the complete settings value persisted on disk. Start Sharing
  reads only Saved, never an uncommitted edit.
- **Draft** is UI-only and receives every field, notification, audio-exclusion,
  and application-list edit. It survives page changes, hiding, and reopening
  the Host window. Revert replaces the whole Draft with Saved.
- **Active** is the complete settings snapshot used by the current share. Saved
  and Active may differ and the Host reports that state explicitly.

Draft validates as one complete candidate. Invalid fields show a local field or
section error; Apply is enabled only when every field is valid and Draft differs
from Saved. Apply atomically commits the whole page:

- Video changes reuse the asynchronous encoder capability probe. Each probe is
  tied to the exact Draft revision that started it; a late result cannot commit
  or overwrite a newer Draft. Unsupported quality remains an error rather than
  being repaired or downgraded.
- If Network is unchanged, the complete candidate is replace-written once. If
  Network changed while stopped, Aercast first binds the candidate listener,
  then saves, then swaps listeners. A bind or save failure keeps both Saved and
  Draft unchanged.
- Any Network change while sharing blocks the entire Apply and tells the Host
  to Stop first; Aercast never partially commits the other categories.
- A successful commit normalizes Draft to the committed Saved value. A failed
  commit retains the prior Saved value and the user's Draft.

After a successful commit, if Saved quality or audio differs from Active,
Settings offers one primary **Apply to current share** action. With no online
Viewer it executes directly; with an online Viewer the same control requires a
second in-place confirmation and does not open a modal. Quality and audio are
rebuilt together while retaining the Portal session, selected source, link
token, Viewer identities, and Viewer history.

The candidate media configuration gets one startup attempt. If it fails before
reaching Sharing, the Host reports the apply failure and immediately restores
the previous successful Active snapshot; Saved remains the new configuration,
so the mismatch and retry action remain visible. Failure to restore the old
snapshot enters the existing recovery path of at most three attempts and then
the terminal error path. Once the candidate reaches Sharing it becomes Active,
clears rollback state, and later faults use the normal recovery policy.

Notifications are sent only while the Host window is hidden for the first
transition into sharing, return to waiting, first online Viewer connection,
last online Viewer disconnection, and Host or network failure. They never
contain a link or token. Pointer behavior and autostart have no setting.
Settings are replace-written internal state at
`$XDG_CONFIG_HOME/aercast/settings.json`, not a public hand-edited interface.
The redesign does not change that JSON schema, external HTTP routes, token
format, or `425`/`404`/`409`/`429` semantics; the Host and Viewer present `409`
as Block.

## Security and network

- The link token is 32 bytes of operating-system CSPRNG output and is a bearer
  credential, not transport security.
- Page, media, telemetry, and controls are same-origin. Invalid, expired, and
  superseded routes all return `404` without revealing prior state.
- Tokens, Viewer IDs, IP addresses, and telemetry must not enter ordinary logs
  or third-party requests. Reverse proxies must redact tokenized paths.
- Forwarded client-IP headers are a loopback-proxy convention, not an
  authentication boundary. A non-loopback TCP peer can never override its
  observed address with those headers.
- Loopback is the safe default. Trusted-LAN binding is explicit. Untrusted or
  public access requires an external HTTPS reverse proxy; Aercast owns no
  domains, certificates, tunnels, port forwarding, or NAT traversal.

## Engineering decisions

- One Rust process owns one Portal source and, while media is awake, one
  GStreamer capture, encode, and mux pipeline per share regardless of Viewer
  count. Completed fMP4 fragments fan out through Axum; bounded per-Viewer
  delivery drops lagging readers.
- Media control carries the complete `ShareSettings` snapshot through
  `Command::Apply(ShareSettings)`, `ShareStop::Apply(ShareSettings)`, and
  `HostEvent::Sharing(ShareSettings)`. The Host keeps `active_share` and
  `applying_share` snapshots so an explicit Apply can reset the recovery budget,
  replace quality and audio together, and restore the prior successful value
  without another Portal selection.
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
  `mp4mux` while real MSE and late-join checks pass. A manual video bitrate uses
  a 100 ms x264 VBV with CBR HRD or a 100 ms VA-API CPB to limit short bursts;
  untouched Custom keeps the encoder default. The VA-API path accepts Portal
  DMA-BUF, lets `vapostproc` pass compatible frames through, and keeps converted
  raw frames in VA memory through `vah264enc`; x264 remains the CPU fallback.
  This is a zero-copy-capable raw video path, not an end-to-end zero-copy claim,
  and remains unqualified until measured on the target hardware.
- For an Auto share that selected VA-API and has not already fallen back, one
  hardware-path failure may consume one media recovery and switch that share to
  a freshly probed x264 plan. Eligibility uses the named
  GStreamer source, structured error domain/code, and `flow-return`: the
  whitelist covers the encoder, video converter, Portal video/format, and H.264
  parser negotiation cases defined in code. EOS, Portal resource failure,
  audio, mux, appsink, unknown sources, and any concurrent non-whitelisted error
  do not trigger fallback. Explicit VA-API never falls back; a successful switch
  clears VA capture caps so x264 renegotiates.
- HTTP state owns the token separately from replaceable media state. Viewer
  telemetry and Host controls use ordinary HTTP, not WebSocket.
- Desktop integration uses iced Wayland/wgpu, `ksni`, direct `zbus`, ashpd, and
  serde. Prefer current dependencies and platform APIs; do not add speculative
  abstraction or cross-platform layers.
- Pull requests run the canonical static checks on `ubuntu-latest`. A pushed
  stable `vX.Y.Z` tag must match the Cargo package version before the same
  checks build and publish x86-64 `.deb` and binary tarball assets.
  Workflow actions use their latest major release; Arch remains source-built.
