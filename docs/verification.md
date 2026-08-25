# Verification record

This file preserves measured development evidence for later contributors and
Codex sessions. [README.md](../README.md) remains the product source of truth;
[AGENTS.md](../AGENTS.md) defines how changes are made and committed.

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

The browser checks above used the installed matching GeckoDriver and
ChromeDriver against the real Portal/PipeWire pipeline. They do not establish
official Google Chrome compatibility, audio support, LAN latency, GNOME/KDE
compatibility, multi-viewer joining, or release readiness.

Selective-audio safety analysis:

- PipeWire 1.6.8 accepts only `out`, `in`, or boolean values for
  `node.passive`; newer `follow` modes are not present in that release.
- A passive Aercast input linked to `Stream/Output/Audio` is still inferred as
  a non-passive link because an ordinary playback stream is not a suspendable
  sink or source.
- The default link factory removes a client-supplied `link.passive` property.
  The PulseAudio per-stream monitor path also creates an ordinary autoconnected
  input and does not provide a passive alternative.
- The unsafe prototype was removed before commit. No regular-graph capture is
  now started by Aercast; the browser proof muxes a generated silent AAC-LC
  track instead. No further live audio smoke test is permitted until a safe
  link contract exists and the host's audio has been confirmed healthy.
- An offline synthetic mux check produced a 48 kHz stereo AAC-LC track lasting
  3.051 seconds and a 1280x720 H.264 track lasting 3.000 seconds. It touched no
  Portal, PipeWire graph, browser, or audio output and is not a live A/V result.

## Re-run gate

Run the static checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

Then run `cargo run -- --monitor` and `cargo run -- --window`. For each browser,
open the printed loopback URL, select **Play**, and verify the exact MIME is
supported, `video.error` is empty, buffered media exists, and `currentTime`
advances for at least two seconds. Validate Zen before Chromium.
