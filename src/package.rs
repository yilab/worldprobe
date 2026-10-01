//! One loaded `.world` package: everything the inspector shows, folded
//! through the format's own Rust reference — this app adds no format
//! semantics of its own, it points them at a directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openworldformat as wt;
use openworldformat::physics::{self as physics_ref, PhysicsWorld, SimOptions, Simulation};
use openworldformat::session::SessionOp;
use openworldformat::state::{StateDoc, fold_state};
use openworldformat::{EditOp, OpLogEntry, WorldDoc, fold_log, fold_path};

/// A loaded package: the base document, the log, the fold, the history,
/// the state trajectory, the validation verdict, and — when the world
/// declares bodies — a physics run.
pub struct Package {
    /// The directory this package was loaded from.
    pub dir: PathBuf,
    /// `package.json`, when present (L3 metadata).
    pub package_json: Option<serde_json::Value>,
    /// The base world document (`manifest.json`).
    pub manifest: wt::WorldManifest,
    /// The session log (`ops.jsonl`), in file order. Torn trailing
    /// lines are skipped and counted, per the spec's tolerance rule.
    pub entries: Vec<OpLogEntry>,
    /// Lines that wouldn't parse (torn tail, or corruption).
    pub torn_lines: Vec<usize>,
    /// The state declaration's initial values, from `state.json`.
    pub state_initial: BTreeMap<String, serde_json::Value>,
    /// The state declaration's field types, from `state.json`.
    pub state_types: BTreeMap<String, String>,
    /// The folded state at the last entry (whole log, linear).
    pub state_values: BTreeMap<String, serde_json::Value>,
    /// State keys no declaration covers (carried, and said so).
    pub undeclared: Vec<String>,
    /// Manifest-level validation issues (world-types' own rules).
    pub issues: Vec<wt::ValidationIssue>,
    /// The document at the last entry, or why the fold refused.
    pub fold: Result<WorldDoc, String>,
    /// The base document as a `WorldDoc`, for per-tip folds.
    pub base: WorldDoc,
    /// Entry id → children ids (branching histories).
    pub children: BTreeMap<String, Vec<String>>,
    /// Tips of the history (entries that are nobody's parent).
    pub tips: Vec<String>,
    /// Named refs from `package.json` (ref → entry id).
    pub refs: BTreeMap<String, String>,
    /// The physics run, when the world declares bodies.
    pub physics: Option<Simulation>,
    /// Bodies the world declares (for the physics header).
    pub physics_world: PhysicsWorld,
}

/// The synthesized identity of one entry (`id`, `parent`): its own when
/// present, else `line-<n>` and the previous entry — the same rule the
/// fold applies, so a linear log is a chain in file order.
fn identified(entries: &[OpLogEntry]) -> Vec<(String, Option<String>)> {
    let mut out = Vec::with_capacity(entries.len());
    let mut previous: Option<String> = None;
    for (n, entry) in entries.iter().enumerate() {
        let id = entry.id.clone().unwrap_or_else(|| format!("line-{n}"));
        let parent = entry.parent.clone().or_else(|| previous.clone());
        out.push((id.clone(), parent));
        previous = Some(id);
    }
    out
}

/// The base document as a `WorldDoc`: entities spawned through the
/// fold's own path so id/name maps stay honest.
fn base_doc(manifest: &wt::WorldManifest) -> Result<WorldDoc, String> {
    let mut doc = WorldDoc::new(manifest.meta.name.as_str());
    doc.environment = manifest.environment.clone();
    doc.camera = manifest.camera.clone();
    doc.avatar = manifest.avatar.clone();
    doc.tours = manifest.tours.clone();
    for entity in &manifest.entities {
        doc.apply(&EditOp::spawn(entity.clone()))
            .map_err(|e| e.to_string())?;
    }
    Ok(doc)
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

    let mut entries = Vec::new();
    let mut torn_lines = Vec::new();
    if let Some(log) = read("ops.jsonl") {
        for (n, line) in log.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(line) {
                Ok(entry) => entries.push(entry),
                Err(_) if n + 1 == log.lines().count() => torn_lines.push(n + 1), // torn tail
                Err(_) => torn_lines.push(n + 1),
            }
        }
    }

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

    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut has_parent: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (id, parent) in identified(&entries) {
        if let Some(parent) = &parent {
            children.entry(parent.clone()).or_default().push(id.clone());
            has_parent.insert(id.clone());
        }
    }
    let tips: Vec<String> = identified(&entries)
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| !has_parent.contains(id))
        .collect();

    let issues = wt::validate_manifest(&manifest, &wt::WorldLimits::default());
    let base = base_doc(&manifest)?;
    let fold = fold_log(&base, &entries).map_err(|e| e.to_string());

    let physics_world = physics_ref::collect_physics(&manifest);
    let physics = (!physics_world.dynamic.is_empty())
        .then(|| physics_ref::simulate(&manifest, &SimOptions::default()));

    Ok(Package {
        dir: dir.to_path_buf(),
        package_json,
        manifest,
        entries,
        torn_lines,
        state_initial,
        state_types,
        state_values,
        undeclared,
        issues,
        fold,
        base,
        children,
        tips,
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
                SessionOp::Merge(record) => format!("Merge · from {}", record.branch),
                SessionOp::Extension(record) => format!("Extension · {}", record.name),
            })
            .collect()
    }
}
