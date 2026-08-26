# Development direction

This document is the source of truth for Aercast's engineering decisions,
active milestone, acceptance criteria, and development workflow. The
[README](../README.md) is deliberately product-facing. Host-specific research,
measurements, and test results belong in [verification.md](verification.md).

## Status

Aercast is pre-alpha. The repository contains a native Linux development proof
for Portal video, selective PipeWire audio, H.264/AAC fMP4, and an iced Host
window. It is not a distributable product yet.

Phases 1 through 4 have development proofs on the recorded niri host. Real
Portal runs completed the Host lifecycle, three concurrent Zen and Chromium
Viewers, and stalled-client isolation. Phase 5, measured optimization, is
active. GNOME, KDE, packaged installation, LAN performance, and release
compatibility remain unverified.

## Product invariants

- Linux and Wayland come first. Screen and window capture use
  `xdg-desktop-portal` and its non-persistent permission lifecycle.
- One Host owns one ephemeral session, one capture/encode/mux pipeline, and one
  HTTP media stream. Every Viewer receives the same encoded bytes.
- The Host serves the Viewer directly. Do not add WebRTC, signaling, ICE,
  STUN, TURN, an SFU, cloud relays, or built-in NAT traversal.
- Selective system audio is an MVP release gate. Excluding an application must
  not move its playback links or change what the Host hears.
- PipeWire and GStreamer own graph and media behavior. Rust coordinates them;
  it does not rebuild codecs, muxing, clocks, or desktop capture.
- Performance and compatibility claims require recorded real-world evidence.

If selective audio cannot work safely on the target desktops without changing
the Host mix, that is a product-direction failure. If MSE misses the latency
target, measure and tune it before considering another browser decoding path;
WebRTC remains out of scope.

## Product decisions

### Host and Viewer

- The first Host is a native, unsandboxed Linux process. GNOME, KDE Plasma, and
  niri are the initial compositor targets. Flatpak is deferred until access to
  both the restricted Portal remote and the regular PipeWire audio graph is
  proven safe and distributable.
- Current stable desktop Firefox is the primary Viewer target. Current stable
  desktop Chromium is secondary but first-class; both must pass the same live
  MSE checks, with Firefox checked first. Zen Browser is the local
  Firefox-family test vehicle on the current development host.
- Google Chrome, Safari, mobile browsers, and distribution Chromium builds
  without H.264/AAC are not initial promises.
- Each share contains exactly one Portal-approved monitor or window. Embedded
  cursor capture is preferred, with hidden cursor as the fallback.

### Session and network

1. App launch starts the HTTP server and creates a share token from 32 bytes of
   operating-system CSPRNG output. It does not start capture.
2. Start Sharing opens a non-persistent Portal session and asks the Host to
   choose one source.
3. One capture/encode/mux pipeline runs independently of Viewer count.
4. End Sharing immediately revokes the old token, closes Viewer streams, then
   cooperatively stops audio, GStreamer, and the Portal session. The next share
   receives a fresh link.

The page and media are same-origin. The development Host binds to loopback by
default; direct LAN use requires an explicit trusted `--bind IP:PORT`. Public
or untrusted use requires an external HTTPS reverse proxy to a loopback or
otherwise private bind. Domains, certificates, public hosting, port forwarding,
tunnels, and NAT traversal are external infrastructure.

The token is a bearer credential, not transport security. It must not enter
ordinary logs or third-party requests. Invalid or expired token routes return
HTTP 404 without revealing session state. A reverse proxy must disable or
redact access logging for tokenized paths.

### Selective audio

The MVP policy is all ordinary PipeWire playback audio minus an exclusion
list; selected-applications-only capture is not planned for MVP. Match stable
identity in this order:

1. `application.id`
2. process binary or application name
3. PID for diagnostics only, never persistence

Playback nodes must be tracked dynamically so rules survive application start,
exit, and restart. Aercast taps each allowed stereo playback stream in a
separate passive capture branch and mixes those branches without touching the
existing speaker routes. Audio-off and no-source states retain a silent track
so the browser's MSE track schema never changes mid-session. Direct ALSA,
exclusive, passthrough, mono, and surround streams are outside the current
prototype contract.

## Architecture baseline

```text
ScreenCast Portal -> restricted PipeWire remote -> video capture ----\
                                                                  |
regular PipeWire graph -> allowed playback-stream taps -> mixer ---+->
     GStreamer H.264 + AAC-LC -> fragmented MP4 -> host HTTP -> browser MSE
```

The implementation remains one Rust process. Dependencies enter only with a
working milestone:

- Phase 1: Tokio, `futures-util`, `ashpd`, and `gstreamer-rs`
- Phase 2: Axum and `gstreamer-app`
- Phase 3: `pipewire-rs`
- Phase 4: `socket2` for Linux stalled-client timeouts and iced with its Wayland
  `wgpu` renderer for the native Host UI

The Viewer is embedded plain HTML, CSS, and JavaScript. Do not add a Node or
TypeScript build chain.

The single media contract is H.264 video plus AAC-LC audio in one multiplexed
fragmented MP4 stream. Use the installed GStreamer `mp4mux` while real MSE,
latency, and late-join checks pass. Consider `isofmp4mux` only after a recorded
failure justifies the additional plugin. Phase 2 deliberately uses one
software encoder; do not add an encoder factory. VA-API, NVENC, DMA-BUF encode,
AMF, and adaptive bitrate wait for profiling.

The final HTTP surface is:

```text
GET /s/{token}
GET /s/{token}/stream
```

A new Viewer receives the latest initialization segment followed by the
current decodable, keyframe-started GOP. That is the complete media cache. A
bounded per-Viewer queue disconnects a slow Viewer instead of backpressuring
GStreamer. Reconnection creates a fresh MediaSource and repeats the same
late-join sequence.

The Viewer must use the exact codec MIME derived from negotiated output, call
`MediaSource.isTypeSupported()`, serialize `SourceBuffer` operations, trim old
buffered ranges, recover from quota pressure, and expose an explicit Play /
Enable Audio action.

## Roadmap and acceptance

### Phase 1: Portal and audio feasibility — complete on the recorded host

- CLI Portal -> PipeWire -> GStreamer local preview.
- Passive application-audio tap proof that preserves the local output route.
- Continuous 60-second niri monitor and window checks. GNOME and KDE remain
  required before compatibility is claimed.

### Phase 2: localhost browser video — complete on the recorded host

- H.264 -> fragmented MP4 -> Axum -> Firefox/Chromium MSE.
- Exact AVC MIME derived from negotiated caps.
- First-frame and first-fragment observations recorded as development data,
  not latency promises.

### Phase 3: selective audio and A/V mux — complete as a development proof

- Dynamic playback registry and identity rematching.
- Stable silent audio track plus allowed-stream mixing and exclusions.
- Game + Discord proxy acceptance: both keep local sink routes; the Viewer
  receives Game and excludes Discord.
- H.264 and AAC-LC share one fMP4 stream and play in Zen and Chromium.

### Phase 4: product lifecycle and multi-Viewer — complete on the recorded host

- Start a long-lived HTTP server and secure waiting link before capture.
- Replace development routes with token routes and revoke on End.
- Cache only init plus the current decodable GOP; reconnect from that boundary.
- Serve three concurrent Viewers from one encoder; a lagging Viewer cannot
  affect the pipeline or peers.
- Add the iced Host UI with Start, End, link, waiting/ended/error states, Viewer
  count, and cooperative cleanup.
- Validate lifecycle and three-Viewer playback on Zen first, then Chromium.

### Phase 5: measured optimization — active

- Establish real 1080p60 and trusted-LAN end-to-end latency measurements.
- Claim less than 250 ms only after it is repeatedly observed.
- Profile before adding exactly one needed hardware encode path and any
  justified DMA-BUF copy reduction.
- Test 1440p60 and 4K60 only after the primary 1080p60 path works.

## Current development constraints

The current niri/AMD proof normalizes the Portal DMA-BUF through the installed
`vapostproc`, then encodes 1280x720 at 30 FPS with `x264enc`. This is a verified
host compatibility path, not a final encoder decision.

On PipeWire 1.6.8, the daemon removes a client-supplied `link.passive=true`
unless its link factory explicitly enables passive client links:

```ini
module.link-factory.args = {
    allow.link.passive = true
}
```

Aercast does not write this setting or restart PipeWire. Without the deliberate
Host opt-in, the prototype rejects the link before capture activation. With
the opt-in, it still verifies passive readback, independent active speaker
routes, active capture links, real data, and the absence of unexpected inputs.
This daemon-wide prerequisite is a packaging and compatibility risk. Full
analysis and observed cleanup state are in [verification.md](verification.md).

The Host UI disables iced's default features and enables only Wayland, `wgpu`,
and Tokio. This removes iced's `tiny-skia` content-renderer fallback; the
remaining transitive `tiny-skia` use is limited to Wayland client-side window
decorations. `wgpu` prefers a high-performance adapter but may still select a
CPU adapter when no compatible GPU exists. The recorded AMD host used its DRM
render node; Aercast does not yet reject software `wgpu` adapters on other
hosts.

The development server admits at most eight concurrent media responses and
sets a 15-second Linux TCP user timeout inherited by accepted sockets. These
bounds keep stopped readers from accumulating resources; eight Viewers is not
a tested release-scale claim.

## Local development checks

Canonical static checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

The current Host opens its native window with one optional source restriction
and repeated audio exclusions:

```sh
cargo run -- --monitor --exclude zen-bin
cargo run -- --window --exclude zen-bin
```

Use **Start Sharing** to open the real Portal picker and **End Sharing** to
revoke the active link and return to a fresh waiting link. Closing the Host
window performs cooperative cleanup. Pass an explicit private `--bind IP:PORT`
only when testing across a trusted LAN. The Portal picker cannot be replaced by
a mock. On the current Host, run Zen checks before Chromium and exclude the
exact Host-local browser PipeWire identity to prevent its playback from feeding
back into the share. Muting or a null browser audio backend is not sufficient.

Before committing, follow the staged Ponytail gate in [AGENTS.md](../AGENTS.md).

## Primary references

- [ScreenCast Portal API](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
- [WirePlumber linking policy](https://pipewire.pages.freedesktop.org/wireplumber/policies/linking.html)
- [GStreamer `mp4mux`](https://gstreamer.freedesktop.org/documentation/isomp4/mp4mux.html)
- [MSE ISO BMFF byte-stream format](https://www.w3.org/TR/mse-byte-stream-format-isobmff/)
- [Linux `TCP_USER_TIMEOUT`](https://www.man7.org/linux/man-pages/man7/tcp.7.html)
- [Firefox codec support](https://support.mozilla.org/en-US/kb/audio-and-video-firefox)
- [Chromium and Google Chrome codec differences](https://chromium.googlesource.com/chromium/src/+/refs/tags/116.0.5845.263/docs/chromium_browser_vs_google_chrome.md)
- PipeWire 1.6.8 source used for the passive-link analysis:
  [`impl-node.c`](https://gitlab.freedesktop.org/pipewire/pipewire/-/blob/1.6.8/src/pipewire/impl-node.c),
  [`impl-link.c`](https://gitlab.freedesktop.org/pipewire/pipewire/-/blob/1.6.8/src/pipewire/impl-link.c), and
  [`module-link-factory.c`](https://gitlab.freedesktop.org/pipewire/pipewire/-/blob/1.6.8/src/modules/module-link-factory.c)

## Non-goals

- WebRTC infrastructure, cloud relays, accounts, meetings, chat, calls, or
  camera sharing
- recording, remote control, clipboard synchronization, or file transfer
- Windows or macOS Hosts
- microservices, a custom media protocol, or speculative cross-platform,
  encoder, transport, configuration, and plugin abstractions
