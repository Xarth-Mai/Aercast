# Aercast

> **Pre-alpha:** Aercast currently defines a product direction. There is no
> runnable application yet, and no performance or compatibility claim below
> has been verified.

Aercast is a Linux/Wayland-first screen-sharing app. A host selects a screen or
window through the Wayland portal, then serves live video and system audio
directly over HTTP to viewers in a modern browser.

**One host. One ephemeral session. One capture/encode/mux pipeline. One HTTP
stream. No cloud.**

## Product philosophy

- **Native permission model.** Capture goes through `xdg-desktop-portal` and
  PipeWire, not compositor-specific shortcuts.
- **Mature media tools.** GStreamer should handle capture, encoding, muxing,
  and clocks. Aercast will not rebuild a media pipeline for the sake of being
  pure Rust.
- **Encode once.** Every viewer receives the same encoded stream; adding a
  viewer must not add another encoder.
- **Selective audio is core.** Viewers should hear system audio except excluded
  applications, while the host's local audio remains unchanged.
- **Measure before optimizing.** Hardware encoding, DMA-BUF, fragment sizing,
  and copy avoidance follow real profiling and compatibility results.
- **Ship vertical slices.** Each milestone must run end to end on real Wayland
  systems before the next layer is added.

## Intended flow

```text
Wayland screen or window + application audio
                    |
        xdg-desktop-portal / PipeWire
                    |
      GStreamer capture, encode, and A/V mux
                    |
          Host HTTP fragmented MP4 stream
                    |
             Browser MediaSource <video>
```

On launch, the intended app starts its HTTP server and creates an ephemeral
share URL backed by a cryptographically secure token with at least about 128
bits of entropy. Capture starts only after the host presses **Start Sharing**
and approves a source in the portal picker.

The share URL is a bearer credential, not a substitute for transport security.
Aercast does not provide TLS termination, public hosting, port forwarding,
reverse proxies, tunnels, or NAT traversal. Those remain external deployment
concerns.

## Technical baseline

The initial baseline to validate is:

- Rust for the host application
- `xdg-desktop-portal` and PipeWire for Wayland capture and audio discovery
- GStreamer through `gstreamer-rs` for the media pipeline
- H.264 video and AAC-LC audio in one fragmented MP4 stream
- HTTP delivery to browser MediaSource Extensions
- `iced`, Tokio, and Axum as host UI/runtime/server candidates

These are working assumptions, not implemented or irreversible decisions. A
component may change when an end-to-end test shows a simpler, more compatible,
or more reliable path.

## Targets

These are targets, not current benchmark results:

- Linux and Wayland, with GNOME, KDE Plasma, and niri/wlroots-like environments
  as the first compatibility targets
- 1080p at 60 FPS as the primary operating point
- less than 250 ms end-to-end latency on a LAN
- 1440p60 and 4K60 validation only after the primary path works; 4K60 should
  use hardware encoding in normal operation

## Roadmap

1. Prove portal and PipeWire video capture through a local diagnostic sink.
2. Deliver H.264 fragmented MP4 over HTTP to a localhost browser.
3. Add dynamic application-audio discovery, exclusions, AAC-LC, and a single
   synchronized A/V mux without changing the host's local mix.
4. Add the minimal product shell: ephemeral link, copy action, Start/End,
   viewer states and count, reconnect, and small-scale multi-viewer fan-out.
5. Profile the working path, then add only the hardware encoding and zero-copy
   optimizations justified by measurements.

## Non-goals

Aercast does not plan to provide:

- WebRTC, signaling, ICE, STUN, TURN, SFUs, or NAT traversal
- cloud media relays, accounts, chat, voice calls, or camera sharing
- recording, remote control, clipboard sync, or file transfer
- Windows or macOS support
- speculative cross-platform, encoder, transport, or plugin abstractions

## Development

There is no buildable source tree yet, so there are no installation, build, or
run commands to publish. Contributors and coding agents must follow
[AGENTS.md](AGENTS.md), including its mandatory Ponytail review before every
commit.

## License

Aercast is licensed under the [Mozilla Public License 2.0](LICENSE).
