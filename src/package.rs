//! One loaded `.world` package: everything the inspector shows, folded
//! through the format's own Rust reference — this app adds no format
//! semantics of its own, it points them at a directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openworldformat as wt;
use openworldformat::physics::{self as physics_ref, PhysicsWorld, SimOptions, Simulation};
use openworldformat::session::{SessionMeta, SessionOp};
use openworldformat::state::{StateDoc, fold_state};
use openworldformat::{EditOp, OpLogEntry, WorldDoc, fold_path};

/// A loaded package: the base document, the log, its history, the fold
/// to the head, the state trajectory, the validation verdict, and — when
/// the world declares bodies — a physics run.
pub struct Package {
    /// The directory this package was loaded from.
    pub dir: PathBuf,
    /// `package.json`, when present (L3 metadata).
    pub package_json: Option<serde_json::Value>,
    /// `manifest.json` as written: the head, the state at the tip of
    /// `main` (spec/package.md, "Head-first").
    pub manifest: wt::WorldManifest,
    /// The session log (`ops.jsonl`) as the crate reads it, in file
    /// order: every entry — or, when a line before the last doesn't
    /// parse, the entries above that line.
    pub entries: Vec<OpLogEntry>,
    /// The last line, numbered from 1, when it was torn and skipped.
    pub torn_line: Option<usize>,
    /// A line before the last that doesn't parse: the log is corrupt
    /// there, and [`Package::entries`] stops above it. (Its `before` is
    /// empty: those entries moved into `entries`.)
    pub corrupt: Option<wt::CorruptLine>,
    /// The state declaration's initial values, from `state.json`.
    pub state_initial: BTreeMap<String, serde_json::Value>,
    /// The state declaration's field types, from `state.json`.
    pub state_types: BTreeMap<String, String>,
    /// The folded state at the last entry (whole log, linear).
    pub state_values: BTreeMap<String, serde_json::Value>,
    /// State keys no declaration covers (carried, and said so).
    pub undeclared: Vec<String>,
    /// Manifest-level validation issues (the crate's own rules).
    pub issues: Vec<wt::ValidationIssue>,
    /// The log read as a tree — identity, parents, children, tips — or
    /// why it isn't one.
    pub history: Result<wt::History, wt::HistoryError>,
    /// The history's tips, in file order; empty when it isn't a tree.
    pub tips: Vec<String>,
    /// The head entry by the spec's rule — `refs.main`, else the last
    /// entry — or `None` when there is no log and the base is the head.
    pub head: Option<String>,
    /// The document at the head — the fold of the path to it — or why
    /// the fold refused.
    pub fold: Result<WorldDoc, String>,
    /// The base document — `snapshots/base.json`, the oldest state the
    /// log folds from, or the manifest itself when the package has no
    /// snapshot — for the fold and the per-tip folds.
    pub base: WorldDoc,
    /// Named refs from `package.json` (ref → entry id).
    pub refs: BTreeMap<String, String>,
    /// The physics run, when the world declares bodies.
    pub physics: Option<Simulation>,
    /// Bodies the world declares (for the physics header).
    pub physics_world: PhysicsWorld,
}

/// The base document the log folds from: `snapshots/base.json` when the
/// package has one, else `manifest.json` itself — a log with no edits
/// folds from the head (spec/package.md, "Head-first"). Built by the
/// reference's own constructor, so the base carries every field a
/// manifest holds and the fold's state is always a whole manifest.
fn base_doc(snapshot: Option<&str>, manifest: &wt::WorldManifest) -> Result<WorldDoc, String> {
    let doc = match snapshot {
        Some(text) => {
            let base: wt::WorldManifest = serde_json::from_str(text)
                .map_err(|e| format!("{}: {e}", wt::package::BASE_SNAPSHOT))?;
            WorldDoc::from_manifest(&base)
        }
        None => WorldDoc::from_manifest(manifest),
    };
    doc.map_err(|e| format!("base: {e}"))
}

/// Load a package directory. `manifest.json` is required; everything
/// else degrades gracefully — an inspector shows what is there.
pub fn load(dir: &Path) -> Result<Package, String> {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();

    let manifest: wt::WorldManifest =
        serde_json::from_str(&read("manifest.json").ok_or_else(|| {
            format!(
                "no manifest.json in {} — is this a .world package?",
                dir.display()
            )
        })?)
        .map_err(|e| format!("manifest.json: {e}"))?;

    // The crate reads the log as the spec says: a torn last line skipped
    // and counted, an earlier bad line refused — with the entries above
    // it, which are still worth showing.
    let (entries, torn_line, corrupt) = match wt::decode_log(&read("ops.jsonl").unwrap_or_default())
    {
        Ok(log) => (log.entries, log.torn_line, None),
        Err(mut corrupt) => (std::mem::take(&mut corrupt.before), None, Some(corrupt)),
    };

    let state_doc: StateDoc = read("state.json")
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let state_initial = state_doc
        .fields
        .iter()
        .map(|(name, field)| (name.clone(), field.initial_value()))
        .collect();
    let state_types = state_doc
        .fields
        .iter()
        .map(|(name, field)| (name.clone(), field.kind.clone()))
        .collect();
    let folded_state = fold_state(&state_doc, &entries);
    let (state_values, undeclared) = (folded_state.values, folded_state.undeclared);

    let package_json =
        read("package.json").and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let refs = package_json
        .as_ref()
        .and_then(|p| p.get("refs"))
        .and_then(|r| r.as_object())
        .map(|object| {
            object
                .iter()
                .filter_map(|(name, id)| id.as_str().map(|id| (name.clone(), id.to_string())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    let history = wt::build_history(&entries);
    let tips = history
        .as_ref()
        .map(|history| history.tips().into_iter().map(str::to_string).collect())
        .unwrap_or_default();
    // The head is the crate's rule over the refs package.json names.
    let mut meta = SessionMeta::new(manifest.meta.name.as_str());
    meta.refs = refs.clone();
    let head = wt::main_tip(&meta, &entries);

    let issues = wt::validate_manifest(&manifest, &wt::WorldLimits::default());
    let base = base_doc(read(wt::package::BASE_SNAPSHOT).as_deref(), &manifest)?;
    // The fold the head-first rule names: the path to the head, not every
    // entry in file order — a fork's two branches never meet in one world.
    let fold = match &head {
        Some(head) => fold_path(&base, &entries, Some(head))
            .map(|(doc, _)| doc)
            .map_err(|e| e.to_string()),
        None => Ok(base.clone()),
    };

    let physics_world = physics_ref::collect_physics(&manifest);
    let physics = (!physics_world.dynamic.is_empty())
        .then(|| physics_ref::simulate(&manifest, &SimOptions::default()));

    Ok(Package {
        dir: dir.to_path_buf(),
        package_json,
        manifest,
        entries,
        torn_line,
        corrupt,
        state_initial,
        state_types,
        state_values,
        undeclared,
        issues,
        history,
        tips,
        head,
        fold,
        base,
        refs,
        physics,
        physics_world,
    })
}

/// Which fields a patch touches, as a compact list.
fn changed_fields(patch: &wt::EntityPatch) -> String {
    let mut fields: Vec<&str> = Vec::new();
    for (name, set) in [
        ("name", patch.name.is_some()),
        ("transform", patch.transform.is_some()),
        ("parent", patch.parent.is_some()),
        ("shape", patch.shape.is_some()),
        ("material", patch.material.is_some()),
        ("light", patch.light.is_some()),
        ("behaviors", patch.behaviors.is_some()),
        ("audio", patch.audio.is_some()),
        ("mesh_asset", patch.mesh_asset.is_some()),
        ("modulations", patch.modulations.is_some()),
        ("instance_of", patch.instance_of.is_some()),
        ("triggers", patch.triggers.is_some()),
    ] {
        if set {
            fields.push(name);
        }
    }
    fields.extend(patch.extra.keys().map(String::as_str));
    if fields.is_empty() {
        "nothing".into()
    } else {
        fields.join(", ")
    }
}

impl Package {
    /// Fold to one tip: the document at that point of the history.
    pub fn fold_to(&self, tip: &str) -> Result<(WorldDoc, Vec<String>), String> {
        fold_path(&self.base, &self.entries, Some(tip)).map_err(|e| e.to_string())
    }

    /// An entry's parent by the crate's identity rule; `None` for the
    /// entry that builds on the base, or when the log isn't a tree.
    pub fn parent_of(&self, id: &str) -> Option<String> {
        self.history
            .as_ref()
            .ok()?
            .parent_of(id)
            .map(str::to_string)
    }

    /// The id of the `n`th entry in file order, as the history names it;
    /// when the log isn't a tree, its own `id`, else its place.
    pub fn entry_id(&self, n: usize) -> String {
        self.history
            .as_ref()
            .ok()
            .and_then(|history| history.id_at(n))
            .map(str::to_string)
            .or_else(|| self.entries.get(n).and_then(|entry| entry.id.clone()))
            .unwrap_or_else(|| format!("entry {}", n + 1))
    }

    /// One line per op, classified — the log view's detail rows.
    pub fn op_summary(entry: &OpLogEntry) -> Vec<String> {
        entry
            .ops
            .iter()
            .map(|op| match op {
                SessionOp::Edit(edit) => match &**edit {
                    EditOp::SpawnEntity { entity } => {
                        format!("Edit · SpawnEntity {} ({})", entity.id.0, entity.name)
                    }
                    EditOp::DeleteEntity { id } => format!("Edit · DeleteEntity {}", id.0),
                    EditOp::ModifyEntity { id, patch } => {
                        format!("Edit · ModifyEntity {} · {}", id.0, changed_fields(patch))
                    }
                    EditOp::SetEnvironment { .. } => "Edit · SetEnvironment".into(),
                    EditOp::SetCamera { .. } => "Edit · SetCamera".into(),
                    EditOp::SetAmbience { .. } => "Edit · SetAmbience".into(),
                    EditOp::SpawnAudioEmitter { name, .. } => {
                        format!("Edit · SpawnAudioEmitter {name}")
                    }
                    EditOp::RemoveAudioEmitter { name, .. } => {
                        format!("Edit · RemoveAudioEmitter {name}")
                    }
                    EditOp::ModifyWorld { patch } => {
                        let mut fields: Vec<&str> = Vec::new();
                        for (name, set) in [
                            ("meta", patch.meta.is_some()),
                            ("environment", patch.environment.is_some()),
                            ("camera", patch.camera.is_some()),
                            ("avatar", patch.avatar.is_some()),
                            ("tours", patch.tours.is_some()),
                            ("soundtrack", patch.soundtrack.is_some()),
                            ("ambience", patch.ambience.is_some()),
                            ("creations", patch.creations.is_some()),
                        ] {
                            if set {
                                fields.push(name);
                            }
                        }
                        format!("Edit · ModifyWorld · {}", fields.join(", "))
                    }
                    EditOp::Batch { ops } => format!("Edit · Batch ({} ops)", ops.len()),
                },
                SessionOp::Tool(record) => format!("Tool · {} · {:?}", record.tool, record.phase),
                SessionOp::Input(record) => format!(
                    "Input · {} {}",
                    record.input.actor,
                    record
                        .input
                        .click
                        .map(|c| format!("(click {c})"))
                        .unwrap_or_default()
                ),
                SessionOp::State(record) => format!(
                    "State · {}",
                    record.state.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
                SessionOp::Clock(record) => {
                    format!(
                        "Clock · {} @{:.1}s",
                        record.clock.playing, record.clock.position_s
                    )
                }
                SessionOp::Merge(record) => format!("Merge · from {}", record.merge.branch),
                SessionOp::Extension(record) => format!("Extension · {}", record.name),
            })
            .collect()
    }
}
