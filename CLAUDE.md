# CLAUDE.md

Guidance for Claude Code when working in this repository.

## What this is

worldprobe — the read-only desktop inspector for Open World Format
`.world` packages (fold, validate, history/branches, diff, state,
physics). egui frontend over the `openworldformat` crate from crates.io
(`0.5.1`; a local `[patch.crates-io]` entry pointing at
`../openworldformat/rust/` builds against the sibling checkout when
wanted, and is never committed: CI has no sibling).

## Rules

- Read-only: worldprobe never writes into a package, and grows no
  viewport and no editor. Drawing a world is a viewer's job, changing
  one an authority's (LocalGPT's app, `localgpt world`); requests for
  either belong in those projects.
- No format semantics in this repo: parsing, folding, validation,
  history and physics belong to the reference crates. worldprobe loads
  directories and draws what the reference says. If a format behavior
  is missing, fix it upstream, not here.
- The package and diff modules are included by the integration test via
  `#[path = "../src/…"]`, the diff module as `diff`, because the package
  module reads its head check through `crate::diff`. If either grows a
  dependency on egui, extract them to a lib target instead.
- egui moves fast: 0.36 renamed `App::update` → `App::ui`, unified the
  panel types (`egui::Panel::top/left`), and made dropped files trait
  objects. Check the vendored source in `~/.cargo/registry` before
  fighting the compiler from memory.
- Conventional commits, no Co-Authored-By trailers. `cargo clippy`,
  `cargo fmt`, and `cargo test` green before reporting completion.
- The sibling-package tests skip when `../songworld` or
  `../openworldformat` are absent — that is by design, not a bug.
- CI: `ci.yml` runs fmt and clippy (Linux) and `cargo test --locked` on
  Linux, macOS and Windows; `release.yml` drafts a GitHub release with
  five targets' binaries on a `vX.Y.Z` tag that matches `Cargo.toml`.
- License: Apache-2.0.
