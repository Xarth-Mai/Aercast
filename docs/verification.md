# Verification record

This document keeps only the latest useful evidence and current blocker for
Aercast compatibility, performance, and completion claims. It is not a test log
or a product specification.

## Current qualification

Aercast v0.1.2 is distributed through GitHub packages, a prebuilt program
archive, and AUR, but no artifact install has been recorded here. The latest
complete real Host/Viewer qualification remains the niri run at revision
`073169b`. Later changes have not repeated that complete workflow. A partial
2026-08-29 Portal, PipeWire, and iOS Safari run rejected a lower Viewer-lag
threshold and restored the smooth 3.0 s policy.

| Current claim | Latest evidence | Current gap |
| --- | --- | --- |
| Idle startup and token rejection | Revision `478f129`: one niri window, loopback-only listener, no Portal or Aercast PipeWire objects, invalid page and stream routes returned `404` | Does not qualify capture, audio, playback, recovery, or current HEAD |
| Release artifacts | GitHub Actions run `33185981523` passed checks and published the v0.1.2 `.deb` and tarball; `makepkg --verifysource` passed for the matching AUR metadata at `92d11ca`; the AUR RPC reports v0.1.2-1 | No clean install and launch check from either artifact source |
| Full product workflow | Revision `073169b` passed the recorded niri workflow | Current v0.1.2 behavior has not repeated that acceptance |
| Cross-platform Viewer | In the 2026-08-29 real iOS A/B described below, the 1.8 s correction reduced reported lag but made playback fall below one frame per second; restoring 3.0 s produced smooth playback with 1.3 s Host-reported lag and about 2 s perceived delay | No safe unified lag reduction was found; exact OS/browser builds and duration remain incomplete, Windows Firefox was not rerun, and neither platform is qualified |
| Media pipeline optimization | Generated pipeline contracts cover selectable AAC rates, 100 ms x264 VBV and VA-API CPB constraints, VA-memory negotiation, and immediate normal-EOF reconnect; the 2026-08-29 real A/B reached iOS playback at 1080p60/16 Mbps with VA-API | DMA-BUF/zero-copy, Host CPU/GPU, and constrained-network measurements remain unrecorded; zero-copy and latency are unqualified |
| Idle-media Host candidate | 2026-08-30 current working tree: formatting, Clippy with warnings denied, all 57 runnable Rust tests, and diff whitespace checks passed; five environment-dependent tests remained ignored | No current Portal, VA-API, vkmark, Zen, Chromium, or external-Viewer run; AMD throughput, power savings, wake latency, and regression limits remain unqualified |
| Desktop lifecycle polish | 2026-08-28 working tree: tray tooltip/count and first/last-Viewer notification contracts, isolated D-Bus single-instance activation, formatting, Clippy, and all 38 runnable Rust tests passed | The current source build has not passed real niri tray, notification, or window-activation checks |

On 2026-08-30, the AUR v5 RPC returned `0.1.2-1`, and an HTTPS
`git ls-remote` resolved `master` to `2a85c6e`, matching the tracked AUR
metadata. This confirms remote metadata publication, not package installation
or launch.

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

For the 2026-08-28 media, cross-platform Viewer, and desktop-lifecycle working
tree, `node --check` on the embedded script, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, all 38 runnable tests, and
`git diff --check` passed; five explicitly environment-dependent tests remained
ignored. This proves generated settings, pipeline, Viewer, tray, and
notification contracts, not real capture, encoding, playback, or window
activation.

For the 2026-08-30 current working tree, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, all 57 runnable Rust tests, and
`git diff --check` passed; five explicitly environment-dependent tests remained
ignored. The restricted sandbox first produced 55 passes and two expected
`EPERM` socket failures; the same complete suite passed with narrowly granted
host socket permission. These checks cover the idle-media demand lifecycle,
structured VA-to-x264 fallback, asynchronous video probing, bounded Viewer
state, proxy trust, telemetry rate limiting, and control-queue behavior. They
do not qualify real Portal capture, VA-API/VAMemory behavior, vkmark, Zen or
Chromium playback, an external Viewer, AMD performance or power savings, a
wake-fragment p95 of 500 ms or less, or a performance regression limit.

The latest Viewer A/B ran `cargo run` on 2026-08-29 on the recorded niri host,
selected a Screen through Portal v5, and used 1920×1080 at 60 FPS, 16 Mbps
VA-API video, and 160 kbps AAC. An iPhone Air using the Safari `604.1` and
AppleWebKit `605.1.15` user-agent components connected over direct IPv6 LAN.
With the hard-correction threshold lowered from 3.0 to 1.8 seconds, it reported
10–30 ms RTT and 1.0–2.0 s playback lag, but actual playback stuttered, dropped
frames, and stayed below one frame per second. After restoring the 3.0 s policy
with the same settings, Host-reported lag was 1.3 s, perceived delay was about
2 s, and playback was smooth. The 1.8 s change remains reverted.

The latest Windows observation remains the revision `0170682` Windows 11
Firefox 154 check through an external HTTPS reverse proxy over IPv6: playback
lag held 0.9–1.3 s for several hours without stalls, reconnects, or audio/video
interruption. The iOS and Windows observations have incomplete exact browser
and OS builds and are not end-to-end latency measurements or platform
qualification.

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
- Current no-Viewer sleep and wake on the real Portal/VA-API path, including a
  wake-fragment p95 of 500 ms or less
- Ryzen 7 5700X / RX 6650 XT A/B measurements for 1080p60 with 1, 3, and 8
  Viewers, 1440p60 with one Viewer, three vkmark runs, VA/VAMemory negotiation,
  Host CPU/GPU/power, and a performance regression no greater than 3%
- A single Viewer at 1440p60/24 Mbps over 30 Mbps with 80 ms RTT, 20 ms jitter,
  and 0.1% loss; VA-API raw-frame zero-copy remains unqualified
- Trusted-LAN end-to-end latency, long-duration load, or public-network
  deployment
