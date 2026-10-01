# CLAUDE.md

Guidance for Claude Code when working in this repository.

## What this is

worldprobe — the desktop inspector for Open World Format `.world`
packages (fold, validate, history/branches, state, physics). egui
frontend over the format's Rust reference crates, consumed by path from
`../localgpt/crates/{world-types,world-sync,world-physics}` until they
extract into `../openworldformat/rust/` (this app is the trigger for
that extraction — see openworldformat/rust/README.md).

## Rules

- No format semantics in this repo: parsing, folding, validation,
  history and physics belong to the reference crates. worldprobe loads
  directories and draws what the reference says. If a format behavior
  is missing, fix it upstream, not here.
- The package module is included by the integration test via
  `#[path = "../src/package.rs"]`; if it grows a dependency on egui,
  extract it to a lib target instead.
- egui moves fast: 0.36 renamed `App::update` → `App::ui`, unified the
  panel types (`egui::Panel::top/left`), and made dropped files trait
  objects. Check the vendored source in `~/.cargo/registry` before
  fighting the compiler from memory.
- Conventional commits, no Co-Authored-By trailers. `cargo clippy`,
  `cargo fmt`, and `cargo test` green before reporting completion.
- The sibling-package tests skip when `../songworld` or
  `../openworldformat` are absent — that is by design, not a bug.
- License: Apache-2.0.
