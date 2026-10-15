# worldprobe

**The desktop inspector for `.world` packages** — worldgit with a
viewport coming. Drop a package folder on the window and see, through
the format's own Rust reference:

| Tab | What it shows |
|---|---|
| **Overview** | schema version vs the reference, base → head entity counts, log stats, torn lines, `package.json` metadata, validation verdict |
| **Entities** | the folded document: id, name, shape, component flags (M/L/B/T/~/A), extension fields |
| **Log** | every entry with its ops classified — edits, tool calls, visitor input, state deltas, clocks, merges, extension ops |
| **Branches** | the history as a tree: tips marked, refs annotated, click a tip to fold to it |
| **Diff** | the world at any two points of the history — the base, any entry, or `manifest.json` as written — compared as JSON: entities added, removed, changed (which fields, old → new), world fields changed |
| **State** | declared fields with types, initials and folded values; undeclared keys carried and named |
| **Physics** | declared bodies (`ext-physics`), the 5 s simulation — contacts, bounces, resting |

The app adds **no format semantics of its own**: the fold, validation,
branching and physics all come from the format's own Rust reference —
the `openworldformat` crate, extracted into the spec repo's `rust/`
(this app's arrival was the extraction's trigger). A path dependency
until the crate publishes to crates.io; a version range after.

## Build

```bash
cargo run    # macOS/Linux/Windows; egui needs no system deps beyond the usual
```

Drop a `.world` directory on the window (or type its path). Try the
sibling repositories' packages: `../songworld/world`,
`../openworldformat/examples/speedrun-fork` — the one with two tips.

## Tests

`cargo test` — when the sibling checkouts (`../songworld`,
`../openworldformat`) are present, the inspector opens their packages
and proves the fold, the history and the state read; CI without
siblings skips, because those packages test themselves in their own
repositories.

## Roadmap

- **Viewport** — a Bevy pane rendering the folded document (the
  ecosystem's `world-bevy` is the prior art).
- **Editor** — append-ops editing: spawn/modify/delete write to
  `ops.jsonl`, undo appends inverses, save compacts. The format's
  native editing story, demonstrated. Comes last on purpose.

## License

Apache-2.0.
