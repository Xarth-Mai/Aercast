# Verification record

This file preserves measured development evidence, research notes, and smoke
results for later contributors and Codex sessions. [README.md](../README.md)
is the product overview, [development.md](development.md) holds engineering
decisions and the active plan, and [AGENTS.md](../AGENTS.md) defines how changes
are made and committed.

## 2026-08-26: niri development host

Environment:

- niri 26.04 and ScreenCast Portal v5
- PipeWire 1.6.8 and GStreamer 1.28.6
- AMD Radeon RX 6650 XT
- Firefox 154.0 and Chromium 151.0.7922.173 at the time of the recorded tests

For later tests on this host, Zen Browser 1.21.15b replaces the removed Firefox
package; GeckoDriver can drive it at `/usr/bin/zen-browser`.

Capture feasibility:

- Monitor preview ran for 65 seconds. Window preview ran for 65 seconds across
  a live resize; both stopped cleanly through SIGINT.
- The Portal source negotiated DMA-BUF `XR24` for a monitor and `AR24` for a
  window. The installed `vapostproc` was required to normalize this host's
  modifier for downstream consumers.
- A 48 kHz stereo application stream stayed connected to local output while a
  second PipeWire link recorded five seconds of non-silent PCM. Removing the
  temporary link did not change the local route during that observation. This
  did not prove that the link was passive or could not keep the application
  runnable after its normal output disappeared.

Local browser video:

- The current proof uses one `x264enc`, `h264parse`, `mp4mux` with a 100 ms
  fragment duration, and `appsink`. Output is fixed at 1280x720 and 30 FPS.
- `imagefreeze is-live=true allow-replace=true` keeps a damage-driven Wayland
  source live at 30 FPS without replaying an idle time gap when the next frame
  arrives.
- A window stream produced 960,660 HTTP media bytes in three seconds. Its first
  encoded frame arrived in 54 ms and its first `moof` in 154 ms after starting
  the pipeline. These are component observations, not end-to-end latency.
- The negotiated MIME was `video/mp4; codecs="avc1.42c01f"`, derived from the
  real AVC configuration record rather than a profile guess.
- Firefox 154 reached `playing`, reported no media error, and advanced 2.526
  seconds during a 2.5-second observation window.
- Chromium 151 reached `playing`, reported no media error, and advanced 2.500
  seconds during a 2.5-second observation window.
- After the AAC path was connected, Zen Browser 1.21.15b and Chromium 151 each
  passed normal and Play-retry runs with the served MIME
  `video/mp4; codecs="avc1.42c01f, mp4a.40.2"`. Its AVC parameter came from
  negotiated caps; AAC-LC was fixed by the pipeline. Both ended unmuted,
  reported no media error, and advanced for about 2.5 seconds. A Play retry
  reused the existing MediaSource and did not open a second stream response.
- Live Portal window runs then passed in Zen and Chromium with the same MIME,
  an unmuted media element, no media error, 720-pixel video, and 2.555 and 2.517
  seconds of advancement respectively. Their first encoded frames arrived in
  36-40 ms and their first fMP4 fragments in 165-166 ms. These remain local
  component observations, not end-to-end latency measurements.

The original video-only checks used the installed matching GeckoDriver and
ChromeDriver against the real Portal/PipeWire pipeline. The first AAC checks
used a synthetic H.264 + AAC fMP4 sample; the later checks used live Portal
output. A null Zen audio backend still created a PipeWire playback node, so the
Host-local Zen runs excluded `zen-bin`; Chromium runs excluded `chromium`.
These checks do not
establish official Google Chrome compatibility, LAN latency, GNOME/KDE
compatibility, multi-viewer joining, or release readiness.

Phase 4 fMP4 boundary research:

- A synthetic H.264 + AAC run through the installed GStreamer 1.28.6
  `mp4mux` and the production `appsink` settings emitted 303 buffers. The
  reconstructed byte stream was `ftyp`, `moov`, then 55 complete `moof` +
  `mdat` pairs, with no trailing bytes or `mfra` under
  `wait-on-eos=false`.
- `appsink` buffers were not fragment boundaries: each `mdat` header and its
  payload arrived separately, and payload arrived in multiple buffers. The
  Phase 4 cache therefore frames declared top-level ISO-BMFF box sizes before
  publishing anything; it does not search payload bytes for box names.
- The real `moov` described audio track 1 and video track 2. Every observed
  `trun` used flags `0x701`, including per-sample flags. The parser found three
  video fragments beginning with sample flags `0x40` and treated `0x100c0` as
  dependent video; `ffprobe` independently reported three keyframes.
- The installed-mux regression check feeds real split `appsink` output through
  the production parser. A separate bytewise fixture covers arbitrary input
  splitting, an audio track preceding a non-fixed video track ID, and malformed
  top-level sizes.
- The current managed execution sandbox denies local TCP binding and
  connections to the user's PipeWire and D-Bus sockets with `Operation not
  permitted`. That restricted no-capture run stopped at listener creation. A
  later explicitly elevated Host-UI run reached the real Wayland and loopback
  sockets; no mock replaces a Portal or browser result.

Phase 4 native Host UI:

- Iced is built with default features disabled and only `wayland`, `wgpu`, and
  `tokio` enabled. `cargo tree -i iced_tiny_skia` returned no package. The
  transitive `tiny-skia` package came only from `sctk-adwaita` through winit for
  Wayland client-side decorations, not from iced's content renderer.
- A real niri run mapped a 560x320 Aercast window and bound the long-lived HTTP
  server to `127.0.0.1:33565`. The process opened `/dev/dri/renderD128`; sysfs
  identified it as `amdgpu`, PCI ID `1002:73EF`, the recorded Radeon RX 6650 XT.
  This proves GPU rendering on this host, not that `wgpu` can never choose a CPU
  adapter elsewhere.
- The first unattended close attempt occurred while the session was locked.
  After a user-authorized unlock through the desktop shell's same-user IPC, an
  unlocked monitor run selected Portal node 64 at 2048x1152. Its first encoded
  frame arrived in 288 ms and its first fMP4 fragment in 414 ms. These are
  component observations, not end-to-end latency measurements.
- One Zen Viewer played the exact MIME
  `video/mp4; codecs="avc1.42c01f, mp4a.40.2"` at 1280x720, reached
  `readyState` 3, stayed unmuted, reported no media error, and advanced 2.202
  seconds. Three concurrent Zen Viewers then advanced 2.288-2.306 seconds with
  the same MIME and no media errors. Three established media sockets belonged
  to one Aercast process, and the Host UI reported `Viewers: 3`.
- End Sharing closed the Portal/media session and Viewer streams, reset the
  count to zero, made the old token return HTTP 404, and exposed a different
  waiting link whose page returned 200 and stream returned 425. A subsequent
  normal compositor window close removed the Host process, window, and
  loopback listener.
- GNOME Portal had already removed its session object after source-picker
  cancellation, so a second `Close` returned an object-not-found error. The
  Host now treats that cleanup call as best effort after a terminal Cancelled
  response. Two consecutive real Start/Cancel cycles returned to the waiting
  Host, and a third Start created a working capture session.
- The Chromium repeat selected Portal node 84 at 2048x1152. Its first encoded
  frame arrived in 246 ms and its first fMP4 fragment in 375 ms. One Chromium
  Viewer advanced 2.245 seconds; three concurrent Viewers then advanced
  2.298-2.299 seconds. All runs used the exact H.264/AAC MIME at 1280x720,
  reached `readyState` 4, stayed unmuted, and reported no media error.
- An intentionally one-byte-per-second media response was opened before the
  three Chromium Viewers. It was absent from the Aercast socket table while
  all three browser sockets remained established; the Host UI reported three
  Viewers and every Chromium check still advanced. This is the recorded real
  stalled-client isolation check.
- No eligible playback source existed during these Phase 4 runs, so their AAC
  track was silent; they did not revalidate selective-audio exclusion. Final
  cleanup left no Aercast, WebDriver, media stream, or listener behind, and
  PipeWire, PipeWire Pulse, and WirePlumber remained active.

Selective-audio safety analysis:

- PipeWire 1.6.8 accepts only `out`, `in`, or boolean values for
  `node.passive`; newer `follow` modes are not present in that release.
- A passive Aercast input linked to `Stream/Output/Audio` is still inferred as
  a non-passive link because an ordinary playback stream is not a suspendable
  sink or source.
- The default link factory removes a client-supplied `link.passive` property.
  The PulseAudio per-stream monitor path also creates an ordinary autoconnected
  input and does not provide a passive alternative.
- The development host therefore loaded a user-managed link-factory drop-in
  with `allow.link.passive = true`. Aercast did not write or reload that
  configuration. On an unmodified 1.6.8 daemon, capture remains inactive, but
  the candidate exists briefly as a non-passive link before failed readback
  tears it down.
- Passive readback alone was insufficient. With Game's speaker links removed
  while Discord kept the same sink runnable, the first prototype continued to
  receive Game. This established the need for an independent active-route
  gate; the prototype was stopped and all graph objects were removed before
  work continued.
- The corrected implementation requires both FL and FR playback ports to have
  active non-Aercast links into an `Audio/Sink`. It deactivates and flushes the
  input before rebuilding, confirms exact passive owned links, and announces
  readiness only after both links are active and one nonempty buffer arrives.
  Mono, surround, duplicate, and unfamiliar channel layouts are skipped by the
  development prototype.
- The final synthetic Game/Discord run used 440 Hz and 880 Hz stereo streams at
  0.001 source volume. Both retained active links to sink 59, whose volume
  stayed at 0.46. With Discord excluded, captured PCM had peak 0.001, RMS
  0.000707107, a 440 Hz magnitude of about 24, and a numerical-noise 880 Hz
  magnitude of about `5.1e-16` in the measured one-second window.
- Removing only Game's FL speaker link removed both Aercast links and left the
  output file exactly 14,417,920 bytes for a further seven seconds while
  Discord remained active. Restoring the full stereo route resumed capture.
- Route enforcement is event-driven: Aercast deactivates when PipeWire reports
  the graph change, but a daemon change can precede its client event briefly.
- Stopping and restarting Game with the same `application.id` but new
  PID/node/port IDs rematched it without restarting Aercast. Adding an
  unexpected Discord link to the Aercast input terminated the capture with
  `unexpected link entered the Aercast audio capture node`.
- The integrated Portal-to-Zen run kept both synthetic local routes active,
  excluded Discord and `zen-bin`, and reported one allowed stream with two
  verified passive links. A Web Audio analyser measured Game's 440 Hz bin at
  -87.68 dB and Discord's 880 Hz bin at -180.41 dB, about 92.7 dB apart, while
  H.264 + AAC playback advanced 2.528 seconds without a media error.
- The active sink routes and unchanged volume are a routing proxy; no human
  listening result or A/V synchronization measurement was recorded.
- After cleanup there were no Aercast, Game, Discord, or test links in the
  graph; PipeWire, PipeWire Pulse, and WirePlumber were active, and default
  output volume remained 0.46.
- An earlier offline synthetic mux check produced a 48 kHz stereo AAC-LC track
  lasting 3.051 seconds and a 1280x720 H.264 track lasting 3.000 seconds.

## Re-run gate

Run the static checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

Then run `cargo run -- --monitor --exclude zen-bin` and `cargo run -- --window
--exclude zen-bin` for Zen; repeat with `--exclude chromium` for Chromium. Copy
the tokenized share URL from the Host window, select **Start Sharing**, approve
the Portal source, then select **Play / Enable Audio** in the Viewer. Verify the
served MIME is supported, `video.error` is empty, buffered media exists, and
`currentTime` advances for at least two seconds. Select **End Sharing** and
verify the old page stops; repeat with the fresh link. Validate Zen before
Chromium.

The exact Host-local browser exclusion prevents its output feeding back into
the share; a null backend alone did not suppress Zen's playback node. Selective
audio testing on PipeWire 1.6.8 additionally requires the deliberate
link-factory opt-in documented in [development.md](development.md); first
record the default sink and volume, use isolated low-volume sources, verify
both local stereo routes before capture, and confirm the graph, services,
default sink, and volume again after cleanup.
