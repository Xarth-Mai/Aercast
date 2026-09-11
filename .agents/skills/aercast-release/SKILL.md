---
name: aercast-release
description: Release Aercast through checked version commits, GitHub tags and release assets, AUR package updates, and local package installation. Use when asked to release Aercast, bump and publish a version, or resume an interrupted release; preparation and checks alone do not publish.
---

# Aercast Release

Execute from the repository root. Read `AGENTS.md`, `README.md`, `docs/development.md`, `docs/verification.md`, `.github/workflows/ci-release.yml`, `.gitmodules`, and the current AUR `PKGBUILD` before releasing. They own project rules, product claims, evidence, CI packaging, and package dependencies; this Skill owns orchestration

## Scope and defaults

- A request to execute the full release includes version commits, an annotated tag, GitHub and AUR pushes, the AUR submodule-pointer commit, and installation on this host. Reuse that authorization; request tool permissions when required by the environment
- Creating or editing this Skill does not execute a release. Requests limited to checking or preparation stop at that boundary; explicit version, target, and scope instructions take precedence
- Default to the next patch version of Aercast's current Cargo version. An explicit new version must be stable `X.Y.Z`, greater than the current version, and absent from local and remote release tags. For a resumed release, recover its existing version and completed steps instead of incrementing again
- Default acceptance is automated checks and release builds. Do not launch a GUI, start capture, stop an existing share, or restart media services. Compilation and installation do not establish real Portal, audio, or browser usability

## 1. Establish the release target

Inspect both repositories with `git status`, branch, remote, and tag information. Fetch the relevant branches and tags; use fast-forward-only updates where needed. Release from main-repository `main` and AUR `master`, after confirming the remotes match this project's GitHub and AUR repositories. If another branch is active, stop before mutations and resolve the intended target rather than silently switching branches

Require a clean, understood release tree and a synchronized AUR checkout. Preserve unrelated edits and stop on unresolved dirty state, branch divergence, or conflicting tags; do not stash, reset, force-push, or include unrelated work automatically. A main branch ahead of its remote is valid when those commits are the intended release content

`aur/aercast` is a Git submodule with a separate history. Initialize it through `.gitmodules` if missing, and verify its checkout and writable remote before editing. Do not stage its package files as ordinary main-repository files

Use IPv4 for AUR Git-over-SSH operations on this host, including fetch, push, dry-run, and remote verification: `git -c core.sshCommand='ssh -4' -C aur/aercast <operation>`. Keep normal SSH host-key verification enabled; do not replace trusted keys or disable checking to work around an IPv6 endpoint mismatch. Apply this per command without changing global SSH configuration

Check availability of Rust/rustfmt/Clippy, Bun, GitHub CLI authentication, `makepkg`, `updpkgsums`, `pacman`, and AUR push access before beginning publication. Use the versions and dependencies required by the checked-out project rather than copying old versions into this Skill

## 2. Prepare and verify the version

Update Aercast's version in `Cargo.toml` and its own package entry in `Cargo.lock`; leave dependency versions unchanged. Update current-release statements in README and the development contract, preserving historical evidence and existing qualification limits

Run these checks on the final versioned content:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
bun test tests/viewer-recovery.test.js
cargo build --locked --release
git diff --check
```

Any failure blocks publication. Diagnose environmental failures and obtain the needed tool permission before rerunning; do not skip failing checks. Record ignored tests and the scope of the result. Changes after validation require the affected checks again; use `docs/verification.md` for current evidence without adding a run diary or claiming unperformed real checks

Stage only the release changes. Follow the repository's `ponytail-review` and staged validation gate, then commit as `chore(release): prepare vX.Y.Z`. Verify that the commit contains the tested version and the working tree is clean

## 3. Publish GitHub

Create an annotated `vX.Y.Z` tag on the verified release commit. Push only `main` and this tag atomically, substituting the actual version:

```sh
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push --atomic origin main refs/tags/vX.Y.Z
```

Verify the remote branch and peeled tag commit. Locate the Actions run for this tag's push and exact commit, not merely the newest main-branch run. Wait for its checks and release job to succeed, then verify the GitHub Release contains `aercast-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` and `aercast_X.Y.Z_amd64.deb`, or the corresponding names defined by the current workflow

Do not publish AUR until GitHub release completion is established. A main-branch CI success alone is insufficient. Report a failed or still-running release accurately; do not manually create a competing release or replace an existing tag to bypass the workflow

## 4. Build and publish AUR

In `aur/aercast`, set `pkgver` to the released version and reset `pkgrel=1`. Retain the current source-tag URL and package dependencies unless the release explicitly requires changes

Run `updpkgsums` against the published tag archive, inspect the resulting source and SHA-256, generate `.SRCINFO` with `makepkg --printsrcinfo > .SRCINFO`, and run `makepkg --verifysource`. Never use `SKIP` or the old release's checksum

Build with `makepkg --cleanbuild --syncdeps`, as the ordinary user, obtaining permission for dependency installation if needed. Ensure the package's `check()` runs; do not use `--nocheck` or reuse a pre-existing package as proof of this build. If output already exists, distinguish it from this attempt and use a deliberate rebuild instead of treating the existing-file exit as success

Identify the exact newly built package via `makepkg --packagelist`; inspect its package metadata and contents to confirm name, version, architecture, executable, desktop entry, and icon. Do not select a package by an ambiguous glob

After build and tests pass, review and commit only `PKGBUILD` and `.SRCINFO` as `chore(release): bump AUR package to X.Y.Z`, following the repository's commit gate. Push AUR `master` and verify its remote commit

Return to the main repository, stage only the new `aur/aercast` gitlink, review and commit as `chore(aur): track aercast X.Y.Z`, then push `main`. Keep the release tag on the original release commit; this follow-up records the package publication without retagging. Observe the follow-up CI result separately from the tag release run

## 5. Install and report

Install the exact package built and inspected above with `sudo pacman -U <package-path>`. Retain package-manager prompts and obtain installation permission through the available tool flow. Do not use an AUR helper's cached package as a substitute for the validated artifact

Verify `pacman -Q aercast` matches the intended `X.Y.Z-1` and use package ownership/file checks to confirm `/usr/bin/aercast`, the desktop entry, and icon are installed. Installation does not replace an already-running process and does not require launching the program

Report the version, release commit and tag, tag Actions run and Release URL, AUR commit, main-repository gitlink commit and CI result, installed package version, actual checks, ignored tests, and any unfinished step. Retain the verified package for installation retry

## Resume and failure handling

Stop downstream publication on failure and report the last completed stage. Before retrying, inspect remote state and existing artifacts; a failed connection may have completed its push

For an existing tag, verify its peeled commit and version before resuming Actions or AUR work. Reuse a matching release instead of creating another version. A conflicting tag or immutable published source requires an explicit resolution; never move or overwrite it

If AUR publication succeeds but installation fails, report publication and installation separately and retry installation of the same verified package. If the gitlink push or its CI fails, retain and report that outstanding step rather than claiming the full workflow completed
