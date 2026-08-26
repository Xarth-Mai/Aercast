# Development direction

This document owns product commitments, exact Host and Viewer behavior,
engineering decisions, settings, security boundaries, the current Phase,
acceptance criteria, risks, non-goals, and the next-Phase entry condition; it
does not own homepage copy, visual tokens, or raw test output.

## Status

Aercast is pre-alpha. Phase 4, single-Viewer core stability, is active. The
repository already proves Portal capture, selective PipeWire audio, H.264/AAC
fMP4 playback, bounded one-encoder fan-out, and a basic iced window on the
recorded niri host. Unit coverage now separates process-token ownership from
replaceable media state and applies Communication-role audio policy. The code
now bounds media recovery without reclassifying a reported audio failure as a
cleanup failure; real browser recovery remains unverified.

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

- The Host has only a GUI. The main window is fixed at `480×640` logical pixels,
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
  and pointer behavior. The active view shows the approved source, share link,
  Copy, Refresh Link, Stop, and Viewer list.
- Stop closes media and the Portal session. The current link remains valid and
  returns Viewers to waiting. A later Start reuses it.
- Refresh Link creates a new token without restarting capture, closes every old
  Viewer stream, clears Viewer history, and makes every old token route return
  the same `404`. Refresh is immediate with no Viewers and requires confirmation
  when any Viewer is connected.
- Process exit is the other operation that invalidates the current token.

### Viewer management

- Each row shows IP address, connection duration, RTT, playback lag, and one
  disconnect action. Online Viewers sort before offline history.
- Each token retains at most 100 in-memory Viewer records. Refresh Link and
  process exit clear them. IP addresses and telemetry are never persisted or
  written to ordinary logs.
- A random tab-scoped Viewer ID merges automatic reconnects into one record.
  Host-disconnected pages stop automatic reconnect but expose manual retry.
- The Viewer page contains only playback, volume, fullscreen, connection state,
  automatic reconnect, and manual retry controls.

### Settings contract

| Area | Required behavior |
|---|---|
| Presets | `720p60 / 6 Mbps`, `1080p60 / 12 Mbps`, `1440p60 / 24 Mbps` |
| Custom quality | Any width and height within the selected encoder's reported capability; preserve aspect ratio and center with black padding when needed |
| Frame rate | `30 / 60 / 120 FPS` |
| Bitrate | Use the encoder's default until manually set; manual range `1–500 Mbps` |
| Encoder | Auto prefers detected hardware and falls back to software; allow a detected implementation to be selected; never expose codec choice |
| System audio | One switch, enabled by default; no microphone |
| Audio exclusions | Communication rule plus enabled defaults for Discord, Vesktop, and Steam Voice; ordinary rules can be toggled, deleted, or added from active PipeWire applications |
| Network | Listen address, port, and optional `scheme://host:port` share base URL |
| Notifications | One switch, enabled by default |

Unsupported saved quality reports the error and blocks Start; it is not repaired
or silently downgraded. Audio-setting edits during sharing take effect only
through **Apply to Current Share**, which performs one coordinated media restart.
Network rebinding is allowed only while stopped and warns that old waiting pages
may not recover. Portal pointer behavior has no setting. Autostart is not
provided.

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

### Desktop implementation

- The product has only a GUI. Existing source, bind, and exclusion command-line
  flags are temporary development inputs and must be removed before Phase 5 is
  accepted.
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

Git history retains the detailed old checklists.

## Phase 4 — single-Viewer core stability

### Goal

Finish the stable media and link lifecycle before adding desktop-shell and
settings surface area:

- Zen first and Chromium second must play one live Viewer through waiting,
  playback, stream restart, Stop, and later Start states without a page reload.
- Stop closes media and the Portal session, then the same token returns to the
  waiting state. Only explicit refresh or process exit invalidates the link.
- A media-only failure while its Portal session remains open restarts the
  capture/audio/encode/mux attempt at most three times. Every attempt keeps the
  token and replaces the media cache so the Viewer reconnects cleanly.
- Portal closure, explicit Stop, explicit Quit, HTTP-server failure, or three
  failed recoveries ends the retry loop. Cleanup runs once at the owning layer.
- Selective audio follows stable identity and the PipeWire Communication role;
  PID never selects or persists a rule.

### Acceptance

- Unit checks prove Stop preserves the token, closes the old body, returns the
  stream to `425`, and lets the same token enter a later media session.
- One retry-policy check proves exactly three retry attempts and proves that
  Stop, Quit, Portal closure, and Host/server failures are not retried.
- An audio-stop check proves a reported media failure remains retryable while
  an unreported cleanup failure remains terminal.
- Audio checks prove identity precedence, Communication exclusion, and the
  irrelevance of a changed process ID.
- A real niri Portal run in Zen, then Chromium, proves initial waiting,
  playback, automatic recovery from an induced media-only failure, Stop to
  waiting on the same URL, and playback after a later Start.
- `cargo fmt --check` and `cargo test` pass; evidence replaces the superseded
  lifecycle record in [verification.md](verification.md).

### Current blockers

- Stop token continuity and the three-recovery policy have unit coverage;
  same-Portal-session recovery still needs real Zen and Chromium evidence.
- Communication-role exclusion is unit-covered with PID-independent stable
  identity, but still needs real PipeWire evidence.

### Risks

- Each recovery opens a fresh restricted PipeWire remote; repeated access to
  the same Portal session must be verified on the real compositor.
- PipeWire 1.6.8 removes client-supplied `link.passive=true` unless the Host has
  deliberately enabled passive client links in its link factory. Aercast must
  fail closed and must never write or reload that daemon-wide setting.
- Browser MSE may expose a recovery failure that component tests cannot model;
  real Zen and Chromium playback are required.

### Non-goals for Phase 4

- daemon window hiding, tray, single-instance activation, settings, theming,
  notifications, Viewer history/telemetry/kick, and link-refresh UI
- multi-Viewer product acceptance, network rebinding, quality selection,
  hardware encoding, DMA-BUF optimization, or performance claims
- release packaging, GNOME/KDE validation, or public-network deployment

## Phase 5 — desktop productization

### Goal

Turn the stable core into the fixed-size niri desktop product described in
[Product behavior](#product-behavior): daemon window and tray lifecycle, single-instance
activation, internal settings, Portal-derived accessible dark theme, Viewer
management and link refresh, quality/network/audio controls, notifications,
and a real three-or-more-Viewer one-encoder acceptance run. Performance work is
limited to paths justified by measurement.

### Entry condition

Phase 5 starts only after every Phase 4 acceptance item has current niri, Zen,
and Chromium evidence and no open token, recovery, or audio-safety blocker.

## Repository non-goals

Do not introduce microservices, multi-process media IPC, a custom media
protocol, speculative cross-platform layers, one-implementation traits or
factories, plugin systems, or unused directory structure. Product non-goals
are authoritative in [Product commitments](#product-commitments).
