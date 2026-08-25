# Aercast

> **Pre-alpha:** Aercast currently provides only a capture-feasibility CLI
> probe. There is no product application yet. Only the host-specific
> feasibility results recorded below have been verified.

Aercast is a Linux/Wayland-first, one-way screen-sharing app for a small number
of viewers. A native host selects one screen or window, then serves live video
and selected system audio directly over HTTP to a browser. It is not a meeting
system.

**One host. One ephemeral session. One capture/encode/mux pipeline. One HTTP
stream. No cloud.**

## Product philosophy

- **Use the native permission model.** Capture goes through
  `xdg-desktop-portal` and PipeWire, not compositor-specific shortcuts.
- **Use mature media components.** GStreamer owns capture, encoding, muxing,
  and clocks; Aercast will not rebuild a media stack for the sake of pure Rust.
- **Encode once.** Every viewer receives the same encoded stream. Adding a
  viewer must not add another encoder.
- **Treat selective audio as a release requirement.** Viewers hear PipeWire
  system audio except excluded applications, while the host's local mix stays
  unchanged.
- **Measure before optimizing.** Hardware encoding, DMA-BUF, fragment sizing,
  and copy avoidance follow real profiling and compatibility results.
- **Ship vertical slices.** Every milestone must run end to end on real
  Wayland systems before the next layer is added.

## Product boundary

The first host target is a native, unsandboxed Linux application. GNOME, KDE
Plasma, and niri are the initial compositor targets, but none is verified yet.
Flatpak support is deferred because selective audio needs access to the user's
regular PipeWire graph in addition to the restricted ScreenCast Portal remote.

The first release-blocking viewer target is the current stable desktop release
of Google Chrome. The page must still check the exact codec MIME at
runtime. Firefox is experimental when the operating system provides H.264 and
AAC decoding and the live stream passes a real smoke test. Distribution-built
Chromium, Safari, and mobile browsers are not initial compatibility promises.
Firefox's general H.264 support depends on platform codecs, while Chromium
builds can omit proprietary codecs; see the
[Mozilla codec documentation](https://support.mozilla.org/en-US/kb/audio-and-video-firefox)
and [Chromium codec documentation](https://www.chromium.org/audio-video/).

The host and viewer use the same HTTP origin. Plain HTTP is acceptable only on
a trusted LAN. Public or otherwise untrusted access requires an external HTTPS
reverse proxy. Aercast does not provide certificates, public hosting, port
forwarding, tunnels, or NAT traversal.

## Session lifecycle

1. On launch, the app starts its HTTP server and creates an opaque share token
   from 32 bytes supplied by the operating-system CSPRNG. No capture has
   started yet.
2. **Start Sharing** creates a non-persistent ScreenCast Portal session. The
   host selects one monitor or window; an embedded cursor is preferred when the
   portal supports it.
3. One capture/encode/mux pipeline serves every connected viewer.
4. **End Sharing** closes the media and Portal sessions, revokes the old token,
   and creates a fresh link for the next session.

The share URL is a bearer credential, not transport security. Tokens must not
be written to ordinary logs or sent to third-party page resources.

## Architecture

```text
ScreenCast Portal -> restricted PipeWire remote -> video capture ----\
                                                                  |
regular PipeWire graph -> allowed playback-stream taps -> mixer ---+->
     GStreamer H.264 + AAC-LC -> fragmented MP4 -> host HTTP -> browser MSE
```

The Portal remote contains only the sources approved by the user. Application
audio therefore comes from a separate connection to the regular PipeWire
graph. Each allowed playback stream is tapped without moving its existing
speaker link, then mixed only for Aercast. This follows the
[Portal ScreenCast lifecycle](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
and [WirePlumber stream-linking model](https://pipewire.pages.freedesktop.org/wireplumber/policies/linking.html).

The MVP audio policy is **all PipeWire playback audio minus exclusions**.
Applications are grouped by `application.id`, then by process binary or
application name when needed; PID is diagnostic data, never persistent
identity. New, removed, and restarted streams must be matched dynamically, and
Aercast excludes itself by default. Direct ALSA, exclusive, or passthrough
streams outside the normal PipeWire graph are not promised. Audio-off and
no-source states retain a silent audio track so the browser's track layout does
not change mid-session.

## Technical direction

Aercast remains one Rust process. Dependencies enter only when their milestone
needs them:

- Phase 1: Tokio, `futures-util`, `ashpd`, and `gstreamer-rs`
- Phase 2: Axum and `gstreamer-app` for HTTP streaming
- Phase 3: `pipewire-rs` for the dynamic audio registry
- Phase 4: `iced` for the host UI

The viewer is embedded plain HTML, CSS, and JavaScript. There is no TypeScript
or Node build chain.

The media contract is H.264 video and AAC-LC audio in one multiplexed
fragmented MP4 stream. Phase 2 first tests GStreamer's existing `mp4mux` in
fragmented mode. `isofmp4mux` is considered only if real MSE, latency, or join
tests show that `mp4mux` is insufficient. The first browser slice uses one
software encoder and no encoder-selection abstraction. VA-API, NVENC, DMA-BUF,
AMF, and adaptive bitrate wait for measured need. See the
[GStreamer `mp4mux` documentation](https://gstreamer.freedesktop.org/documentation/isomp4/mp4mux.html).

The final HTTP surface is deliberately small:

```text
GET /s/{token}
GET /s/{token}/stream
```

The first route serves the viewer; the second is a continuous fMP4 response.
An invalid token returns HTTP 404. A new stream response starts with the
latest init segment and the latest decodable keyframe-started GOP. That is the
entire server-side media cache. A slow viewer is disconnected and reconnects
instead of applying backpressure to GStreamer.

The viewer derives the exact codec MIME from the real pipeline output, calls
`MediaSource.isTypeSupported()`, serializes `SourceBuffer` appends, removes old
buffered ranges, handles quota failures, and offers an explicit **Play / Enable
Audio** action. It does not parse MP4 transport chunks in JavaScript. The byte
stream must satisfy the [MSE ISO BMFF format](https://www.w3.org/TR/mse-byte-stream-format-isobmff/).

## Targets

These remain targets, not benchmark results:

- 1080p at 60 FPS as the primary operating point
- less than 250 ms end-to-end latency on a LAN
- one encoder serving at least three simultaneous viewers
- a new viewer starting within one GOP
- 1440p60 and 4K60 only after the primary path works; 4K60 should use hardware
  encoding in normal operation

## Current milestone: capture feasibility

The current milestone contains no desktop UI, HTTP server, encoder, or product
scaffolding.

1. Build one CLI probe for Portal -> PipeWire -> GStreamer local preview.
2. Use existing PipeWire tools to prove that one application's playback stream
   can be tapped without interrupting or changing its local output.
3. Record Portal capabilities, selected stream identity, negotiated caps,
   memory type, first-frame time, and errors.
4. Handle user cancellation, session closure, PipeWire disconnect, and Ctrl-C.
5. Preview continuously for 60 seconds on the current niri environment. GNOME
   and KDE must pass the same smoke check before compatibility is claimed.

### Recorded feasibility result: 2026-08-26

These observations apply only to the current host: niri 26.04, ScreenCast
Portal v5, PipeWire 1.6.8, GStreamer 1.28.6, and an AMD Radeon RX 6650 XT.

- Monitor preview ran for 65 seconds and window preview continued for 65
  seconds across a live resize; both then stopped cleanly through SIGINT.
- Successful source caps used DMA-BUF with `XR24` for monitor capture and
  `AR24` for window capture. First buffers arrived 17-27 ms after the probe
  started the pipeline transition; this is not end-to-end latency.
- Direct `waylandsink` and `glimagesink` could not consume the modifier first
  offered on this host. The installed `vapostproc` negotiated a compatible
  modifier and normalized frames to `BGRx` for local preview. This is a current
  compatibility path, not a future encoder decision.
- A 48 kHz stereo application stream remained linked to the local output while
  a second capture link recorded five seconds of non-silent PCM. Removing the
  temporary capture stream left the local route unchanged.

GNOME, KDE, other GPUs, and systems without the required GStreamer VA plugin
remain unverified.

## Later milestones

1. **Local browser video:** H.264 -> fMP4 -> Axum -> Chrome MSE; record first
   frame, fragment behavior, and latency rather than claiming success in
   advance.
2. **Selective A/V:** dynamically track playback nodes, apply exclusions, mix
   AAC-LC with video, and verify that excluded audio remains audible locally
   but absent for the viewer.
3. **Product lifecycle:** add iced, secure link handling, Start/End, waiting and
   error states, viewer count, reconnect, and three-viewer fan-out from one
   encoder.
4. **Measured optimization:** validate 1080p60 and LAN latency, then add one
   hardware path and only the copy reductions justified by profiling before
   testing higher resolutions.

If selective audio cannot work without altering the host mix on the target
desktops, it is a product-direction failure, not a feature to silently remove.
If MSE misses the latency target, measure and tune it before considering a
different browser decoding path; WebRTC remains out of scope.

## Non-goals

Aercast does not plan to provide:

- WebRTC, signaling, ICE, STUN, TURN, SFUs, or NAT traversal
- cloud media relays, accounts, chat, voice calls, or camera sharing
- recording, remote control, clipboard sync, or file transfer
- Windows or macOS support
- microservices, a custom media protocol, or speculative cross-platform,
  encoder, transport, and plugin abstractions

## Development

The current source tree builds only the Phase 1 CLI probe. It opens the system
ScreenCast Portal picker and previews the selected monitor or window locally:

```sh
cargo fmt --check
cargo test
cargo run -- --monitor
```

The optional `--monitor` or `--window` argument restricts the Portal source
type. With no argument, the Portal may offer both. The current niri/AMD preview
path requires the GStreamer `pipewiresrc`, `vapostproc`, and `waylandsink`
elements. Press Ctrl-C to stop the preview and close the Portal session.
Contributors and coding agents must follow [AGENTS.md](AGENTS.md), including its
mandatory Ponytail review before every commit.

## License

Aercast is licensed under the [Mozilla Public License 2.0](LICENSE).
