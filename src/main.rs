//! worldprobe — the desktop inspector for `.world` packages.
//!
//! Fold, validate, walk the history and its branches, watch the state
//! trajectory, run the physics. The format semantics are the Rust
//! reference's (world-types, world-sync, world-physics); this app only
//! points them at a directory and shows what they say.

mod package;

use eframe::egui;
use localgpt_world_types as wt;
use package::Package;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_title("worldprobe — the .world inspector"),
        ..Default::default()
    };
    eframe::run_native(
        "worldprobe",
        options,
        Box::new(|_cc| Ok(Box::new(WorldProbe::default()))),
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
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Tab {
    #[default]
    Overview,
    Entities,
    Log,
    Branches,
    State,
    Physics,
}

impl eframe::App for WorldProbe {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Drag a .world folder onto the window to open it.
        for file in ui.ctx().input(|i| i.raw.dropped_files.clone()) {
            let path = file.path();
            self.open(path.display().to_string());
        }
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
                }
            });
        });
        if let Some(error) = &self.error {
            egui::Panel::top("error").show(ui, |ui| {
                ui.colored_label(egui::Color32::from_rgb(230, 120, 100), format!("⚠ {error}"));
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
                ui.selectable_value(&mut self.tab, Tab::State, "State");
                ui.selectable_value(&mut self.tab, Tab::Physics, "Physics");
            });
            ui.separator();
            match self.tab {
                Tab::Overview => Self::overview(ui, package),
                Tab::Entities => Self::entities(ui, package),
                Tab::Log => Self::log(ui, package, &mut self.selected_entry),
                Tab::Branches => Self::branches(ui, package, &mut self.selected_tip),
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
        match package::load(std::path::Path::new(&path)) {
            Ok(package) => {
                self.error = None;
                self.package = Some(package);
            }
            Err(error) => {
                self.error = Some(error);
                self.package = None;
            }
        }
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
            ui.label(format!("{} → {}", package.manifest.entities.len(), folded));
            ui.end_row();
            ui.label("Log entries");
            let mut line = package.entries.len().to_string();
            if !package.torn_lines.is_empty() {
                line.push_str(&format!(
                    " ({} torn/unparseable: {})",
                    package.torn_lines.len(),
                    package
                        .torn_lines
                        .iter()
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            ui.label(line);
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

    fn entities(ui: &mut egui::Ui, package: &Package) {
        ui.label("The document at the last entry (the fold):");
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
                    let id = entry.id.clone().unwrap_or_else(|| format!("line-{n}"));
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
                ui.heading(format!(
                    "Entry {} (rev {})",
                    entry.id.clone().unwrap_or_else(|| format!("line-{n}")),
                    entry.revision
                ));
                ui.label(format!(
                    "Author: {} · {} ms",
                    entry.author.name, entry.timestamp_ms
                ));
                if let Some(parent) = &entry.parent {
                    ui.label(format!("Parent: {parent}"));
                }
                ui.add_space(6.0);
                for line in Package::op_summary(entry) {
                    ui.monospace(line);
                }
            });
        }
    }

    fn branches(ui: &mut egui::Ui, package: &Package, selected_tip: &mut Option<String>) {
        ui.label(format!(
            "{} tips across {} entries — fold to any tip:",
            package.tips.len(),
            package.entries.len()
        ));
        ui.add_space(4.0);
        fn walk(
            ui: &mut egui::Ui,
            package: &Package,
            id: &str,
            depth: usize,
            selected: &mut Option<String>,
        ) {
            let refs: Vec<String> = package
                .refs
                .iter()
                .filter(|(_, tip)| tip.as_str() == id)
                .map(|(name, _)| name.clone())
                .collect();
            let is_tip = package.tips.iter().any(|t| t == id);
            let indent = "    ".repeat(depth);
            let mark = if is_tip { "●" } else { "├" };
            let refs = if refs.is_empty() {
                String::new()
            } else {
                format!(
                    "  ← {}",
                    refs.iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let label = format!("{indent}{mark} {id}{refs}");
            if ui
                .selectable_label(selected.as_deref() == Some(id), label)
                .clicked()
            {
                *selected = Some(id.to_string());
            }
            if let Some(children) = package.children.get(id) {
                for child in children {
                    walk(ui, package, child, depth + 1, selected);
                }
            }
        }
        let mut selected = selected_tip.clone();
        let roots: Vec<String> = package
            .entries
            .iter()
            .enumerate()
            .filter(|(n, entry)| entry.parent.is_none() || *n == 0)
            .map(|(n, entry)| entry.id.clone().unwrap_or_else(|| format!("line-{n}")))
            .collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for root in &roots {
                walk(ui, package, root, 0, &mut selected);
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
                    ui.colored_label(egui::Color32::from_rgb(230, 120, 100), format!("✗ {error}"));
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
                egui::Color32::from_rgb(220, 180, 100),
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
            "{} dynamic · {} static · {} kinematic (base manifest; log-spawned bodies not yet folded in)",
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
}
