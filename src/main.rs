//! worldprobe — the read-only inspector for `.world` packages.
//!
//! Fold, validate, walk the history and its branches, compare any two
//! points of it, watch the state trajectory, run the physics. The format
//! semantics are the Rust reference's (the `openworldformat` crate); this
//! app only points them at a directory and shows what they say, and
//! never writes.

mod diff;
mod package;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use diff::{Change, FieldDiff, WorldDiff};
use eframe::egui;
use openworldformat as wt;
use package::Package;

const USAGE: &str = "\
usage: worldprobe [PACKAGE]

The read-only inspector for .world packages. PACKAGE is a package folder
to open at start; without one, drop a folder on the window or type its
path. An open package is read again whenever its files change.";

/// What refuses or disagrees, and what only warns.
const ERROR: egui::Color32 = egui::Color32::from_rgb(230, 120, 100);
const WARNING: egui::Color32 = egui::Color32::from_rgb(220, 180, 100);

fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let package = args.next();
    match package.as_ref().and_then(|arg| arg.to_str()) {
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return Ok(());
        }
        Some("-V" | "--version") => {
            println!("worldprobe {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ if args.next().is_some() => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        _ => {}
    }
    let package = package.map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_title("worldprobe — the .world inspector"),
        ..Default::default()
    };
    eframe::run_native(
        "worldprobe",
        options,
        Box::new(move |_cc| {
            let mut probe = WorldProbe::default();
            if let Some(dir) = package {
                probe.open(dir.display().to_string());
            }
            Ok(Box::new(probe))
        }),
    )
}

#[derive(Default)]
struct WorldProbe {
    path: String,
    package: Option<Package>,
    error: Option<String>,
    tab: Tab,
    selected_entry: Option<usize>,
    selected_tip: Option<String>,
    diff: DiffView,
    watch: Option<Watch>,
}

/// How often an open package's files are looked at for a change.
const WATCH_EVERY: Duration = Duration::from_millis(500);

/// The files an authority writes, and the declaration beside them: a
/// change to the package is a change to one of these.
const WATCHED: [&str; 5] = [
    "ops.jsonl",
    "package.json",
    "manifest.json",
    wt::package::BASE_SNAPSHOT,
    "state.json",
];

/// The watched files as they stand on disk: each one's length and
/// modification time, or `None` while it isn't there.
type Stamp = Vec<Option<(u64, SystemTime)>>;

fn stamp(dir: &Path) -> Stamp {
    WATCHED
        .iter()
        .map(|name| {
            let meta = std::fs::metadata(dir.join(name)).ok()?;
            Some((
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ))
        })
        .collect()
}

/// Watching the open package: its files as they stood when it was read,
/// and a change seen but not yet settled.
struct Watch {
    stamp: Stamp,
    checked: Instant,
    settling: Option<Stamp>,
}

impl Watch {
    fn from(stamp: Stamp) -> Self {
        Self {
            stamp,
            checked: Instant::now(),
            settling: None,
        }
    }

    /// Whether the files as they stand now call for a fresh read. A change
    /// is read once it holds still for one more look, so an authority's
    /// writes — the entry, then `package.json`, then the manifest — are
    /// read together rather than one at a time.
    fn settled(&mut self, now: Stamp) -> bool {
        if now == self.stamp {
            self.settling = None;
            false
        } else if self.settling.as_ref() == Some(&now) {
            true
        } else {
            self.settling = Some(now);
            false
        }
    }
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Tab {
    #[default]
    Overview,
    Entities,
    Log,
    Branches,
    Diff,
    State,
    Physics,
}

/// A point of the history the Diff tab compares: the base, the fold to
/// an entry, or `manifest.json` as written (the head).
#[derive(Default, Clone, PartialEq, Debug)]
enum DiffPoint {
    #[default]
    Base,
    Entry(String),
    Manifest,
}

/// The Diff tab's state: the two points, the selectors' rows, and the
/// diff for the pair it was last computed for — both sides fold once
/// per selection, not once per frame.
#[derive(Default)]
struct DiffView {
    from: DiffPoint,
    to: DiffPoint,
    points: Vec<(DiffPoint, String)>,
    computed: Option<(DiffPoint, DiffPoint, Result<WorldDiff, String>)>,
}

impl DiffView {
    /// The selectors for a package, set to the last change: the head
    /// entry against its parent (the base when it has none).
    fn for_package(package: &Package) -> Self {
        let mut points = vec![(DiffPoint::Base, "base".to_string())];
        for (n, entry) in package.entries.iter().enumerate() {
            let id = package.entry_id(n);
            let mark = if package.tips.contains(&id) {
                "●"
            } else {
                " "
            };
            let summary = Package::op_summary(entry).join(" · ");
            let label = format!("{mark} {id} · rev {} · {summary}", entry.revision);
            points.push((DiffPoint::Entry(id), label));
        }
        points.push((
            DiffPoint::Manifest,
            "manifest.json (the head as written)".to_string(),
        ));
        let head = package.head.clone();
        // When manifest.json isn't the fold to the head, that is the
        // difference worth opening on; otherwise the last change.
        let drifted = package.head_drift.as_ref().is_some_and(|d| !d.is_empty());
        let (from, to) = if drifted {
            (
                head.map(DiffPoint::Entry).unwrap_or_default(),
                DiffPoint::Manifest,
            )
        } else {
            (
                head.as_deref()
                    .and_then(|head| package.parent_of(head))
                    .map(DiffPoint::Entry)
                    .unwrap_or_default(),
                head.map(DiffPoint::Entry).unwrap_or(DiffPoint::Manifest),
            )
        };
        Self {
            from,
            to,
            points,
            computed: None,
        }
    }
}

impl eframe::App for WorldProbe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Drag a .world folder onto the window to open it.
        for file in ui.ctx().input(|i| i.raw.dropped_files.clone()) {
            let path = file.path();
            self.open(path.display().to_string());
        }
        self.watch_for_changes(ui.ctx());
        egui::Panel::top("bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Package");
                let response = ui.text_edit_singleline(&mut self.path);
                if (ui.button("Open").clicked() || response.lost_focus())
                    && !self.path.trim().is_empty()
                {
                    self.open(self.path.trim().to_string());
                }
                if let Some(package) = &self.package {
                    ui.separator();
                    ui.monospace(package.dir.display().to_string());
                    ui.weak("· read again when its files change");
                }
            });
        });
        if let Some(error) = &self.error {
            egui::Panel::top("error").show(ui, |ui| {
                ui.colored_label(ERROR, format!("⚠ {error}"));
            });
        }
        egui::CentralPanel::default_margins().show(ui, |ui| {
            let Some(package) = &self.package else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        "Drop a .world package folder here — or type its path above.\n\
                         The fold, the validation, the history and the state all come\n\
                         from the format's Rust reference; this app just shows them.",
                    )
                });
                return;
            };
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Overview, "Overview");
                ui.selectable_value(&mut self.tab, Tab::Entities, "Entities");
                ui.selectable_value(&mut self.tab, Tab::Log, "Log");
                ui.selectable_value(&mut self.tab, Tab::Branches, "Branches");
                ui.selectable_value(&mut self.tab, Tab::Diff, "Diff");
                ui.selectable_value(&mut self.tab, Tab::State, "State");
                ui.selectable_value(&mut self.tab, Tab::Physics, "Physics");
            });
            ui.separator();
            match self.tab {
                Tab::Overview => Self::overview(ui, package),
                Tab::Entities => Self::entities(ui, package),
                Tab::Log => Self::log(ui, package, &mut self.selected_entry),
                Tab::Branches => Self::branches(ui, package, &mut self.selected_tip),
                Tab::Diff => Self::diff(ui, package, &mut self.diff),
                Tab::State => Self::state(ui, package),
                Tab::Physics => Self::physics(ui, package),
            }
        });
    }
}

impl WorldProbe {
    fn open(&mut self, path: String) {
        self.path = path.clone();
        self.selected_entry = None;
        self.selected_tip = None;
        self.diff = DiffView::default();
        self.watch = None;
        let dir = Path::new(&path);
        let before = stamp(dir);
        match package::load(dir) {
            Ok(package) => {
                self.error = None;
                self.diff = DiffView::for_package(&package);
                self.watch = Some(Watch::from(before));
                self.package = Some(package);
            }
            Err(error) => {
                self.error = Some(error);
                self.package = None;
            }
        }
    }

    /// Look at the open package's files every [`WATCH_EVERY`], and read it
    /// again once a change has settled.
    fn watch_for_changes(&mut self, ctx: &egui::Context) {
        let (Some(package), Some(watch)) = (&self.package, &mut self.watch) else {
            return;
        };
        ctx.request_repaint_after(WATCH_EVERY);
        if watch.checked.elapsed() < WATCH_EVERY {
            return;
        }
        watch.checked = Instant::now();
        if watch.settled(stamp(&package.dir)) {
            let dir = package.dir.clone();
            self.reload(&dir);
        }
    }

    /// Read the open package again, keeping the tab and whatever was
    /// selected wherever it still exists. A package that no longer loads
    /// leaves the last good read on screen and says why.
    fn reload(&mut self, dir: &Path) {
        let before = stamp(dir);
        self.watch = Some(Watch::from(before));
        let package = match package::load(dir) {
            Ok(package) => package,
            Err(error) => {
                self.error = Some(format!("reload: {error}"));
                return;
            }
        };
        self.error = None;
        if self
            .selected_entry
            .is_some_and(|n| n >= package.entries.len())
        {
            self.selected_entry = None;
        }
        if self.selected_tip.as_deref().is_some_and(|tip| {
            package
                .history
                .as_ref()
                .is_ok_and(|history| history.index_of(tip).is_none())
        }) {
            self.selected_tip = None;
        }
        let mut diff = DiffView::for_package(&package);
        let exists = |point: &DiffPoint| diff.points.iter().any(|(p, _)| p == point);
        if exists(&self.diff.from) && exists(&self.diff.to) {
            diff.from = self.diff.from.clone();
            diff.to = self.diff.to.clone();
        }
        self.diff = diff;
        self.package = Some(package);
    }

    fn overview(ui: &mut egui::Ui, package: &Package) {
        egui::Grid::new("overview").num_columns(2).show(ui, |ui| {
            ui.label("Schema");
            let version = package.manifest.version;
            ui.label(if version == wt::world::WORLD_SCHEMA_VERSION {
                format!("v{version} (current)")
            } else {
                format!(
                    "v{version} (reference reads v{})",
                    wt::world::WORLD_SCHEMA_VERSION
                )
            });
            ui.end_row();
            ui.label("Name");
            ui.label(package.manifest.meta.name.as_str());
            ui.end_row();
            ui.label("Entities (base → head)");
            let folded = package
                .fold
                .as_ref()
                .map(|doc| doc.len().to_string())
                .unwrap_or_else(|e| format!("fold refused: {e}"));
            ui.label(format!("{} → {}", package.base.len(), folded));
            ui.end_row();
            ui.label("Log entries");
            let mut line = package.entries.len().to_string();
            if let Some(torn) = package.torn_line {
                line.push_str(&format!(
                    " · the torn last line ({torn}) skipped, as the spec allows"
                ));
            }
            ui.label(line);
            ui.end_row();
            if let Some(corrupt) = &package.corrupt {
                ui.label("Log");
                ui.colored_label(
                    ERROR,
                    format!(
                        "✗ {corrupt} — the {} entries above it are shown; nothing after it can be read",
                        package.entries.len()
                    ),
                );
                ui.end_row();
            }
            if let Err(error) = &package.history {
                ui.label("History");
                ui.colored_label(ERROR, format!("✗ not a tree: {error}"));
                ui.end_row();
            }
            ui.label("Head");
            ui.label(match &package.head {
                None => "the base — there is no log".to_string(),
                Some(head) if package.refs.get("main") == Some(head) => format!("{head} ← main"),
                Some(head) => format!("{head} (the last entry; no main ref)"),
            });
            ui.end_row();
            ui.label("Tours");
            ui.label(package.manifest.tours.len().to_string());
            ui.end_row();
            if let Some(meta) = &package.package_json {
                for key in ["name", "profiles", "head_revision", "forked_from"] {
                    if let Some(value) = meta.get(key) {
                        ui.label(format!("package.{key}"));
                        ui.monospace(value.to_string());
                        ui.end_row();
                    }
                }
            }
        });

        ui.add_space(8.0);
        ui.heading("Consistency");
        Self::consistency(ui, package);

        ui.add_space(8.0);
        ui.heading("Validation");
        let errors = package
            .issues
            .iter()
            .filter(|i| matches!(i.severity, wt::Severity::Error))
            .count();
        let warnings = package.issues.len() - errors;
        ui.label(format!("{errors} errors, {warnings} warnings"));
        egui::ScrollArea::vertical().show(ui, |ui| {
            for issue in &package.issues {
                let mark = match issue.severity {
                    wt::Severity::Error => "✗",
                    wt::Severity::Warning => "⚠",
                };
                ui.monospace(format!("{mark} {}", issue.message));
            }
        });
    }

    /// Whether the package agrees with itself: `manifest.json` against the
    /// fold to the head, and the files against the hashes `package.json`
    /// records. Asset hashes are not checked here yet.
    fn consistency(ui: &mut egui::Ui, package: &Package) {
        let at = package.head.as_deref().unwrap_or("the base");
        match &package.head_drift {
            None => {
                ui.colored_label(
                    WARNING,
                    "? manifest.json can't be checked: the fold to the head refused (above)",
                );
            }
            Some(drift) if drift.is_empty() => {
                ui.label(format!("✓ manifest.json is the fold to {at}"));
            }
            Some(drift) => {
                ui.colored_label(
                    ERROR,
                    format!(
                        "✗ manifest.json isn't the fold to {at}: {} entities and {} world fields differ — the Diff tab opens on them",
                        drift.entities.len(),
                        drift.world.len()
                    ),
                );
            }
        }
        if package.integrity.is_empty() {
            ui.weak("package.json records no integrity hashes");
        }
        for check in &package.integrity {
            let (file, key) = (check.file, check.key);
            if check.holds() {
                ui.label(format!(
                    "✓ {file} holds the bytes package.json names ({key})"
                ));
            } else if check.actual.is_none() {
                ui.colored_label(
                    ERROR,
                    format!("✗ {file} is missing, and package.json names its bytes ({key})"),
                );
            } else {
                ui.colored_label(
                    ERROR,
                    format!(
                        "✗ {file} isn't the bytes package.json names ({key}): written since, by hand or by a checkout"
                    ),
                );
            }
        }
    }

    fn entities(ui: &mut egui::Ui, package: &Package) {
        ui.label("The document at the head (the fold of the path to it):");
        ui.add_space(4.0);
        let Ok(doc) = &package.fold else {
            ui.label("The fold refused — see Overview.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("entities")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    ui.monospace("id");
                    ui.monospace("name");
                    ui.monospace("shape");
                    ui.monospace("components");
                    ui.monospace("ext");
                    ui.end_row();
                    for entity in doc.entities() {
                        ui.monospace(entity.id.0.to_string());
                        ui.monospace(entity.name.as_str());
                        ui.label(
                            entity
                                .shape
                                .as_ref()
                                .map(|s| match s {
                                    wt::Shape::Cuboid { .. } => "Cuboid",
                                    wt::Shape::Sphere { .. } => "Sphere",
                                    wt::Shape::Cylinder { .. } => "Cylinder",
                                    wt::Shape::Cone { .. } => "Cone",
                                    wt::Shape::Capsule { .. } => "Capsule",
                                    wt::Shape::Torus { .. } => "Torus",
                                    wt::Shape::Plane { .. } => "Plane",
                                    wt::Shape::Pyramid { .. } => "Pyramid",
                                    wt::Shape::Tetrahedron { .. } => "Tetrahedron",
                                    wt::Shape::Icosahedron { .. } => "Icosahedron",
                                    wt::Shape::Wedge { .. } => "Wedge",
                                    wt::Shape::Prism { .. } => "Prism",
                                })
                                .unwrap_or("—")
                                .to_string(),
                        );
                        let mut components = String::new();
                        for (letter, has) in [
                            ("M", entity.material.is_some()),
                            ("L", entity.light.is_some()),
                            ("B", !entity.behaviors.is_empty()),
                            ("T", !entity.triggers.is_empty()),
                            ("~", !entity.modulations.is_empty()),
                            ("A", entity.audio.is_some()),
                            ("mesh", entity.mesh_asset.is_some()),
                        ] {
                            if has {
                                components.push_str(letter);
                                components.push(' ');
                            }
                        }
                        ui.label(components.trim_end().to_string());
                        ui.label(entity.extra.keys().cloned().collect::<Vec<_>>().join(", "));
                        ui.end_row();
                    }
                });
        });
        ui.add_space(4.0);
        ui.small("M material · L light · B behaviors · T triggers · ~ modulations · A audio");
    }

    fn log(ui: &mut egui::Ui, package: &Package, selected: &mut Option<usize>) {
        ui.horizontal(|ui| {
            ui.label(format!("{} entries", package.entries.len()));
            if let Some(n) = *selected {
                let entry = &package.entries[n];
                ui.separator();
                ui.monospace(format!(
                    "rev {} · {} · t+{}ms",
                    entry.revision, entry.author.name, entry.timestamp_ms
                ));
            }
        });
        ui.add_space(4.0);
        let selected_now = *selected;
        let mut new_selection = None;
        egui::Panel::left("entries").show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (n, entry) in package.entries.iter().enumerate() {
                    let id = package.entry_id(n);
                    let summary = Package::op_summary(entry).join(" · ");
                    let is_tip = package.tips.contains(&id);
                    let label = format!("{} {} — {}", if is_tip { "●" } else { " " }, id, summary);
                    if ui
                        .selectable_label(selected_now == Some(n), label)
                        .clicked()
                    {
                        new_selection = Some(n);
                    }
                }
            });
        });
        let current = new_selection.or(*selected);
        if let Some(n) = current {
            *selected = Some(n);
            egui::ScrollArea::vertical().show(ui, |ui| {
                let entry = &package.entries[n];
                let id = package.entry_id(n);
                ui.heading(format!("Entry {id} (rev {})", entry.revision));
                ui.label(format!(
                    "Author: {} · {} ms",
                    entry.author.name, entry.timestamp_ms
                ));
                // The parent the history resolves — the entry's own, or
                // the one before it — not only one the line spells out.
                let parent = match &package.history {
                    Ok(_) => package
                        .parent_of(&id)
                        .unwrap_or_else(|| "the base".to_string()),
                    Err(_) => entry.parent.clone().unwrap_or_else(|| "—".to_string()),
                };
                ui.label(format!("Parent: {parent}"));
                ui.add_space(6.0);
                for line in Package::op_summary(entry) {
                    ui.monospace(line);
                }
            });
        }
    }

    fn branches(ui: &mut egui::Ui, package: &Package, selected_tip: &mut Option<String>) {
        let history = match &package.history {
            Ok(history) => history,
            Err(error) => {
                ui.colored_label(ERROR, format!("✗ The log isn't a tree: {error}"));
                return;
            }
        };
        ui.label(format!(
            "{} tips across {} entries — fold to any tip:",
            package.tips.len(),
            package.entries.len()
        ));
        ui.add_space(4.0);
        let mut selected = selected_tip.clone();
        // The entries that build on the base start the tree; a branch is
        // indented where it forks, and a run of single children stays in
        // its column, so a long linear log draws as one flat list.
        let mut stack: Vec<(&str, usize)> = history
            .ids()
            .iter()
            .rev()
            .map(String::as_str)
            .filter(|id| history.parent_of(id).is_none())
            .map(|id| (id, 0))
            .collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            while let Some((id, depth)) = stack.pop() {
                let refs: Vec<&str> = package
                    .refs
                    .iter()
                    .filter(|(_, tip)| tip.as_str() == id)
                    .map(|(name, _)| name.as_str())
                    .collect();
                let mark = if package.tips.iter().any(|t| t == id) {
                    "●"
                } else {
                    "├"
                };
                let refs = if refs.is_empty() {
                    String::new()
                } else {
                    format!("  ← {}", refs.join(", "))
                };
                let label = format!("{}{mark} {id}{refs}", "    ".repeat(depth));
                if ui
                    .selectable_label(selected.as_deref() == Some(id), label)
                    .clicked()
                {
                    selected = Some(id.to_string());
                }
                let children = history.children_of(id);
                let depth = if children.len() > 1 { depth + 1 } else { depth };
                stack.extend(children.into_iter().rev().map(|child| (child, depth)));
            }
        });
        *selected_tip = selected.clone();
        if let Some(tip) = &selected {
            ui.separator();
            match package.fold_to(tip) {
                Ok((doc, path)) => {
                    ui.label(format!(
                        "Fold to {tip}: {} entities, path {}",
                        doc.len(),
                        path.join(" → ")
                    ));
                }
                Err(error) => {
                    ui.colored_label(ERROR, format!("✗ {error}"));
                }
            }
        }
    }

    fn state(ui: &mut egui::Ui, package: &Package) {
        if package.state_types.is_empty() {
            ui.label("No state.json — this world declares no game state.");
            return;
        }
        ui.label("The state at the last entry (whole log, linear):");
        ui.add_space(4.0);
        egui::Grid::new("state")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.monospace("field");
                ui.monospace("type");
                ui.monospace("initial");
                ui.monospace("value");
                ui.end_row();
                for (name, kind) in &package.state_types {
                    ui.monospace(name);
                    ui.label(kind);
                    ui.monospace(
                        package
                            .state_initial
                            .get(name)
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    );
                    ui.monospace(
                        package
                            .state_values
                            .get(name)
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    );
                    ui.end_row();
                }
                for (name, value) in &package.state_values {
                    if package.state_types.contains_key(name) {
                        continue;
                    }
                    ui.monospace(name);
                    ui.label("(undeclared)");
                    ui.monospace("—");
                    ui.monospace(value.to_string());
                    ui.end_row();
                }
            });
        if !package.undeclared.is_empty() {
            ui.add_space(4.0);
            ui.colored_label(
                WARNING,
                format!(
                    "⚠ carried, declared by no one: {}",
                    package.undeclared.join(", ")
                ),
            );
        }
    }

    fn physics(ui: &mut egui::Ui, package: &Package) {
        let world = &package.physics_world;
        if world.dynamic.is_empty() && world.statics.is_empty() {
            ui.label("No ext-physics bodies declared — nothing to simulate.");
            return;
        }
        ui.label(format!(
            "{} dynamic · {} static · {} kinematic (manifest.json, the head as written)",
            world.dynamic.len(),
            world.statics.len(),
            world.kinematic.len()
        ));
        ui.label(format!("Gravity {:?}", world.gravity));
        let Some(sim) = &package.physics else {
            return;
        };
        ui.add_space(6.0);
        ui.label(format!(
            "5 s simulated: {} contacts, {} bounces, settled at {:.2} s",
            sim.contacts.len(),
            sim.bounces.len(),
            sim.settled_s
        ));
        ui.add_space(4.0);
        ui.heading("Contacts");
        egui::ScrollArea::vertical().show(ui, |ui| {
            for contact in &sim.contacts {
                ui.monospace(format!(
                    "{:6.2}s  {} → {}  ({:.1} m/s)",
                    contact.t_s, contact.body, contact.other, contact.normal_speed
                ));
            }
        });
        ui.heading("Resting");
        egui::Grid::new("resting").num_columns(2).show(ui, |ui| {
            for (name, at) in &sim.resting {
                ui.monospace(name);
                ui.monospace(format!("[{:.2}, {:.2}, {:.2}]", at[0], at[1], at[2]));
                ui.end_row();
            }
        });
    }

    fn diff(ui: &mut egui::Ui, package: &Package, view: &mut DiffView) {
        ui.horizontal(|ui| {
            ui.label("from");
            Self::point_selector(ui, "diff-from", &view.points, &mut view.from);
            if ui.button("⇄").on_hover_text("Swap the two sides").clicked() {
                std::mem::swap(&mut view.from, &mut view.to);
            }
            ui.label("to");
            Self::point_selector(ui, "diff-to", &view.points, &mut view.to);
        });
        ui.add_space(4.0);
        let stale = view
            .computed
            .as_ref()
            .is_none_or(|(from, to, _)| from != &view.from || to != &view.to);
        if stale {
            let result = match (
                Self::world_at(package, &view.from),
                Self::world_at(package, &view.to),
            ) {
                (Ok(from), Ok(to)) => Ok(diff::diff_worlds(&from, &to)),
                (from, to) => Err([("from", from.err()), ("to", to.err())]
                    .into_iter()
                    .filter_map(|(side, error)| error.map(|error| format!("{side}: {error}")))
                    .collect::<Vec<_>>()
                    .join("\n")),
            };
            view.computed = Some((view.from.clone(), view.to.clone(), result));
        }
        let Some((_, _, result)) = &view.computed else {
            return;
        };
        let diff = match result {
            Ok(diff) => diff,
            Err(error) => {
                ui.colored_label(ERROR, format!("✗ {error}"));
                return;
            }
        };
        ui.label(format!(
            "{} entities added · {} removed · {} changed · {} world field{} changed",
            diff.count(Change::Added),
            diff.count(Change::Removed),
            diff.count(Change::Changed),
            diff.world.len(),
            if diff.world.len() == 1 { "" } else { "s" }
        ));
        ui.add_space(4.0);
        if diff.is_empty() {
            ui.label("No differences — the world is the same at both points.");
            return;
        }
        egui::ScrollArea::both().show(ui, |ui| {
            // A diff line extends the grid sideways rather than wrapping;
            // the area scrolls.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            egui::Grid::new("diff")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.monospace("id");
                    ui.monospace("name");
                    ui.monospace("change");
                    ui.monospace("fields");
                    ui.end_row();
                    for entity in &diff.entities {
                        ui.monospace(entity.id.as_str());
                        ui.monospace(entity.name.as_str());
                        ui.colored_label(Self::change_color(entity.change), entity.change.label());
                        ui.monospace(
                            entity
                                .fields
                                .iter()
                                .map(FieldDiff::line)
                                .collect::<Vec<_>>()
                                .join("\n"),
                        );
                        ui.end_row();
                    }
                    for field in &diff.world {
                        ui.monospace("world");
                        ui.monospace(field.path.as_str());
                        let change = field.change();
                        ui.colored_label(Self::change_color(change), change.label());
                        ui.monospace(field.values());
                        ui.end_row();
                    }
                });
        });
    }

    /// One side's selector: the base, every entry (tips marked ●), the
    /// manifest as written.
    fn point_selector(
        ui: &mut egui::Ui,
        salt: &str,
        points: &[(DiffPoint, String)],
        current: &mut DiffPoint,
    ) {
        let selected = points
            .iter()
            .find(|(point, _)| point == current)
            .map(|(_, label)| label.as_str())
            .unwrap_or("—");
        egui::ComboBox::from_id_salt(salt)
            .width(400.0)
            .truncate()
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (point, label) in points {
                    ui.selectable_value(current, point.clone(), label.as_str());
                }
            });
    }

    /// The world at one point, as the manifest the fold writes — JSON,
    /// for the diff.
    fn world_at(package: &Package, point: &DiffPoint) -> Result<serde_json::Value, String> {
        let value = match point {
            DiffPoint::Base => serde_json::to_value(package.base.to_manifest()),
            DiffPoint::Entry(id) => serde_json::to_value(package.fold_to(id)?.0.to_manifest()),
            DiffPoint::Manifest => serde_json::to_value(&package.manifest),
        };
        value.map_err(|e| e.to_string())
    }

    fn change_color(change: Change) -> egui::Color32 {
        match change {
            Change::Added => egui::Color32::from_rgb(120, 200, 120),
            Change::Removed => ERROR,
            Change::Changed => WARNING,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A package the test may write to: an empty world and a log of
    /// history-only entries, so the manifest is its own base.
    fn scratch(name: &str, entries: usize) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("worldprobe-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch package dir");
        let manifest = wt::WorldDoc::new("probe").to_manifest();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).expect("manifest serializes"),
        )
        .expect("manifest.json");
        let log: String = (0..entries)
            .map(|n| format!("{{\"revision\":0,\"ops\":[],\"timestamp_ms\":{n}}}\n"))
            .collect();
        std::fs::write(dir.join("ops.jsonl"), log).expect("ops.jsonl");
        dir
    }

    /// Every tab, drawn headless for every package at hand — the siblings'
    /// when they are there, and scratch ones with a torn line, a corrupt
    /// line and a parent the log doesn't hold — so a tab that panics fails
    /// here rather than in a window.
    #[test]
    fn every_tab_draws_every_package() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut dirs: Vec<PathBuf> = [
            "openworldformat/examples/hello-world",
            "openworldformat/examples/speedrun-fork",
            "openworldformat/examples/forked-exploration",
            "openworldformat/examples/the-drop-test",
            "songworld/world",
        ]
        .iter()
        .map(|pkg| root.join(pkg))
        .filter(|dir| dir.is_dir())
        .collect();
        let entry = |id: &str, parent: Option<&str>| {
            let parent = parent
                .map(|p| format!(",\"parent\":\"{p}\""))
                .unwrap_or_default();
            format!("{{\"revision\":0,\"ops\":[],\"id\":\"{id}\"{parent}}}\n")
        };
        let mut scratches = Vec::new();
        for (name, log) in [
            ("draw-torn", format!("{}{{\"rev", entry("e1", None))),
            (
                "draw-corrupt",
                format!("{}{{\"rev\n{}", entry("e1", None), entry("e3", None)),
            ),
            (
                "draw-orphan",
                format!("{}{}", entry("e1", None), entry("e2", Some("gone"))),
            ),
        ] {
            let dir = scratch(name, 0);
            std::fs::write(dir.join("ops.jsonl"), log).expect("ops.jsonl");
            scratches.push(dir.clone());
            dirs.push(dir);
        }

        for dir in &dirs {
            let package = package::load(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            let mut selected = (!package.entries.is_empty()).then_some(0);
            let mut tip = package.head.clone();
            let mut view = DiffView::for_package(&package);
            let ctx = egui::Context::default();
            for tab in 0..7 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| match tab {
                    0 => WorldProbe::overview(ui, &package),
                    1 => WorldProbe::entities(ui, &package),
                    2 => WorldProbe::log(ui, &package, &mut selected),
                    3 => WorldProbe::branches(ui, &package, &mut tip),
                    4 => WorldProbe::diff(ui, &package, &mut view),
                    5 => WorldProbe::state(ui, &package),
                    _ => WorldProbe::physics(ui, &package),
                });
                // No painter here: the font atlas delta is dropped on purpose.
                output.textures_delta.clear();
            }
        }
        for dir in scratches {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_drifted_head_opens_the_diff_on_it() {
        // No log, a base that is the empty world, and a manifest.json
        // holding a lamp: the head isn't the fold, and the Diff tab opens
        // on exactly that.
        let dir = scratch("drift", 0);
        std::fs::create_dir_all(dir.join("snapshots")).expect("snapshots/");
        std::fs::write(
            dir.join(wt::package::BASE_SNAPSHOT),
            serde_json::to_vec(&wt::WorldDoc::new("probe").to_manifest()).expect("base"),
        )
        .expect("the base snapshot");
        let mut head = wt::WorldDoc::new("probe");
        head.apply(&wt::EditOp::spawn(wt::WorldEntity::new(1, "lamp")))
            .expect("a lamp");
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&head.to_manifest()).expect("manifest"),
        )
        .expect("manifest.json");
        let package = package::load(&dir).unwrap_or_else(|e| panic!("{e}"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            package
                .head_drift
                .as_ref()
                .is_some_and(|drift| drift.count(Change::Added) == 1),
            "{:?}",
            package.head_drift
        );
        let view = DiffView::for_package(&package);
        assert_eq!(view.from, DiffPoint::Base);
        assert_eq!(view.to, DiffPoint::Manifest);
    }

    #[test]
    fn a_change_is_read_once_it_settles_and_the_view_survives_it() {
        use std::io::Write as _;
        let dir = scratch("watch", 1);
        let mut probe = WorldProbe::default();
        probe.open(dir.display().to_string());
        assert_eq!(probe.package.as_ref().map(|p| p.entries.len()), Some(1));
        probe.tab = Tab::Log;
        probe.selected_entry = Some(0);

        // An authority appends: the change is seen at one look, and read
        // at the next if it held still.
        let mut watch = Watch::from(stamp(&dir));
        assert!(!watch.settled(stamp(&dir)), "nothing changed");
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("ops.jsonl"))
            .expect("the log opens for append");
        writeln!(log, "{{\"revision\":0,\"ops\":[]}}").expect("an entry appends");
        assert!(!watch.settled(stamp(&dir)), "seen, not yet read");
        assert!(watch.settled(stamp(&dir)), "it held still: read it");

        probe.reload(&dir);
        assert_eq!(probe.package.as_ref().map(|p| p.entries.len()), Some(2));
        assert!(probe.tab == Tab::Log, "the tab stays");
        assert_eq!(probe.selected_entry, Some(0), "the selection stays");
        assert!(probe.error.is_none());

        // A read that fails leaves the last good one on screen.
        std::fs::write(dir.join("manifest.json"), "{").expect("a broken manifest");
        probe.reload(&dir);
        assert_eq!(probe.package.as_ref().map(|p| p.entries.len()), Some(2));
        assert!(
            probe
                .error
                .as_deref()
                .is_some_and(|e| e.starts_with("reload: ")),
            "{:?}",
            probe.error
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Diff tab drawn headless against the spec's fork, when the
    /// sibling checkout is there (absent, it skips — by design): the
    /// defaults name the last change on main, drawing folds both sides,
    /// a new selection refolds, and a tip the log lacks says so.
    #[test]
    fn the_diff_tab_draws_the_fork() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../openworldformat/examples/speedrun-fork");
        if !dir.is_dir() {
            return;
        }
        let package = package::load(&dir).unwrap_or_else(|e| panic!("{e}"));
        let mut view = DiffView::for_package(&package);
        assert_eq!(view.from, DiffPoint::Entry("e1".into()));
        assert_eq!(view.to, DiffPoint::Entry("e2".into()));
        assert_eq!(view.points.len(), package.entries.len() + 2);
        assert_eq!(view.points[0].1, "base");
        assert!(
            view.points[2]
                .1
                .starts_with("  e2 · rev 2 · Edit · SpawnEntity 21 (banner)"),
            "{}",
            view.points[2].1
        );
        assert!(
            view.points[3].1.starts_with("● e3 · rev 2 · Input · kai"),
            "{}",
            view.points[3].1
        );

        let ctx = egui::Context::default();
        let draw = |view: &mut DiffView| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                WorldProbe::diff(ui, &package, view);
            });
            // No painter here: the font atlas delta is dropped on purpose.
            output.textures_delta.clear();
        };

        // e1 → e2: the banner.
        draw(&mut view);
        let Some((_, _, Ok(diff))) = &view.computed else {
            panic!("no diff: {:?}", view.computed);
        };
        assert_eq!(diff.count(Change::Added), 1);
        assert_eq!(diff.entities[0].name, "banner");

        // base → manifest.json: everything the log did.
        view.from = DiffPoint::Base;
        view.to = DiffPoint::Manifest;
        draw(&mut view);
        let Some((_, _, Ok(diff))) = &view.computed else {
            panic!("no diff: {:?}", view.computed);
        };
        assert_eq!(diff.count(Change::Added), 2);
        assert_eq!(diff.world.len(), 1, "{:?}", diff.world);
        assert_eq!(diff.world[0].path, "next_entity_id");

        // A tip the log lacks: the fold refuses, the tab says which side.
        view.to = DiffPoint::Entry("e9".into());
        draw(&mut view);
        let refused = match &view.computed {
            Some((_, _, Err(error))) => error.clone(),
            other => panic!("expected a refusal: {other:?}"),
        };
        assert!(
            refused.starts_with("to: ") && refused.contains("e9"),
            "{refused}"
        );
    }
}
