# Development direction

This document owns product commitments, exact Host and Viewer behavior,
engineering decisions, settings, security boundaries, the current Phase,
acceptance criteria, risks, non-goals, and the next-Phase entry condition; it
does not own homepage copy, visual tokens, or raw test output.

## Status

Aercast is pre-alpha. Phase 5, desktop productization, is active. Phase 4 is
complete: current niri runs prove the same-page Zen-first and Chromium-second
waiting, playback, bounded media recovery, Stop, and later Start lifecycle,
while a real PipeWire graph proves Communication exclusion and PID-independent
stable identity without changing the Host's sink routes.

## Product commitments

- Aercast is a lightweight, native Linux/Wayland screen-sharing GUI for gamers.
  niri is the formal validation environment; other Wayland desktops are
  best-effort until separately verified. Flatpak is deferred.
- Current stable desktop Firefox is the primary Viewer target and is validated
  before first-class Chromium. Zen Browser is the local Firefox-family test
  vehicle. Safari, mobile browsers, and codec-incomplete builds are not launch
  promises.
- The Host chooses exactly one monitor or window through the non-persistent XDG
  ScreenCast Portal. Capture never starts before explicit approval.
- One synchronized H.264/AAC-LC fMP4 stream is encoded once and sent directly
  over HTTP to every Viewer. Video and audio are never split into separate
  public streams.
- Selective system audio is a release gate. Exclusions must survive application
  restart and must not change what the Host hears. There is no microphone or
  selected-applications-only audio mode.
- Aercast is not a meeting, recording, or remote-access platform. Do not add
  accounts, chat, calls, cameras, WebRTC, signaling, ICE, STUN, TURN, an SFU,
  cloud relays, recording, remote control, clipboard, file transfer, or built-in
  NAT traversal.

## Product behavior

### Window, tray, and application lifecycle

- The Host has only a GUI. The main window is fixed at `700×440` logical pixels,
  is not resizable, and uses ordinary system decorations. Its fixed size lets
  niri's native heuristic float it without a user window rule.
- iced runs with daemon lifetime. Closing the window always hides it, including
  when no tray watcher exists; it does not stop sharing or exit Aercast.
- Only one process instance may run. A later launch displays and activates the
  existing window.
- `ksni` supplies one fixed, state-independent StatusNotifierItem using
  `assets/aercast-icon.png`. Primary activation always displays the window.
  Its menu dynamically shows status, Show, Start or Copy/Stop, and Quit.
- Quitting during an active share requires confirmation. A confirmed Quit
  revokes the token, disconnects Viewers, closes media and Portal state, removes
  the tray item and listener, then exits.

### Host share controls

- The share view has a settings icon at the top right; the settings view has a
  back icon. There is no Host video preview.
- **Start Sharing** opens the Portal picker, which owns monitor/window choice
  and pointer behavior. The same stateful action becomes **Cancel** during
  selection and **Stop Sharing** while active. The view also shows the approved
  source, share link, Copy, Refresh Link, and Viewer list.
- Stop closes media and the Portal session. The current link remains valid and
  returns Viewers to waiting. A later Start reuses it.
- Refresh Link is available while waiting or sharing. It creates a new token
  without restarting capture, closes every old Viewer stream, clears Viewer
  history, and makes every old token route return the same `404`. Refresh is
  immediate with no Viewers and requires confirmation when any Viewer is
  connected.
- Process exit is the other operation that invalidates the current token.

### Viewer management

- Each row shows IP address, connection duration, RTT, playback lag, and one
  disconnect action. Online Viewers sort before offline history.
- The Viewer reports the previous successful telemetry request's round-trip time
  and buffered media end minus playback position every two seconds. Offline
  telemetry, or telemetry at least six seconds old, displays as unavailable.
- Each token retains at most 100 in-memory Viewer records. Refresh Link and
  process exit clear them. IP addresses and telemetry are never persisted or
  written to ordinary logs.
- A random tab-scoped Viewer ID merges automatic reconnects into one record.
  Host-disconnected pages stop automatic reconnect but expose manual retry.
- A Host disconnect remains in force through Stop, Later Start, and media-only
  recovery until that page retries; Refresh Link clears it with Viewer history.
- Connection duration accumulates across reconnects of that tab from the Host's
  monotonic clock and freezes while its record is offline.
- The Viewer page contains only playback, volume, fullscreen, connection state,
  automatic reconnect, and manual retry controls.

### Settings contract

| Area | Required behavior |
| --- | --- |
| Presets | `720p60 / 6 Mbps`, `1080p60 / 12 Mbps`, `1440p60 / 24 Mbps` |
| Custom quality | Any width and height within the selected encoder's reported capability; preserve aspect ratio and center with black padding when needed |
| Frame rate | `30 / 60 / 120 FPS` |
| Bitrate | Presets use their listed bitrate; Custom uses the encoder default until manually set; manual range `1–500 Mbps` |
| Encoder | Auto prefers detected hardware and falls back to software; allow a detected implementation to be selected; never expose codec choice |
| System audio | One switch, enabled by default; no microphone |
| Audio exclusions | Communication rule plus enabled defaults for Discord, Vesktop, and Steam Voice; ordinary rules can be toggled, deleted, or added from active PipeWire applications |
| Network | Loopback and `8877/TCP` by default; configurable listen address, port, and optional `scheme://host:port` share base URL |
| Notifications | One switch, enabled by default |

Unsupported saved quality reports the error and blocks Start; it is not repaired
or silently downgraded. Quality edits are saved through **Save quality** and
take effect on the next Start; an active share and its media-only recoveries
retain the quality snapshot taken before its Portal picker opened.
Audio-setting edits during sharing take effect only
through **Apply to Current Share**, which performs one coordinated media restart.
Network rebinding is allowed only while stopped and warns that old waiting pages
may not recover. While the Host window is hidden, enabled desktop notifications
report the first transition into sharing, the return to waiting after an active
share, and Host or network failure. They never include the share link or token.
Portal pointer behavior has no setting. Autostart is not provided.

Settings are internal application state at
`$XDG_CONFIG_HOME/aercast/settings.json`; the file is not a public hand-edited
interface.

## Security and network boundary

- The token is 32 bytes of operating-system CSPRNG output encoded for the link.
  It is a bearer credential, not transport security.
- Page, media, telemetry, and controls are same-origin. Invalid, expired, and
  superseded token routes return the same `404` without revealing prior state.
- Tokens, Viewer IDs, IP addresses, and telemetry must not enter ordinary logs
  or third-party requests. Reverse proxies must disable or redact access logs
  for tokenized paths.
- Plain HTTP is limited to a trusted LAN. Public or otherwise untrusted access
  requires an external HTTPS reverse proxy. Aercast does not own domains,
  certificates, proxies, tunnels, port forwarding, or NAT traversal.

## Engineering decisions

### Process and media

- Aercast is one Rust process. It owns one Portal source and one
  capture/encode/mux pipeline per active share, regardless of Viewer count.
- Video enters through the restricted PipeWire remote returned by the
  ScreenCast Portal. Application audio is observed through the user's regular
  PipeWire graph without moving or muting existing output links.
- GStreamer owns capture conversion, clocks, H.264, AAC-LC, and fragmented MP4.
  Rust coordinates the lifecycle and fans out completed media fragments.
- The installed `mp4mux` remains the muxer while real MSE and late-join checks
  pass. `isofmp4mux` requires a recorded failure before it is considered.
- New Viewers receive only the latest initialization segment and current
  keyframe-started GOP. Bounded per-Viewer delivery disconnects lagging readers
  instead of backpressuring the media pipeline.
- Hardware encoding and DMA-BUF copy reduction are measurement-driven work.
  Do not build an encoder factory before a second verified implementation
  exists.

```text
ScreenCast Portal -> restricted PipeWire remote -> video capture ----\
                                                                  |
regular PipeWire graph -> allowed playback taps -> audio mixer ----+->
           GStreamer H.264 + AAC-LC -> fMP4 -> Axum -> browser MSE
```

### Session and HTTP lifecycle

- App startup creates the process token described in
  [Security and network boundary](#security-and-network-boundary) and starts the
  HTTP listener; it does not start capture.
- HTTP state owns the token separately from replaceable media state so Stop and
  recovery can close a media hub without rotating the link.
- Future Viewer heartbeat, telemetry, and Host control use ordinary Axum/Tokio
  HTTP requests; do not add WebSocket.
- Loopback is the safe default. A trusted-LAN bind is explicit. Public base
  URLs and TLS termination are external infrastructure.

### Audio policy

- The policy is all eligible system playback except exclusions; there is no
  selected-applications-only mode.
- Stable application identity is `application.id`, then process binary, then
  application name. PID is diagnostic data only and never affects policy.
- PipeWire playback nodes with `media.role=Communication` are excluded by the
  Phase 4 core rule. Phase 5 settings add editable default rules for Discord,
  Vesktop, and Steam Voice.
- Each allowed stereo playback stream is tapped through exact verified passive
  links and must retain an independent active route to an audio sink. Unknown,
  mono, surround, duplicate, or unsafe graph layouts stay silent.
- Audio-off and no-source states keep a silent track so the MSE track schema
  does not change during a share.

### PipeWire 1.6.8 compatibility

Selective system audio on PipeWire 1.6.8 requires the user to explicitly create
`${XDG_CONFIG_HOME:-$HOME/.config}/pipewire/pipewire.conf.d/90-aercast-passive-links.conf`
with:

```ini
module.link-factory.args = {
    allow.link.passive = true
}
```

Apply it by signing out and back in, then enable **System Audio** in Aercast.
The setting permits every client with link-creation access to request passive
links through the user's PipeWire daemon, not only Aercast. Aercast and its
package never create or edit the file, or reload or restart PipeWire; without
the opt-in, Aercast rejects non-passive capture links before activating audio.
Remove the file and sign out and back in to revoke it.

This is a temporary compatibility path, not a release requirement. A
publishable AUR package must use a proven client-side selective capture design
without first-use daemon configuration. Whole-sink monitor capture remains
invalid because it cannot preserve per-application and Communication
exclusions.

### Desktop implementation

- The product has only a GUI. Audio exclusions live only in internal settings;
  there is no command-line control surface.
- iced uses Wayland, Tokio, and the `wgpu` renderer. Phase 5 moves the lifecycle
  to `iced::daemon`; hidden means no mapped window while the process continues.
- The supplied `assets/aercast-icon.png` is the single application and tray
  brand image. UI controls use the minimal bundled symbolic SVG set defined by
  [ui-design.md](ui-design.md).
- Phase 5 uses `ksni` for StatusNotifierItem, direct `zbus` for single-instance
  activation and notifications, the ashpd Settings Portal API for appearance
  preferences, and `serde`/`serde_json` for internal settings. No GTK,
  Libadwaita, Node, TypeScript, icon-theme, font, URL, UUID, or config-directory
  dependency is planned.
- Resolve the settings path with the standard XDG config fallback when
  `XDG_CONFIG_HOME` is unset. Saving must be replace-based so a failed write
  does not destroy the last valid file.

## Completed work

- **Phase 1 — 2026-08-25:** Portal monitor/window capture and passive-audio
  feasibility were proven on the recorded niri host.
  [Evidence](verification.md#portal-capture)
- **Phase 2 — 2026-08-25:** Local H.264 fMP4 playback with negotiated codec MIME
  was proven in Firefox-family and Chromium engines.
  [Evidence](verification.md#browser-playback)
- **Phase 3 — 2026-08-26:** Dynamic identity rematching, safe selective stereo
  audio, AAC-LC muxing, and live browser playback were proven on the recorded
  host. [Evidence](verification.md#selective-audio)
- **Phase 4 — 2026-08-27:** Zen and Chromium completed the same-link lifecycle
  through bounded same-Portal media recovery, Stop, and later Start; real
  PipeWire policy excluded Communication streams across PID changes without
  disturbing local sink routes. [Evidence](verification.md#phase-4-lifecycle)

Git history retains the detailed old checklists.

## Phase 5 — desktop productization

### Phase 5 goal

Turn the stable core into the fixed-size niri desktop product described in
[Product behavior](#product-behavior): daemon window and tray lifecycle, single-instance
activation, internal settings, Portal-derived accessible dark theme, Viewer
management and link refresh, quality/network/audio controls, notifications,
and a real three-or-more-Viewer one-encoder acceptance run. Performance work is
limited to paths justified by measurement.

### Acceptance

- A real niri run proves the fixed automatically floating window, hide and tray
  restore, single-instance activation, confirmed active-share Quit, and clean
  removal of the window, tray item, listener, Portal, and media graph.
- Real GUI use proves internal quality, encoder, audio exclusion, network, and
  notification settings persist and obey their documented next-Start,
  apply-to-current-share, validation, and stopped-only boundaries.
- Portal-derived dark appearance and the documented keyboard, focus,
  accessibility, and reduced-motion behavior pass a visual and interactive
  check on the recorded host.
- Viewer telemetry, Host disconnect/manual retry, Refresh Link `404` behavior,
  and the 100-record bound pass their smallest checks and a real Viewer flow.
- Three or more real Viewers share one encoder, continue after one stalled
  reader is removed, and cleanly complete the desktop workflow in Zen first and
  Chromium second.
- Canonical static checks pass and current evidence replaces every superseded
  Phase 5 blocker in [verification.md](verification.md).

### Current blockers

- Current real evidence is still required for desktop shell lifecycle,
  settings boundaries, appearance and accessibility, notifications, Viewer
  management, explicit link refresh, and final three-or-more-Viewer acceptance.
- The PipeWire 1.6.8 passive-link opt-in remains a packaging compatibility
  blocker; Aercast must continue to fail closed and must not edit or reload the
  daemon-wide setting.

## Repository non-goals

Do not introduce microservices, multi-process media IPC, a custom media
protocol, speculative cross-platform layers, one-implementation traits or
factories, plugin systems, or unused directory structure. Product non-goals
are authoritative in [Product commitments](#product-commitments).
