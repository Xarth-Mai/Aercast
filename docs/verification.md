# Verification record

This document keeps only the latest useful evidence and current blocker for
Aercast compatibility, performance, and completion claims. It is not a test log
or a product specification.

## Current qualification

Aercast v0.1.5 is tagged and its AUR package has passed a local upgrade install on CachyOS. GitHub binary-asset publication is still pending in run `34497535046`. The latest complete real Host/Viewer qualification remains the niri run at revision `073169b`. Later changes have not repeated that complete workflow. A partial 2026-08-29 Portal, PipeWire, and iOS Safari run rejected a lower Viewer-lag threshold and restored the smooth 3.0 s policy

| Current claim | Latest evidence | Current gap |
| --- | --- | --- |
| Idle startup and token rejection | Revision `478f129`: one niri window, loopback-only listener, no Portal or Aercast PipeWire objects, invalid page and stream routes returned `404` | Does not qualify capture, audio, playback, recovery, or current HEAD |
| Release artifacts | v0.1.5 source checksum and `makepkg --noconfirm` passed; AUR `master` is `3df00bd`; local upgrade reports `aercast 0.1.5-1`, eight intact files, and no missing dynamic libraries | Upgrade install only; the existing process was left running, so new-version GUI launch and clean installation remain unqualified; GitHub run `34497535046` is pending |
| Full product workflow | Revision `073169b` passed the recorded niri workflow | Current v0.1.5 behavior has not repeated that acceptance |
| Cross-platform Viewer | In the 2026-08-29 real iOS A/B described below, the 1.8 s correction reduced reported lag but made playback fall below one frame per second; restoring 3.0 s produced smooth playback with 1.3 s Host-reported lag and about 2 s perceived delay | No safe unified lag reduction was found; exact OS/browser builds and duration remain incomplete, Windows Firefox was not rerun, and neither platform is qualified |
| Media pipeline optimization | Generated pipeline contracts cover selectable AAC rates, 100 ms x264 VBV and VA-API CPB constraints, VA-memory negotiation, and immediate normal-EOF reconnect; the 2026-08-29 real A/B reached iOS playback at 1080p60/16 Mbps with VA-API | DMA-BUF/zero-copy, Host CPU/GPU, and constrained-network measurements remain unrecorded; zero-copy and latency are unqualified |
| Stability fixes | The [2026-09-10 checks](#stability-checks) cover sleeping Apply, last-successful-snapshot retention, authorized HTTP wake, and Viewer timeout/retry behavior | Real Portal, audio, desktop-browser, iPhone, and clean-install acceptance was explicitly skipped; no new platform or performance qualification |
| Host module split | The [module-split checks](#module-split-checks) at `f83a658` cover all runnable Rust tests and a real niri Portal capture start/stop | Partial source-build smoke only; selective audio and browser playback remain unqualified |
| Desktop lifecycle polish | 2026-08-28 working tree: tray tooltip/count and first/last-Viewer notification contracts, isolated D-Bus single-instance activation, formatting, Clippy, and all 38 runnable Rust tests passed | The current source build has not passed real niri tray, notification, or window-activation checks |

## Host readability check

The brighter Host text palette passed `cargo test appearance::tests --quiet` (one passed, one isolated-session-bus test ignored), including WCAG AA contrast checks for primary and secondary text against all four neutral surfaces in normal and high-contrast modes. `cargo test ui::tests --quiet` passed 15 tests with one isolated-session-bus test ignored; `cargo fmt --check` and `git diff --check` passed

Visual checks at `640×480` and `960×640`, text fit across selected controls, and real keyboard-focus checks remain unverified. The command environment exposes neither `WAYLAND_DISPLAY` nor `NIRI_SOCKET`, and the available desktop tool cannot target niri windows; automated checks do not establish rendered readability or absence of overflow

## Overview status check

`cargo test ui::tests --quiet` passed 16 tests with one isolated-session-bus test ignored. Coverage includes combined Screen/Window labels, actual idle state, sleeping settings updates, wake, stop, new sharing, and unmodified error text. `cargo test share_session --quiet` passed all 11 tests with local socket permission, including active-versus-sleeping events and the fixed idle grace; the sandbox-only run had three `EPERM` bind failures. `cargo fmt --check` and `git diff --check` passed

Real niri verification of the combined status, idle transitions, and relocated Share link guidance remains pending under the desktop-access limitation described in the Host readability check

## Concurrent media failure recovery check

On 2026-09-12, `cargo test media::tests -- --nocapture` passed four tests with two host-stack tests ignored; `cargo test audio -- --nocapture` passed six matching tests; `cargo test share_session -- --nocapture` passed 11 tests with local socket permission. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `git diff --check` passed. The regression verifies that concurrent video failure and audio `Flushing` retain both messages as a retryable error, controls remain authoritative, and actual audio cleanup failures remain terminal. The existing recovery-policy check verifies the three-attempt limit. These checks do not reproduce a real live-stream fault or qualify automatic Portal/PipeWire/browser recovery

The captured `/tmp/aercast-live.log` reports `portal-video` buffer removal and destroyed PipeWire linkage at process time 5.29 seconds, followed by another successful pipeline at 9.29 seconds. The timing is consistent with possible idle sleep/wake, but the log does not identify that transition and contains no `Flushing` failure. It therefore does not establish the cause of the reported live fault. Audio push errors now include current and pending appsrc state, and concurrent queued GStreamer errors are retained for the next reproduction

## Host restart check

On 2026-09-12, `cargo test ui::tests -- --nocapture` passed 15 tests with one isolated-session-bus test ignored. The new regression checks terminal-error link cleanup, restart gating, duplicate restart rejection, retained unsaved Draft, return to the Start transition, and rejection during Quit. `cargo test share_session -- --nocapture` passed 11 tests with local socket permission; the sandbox-only run had three `EPERM` bind failures. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `git diff --check` passed. These are UI state and Host control checks; real GUI restart, new-link browser playback, and Portal re-selection after a live fault remain unverified. The reported live `Flushing` failure has not been reproduced in the captured log, so its root cause remains unresolved

## Viewer correction check

On 2026-09-12, `bun test tests/viewer-recovery.test.js` passed 13 tests with 251 assertions and `git diff --check` passed. The shipped Viewer script was exercised with browser API doubles to verify normal speed through 2 seconds of lag, continuous acceleration above 2 seconds capped at 1.08× from 3 seconds, a 1.5-second automatic-correction target, no automatic seek at 5 seconds of lag, transient lag rejection, sustained-lag correction above 5 seconds, a 10-second correction cooldown, normal-speed recovery, and timer reset during pause, seeking, and hidden-page appends. The new acceleration curve, correction target, 3-second persistence, and 10-second cooldown values have not been compared on a real iPhone; intermittent iOS stutter remains unresolved at the device-validation level. These checks do not establish decoder behavior, smooth playback, or end-to-end latency

## Fixed H.264 High check

On 2026-09-12, `cargo test software_high_profile -- --ignored --nocapture` passed a real local x264 negotiation check for fixed High on synthetic 320×240 input with B frames configured off. `cargo test settings::tests -- --nocapture` passed three tests; `cargo test ui::tests -- --nocapture` passed 15 with one environment-dependent test ignored; `cargo test media::tests -- --nocapture` passed four with three environment-dependent tests ignored, including pipeline contracts for fixed High and disabled B frames on both encoders. `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `git diff --check` passed. These checks do not establish VA-API High negotiation, browser playback/reconnection, or comparative quality and performance

## Encoder quality check

Before the explicit full-range conversion change on 2026-09-12, GStreamer `1.28.7` and FFmpeg `n9.0.1` encoded the same 120-frame synthetic `testsrc2` input at 1280×720, 60 FPS, and 6 Mbps using x264 `ultrafast` and `superfast`. Both outputs decoded as Constrained Baseline, `yuv420p`, and zero B frames. Overall SSIM against the raw reference rose from `0.969337` to `0.969998`; luma SSIM slightly fell from `0.962865` to `0.962628`, so this is a small aggregate improvement on one synthetic sample, not a general visual-quality claim

Reproduce the sample and comparison with:

```sh
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=1280x720:rate=60 -frames:v 120 -pix_fmt yuv420p -f rawvideo -y /tmp/aercast-quality-reference.yuv
for preset in ultrafast superfast; do
  gst-launch-1.0 -q filesrc location=/tmp/aercast-quality-reference.yuv ! rawvideoparse format=i420 width=1280 height=720 framerate=60/1 ! x264enc tune=zerolatency speed-preset="$preset" bitrate=6000 vbv-buf-capacity=100 nal-hrd=cbr key-int-max=60 ! video/x-h264,profile=constrained-baseline ! h264parse ! mp4mux fragment-duration=100 ! filesink location="/tmp/aercast-quality-$preset.mp4"
  ffprobe -v error -select_streams v:0 -show_entries stream=profile,pix_fmt,has_b_frames -of compact "/tmp/aercast-quality-$preset.mp4"
  ffmpeg -hide_banner -i "/tmp/aercast-quality-$preset.mp4" -f rawvideo -pixel_format yuv420p -video_size 1280x720 -framerate 60 -i /tmp/aercast-quality-reference.yuv -lavfi ssim -f null -
done
```

`cargo test media::tests -- --nocapture` passed 3 tests with 2 environment-dependent tests ignored; the additional `cargo test software_video_expands_limited_range_pixels -- --ignored --nocapture` passed 1 real software-conversion test; `cargo fmt --check` and `git diff --check` passed. The tool environment did not expose `vah264enc`. Balanced VA-API quality, sustained encoder load, real Portal capture, and browser playback with these presets remain unverified

The final software pipeline was additionally exercised with 60 RGBx SMPTE frames at 1280×720/60 FPS, its generated converter and encoder chain, `h264parse`, and `mp4mux fragment-duration=100`. `ffprobe -v error -select_streams v:0 -show_entries stream=profile,pix_fmt,has_b_frames,color_range,color_space,color_transfer,color_primaries -of compact /tmp/aercast-full-bt709.mp4` reported Constrained Baseline, `yuvj420p` (FFmpeg's full-range 8-bit 4:2:0 representation), zero B frames, `color_range=pc`, and BT.709 for matrix, transfer, and primaries

The pixel test feeds Limited BT.709 I420 black/white frames through the generated software conversion chain. The original direct I420 conversion retained 16/235 despite full-range output caps; normalization through RGBx produced black 0 and white 251, within the test's rounding tolerance. This verifies actual range expansion in addition to metadata, with rounding loss in the YUV-to-RGB-to-YUV conversion. It does not establish colorimetric accuracy, HDR handling, hardware range conversion, or browser display fidelity. The preceding SSIM comparison isolates encoder presets and does not measure the final full-range conversion chain

## v0.1.5 package verification

On 2026-09-10, release revision `29d8c48` passed `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (58 passed, five explicitly ignored environment-dependent tests), `bun test tests/viewer-recovery.test.js` (12 passed, 225 assertions), and `git diff --check`

In `aur/aercast`, `makepkg --verifysource` validated the GitHub v0.1.5 tag archive and `CARGO_TARGET_DIR=/home/lzzz/MyProjects/Aercast/target makepkg --noconfirm` built the release binary and passed the same 58 Rust tests; the extracted source's `target` symlink pointed to that shared build directory for packaging

`sudo -n pacman -U --noconfirm /home/lzzz/MyProjects/Aercast/aur/aercast/aercast-0.1.5-1-x86_64.pkg.tar.zst` upgraded the installed 0.1.4-1 package; `pacman -Q aercast` returned `aercast 0.1.5-1`, `pacman -Qkk aercast` reported eight files and zero altered files, and `ldd /usr/bin/aercast` had no missing libraries

AUR publication used `git -c core.sshCommand='ssh -4 -o StrictHostKeyChecking=yes' push origin master`, and remote readback matched `3df00bd`. The existing Aercast process was preserved; these checks establish source build, package tests, publication, and local upgrade integrity, not a new-version GUI launch or real capture/playback acceptance

## Stability checks

Production revision: `7ad834c` (2026-09-10), comprising the sleeping-Apply fix `97f38fc`, archive/CI fix `2d29e7c`, Viewer recovery `a225517`, and Bun runner conversion `7ad834c`. The final bundled test revision additionally isolates read and append timeout assertions from the playback watchdog

Environment: CachyOS, Linux `7.2.3-1-cachyos`, Rust/Cargo `1.98.1`, Bun `1.4.0`

| Command or scenario | Observed result | Evidence boundary |
| --- | --- | --- |
| `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` | Passed | Static Rust checks |
| `cargo test` | 58 passed, 0 failed, 5 explicitly ignored environment-dependent tests | Includes the real local HTTP route and control loop, with permission to bind temporary loopback ports; no Portal or media-pipeline acceptance |
| `cargo test sleeping -- --nocapture` and `cargo test media_apply -- --nocapture` | 3 sleeping checks and 2 apply checks passed | Ready grace reaches Sleep; repeated Apply emits the new Active snapshot while control stays pending; a valid HTTP stream request returns `425` and releases Wake with the latest candidate and original rollback snapshot |
| `bun test tests/viewer-recovery.test.js` | 12 passed, 0 failed, 225 assertions | Uses the shipped Viewer script with browser API doubles and a virtual clock; separately exercises connection, read, append, decoder, media-source opening, waiting, pause/visibility, bounded jitter, normal EOF, and inactive retry; no real-browser playback claim |
| `git diff --check` and staged whitespace checks | Passed | Local patches only; GitHub Actions has not run these new commits |
| Downloaded v0.1.4 tarball; `tar -xzf aercast-*.tar.gz` then `cd aercast-v*-x86_64-unknown-linux-gnu`; executable-mode and `ldd ./aercast` checks | Archive directory and executable match the corrected README; no missing dynamic libraries on this host | Extraction and local dependency resolution only; `./aercast` from the release package and clean installation were not accepted |
| CI YAML trigger check | `main` and `v*` pushes enabled; release restricted to version-tag push; Bun test command configured | Local configuration inspection, not a hosted CI run |

Real acceptance was skipped for the stability-fix batch at the user’s request. The later partial source-build capture check is recorded below; it does not qualify package installation, selective audio, browser playback, or recovery

## Module-split checks

Source revision: `f83a658` (2026-09-10), following media/Portal extraction at `59ca401`; the executable entry point and shared contracts remain in `main.rs`, with UI and share-session implementations tested separately

Environment: CachyOS, Linux `7.2.3-1-cachyos`, niri with ScreenCast Portal v5, PipeWire `1.6.8`, GStreamer `1.28.7`, Rust/Cargo `1.98.1`, Bun `1.4.0`; the existing saved profile selected 1920×1080 at 60 FPS, 12 Mbps VA-API video and 160 kbps audio

| Command or scenario | Observed result | Evidence boundary |
| --- | --- | --- |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `git diff --cached --check` | Passed; Rust tests: 58 passed, 5 explicitly ignored | Existing contracts retained under their owning modules; no new platform qualification |
| `cargo test share_session -- --nocapture` | 11 passed, including sleeping Apply followed by a valid Viewer wake, rollback snapshots, idle grace, and bounded recovery | Control-loop and local HTTP checks; no real failed-pipeline rollback injection |
| `bun test tests/viewer-recovery.test.js` | 12 passed, 225 assertions | Viewer script unchanged; browser API doubles only |
| `WAYLAND_DISPLAY=wayland-1 cargo run`; Start Sharing; select Display and approve Share through the real Portal chooser | Selected Screen, opened the PipeWire source, and produced the first encoded frame at 46 ms and first fMP4 fragment at 171 ms | Single startup sample, with no Viewer connected; these are startup timings, not end-to-end latency |
| Stop Sharing, then SIGINT to the test process; inspect `pw-dump` and `ss -ltnp` | GUI returned to Ready with no active media; test process, port `8877` listener, and Aercast-named PipeWire nodes were absent after cleanup; desktop lock restored | Normal stop and process cleanup only |

Audio initialization logged `capture node did not retain node.passive=in` and `waiting for exactly one FL and one FR port`. This run did not establish whether those diagnostics were transient, and did not execute the allowed/Communication stereo-signal graph checks. Selective-audio correctness, real Viewer wake, same-link restart, failure rollback, desktop/iPhone playback, long-duration stability, and release-package installation remain unverified at this revision

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
- Clean installation and new-version launch from AUR; installation and launch from `.deb` or a prebuilt release asset
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
