//! One loaded `.world` package: everything the inspector shows, folded
//! through the format's own Rust reference — this app adds no format
//! semantics of its own, it points them at a directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use localgpt_world_sync::WorldDoc;
use localgpt_world_sync::oplog::OpLogEntry;
use localgpt_world_sync::session::{SessionOp, fold_log, fold_path};
use localgpt_world_types as wt;
use wt::EditOp;

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
    pub physics: Option<localgpt_world_physics::Simulation>,
    /// Bodies the world declares (for the physics header).
    pub physics_world: localgpt_world_physics::PhysicsWorld,
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

/// Fold the state: declared fields set (null resets the initial),
/// map subkeys set and remove, undeclared keys carried and named.
/// The whole log, linear — per-tip folding is the branch view's fold.
fn fold_state(
    initial: &BTreeMap<String, serde_json::Value>,
    types: &BTreeMap<String, String>,
    entries: &[OpLogEntry],
) -> (BTreeMap<String, serde_json::Value>, Vec<String>) {
    let mut values = initial.clone();
    let mut undeclared: Vec<String> = Vec::new();
    for entry in entries {
        for op in &entry.ops {
            let SessionOp::State(record) = op else {
                continue;
            };
            for (key, value) in &record.state {
                if types.contains_key(key) {
                    match value {
                        serde_json::Value::Null => {
                            values.insert(key.clone(), initial[key].clone());
                        }
                        v => {
                            values.insert(key.clone(), v.clone());
                        }
                    }
                    continue;
                }
                if let Some((base, inner)) = key.split_once('.')
                    && types.get(base).map(String::as_str) == Some("map")
                    && let Some(serde_json::Value::Object(map)) = values.get_mut(base)
                {
                    match value {
                        serde_json::Value::Null => {
                            map.remove(inner);
                        }
                        v => {
                            map.insert(inner.to_string(), v.clone());
                        }
                    }
                    continue;
                }
                match value {
                    serde_json::Value::Null => {
                        values.remove(key);
                    }
                    v => {
                        values.insert(key.clone(), v.clone());
                        if !undeclared.contains(key) {
                            undeclared.push(key.clone());
                        }
                    }
                }
            }
        }
    }
    (values, undeclared)
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

    let (state_initial, state_types) = read("state.json")
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .map(|doc| {
            let mut initial = BTreeMap::new();
            let mut types = BTreeMap::new();
            if let Some(fields) = doc.get("fields").and_then(|f| f.as_object()) {
                for (name, field) in fields {
                    if let Some(kind) = field.get("type").and_then(|t| t.as_str()) {
                        types.insert(name.clone(), kind.to_string());
                        let default = match kind {
                            "int" | "float" => serde_json::json!(0),
                            "bool" => serde_json::json!(false),
                            "string" => serde_json::json!(""),
                            "map" => serde_json::json!({}),
                            "list" => serde_json::json!([]),
                            _ => serde_json::Value::Null,
                        };
                        initial.insert(
                            name.clone(),
                            field.get("initial").cloned().unwrap_or(default),
                        );
                    }
                }
            }
            (initial, types)
        })
        .unwrap_or_default();

    let (state_values, undeclared) = fold_state(&state_initial, &state_types, &entries);

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

    let physics_world = localgpt_world_physics::collect_physics(&manifest);
    let physics = (!physics_world.dynamic.is_empty()).then(|| {
        localgpt_world_physics::simulate(&manifest, &localgpt_world_physics::SimOptions::default())
    });

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
