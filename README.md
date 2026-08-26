# Aercast

**Lightweight, direct screen sharing for Linux Wayland.**

Aercast shares one screen or window, with selective system audio, directly to
a small group of browser viewers. It is built for gamers, not meetings.

## Why Aercast

- Native Wayland screen and window selection through the system Portal
- Selective audio that does not change what the Host hears
- Direct playback in Firefox-family and Chromium browsers
- One encoded H.264/AAC stream for every Viewer
- No accounts, Viewer install, or Aercast cloud relay

## Status

Aercast is **pre-alpha**, with no packaged release or supported installation
procedure yet. Earlier niri development runs demonstrated live H.264/AAC
playback and one-encoder multi-Viewer fan-out in Zen Browser and Chromium, plus
selective-audio routing with Zen. The current build has not completed an
end-to-end qualification run. [See the evidence.](docs/verification.md)

GNOME, KDE, packaged installation, hardware encoding, 1080p60, and trusted-LAN
latency targets are not yet verified.

## How it works

```text
Wayland Portal -> PipeWire -> GStreamer -> HTTP/fMP4 -> Browser MSE
```

One native Rust process sends the same synchronized stream directly to every
Viewer. Its compact Host UI starts and stops shares, manages the link, and
shows connected Viewers.

## Security

Share links use private random bearer tokens. Plain HTTP is for trusted LANs;
Internet-facing use requires an external HTTPS reverse proxy. Aercast provides
no tunnels, relays, or NAT traversal.

## Documentation

- [Development direction and product decisions](docs/development.md)
- [Verification evidence](docs/verification.md)
- [UI design](docs/ui-design.md)
- [Contributor instructions](AGENTS.md)

## License

Aercast is licensed under the [Mozilla Public License 2.0](LICENSE).
