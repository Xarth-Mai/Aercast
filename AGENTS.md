# AGENTS.md

These instructions apply to the entire repository. Read [README.md](README.md)
before changing code; it is the source of truth for the product direction,
current milestone, and non-goals.

## Product invariants

- Build for Linux and Wayland first. Screen and window capture must use the
  `xdg-desktop-portal` security model.
- The host serves media directly over HTTP. Do not add WebRTC, signaling, ICE,
  STUN, TURN, an SFU, a cloud relay, or NAT traversal.
- Capture, encode, and mux once, then fan out the same media to every viewer.
- Do not capture before the host explicitly starts sharing and approves a
  portal source. Share tokens must come from a cryptographically secure RNG and
  provide at least about 128 bits of entropy.
- Put video and audio in one synchronized stream. Application-audio exclusions
  must survive stream restarts and must not change what the host hears locally;
  do not identify applications by PID alone.
- Use PipeWire and GStreamer rather than rebuilding mature media behavior in
  Rust. Public reachability and TLS termination remain external concerns.

## How to work

- Implement only the current roadmap milestone and its smallest runnable
  vertical slice. Do not add adjacent features "while here."
- Understand the real flow before editing. For a bug, find every caller and fix
  the shared root cause once.
- Prefer the standard library, native platform features, and existing
  dependencies. Add a dependency only when they cannot meet a current need.
- Do not create traits, factories, plugin systems, configuration, module trees,
  or cross-platform layers for a second implementation that does not exist.
  Conceptual boundaries do not require directories until the code needs them.
- Leave one smallest runnable check for non-trivial branches, loops, parsers,
  and security-sensitive paths. Each milestone must pass a real build/test or
  smoke check; a mock alone is not completion.
- Back performance and compatibility claims with recorded measurements on the
  relevant compositor, browser, and hardware. Optimize only a measured
  bottleneck.
- Keep public documentation honest: distinguish implemented behavior, verified
  results, working assumptions, and targets.

No build or test commands exist yet. Add the smallest canonical commands when
they do; never invent placeholders.

## Before every commit

1. Stage only the intended changes.
2. Run `ponytail-review` against `git diff --cached`.
3. Resolve every valid `delete`, `stdlib`, `native`, `yagni`, and `shrink`
   finding, then restage and repeat until the result is `Lean already. Ship.`
4. Run `git diff --cached --check` and the smallest relevant build, test, or
   real smoke check.
5. Any tracked-file change after review invalidates the review; repeat it
   before committing.

Ponytail review is a complexity pass, not a replacement for correctness,
security, performance, or test review. It must never remove required input
validation, error handling that prevents data loss, security controls,
accessibility, or explicitly requested behavior.
