# Verification record

This document keeps only the latest useful evidence and current blocker for
Aercast compatibility, performance, and completion claims. It is not a test log
or a product specification.

## Current qualification

Aercast v0.1.1 is distributed through AUR, distribution packages, and a
prebuilt program archive, but no artifact install has been recorded here. The
latest complete real Host/Viewer qualification remains the Phase 5 niri run at
revision `073169b`. The current cross-platform Viewer change has automated
coverage only; it has not repeated the real Portal, PipeWire, Zen, Chromium, or
iOS Safari workflow.

| Current claim | Latest evidence | Current gap |
| --- | --- | --- |
| Idle startup and token rejection | Revision `478f129`: one niri window, loopback-only listener, no Portal or Aercast PipeWire objects, invalid page and stream routes returned `404` | Does not qualify capture, audio, playback, recovery, or current HEAD |
| Release artifacts | AUR metadata for v0.1.1 exists at revision `ef8cdc3`; other assets are distributed outside the recorded checks | No clean install and launch check from any artifact |
| Full product workflow | Phase 5 passed at `073169b` | Current v0.1.1 behavior has not repeated that acceptance |
| Cross-platform Viewer | User check after `abce8c0`: desktop Chrome no longer stuttered; iOS 27 Safari connected but rendered only one frame and a short audio slice every few seconds at the current 1080p60/12 Mbps setting | Safari repeatedly seeks while asynchronous autoplay is still starting; no platform is qualified by this short check |

## Recorded environment

Unless noted otherwise, the qualified baseline used:

| Component | Value |
| --- | --- |
| Distribution/kernel | CachyOS rolling, Linux `7.2.0-1-cachyos` |
| Compositor/Portal | niri `26.04` (`8ed0da4`), ScreenCast Portal v5 |
| Media | PipeWire `1.6.8`, GStreamer `1.28.6` |
| GPU/renderer | AMD Radeon RX 6650 XT, iced `wgpu` on `/dev/dri/renderD128` |
| Encoder | `x264enc`, 1280×720 at 30 FPS and 2.5 Mbps |
| Audio | AAC-LC, 48 kHz stereo at 128 kbps |
| Browsers | Zen Browser `1.21.15b` first, Chromium `151.0.7922.173` second |

Evidence applies only to the stated revision, environment, and scenario.

## Qualified baseline

| Area | Revision/date | Real result |
| --- | --- | --- |
| Portal capture | Recorded baseline, 2026-08-25 | niri monitor and window selection ran for 65 seconds and stopped cleanly; live resize survived. Monitor negotiated DMA-BUF `XR24`, window `AR24`; the AMD software path required installed `vapostproc`. First encoded frame was 246–288 ms and first complete fMP4 fragment 375–414 ms. |
| Browser playback | Recorded baseline, 2026-08-25 | Zen and Chromium accepted `video/mp4; codecs="avc1.42c01f, mp4a.40.2"` and played unmuted. Three concurrent Viewers advanced about 2.3 s each from one encoder; a deliberately stalled reader was dropped without stopping them. Late join began from a confirmed video keyframe. |
| App audio exclusions | 2026-08-27 | On PipeWire 1.6.8 without a daemon override, one allowed 440 Hz stereo stream kept its sink route and was captured through two exact passive-input links; a Communication stream was excluded. Restart rematched by stable identity, teardown cleaned up, and the earlier browser analyser measured the excluded 880 Hz signal about 92.7 dB lower. |
| Same-link lifecycle | `e54f1b2`, 2026-08-27 | Zen and Chromium reused one page through playback, media replacement, Stop, and later Start without navigation. Replacement resumed in 17–24 ms to first frame and about 158–161 ms to first fragment. Three recoveries were bounded; the next failure became terminal. |
| Desktop product | `073169b`, 2026-08-27 | niri verified hide/restore, single instance, tray lifecycle, Portal start while hidden, notifications without tokens, settings persistence and stopped-only network rebind. Zen then Chromium each reached three Viewers from one encoder; refresh revoked the old link and lagging-reader isolation preserved the other Viewers. |

The timings above are local component observations, not display-to-display or
trusted-LAN latency measurements.

## Latest selective-audio graph check

The 2026-08-27 PipeWire run removed the user passive-link override and confirmed
that its merged daemon configuration was empty. `node.passive=in` was read back
on the Aercast capture node; Aercast-created links contained the exact endpoints
and no client-supplied `link.passive`. The allowed application retained its
existing active sink links, while the Communication application had no Aercast
link. Stopping and restarting the allowed source removed and recreated only the
expected objects, and final Stop left no Aercast or test graph objects.

This proves graph policy and route preservation, not human-perceived audio
quality. The latest signal-level browser evidence remains the analyser result in
the qualified baseline.

## Static acceptance at the baseline

At revision `073169b`, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, 37 tests with five explicit
environment-dependent ignores, and `git diff --check` passed. These results do
not qualify later revisions.

For the 2026-08-28 cross-platform Viewer working tree, `node --check` on the
embedded script, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
all 38 runnable tests, and `git diff --check` passed; five explicitly
environment-dependent tests remained ignored. This proves the generated Viewer
contract and existing Host behavior, not playback in any browser.

The latest real Viewer check used the existing HTTPS path and 1080p60/12 Mbps
VA-API stream. It confirms the Chrome buffering fix and exposes the current
iOS Safari playback blocker, but lacks the exact Chrome version and two-minute
acceptance duration required for qualification.

## Not yet qualified

- Current HEAD through a complete real GUI, Portal, selective-audio, Zen, and
  Chromium acceptance run
- iOS Safari 17.1 or newer, including the available iOS 27 device at 720p60 and
  1080p60
- Windows Chrome, Edge, and Firefox; macOS Safari; and Android Chrome and
  Firefox
- Mobile 1440p or 120 FPS playback
- Installation and launch from AUR, `.deb`, or a prebuilt release asset
- GNOME, KDE, other distributions, stable desktop Firefox, or Google Chrome
- Hardware encoding; 1080p60, 1440p60, or 120 FPS
- Trusted-LAN end-to-end latency, long-duration load, or public-network
  deployment
