# Development direction

This document owns product commitments, exact Host and Viewer behavior,
engineering decisions, settings, security boundaries, the current Phase,
acceptance criteria, risks, non-goals, and the next-Phase entry condition; it
does not own homepage copy, visual tokens, or raw test output.

## Status

Aercast is pre-alpha. The planned product phases are complete on the recorded
niri baseline. Phase 5 qualification proves the fixed desktop and tray
lifecycle, settings boundaries, Portal-derived appearance, notifications,
Viewer management, link refresh, and one-encoder three-Viewer workflows in Zen
first and Chromium second. The current interface polish and PipeWire graph-race
fix have automated coverage but have not repeated that real Host and Viewer
qualification. There is no packaged release; the last real selective-audio
graph and lifecycle verification passed on PipeWire 1.6.8 without a daemon-wide
passive-link override.

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

- The Host has only a GUI. The main window is fixed at `920×520` logical pixels,
  is not resizable, and uses ordinary system decorations. Its fixed size lets
  niri's native heuristic float it without a user window rule.
- The in-content close button and the compositor close action both hide the
  window without stopping an active share.
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

- The top navigation switches between **Main**, **Viewers**, and **Settings**.
  There is no Host video preview.
- **Start Sharing** opens the Portal picker, which owns monitor/window choice
  and pointer behavior. The same stateful action becomes **Cancel** during
  selection and **Stop Sharing** while active. This primary action is centered
  at the bottom of the Share view. The view also shows the approved source and
  share link. Copy and Refresh Link are icon buttons; successful Copy briefly
  replaces its icon with a check mark.
- Stop closes media and the Portal session. The current link remains valid and
  returns Viewers to waiting. A later Start reuses it.
- Refresh Link is available while waiting or sharing. It creates a new token
  without restarting capture, closes every old Viewer stream, clears Viewer
  history, and makes every old token route return the same `404`. Refresh is
  immediate with no Viewers and requires confirmation when any Viewer is
  connected.
- Process exit is the other operation that invalidates the current token.

### Viewer management

- The dedicated Viewers page shows the online/total count and Viewer list.
- Each row shows IP address, connection duration, RTT, playback lag, and one
  disconnect action. The displayed IP prefers a reverse proxy's `X-Real-IP`,
  then the first `X-Forwarded-For` address, and otherwise uses the TCP peer.
  These headers affect display only; the proxy must replace client-supplied
  values. When multiple Viewers share the same IP, sequential indices disambiguate
  them. Online Viewers sort before offline history.
- The Viewer reports the previous successful telemetry request's round-trip time
  and buffered media end minus playback position every two seconds. Offline
  telemetry, or telemetry at least six seconds old, displays as unavailable.
- Each token retains at most 100 in-memory Viewer records. Refresh Link and
  process exit clear them. IP addresses and telemetry are never persisted or
  written to ordinary logs.
- A random browser-scoped Viewer ID merges automatic reconnects into one record.
  Opening or resuming playback in another tab of the same browser takes over the
  active session.
- A Host disconnect permanently blocks that Viewer from reconnecting through
  Stop, later Start, and media recoveries; Refresh Link and process exit clear it
  with Viewer history.
- Connection duration accumulates across reconnects of that browser from the Host's
  monotonic clock and freezes while its record is offline.
- The Viewer fills the browser viewport with one square-cornered, `contain`-fit
  video and only the browser's native playback, volume, and fullscreen controls;
  it has no Aercast overlay or visible status text. Page load first attempts
  unmuted playback and retries muted when autoplay policy rejects sound. A
  user-selected native muted state is remembered locally, while that policy
  fallback is not stored as preference. Manual timeline seeking returns to the
  live edge. Playback between 350 ms and 2 s behind the latest buffered media
  catches up by gradually increasing playback rate (up to 1.15×); playback more
  than 2 s behind seeks to 150 ms behind the live edge. Automatic reconnect
  continues until the share
  ends or the Host disconnects the Viewer.
  Connection and media diagnostics remain only in the console and document
  state attributes.

### Settings contract

| Area | Required behavior |
| --- | --- |
| Presets | `720p60 / 6 Mbps`, `1080p60 / 12 Mbps`, `1440p60 / 24 Mbps` |
| Custom quality | Any width and height within the selected encoder's reported capability; preserve aspect ratio and center with black padding when needed |
| Frame rate | `30 / 60 / 120 FPS` |
| Bitrate | Presets use their listed bitrate; Custom uses the encoder default until manually set; manual range `1–500 Mbps` |
| Encoder | Auto prefers detected hardware and falls back to software; allow a detected implementation to be selected; never expose codec choice |
| System audio | One switch, enabled by default; no microphone |
| Audio exclusions | Communication rule is enabled by default and can be toggled but not deleted; enabled defaults for Discord, Vesktop, and Steam Voice can be toggled or deleted, and ordinary rules can be added from active PipeWire applications |
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
- Development builds lightly optimize Aercast and fully optimize dependencies;
  release builds use thin LTO, one codegen unit, and stripped symbols while
  preserving unwind-based cleanup.
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
- PipeWire playback nodes with `media.role=Communication` are excluded by
  default through a permanent, user-toggleable rule. Phase 5 settings add
  editable default rules for Discord, Vesktop, and Steam Voice.
- Each allowed stereo playback stream is tapped from its existing output ports
  into an Aercast capture node exported and read back with `node.passive=in`.
  Its input ports inherit that passive mode. Aercast does not request
  `link.passive`; it verifies each exact link's endpoints, object serial, and
  state while PipeWire derives runnable behavior from the node and port modes.
  The playback stream must retain an independent active route to an audio sink.
  Unknown, mono, surround, duplicate, or unsafe graph layouts stay silent.
- Audio-off and no-source states keep a silent track so the MSE track schema
  does not change during a share.

### PipeWire 1.6.8 compatibility

Selective system audio on PipeWire 1.6.8 requires no daemon-wide
`allow.link.passive` override. Aercast marks its capture node's input direction
passive through `node.passive=in`, waits for PipeWire NodeInfo to read back that
value, and creates exact links without a client-supplied `link.passive`.
PipeWire 1.6.8 does not duplicate this derived mode into the link properties;
passivity is the inherited input-port scheduling behavior. Whole-sink monitor
capture remains invalid because it cannot preserve per-application and
Communication exclusions.

Dynamic graph teardown may report `ENOENT` through either the removed object or
PipeWire core object `0`; both mean the resource already vanished and trigger
the normal graph rebuild instead of ending the share.

### Desktop implementation

- The product has only a GUI. Audio exclusions live only in internal settings;
  there is no command-line control surface.
- iced uses Wayland, Tokio, and the `wgpu` renderer. Phase 5 moves the lifecycle
  to `iced::daemon`; hidden means no mapped window while the process continues.
- The supplied `assets/aercast-icon.png` is the single application, tray, and
  Viewer favicon brand image. UI controls use the minimal bundled symbolic SVG
  set defined by [ui-design.md](ui-design.md).
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
- **Phase 5 — 2026-08-27:** The fixed niri desktop product, settings, tray,
  notifications, Viewer controls, link refresh, and one-encoder three-Viewer
  workflow completed current-source acceptance in Zen and Chromium.
  [Evidence](verification.md#phase-5-desktop-productization)

Git history retains the detailed old checklists.

## Repository non-goals

Do not introduce microservices, multi-process media IPC, a custom media
protocol, speculative cross-platform layers, one-implementation traits or
factories, plugin systems, or unused directory structure. Product non-goals
are authoritative in [Product commitments](#product-commitments).
