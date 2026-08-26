# Verification record

This document owns the latest real evidence and current blocker behind Aercast
compatibility, performance, and completion claims; it does not own product
plans, future targets, visual rules, or append-only raw logs.

## Current niri host

Unless a record says otherwise, the current evidence was collected on this
environment:

| Field | Value |
|---|---|
| Date | 2026-08-26 |
| Distribution | CachyOS rolling, Linux `7.2.0-1-cachyos` |
| Compositor | niri `26.04` (`8ed0da4`), ScreenCast Portal v5 |
| Media services | PipeWire `1.6.8`, GStreamer `1.28.6` |
| GPU | AMD Radeon RX 6650 XT, amdgpu, PCI `1002:73ef` |
| Content renderer | iced `wgpu` on `/dev/dri/renderD128` |
| Media encoder | GStreamer `x264enc` software H.264, 1280×720 at 30 FPS, 2.5 Mbps |
| Audio encoder | GStreamer AAC-LC, 48 kHz stereo, 128 kbps |
| Firefox-family vehicle | Zen Browser `1.21.15b`, checked first |
| Chromium vehicle | Chromium `151.0.7922.173` |

This host proves only the recorded scenarios. It does not establish support on
another compositor, GPU, encoder, distribution, or browser build.

## Portal capture

**Scenario:** real niri ScreenCast Portal monitor and window selection into the
restricted PipeWire remote and GStreamer pipeline.

**Reproducible development commands at the time of the run:**

```sh
cargo run -- --monitor --exclude zen-bin
cargo run -- --window --exclude zen-bin
```

**Result:** both source types ran continuously for 65 seconds and stopped
cleanly. A live window resize did not terminate capture. Monitor capture
negotiated DMA-BUF `XR24`; window capture negotiated `AR24`. The current AMD
path required installed `vapostproc` to normalize the modifier for downstream
software encoding.

The most recent full Host lifecycle selected a 2048×1152 monitor. Zen's run
received Portal node 64; Chromium's repeat received node 84. The component
observations were:

| Browser run | First encoded frame | First complete fMP4 fragment |
|---|---:|---:|
| Zen | 288 ms | 414 ms |
| Chromium | 246 ms | 375 ms |

These are startup component timings from one local machine, not display-to-
display or LAN latency measurements.

## Browser playback

**Scenario:** one live Portal source, one `x264enc`, H.264 and AAC-LC in one
`mp4mux` fragmented MP4 stream, served locally to real browser MSE. Zen was run
before Chromium. Each Host-local browser identity was excluded from system
audio to avoid feedback.

**Result:** both engines accepted the exact negotiated MIME
`video/mp4; codecs="avc1.42c01f, mp4a.40.2"`, played 1280×720 video unmuted, and
reported no media error.

| Browser | Scenario | Playback advancement | Final readyState |
|---|---|---:|---:|
| Zen `1.21.15b` | one Viewer | 2.202 s | 3 |
| Zen `1.21.15b` | three concurrent Viewers | 2.288–2.306 s each | at least 3 |
| Chromium `151.0.7922.173` | one Viewer | 2.245 s | 4 |
| Chromium `151.0.7922.173` | three concurrent Viewers | 2.298–2.299 s each | 4 |

Three established media sockets belonged to one Aercast process and Host UI
count reached three. An intentionally one-byte-per-second response was dropped
while all three Chromium responses stayed established and advanced. This is
current evidence for one-encoder fan-out and stalled-reader isolation, not
Phase 5 product acceptance.

The fMP4 late-join parser was also checked against real installed `mp4mux`
output. It reconstructed `ftyp` + `moov`, then complete `moof` + `mdat` pairs,
identified the negotiated video track when audio preceded it, and began replay
at an independently confirmed video keyframe. Appsink buffer boundaries were
not assumed to be MP4 box boundaries.

## Selective audio

**Scenario:** two low-volume 48 kHz stereo playback streams represented Game
at 440 Hz and Discord at 880 Hz. Both retained active links to sink 59 while
Aercast used exact passive capture links. Discord and the Host-local Zen
identity were excluded; the integrated Portal stream played in Zen.

**Result:**

- The sink volume remained `0.46` during the isolated observation and both
  applications retained their local stereo routes.
- Captured PCM measured peak `0.001`, RMS `0.000707107`, a 440 Hz magnitude near
  `24`, and only numerical-noise 880 Hz magnitude near `5.1e-16`.
- The browser analyser measured the allowed 440 Hz bin at `-87.68 dB` and the
  excluded 880 Hz bin at `-180.41 dB`, about `92.7 dB` apart. H.264/AAC playback
  advanced 2.528 seconds without a media error.
- Removing one Game speaker channel removed Aercast's Game capture links while
  Discord kept the sink active. Restoring the full stereo route resumed Game
  capture.
- Restarting Game with the same `application.id` and different PID/node/port
  IDs rematched it without restarting Aercast.
- Cleanup left no Aercast or test link in the graph; PipeWire, PipeWire Pulse,
  and WirePlumber remained active.

This is routing and signal evidence, not a human listening result or an A/V
synchronization measurement.

### PipeWire 1.6.8 prerequisite

PipeWire `1.6.8` removes a client-supplied `link.passive=true` unless its link
factory has deliberately enabled passive client links with
`allow.link.passive = true`. Aercast neither writes nor reloads that daemon-wide
setting. It fails closed when exact passive readback, active local sink routes,
expected endpoints, active status, and real data are not all present.

This prerequisite remains a packaging and compatibility blocker. No evidence
yet covers the requested automatic `media.role=Communication` exclusion.

## Current lifecycle evidence and blockers

The 2026-08-26 real niri run proved that the current development server can
provide a waiting page, start a Portal share, survive two Start/Cancel cycles,
serve one and three Viewers, isolate a stalled response, and cleanly remove its
Portal session, media pipeline, audio graph objects, listener, and process.

That run also confirmed behavior superseded by the current product contract:

- the mapped window was `560×320`, resizable, and tiled;
- closing the window exited the process;
- Stop rotated the token, made the old link `404`, and generated a new waiting
  link.

The current Phase 4 therefore remains incomplete until a fresh real run proves:

- Stop returns the same token to `425` waiting;
- a media-only failure recovers automatically on the same Portal session and
  token, with no more than three retries;
- Zen first and Chromium second recover in the existing page and play again
  after a later Start;
- Communication-role audio is excluded without PID matching.

Phase 5 additionally lacks current evidence for the fixed `480×640` window,
niri automatic floating, hide and tray restore, single-instance activation,
fixed dark/accent/accessibility behavior, settings, Viewer telemetry and kick,
explicit link refresh, notifications, and the final three-or-more-Viewer
desktop workflow.

There is no current evidence for a packaged install, GNOME/KDE support, real
stable Firefox rather than the Zen-family vehicle, official Google Chrome,
mobile browsers, hardware encoding, 1080p60, 1440p60, 120 FPS, trusted-LAN
end-to-end latency, or public-network deployment.
