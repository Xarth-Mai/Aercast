# AGENTS.md

These instructions apply to the entire repository. Read [README.md](README.md),
[docs/development.md](docs/development.md),
[docs/verification.md](docs/verification.md), and
[docs/ui-design.md](docs/ui-design.md) before changing their respective areas.

## Change routing

Aercast maintains exactly four formal project documents. Do not add an
architecture, roadmap, configuration, or documentation-index file.

| Change | Authoritative document |
|---|---|
| Project homepage: what Aercast is, why it exists, current usability, short data flow, support summary, and document links | `README.md` |
| Product commitment, exact Host/Viewer behavior, settings, security boundary, engineering decision, active Phase, acceptance, risk, non-goal, or next-Phase entry condition | `docs/development.md` |
| Compatibility, performance, or completion evidence from a real check | `docs/verification.md` |
| Visual token, component appearance, typography, spacing, color, motion, icon, or accessibility rule | `docs/ui-design.md` |

One fact has one authoritative home; other documents link to it instead of
copying it. Update the owning document in the same commit whenever code changes
product behavior, an engineering decision, a verification conclusion, or a UI
token. Keep README product-facing and free of governance text, requirement
classification, exact values, acceptance language, Phase checklists, development
commands, implementation research, and test logs. Its job is only to explain
what Aercast is, why someone would use it, whether it is usable today, roughly
how it works, and where details live.

When a Phase completes, compress it in `docs/development.md` to one dated line
with a conclusion and verification anchor; Git history keeps the old checklist.
Keep only the latest valid evidence and current blocker for each claim in
`docs/verification.md`, never an append-only run diary or raw long output.

## How to work

- Implement only the active Phase in `docs/development.md` and its smallest
  runnable vertical slice. Do not add adjacent features while here.
- Trace the real flow and every caller before editing. Fix a shared root cause
  once instead of patching each symptom.
- Prefer the standard library, Linux/Wayland platform capabilities, and current
  dependencies. Add a dependency only when they cannot meet a current need.
- Do not create speculative traits, factories, plugin systems, configuration,
  cross-platform layers, module trees, or unused directories.
- Leave one smallest runnable check for non-trivial branches, loops, parsers,
  and security-sensitive paths. A mock never replaces a required real Portal,
  PipeWire, compositor, browser, or hardware check.
- Base performance and compatibility claims on recorded measurements from the
  relevant compositor, browser, GPU, and encoder. Optimize only a measured
  bottleneck.
- Preserve required validation, cleanup, accessibility, security controls, and
  error handling that prevents data loss.

The canonical static checks are:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git diff --check
```

A real capture smoke starts the GUI with `cargo run`, selects **Start Sharing**,
and requires an interactive Portal source choice. Never simulate Portal consent.
Use Zen as the local Firefox-family vehicle first, then Chromium. Record only
the reproducible command, environment, scenario, result, measurement, and
current failure reason in `docs/verification.md`.

## Before every commit

1. Stage only the intended changes.
2. Run `ponytail-review` against `git diff --cached`.
3. Resolve every valid `delete`, `stdlib`, `native`, `yagni`, and `shrink`
   finding, then restage and repeat until the exact result is
   `Lean already. Ship.`
4. Run `git diff --cached --check` and the smallest relevant build, test, or
   real smoke check.
5. Any tracked-file change after review invalidates the result; repeat the
   review before committing.

Ponytail is a complexity gate, not a substitute for correctness, security,
performance, test, or accessibility review. It must not remove explicitly
required behavior.
