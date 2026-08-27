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
procedure yet. The current source build has completed its niri end-to-end
qualification: Portal capture, selective audio, same-link recovery, desktop
and tray lifecycle, settings, Viewer management, and one-encoder three-Viewer
playback were exercised in Zen Browser first and Chromium second.
[See the evidence.](docs/verification.md)

GNOME, KDE, packaged installation, hardware encoding, 1080p60, and trusted-LAN
latency targets are not yet verified.

## Quick start

After installing a development build:

1. On PipeWire 1.6.8, keep **System Audio** off unless you have manually
   enabled the daemon-wide `allow.link.passive` compatibility option. It
   applies to every permitted client of your user PipeWire daemon, not only
   Aercast. Follow the
   [compatibility note](docs/development.md#pipewire-168-compatibility).
2. Open Aercast, select **Start Sharing**, then approve a screen or window in
   the system Portal.
3. Send the displayed link to a trusted Viewer. **Stop** ends capture while
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
