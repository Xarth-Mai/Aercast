---
name: aercast-real-smoke
description: Run Aercast's real niri Portal, PipeWire selective-audio, and browser smoke checks on the supported host. Use for host acceptance or when changing capture, audio graph, recovery, or Viewer playback behavior; do not use as a mock-only substitute.
---

# Aercast Real Smoke

Read `AGENTS.md`, `docs/development.md`, and `docs/verification.md` first. Test the
current source build on the real host; never replace Portal, PipeWire,
compositor, browser, or signal checks with mocks.

## Selective-audio smoke

1. Confirm the requested PipeWire version and that
   `90-aercast-passive-links.conf` is absent. `pw-config -n pipewire.conf merge
   module.link-factory.args` must not enable `allow.link.passive`. Ask before
   restarting user media services because that interrupts live audio.
2. Start one stereo allowed signal and one stereo `Communication` signal with
   distinct stable identities. Capture their sink-link baseline before starting
   Aercast:

   ```sh
   scripts/verify_audio_graph.sh baseline ALLOWED_NODE EXCLUDED_NODE /tmp/aercast-links.json
   ```
3. Run `cargo run`, invoke **Start Sharing**, and complete the real Portal source
   choice. Do not call the Portal backend directly or fake its response. Follow
   the current `AGENTS.md` rule for any permission-assisted confirmation. If it
   permits Agent confirmation and the user approves elevated input, locate the
   Portal controls by AT-SPI name; use a temporary, user-owned `ydotoold` socket
   only when the Wayland accessibility backend cannot toggle the source card,
   and stop that daemon immediately after authorization.
4. After Aercast reports selective audio active, run:

   ```sh
   scripts/verify_audio_graph.sh active ALLOWED_NODE EXCLUDED_NODE /tmp/aercast-links.json
   ```

   This checks `node.passive=in`, exact active allowed-to-capture links, absence
   of client-created `link.passive`, unchanged sink routes, and Communication
   exclusion.
5. Pause or exit the allowed source and run `verify_audio_graph.sh stopped ...`.
   Resume or recreate it with the same stable identity, then run `active` again
   without the old baseline file. Confirm Aercast logs a new active transition.
6. If browser playback is in scope, use Zen first and Chromium second. Confirm
   the allowed signal reaches playback and the Communication signal does not.
7. Stop Aercast normally and remove every temporary source, file, automation
   daemon, and process. Record only reproducible commands, environment, result,
   measurement, and the current failure in `docs/verification.md`.

Run the graph helper from this skill directory or by its absolute path. Treat a
nonzero exit as a failed smoke; inspect the printed assertion before changing
code or evidence.
