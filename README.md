# Aercast

**Share your Linux screen without turning it into a meeting.**

Aercast is a Linux/Wayland-first screen-sharing app for a small group of
viewers. Pick one screen or window, exclude applications from Viewer audio,
and share a direct browser link. There are no accounts, meeting rooms, cloud
media relays, or viewer installs.

> **Availability:** Aercast is pre-alpha and does not have a packaged release
> yet. The description below is the launch product contract, not a claim that
> every capability is already implemented or verified.

## Why Aercast

- **Your desktop stays in control.** Screen and window selection use the
  native Wayland Portal permission dialog for every share.
- **Share system audio selectively.** Exclude applications such as private
  voice chat while keeping their sound unchanged for the Host.
- **View in a browser.** Viewers open one temporary link in a supported desktop
  browser; no Aercast client is required.
- **Encode once, serve everyone.** A single media pipeline supplies every
  Viewer, keeping Host work independent of the number of viewers.
- **Direct by design.** Media travels from the Host to Viewers over HTTP. No
  Aercast cloud or relay sits in the path.

**One host. One ephemeral session. One media stream. No cloud.**

## How a share works

1. The Host opens Aercast and receives a temporary private link. Capture has
   not started.
2. **Start Sharing** opens the system picker. The Host approves one screen or
   window.
3. Viewers open the link in a browser and select **Play / Enable Audio**.
4. **End Sharing** stops capture, closes the system permission session, and
   permanently invalidates that link.

The launch audio policy is straightforward: viewers hear normal PipeWire
system audio except applications on the Host's exclusion list. Exclusions do
not reroute, mute, or otherwise change the Host's local audio.

## Launch scope

The Host application targets native, unsandboxed Linux on Wayland, initially
GNOME, KDE Plasma, and niri. Flatpak is not in the initial launch scope.

Current stable desktop Firefox is the primary Viewer target. Current stable
desktop Chromium is a first-class secondary target and must pass the same
playback checks, with Firefox validated first. Browser playback requires
working operating-system H.264 and AAC support. Safari, mobile browsers,
distribution builds missing those codecs, Windows Hosts, and macOS Hosts are
not launch compatibility promises.

## Privacy and networking

The share link is an unguessable, session-scoped bearer credential. Aercast
does not persist Portal permission, capture before explicit approval, load
third-party resources in the Viewer page, or keep an old link valid after the
Host ends a share.

Plain HTTP is suitable only for a trusted LAN. Public or otherwise untrusted
access must use HTTPS supplied by infrastructure the Host controls. Aercast
does not provide domains, certificates, reverse proxies, port forwarding,
tunnels, or NAT traversal.

## What Aercast is not

Aercast is not a conferencing or remote-access platform. It does not provide
WebRTC signaling, cloud relays, accounts, chat, calls, cameras, recording,
remote control, clipboard synchronization, or file transfer.

## License

Aercast is licensed under the [Mozilla Public License 2.0](LICENSE).
