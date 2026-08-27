# Verification record

This document owns the latest real evidence and current blocker behind Aercast
compatibility, performance, and completion claims; it does not own product
plans, future targets, visual rules, or append-only raw logs.

## Recorded niri baseline

Unless a record says otherwise, the evidence below was collected in this
environment:

| Field | Value |
| --- | --- |
| Revision | `ffc70b5` (`feat: complete multi-viewer host lifecycle`) |
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

This baseline proves only the recorded scenarios. It does not qualify current
HEAD or establish support on another compositor, GPU, encoder, distribution, or
browser build.

## Current unqualified changes

The post-Phase 5 Host layout, tray and favicon scaling, Viewer-control polish,
and PipeWire missing-resource recovery have only automated checks. At the
user's request, no real GUI, Portal, PipeWire graph, Zen, or Chromium acceptance
was run for these changes. The Phase 5 record below remains the latest valid
real evidence and does not qualify them.

## Current idle smoke

**Revision:** `478f129`.

**Scenario:** the GUI was launched on the recorded niri host and left in its
initial waiting state without pressing **Start Sharing**.

**Host terminal:**

```sh
cargo run
```

**Inspection terminal:**

```sh
AERCAST_PID=3084642
AERCAST_PORT=38677
niri msg windows
pw-dump | jq --arg pid "$AERCAST_PID" \
  '[.[] | select(.info.props["application.process.id"] == $pid)] | length'
ss -ltnp | rg aercast
curl -o /dev/null -w '%{http_code}\n' http://127.0.0.1:$AERCAST_PORT/s/invalid
curl -o /dev/null -w '%{http_code}\n' http://127.0.0.1:$AERCAST_PORT/s/invalid/stream
```

**Result:** Aercast mapped one waiting window and listened only on
`127.0.0.1`; no Portal chooser appeared, and the PipeWire query returned zero
objects exposing the Aercast process ID. Both invalid-token routes returned
`404`.

This is evidence only for idle startup, loopback binding, and invalid-token
handling on this revision. It does not prove capture, selective audio, browser
playback, or recovery.

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
| --- | ---: | ---: |
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
| --- | --- | ---: | ---: |
| Zen `1.21.15b` | one Viewer | 2.202 s | 3 |
| Zen `1.21.15b` | three concurrent Viewers | 2.288–2.306 s each | at least 3 |
| Chromium `151.0.7922.173` | one Viewer | 2.245 s | 4 |
| Chromium `151.0.7922.173` | three concurrent Viewers | 2.298–2.299 s each | 4 |

Three established media sockets belonged to one Aercast process and Host UI
count reached three. An intentionally one-byte-per-second response was dropped
while all three Chromium responses stayed established and advanced. This is
baseline evidence for one-encoder fan-out and stalled-reader isolation, not
current-HEAD or Phase 5 product acceptance.

The fMP4 late-join parser was also checked against real installed `mp4mux`
output. It reconstructed `ftyp` + `moov`, then complete `moof` + `mdat` pairs,
identified the negotiated video track when audio preceded it, and began replay
at an independently confirmed video keyframe. Appsink buffer boundaries were
not assumed to be MP4 box boundaries.

## Selective audio

**Current policy run:** 2026-08-27 on the recorded PipeWire `1.6.8` niri host.
The user override
`90-aercast-passive-links.conf` was removed, the user media services were
restarted, and this command returned an empty object:

```sh
pw-config -n pipewire.conf merge module.link-factory.args
```

Two real nonzero stereo signals were then fed into PipeWire: a 440 Hz allowed
Music stream named `aercast-game-test`, and an 880 Hz Communication stream named
`aercast-discord-test`. Aercast was started with `cargo run`, and a display was
approved through the real ScreenCast Portal. It reported:

```text
Selective audio active: 1 playback stream(s), 2 verified passive links.
```

`pw-cli info` and `pw-dump` read back `node.passive=in` on
`aercast-selective-audio`. The two Aercast-created links contained only exact
endpoints, `object.linger=false`, and the server-added async/object properties;
neither contained client-supplied `link.passive`. Both were active and carried
real data. The allowed stream retained its two original active sink links with
the same IDs, serials, and endpoints. The Communication stream retained its two
active sink links and had no link to Aercast.

Exiting the allowed stream removed its node and both Aercast links; the Aercast
capture node became `idle`, while the Communication sink links stayed active.
Restarting the 440 Hz stream with the same `application.id` created a new object
serial and Aercast automatically returned to one stream and two exact links.
The final tray Stop returned to `Status: Ready`, printed only `Ending share.`,
and normal Quit plus test-source cleanup left no Aercast or test graph objects.

This current run proves real graph policy, PID-independent identity rematching,
Communication exclusion, inherited passive scheduling, preservation of the
Host's local route, stopped-source idleness, automatic replay recovery, and
clean Stop without daemon configuration. The sink was still muted and no new
browser analyser was recorded, so this is not a human-listening measurement.
The latest signal-level browser baseline remains the recorded Phase 3 run: an
allowed 440 Hz source survived capture and AAC playback while an excluded
880 Hz source measured about 92.7 dB lower in the browser analyser.

## Phase 4 lifecycle

**Revision:** `e54f1b2`, 2026-08-27.

**Scenario:** real niri Portal monitor capture was exercised in Zen first and
Chromium second. Each browser used one existing page through waiting, playback,
media replacement, Stop, and later Start. The Host used an isolated settings
directory with system audio off for browser lifecycle isolation:

```sh
env XDG_CONFIG_HOME=/tmp/aercast-real-smoke \
  NIRI_SOCKET=/run/user/1000/niri.wayland-1.1409.sock \
  WAYLAND_DISPLAY=wayland-1 DISPLAY=:0 cargo run
zen-browser --new-window about:blank
chromium --user-data-dir=/tmp/aercast-chromium-profile \
  --ozone-platform=wayland --new-window about:blank
pw-dump | jq '.[] | select(.info.props["node.name"] == "aercast")'
pw-cli destroy AERCAST_INPUT_NODE_ID
curl -H "Aercast-Viewer-ID: $VIEWER_ID" \
  -o /dev/null -w '%{http_code}\n' "$SHARE_URL/stream"
```

**Zen result:** an induced destruction of only Aercast's video input preserved
Portal source node 92. The same Portal session and page recovered exactly three
times. Recovery first-frame timings were 17, 24, and 21 ms; complete fragment
timings were 158, 159, and 159 ms. Destroying the fourth replacement input
entered the explicit terminal media error and did not log or create a fourth
recovery. A separate uninterrupted run used Portal node 97 for initial playback
(24 ms first frame, 160 ms first fragment), Stop returned the same page to
waiting, and a later Portal authorization on node 90 resumed playback in that
unchanged page (25 ms and 161 ms). No reload or navigation occurred between
those states.

**Chromium result:** Portal source node 92 produced initial playback at 23 ms
and 159 ms. Destroying Aercast input node 82 logged recovery 1/3, kept Portal
node 92 alive, created replacement input node 99, and resumed the same page at
21 ms and 159 ms. Stop returned that page to waiting. A request carrying the
same 64-hex token and existing 32-hex Viewer identity returned `425`; a later
Portal authorization on node 88 resumed the unchanged page at 20 ms and
159 ms. Zen's same-token Stop boundary was independently checked with the same
identity-aware request and also returned `425`.

Normal final Stop and process exit removed the Portal source, Aercast media and
audio nodes, HTTP listener, Host, and browser test processes. These are local
component startup timings and lifecycle observations, not display-to-display
or LAN latency measurements.

## Phase 5 desktop productization

**Revision:** `073169b`, 2026-08-27.

**Desktop shell:** niri reported one fixed `700×440` logical window with
`is_floating=true`. Closing it removed every mapped Aercast window while the
same PID, loopback listener, owned `org.aercast.Aercast` name, and
StatusNotifierItem remained. Calling the standard tray `Activate` method
restored one floating window for the same PID. A second `cargo run` exited after
activating that existing instance. The dynamic tray menu changed from
`Status: Ready / Show / Start / Quit` to
`Status: Sharing / Show / Copy Link / Stop / Quit`.

Starting from the tray while the Host was hidden opened the real Portal and
began sharing at 24 ms to the first encoded frame and 161 ms to the first
fragment. The notification call contained only `Screen sharing started` and
`Aercast is now sharing.`; it contained no URL or token. The current Appearance
Portal values were dark color scheme 1, normal contrast, normal motion, and
accent sRGB `(0.902, 0.176, 0.259)`, matching the rendered dark UI and accent
focus/selection treatment. Unit checks cover the derived WCAG contrast,
high-contrast/reduced-motion branches, keyboard focus wrap and scroll reveal,
and notification visibility boundaries.

**Settings:** real GUI interaction saved `1080p60 / 12 Mbps`, read it back from
the replace-written JSON, and restored `720p60 / 6 Mbps`. The detected encoder
choices were Auto, VA-API hardware, and Software (x264). System audio, the three
default exclusion rules, active-application refresh, loopback address, port,
optional share base URL, stopped-only warning, and notifications were all
present. Notifications were toggled off, read back, and restored on. During an
active share, changing the port draft to 8878 left **Apply Network** disabled
and the listener on 8877. While stopped, the same control rebound the real
listener from 8877 to 8878 and back to 8877, with the persisted value following
each successful bind. A process restart retained 720p60, audio off,
notifications on, and loopback 8877. Unit checks cover invalid quality/network
input, failed replace preservation, audio apply-to-current-share restart, and
the 100-record Viewer bound.

**Viewer workflow:** Zen first reached `3/3` online Viewers with one Aercast
process and one Portal video input; the visible row reported 2 ms RTT and
1019 ms playback lag. Host Disconnect moved it to `2/3`; the affected page
stopped automatic reconnect, displayed `Disconnected by the Host.` and a Retry
button, then manual Retry returned the same history to `3/3`. Refresh Link with
three online Viewers required confirmation, cleared history, kept the same
Portal video input and encoder pipeline, and made a GET from an old Viewer URL
return `404`.

Chromium then reached `3/3` on the refreshed link; the visible row reported
2 ms RTT and 576 ms lag. A fourth identity completed a real media request and
then stopped reading. The bounded fan-out removed it within ten seconds, so the
Host showed `3/4` while all three Chromium Viewers remained online and the graph
still contained exactly one Aercast video input. Active-share tray Quit opened
an in-window confirmation. Cancel preserved the original PID and listener;
confirmed Quit printed `Stopping Aercast.` and removed the window, Portal,
media graph, listener, StatusNotifierItem, owned bus name, and process.

The canonical checks passed on the accepted tree: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, 37 tests passed with five explicit
environment-dependent ignores, and `git diff --check` passed. These are local
functional and component observations, not a LAN latency or long-duration load
claim.

There is no current evidence for a packaged install, GNOME/KDE support, real
stable Firefox rather than the Zen-family vehicle, official Google Chrome,
mobile browsers, hardware encoding, 1080p60, 1440p60, 120 FPS, trusted-LAN
end-to-end latency, or public-network deployment.
