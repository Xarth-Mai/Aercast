# Aercast

> **Pre-alpha:** Aercast currently provides a loopback, single-viewer A/V
> schema proof with silent audio. There is no product UI, selective audio,
> share token, or multi-viewer session yet. Only the host-specific results
> recorded below are verified.

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
Plasma, and niri are the initial compositor targets. The recorded niri host
below is the only verified environment so far. Flatpak support is deferred
because selective audio needs access to the user's regular PipeWire graph in
addition to the restricted ScreenCast Portal remote.

The primary viewer target is the current stable desktop release of Mozilla
Firefox. Current stable desktop Chromium is a secondary, first-class target:
both must pass the same live MSE smoke tests, but implementation and
compatibility choices are validated on Firefox first. The page checks the exact
codec MIME at runtime. Firefox still requires the operating system to provide
H.264 and AAC decoding; a missing codec is an actionable compatibility error,
not an experimental support tier. Google Chrome, Safari, and mobile browsers
are not initial compatibility promises. Chromium builds can omit proprietary
codecs; see the
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

## Completed milestone: capture feasibility

This milestone contained no desktop UI, HTTP server, encoder, or product
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

## Completed milestone: localhost browser video

The current proof serves an embedded plain-JavaScript MSE viewer on loopback.
It uses one software H.264 encoder, a 100 ms `mp4mux` fragment target, an exact
codec MIME derived from negotiated AVC data, and a bounded stream queue that
cannot backpressure GStreamer. It deliberately supports only the first Viewer;
tokens, GOP replay, reconnect, and fan-out belong to the lifecycle milestone.

On the recorded niri host, both Firefox 154 and Chromium 151 played the real
Portal stream without a media error and advanced for the full 2.5-second test
window. A window stream's first encoded frame arrived in 54 ms and its first
fMP4 fragment in 154 ms. These are local component observations, not a LAN or
end-to-end latency claim. See [the verification record](docs/verification.md).

## Current milestone: selective audio and A/V

1. Dynamically track ordinary PipeWire playback nodes and rematch applications
   across start, exit, and restart.
2. Tap allowed streams without moving their existing speaker links, exclude
   Aercast itself, and preserve a silent track when audio is off or absent.
3. Add AAC-LC to the existing video pipeline and produce one synchronized fMP4
   stream whose track layout does not change mid-session.
4. Verify Game + Discord: the host hears both while the Viewer hears Game but
   not Discord.

### Current safety blocker

The verified host runs PipeWire 1.6.8. With its default daemon configuration,
`node.passive=in` cannot make a link from an ordinary playback stream passive,
and the link factory ignores a client's `link.passive=true`. Such a tap could
therefore keep the target stream runnable after its normal output disappears.
Aercast will not create that link or change the user's PipeWire daemon
configuration. The current pipeline carries only a permanent silent AAC track.

Selective-audio work resumes only when the passive behavior can be enforced and
verified without changing the host mix. The relevant 1.6.8 behavior is defined
by PipeWire's
[`impl-node.c`](https://gitlab.freedesktop.org/pipewire/pipewire/-/blob/1.6.8/src/pipewire/impl-node.c),
[`impl-link.c`](https://gitlab.freedesktop.org/pipewire/pipewire/-/blob/1.6.8/src/pipewire/impl-link.c),
and
[`module-link-factory.c`](https://gitlab.freedesktop.org/pipewire/pipewire/-/blob/1.6.8/src/modules/module-link-factory.c).

## Later milestones

1. **Product lifecycle:** add iced, secure link handling, Start/End, waiting and
   error states, viewer count, reconnect, and three-viewer fan-out from one
   encoder.
2. **Measured optimization:** validate 1080p60 and LAN latency, then add one
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

The current source tree builds the loopback browser proof with Portal video and
a silent AAC track. It opens the ScreenCast Portal picker, then prints a one-use
local Viewer URL:

```sh
cargo fmt --check
cargo test
cargo run -- --monitor
```

The optional `--monitor` or `--window` argument restricts the Portal source
type. With no argument, the Portal may offer both. The current niri/AMD path
requires the GStreamer `pipewiresrc`, `vapostproc`, `imagefreeze`, `x264enc`,
`h264parse`, `audiotestsrc`, `avenc_aac`, `aacparse`, `mp4mux`, and `appsink`
elements. Open the printed URL, select **Play**, and press Ctrl-C to stop the
stream and close the Portal session.
Contributors and coding agents must follow [AGENTS.md](AGENTS.md), including its
mandatory Ponytail review before every commit. Measured development evidence is
kept in [docs/verification.md](docs/verification.md).

## License

Aercast is licensed under the [Mozilla Public License 2.0](LICENSE).
