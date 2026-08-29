# Aercast

**Share your Linux Wayland screen directly with anyone who has a browser.**

Aercast shares one screen or window and its sound directly from your computer.
You can leave out the sound from apps such as Discord or Steam Voice. Viewers
just open a link—no account or app installation needed.

## Why Aercast

- Choose a screen or window with the standard Wayland sharing dialog
- Exclude sound from specific apps without changing what you hear
- Let viewers watch directly in Firefox-family and Chromium browsers
- Share with several viewers without encoding a separate stream for each one
- See who's connected and block a Viewer until you refresh the share link

## Install

Aercast v0.1.2 is an early release for x86-64 Linux.

### Arch Linux

The AUR package remains at v0.1.1; see the
[current release gap](docs/verification.md#current-qualification). Install it
with:

```sh
paru -Syu aercast
```

### Debian and Ubuntu

Download the `.deb` from the [latest release], then install it:

```sh
sudo apt install ./aercast_*.deb
```

### Other Linux distributions

Download the prebuilt `.tar` from the [latest release], extract it, and run the
included program:

```sh
tar -xf aercast-*.tar
./aercast
```

[latest release]: https://github.com/Xarth-Mai/Aercast/releases/latest

## Try it

1. Open Aercast and select **Start Sharing**.
2. Approve one screen or window in the system Portal.
3. Open the displayed link in a browser on the same computer.

Aercast listens only on `127.0.0.1` by default. To share across a trusted LAN,
open **Settings → Network**, use the Host's unicast LAN IP as the listen
address, apply the change, and send the new link.

## Current support

The complete Host and Viewer workflow has been exercised on niri with Zen
Browser first and Chromium second. GNOME, KDE, stable Firefox, Google Chrome,
mobile browsers, hardware encoding, higher-quality presets, and public-network
deployment have not yet received the same qualification.

```text
Wayland Portal -> PipeWire -> GStreamer -> HTTP/fMP4 -> Browser MSE
```

Share links are private bearer credentials. Plain HTTP is limited to a trusted
LAN; Internet-facing use requires an external HTTPS reverse proxy. Aercast does
not provide tunnels, relays, certificates, or NAT traversal.

## Project documents

- [Product behavior and engineering decisions](docs/development.md)
- [Verification evidence and current gaps](docs/verification.md)
- [UI design](docs/ui-design.md)
- [Contributor instructions](AGENTS.md)

## License

[Mozilla Public License 2.0](LICENSE)
