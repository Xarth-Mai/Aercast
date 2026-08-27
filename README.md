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
procedure yet. Its Phase 5 niri baseline completed end-to-end qualification:
Portal capture, selective audio, same-link recovery, desktop and tray lifecycle,
settings, Viewer management, and one-encoder three-Viewer playback were
exercised in Zen Browser first and Chromium second. The latest interface polish
and PipeWire graph-race fix have automated coverage but have not repeated that
real Host and browser check.
[See the evidence and current gap.](docs/verification.md)

GNOME, KDE, packaged installation, hardware encoding, 1080p60, and trusted-LAN
latency targets are not yet verified.

## Quick start

After installing a development build:

1. Open Aercast, select **Start Sharing**, then approve a screen or window in
   the system Portal.
2. Send the displayed link to a trusted Viewer. **Stop** ends capture while
   keeping that link ready for a later share.

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
