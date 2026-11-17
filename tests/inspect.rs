//! The inspector inspects what the ecosystem actually ships: when the
//! sibling checkouts are present (cloned side by side in one folder, as
//! the README describes), load their packages and prove the fold, the
//! history, and the state all read. CI with no siblings skips — the
//! packages' own repositories test them there; this test is for the
//! desk where they sit side by side.

use std::path::PathBuf;

fn sibling(relative: &str) -> Option<PathBuf> {
    // The siblings sit next to this repository (`../songworld`,
    // `../openworldformat`), as the README says.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative);
    path.is_dir().then_some(path)
}

#[test]
fn inspects_the_songworld_packages() {
    let Some(root) = sibling("songworld") else {
        return;
    };
    for pkg in ["world", "world-recording"] {
        let package = crate_load(&root.join(pkg));
        assert!(package.fold.is_ok(), "{pkg}: fold refused");
        assert!(package.entries.len() >= 3, "{pkg}: no session");
        assert_eq!(
            package.state_values.get("plays.count"),
            Some(&serde_json::json!(1))
        );
        assert!(
            package.manifest.entities.len() >= 15,
            "{pkg}: world too small to be the demo"
        );
    }
}

#[test]
fn inspects_the_spec_examples() {
    let Some(root) = sibling("openworldformat") else {
        return;
    };
    for pkg in [
        "examples/hello-world",
        "examples/speedrun-fork",
        "examples/the-drop-test",
    ] {
        let package = crate_load(&root.join(pkg));
        assert!(package.fold.is_ok(), "{pkg}: fold refused");
    }
    // The fork: two tips, fold either.
    let fork = crate_load(&root.join("examples/speedrun-fork"));
    assert_eq!(fork.tips, vec!["e3", "e4"]);
    assert!(fork.fold_to("e3").is_ok());
    assert!(fork.fold_to("e4").is_ok());
    // The drop test's physics: bodies declared, contact recorded.
    let drop_test = crate_load(&root.join("examples/the-drop-test"));
    let Some(sim) = &drop_test.physics else {
        panic!("the-drop-test declares no bodies");
    };
    assert!(
        sim.contacts
            .iter()
            .any(|c| c.body == "ball" && c.other == "switch_pad")
    );
}

#[test]
fn diffs_the_spec_examples_history() {
    let Some(root) = sibling("openworldformat") else {
        return;
    };
    let world_at = |package: &package_lib::Package, tip: Option<&str>| -> serde_json::Value {
        let manifest = match tip {
            None => package.base.to_manifest(),
            Some(tip) => package
                .fold_to(tip)
                .unwrap_or_else(|e| panic!("{tip}: {e}"))
                .0
                .to_manifest(),
        };
        serde_json::to_value(manifest).unwrap()
    };
    // The fork: base → e2 raises the flag and the banner; the two runs
    // after it change no entity.
    let fork = crate_load(&root.join("examples/speedrun-fork"));
    let raised = diff::diff_worlds(&world_at(&fork, None), &world_at(&fork, Some("e2")));
    assert_eq!(
        raised
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e.name.as_str(), e.change))
            .collect::<Vec<_>>(),
        vec![
            ("20", "checkpoint_flag", diff::Change::Added),
            ("21", "banner", diff::Change::Added),
        ]
    );
    assert_eq!(
        raised
            .world
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        vec!["next_entity_id"]
    );
    assert!(
        diff::diff_worlds(&world_at(&fork, Some("e3")), &world_at(&fork, Some("e4"))).is_empty()
    );
    // The castle: main keeps the garden, the variant digs the moat.
    let castle = crate_load(&root.join("examples/forked-exploration"));
    let variant = diff::diff_worlds(
        &world_at(&castle, Some("e3")),
        &world_at(&castle, Some("e5")),
    );
    assert_eq!(
        variant
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e.change))
            .collect::<Vec<_>>(),
        vec![("102", diff::Change::Removed), ("103", diff::Change::Added),]
    );
    // Every example's fold to main is its manifest.json, read the way
    // the spec's equality reads them (spec/package.md, "Head-first").
    for pkg in [
        "examples/hello-world",
        "examples/speedrun-fork",
        "examples/forked-exploration",
        "examples/the-drop-test",
    ] {
        let package = crate_load(&root.join(pkg));
        let head = package
            .head
            .clone()
            .unwrap_or_else(|| panic!("{pkg}: no head"));
        let written = serde_json::to_value(&package.manifest).unwrap();
        let diff = diff::diff_worlds(&world_at(&package, Some(&head)), &written);
        assert!(
            diff.is_empty(),
            "{pkg}: the fold to {head} is not manifest.json: {diff:#?}"
        );
    }
}

#[test]
fn lists_a_behavior_whose_target_is_gone() {
    // The crate's validation (openworldformat 0.3.6) reaches the issues
    // list: a `look_at` whose target is not in the world is a warning, not
    // a refusal — the world still folds.
    let Some(root) = sibling("openworldformat") else {
        return;
    };
    let mut manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("examples/hello-world/manifest.json"))
            .expect("the spec's hello-world manifest"),
    )
    .expect("hello-world parses");
    let id = manifest["next_entity_id"].as_u64().unwrap_or(500);
    let gone = id + 1;
    let mut watcher = manifest["entities"][0].clone();
    watcher["id"] = serde_json::json!(id);
    watcher["name"] = serde_json::json!("watcher");
    watcher["behaviors"] = serde_json::json!([{"LookAt": {"target": gone}}]);
    manifest["entities"]
        .as_array_mut()
        .expect("entities")
        .push(watcher);
    manifest["next_entity_id"] = serde_json::json!(id + 1);

    let dir = std::env::temp_dir().join(format!("worldprobe-dangling-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch package dir");
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
    )
    .expect("manifest.json");
    std::fs::write(dir.join("ops.jsonl"), b"").expect("ops.jsonl");
    let package = crate_load(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        package.fold.is_ok(),
        "a dangling target does not refuse the fold"
    );
    assert!(
        package.issues.iter().any(|i| {
            matches!(i.severity, openworldformat::Severity::Warning)
                && i.message.contains("look_at")
                && i.message.contains(&gone.to_string())
        }),
        "no dangling-target warning among {:?}",
        package.issues
    );
}

#[test]
fn keeps_a_behavior_kind_the_crate_does_not_know() {
    // Since openworldformat 0.5.0 a behavior kind the crate does not know
    // reads as `BehaviorDef::Other`, folds through and writes back as it
    // came, where it used to refuse the whole manifest — so the inspector
    // opens a world from a producer whose behaviors it has never heard of.
    let Some(root) = sibling("openworldformat") else {
        return;
    };
    let mut manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("examples/hello-world/manifest.json"))
            .expect("the spec's hello-world manifest"),
    )
    .expect("hello-world parses");
    let id = manifest["next_entity_id"].as_u64().unwrap_or(500);
    let wobble = serde_json::json!({"Wobble": {"how": 3}});
    let mut wobbler = manifest["entities"][0].clone();
    wobbler["id"] = serde_json::json!(id);
    wobbler["name"] = serde_json::json!("wobbler");
    wobbler["behaviors"] = serde_json::json!([wobble]);
    manifest["entities"]
        .as_array_mut()
        .expect("entities")
        .push(wobbler);
    manifest["next_entity_id"] = serde_json::json!(id + 1);

    let dir = std::env::temp_dir().join(format!("worldprobe-unknown-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch package dir");
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
    )
    .expect("manifest.json");
    std::fs::write(dir.join("ops.jsonl"), b"").expect("ops.jsonl");
    let package = crate_load(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        package.fold.is_ok(),
        "an unknown kind does not refuse the fold"
    );
    let kept = package
        .manifest
        .entities
        .iter()
        .find(|e| e.name.0 == "wobbler")
        .expect("the wobbler is in the manifest");
    assert_eq!(
        serde_json::to_value(&kept.behaviors).expect("behaviors serialize"),
        serde_json::json!([wobble]),
        "the kind writes back as it came"
    );
}

/// A package written here: an empty world as its own base, the log given,
/// and `package.json` when given. No sibling needed.
fn scratch(name: &str, log: &str, package_json: Option<&str>) -> PathBuf {
    // Its own prefix: the tests run in parallel in one process, and the
    // older ones above make directories of their own.
    let dir =
        std::env::temp_dir().join(format!("worldprobe-scratch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch package dir");
    let manifest = openworldformat::WorldDoc::new("probe").to_manifest();
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&manifest).expect("manifest serializes"),
    )
    .expect("manifest.json");
    std::fs::write(dir.join("ops.jsonl"), log).expect("ops.jsonl");
    if let Some(text) = package_json {
        std::fs::write(dir.join("package.json"), text).expect("package.json");
    }
    dir
}

/// One history-only entry: it folds to nothing, so the world needs no
/// base snapshot.
fn entry(revision: u64) -> String {
    format!("{{\"revision\":{revision},\"ops\":[]}}\n")
}

#[test]
fn reads_the_log_as_the_spec_says() {
    // A torn last line loses only itself, and is counted.
    let torn = scratch(
        "torn",
        &format!("{}{}{{\"revision\":3,\"op", entry(1), entry(2)),
        None,
    );
    let package = crate_load(&torn);
    let _ = std::fs::remove_dir_all(&torn);
    assert_eq!(package.entries.len(), 2);
    assert_eq!(package.torn_line, Some(3));
    assert!(package.corrupt.is_none());
    assert!(package.fold.is_ok());

    // A bad line before the last is corruption: the entries above it are
    // shown and the rest is not folded around it, as a hand-split reader
    // used to.
    let corrupt = scratch(
        "corrupt",
        &format!("{}{{\"revision\":2,\"op\n{}", entry(1), entry(3)),
        None,
    );
    let package = crate_load(&corrupt);
    let _ = std::fs::remove_dir_all(&corrupt);
    assert_eq!(
        package.entries.len(),
        1,
        "only the entry above the bad line"
    );
    assert_eq!(package.torn_line, None);
    let line = package.corrupt.as_ref().expect("the corruption is named");
    assert_eq!(line.line, 2);
    assert!(
        line.to_string()
            .starts_with("line 2 of the log doesn't parse"),
        "{line}"
    );
}

#[test]
fn the_head_is_mains_tip_and_a_dangling_main_says_so() {
    // main names an entry the log doesn't hold: the head is what main
    // says, and the fold to it refuses, naming it — not the last entry,
    // quietly, as a hand-rolled head used to.
    let dangling = scratch(
        "main-elsewhere",
        &format!("{}{}", entry(1), entry(2)),
        Some(r#"{"format_version": 2, "name": "probe", "refs": {"main": "e9"}}"#),
    );
    let package = crate_load(&dangling);
    let _ = std::fs::remove_dir_all(&dangling);
    assert_eq!(package.head.as_deref(), Some("e9"));
    let refused = package.fold.as_ref().expect_err("the fold refuses");
    assert!(refused.contains("e9"), "{refused}");

    // A parent the log doesn't hold: not a tree, and the fold refuses.
    let orphan = scratch(
        "orphan",
        "{\"revision\":1,\"ops\":[],\"id\":\"e1\"}\n{\"revision\":2,\"ops\":[],\"id\":\"e2\",\"parent\":\"gone\"}\n",
        None,
    );
    let package = crate_load(&orphan);
    let _ = std::fs::remove_dir_all(&orphan);
    assert!(
        matches!(&package.history, Err(openworldformat::HistoryError::UnknownParent { parent, .. }) if parent == "gone"),
        "{:?}",
        package.history
    );
    assert!(package.tips.is_empty());
    assert!(package.fold.is_err());

    // The spec's castle: main keeps the garden, the variant digs the moat.
    // The fold is main's path, never both branches in one world.
    let Some(root) = sibling("openworldformat") else {
        return;
    };
    let castle = crate_load(&root.join("examples/forked-exploration"));
    assert_eq!(castle.head.as_deref(), Some("e3"));
    let head = castle.fold.as_ref().expect("the castle folds");
    assert!(head.get(102).is_some(), "the garden is on main");
    assert!(head.get(103).is_none(), "the moat is on the variant only");
}

#[test]
fn every_sibling_package_agrees_with_itself() {
    // What the ecosystem ships: each head is the fold to main, and each
    // file holds the bytes package.json names.
    let mut packages = Vec::new();
    if let Some(root) = sibling("openworldformat") {
        for pkg in [
            "examples/hello-world",
            "examples/speedrun-fork",
            "examples/forked-exploration",
            "examples/the-drop-test",
        ] {
            packages.push(root.join(pkg));
        }
    }
    if let Some(root) = sibling("songworld") {
        packages.push(root.join("world"));
        packages.push(root.join("world-recording"));
    }
    for dir in packages {
        let package = crate_load(&dir);
        let drift = package
            .head_drift
            .as_ref()
            .unwrap_or_else(|| panic!("{}: the fold refused", dir.display()));
        assert!(drift.is_empty(), "{}: {drift:#?}", dir.display());
        assert!(
            !package.integrity.is_empty(),
            "{}: no hashes recorded",
            dir.display()
        );
        for check in &package.integrity {
            assert!(check.holds(), "{}: {check:?}", dir.display());
        }
    }
}

#[test]
fn a_package_that_disagrees_with_itself_says_where() {
    // The history holds nothing, yet manifest.json has a lamp the base
    // doesn't; package.json names bytes neither file holds, and a log
    // that isn't there.
    let dir = scratch(
        "disagrees",
        "",
        Some(r#"{"world_sha256": "00", "log_sha256": "00"}"#),
    );
    std::fs::remove_file(dir.join("ops.jsonl")).expect("no log after all");
    std::fs::create_dir_all(dir.join("snapshots")).expect("snapshots/");
    std::fs::write(
        dir.join(openworldformat::package::BASE_SNAPSHOT),
        serde_json::to_vec(&openworldformat::WorldDoc::new("probe").to_manifest())
            .expect("base serializes"),
    )
    .expect("the base snapshot");
    let mut head = openworldformat::WorldDoc::new("probe");
    head.apply(&openworldformat::EditOp::spawn(
        openworldformat::WorldEntity::new(1, "lamp"),
    ))
    .expect("a lamp");
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&head.to_manifest()).expect("manifest serializes"),
    )
    .expect("manifest.json");
    let package = crate_load(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    let drift = package
        .head_drift
        .as_ref()
        .expect("the fold holds, so the head is checked");
    assert_eq!(drift.count(diff::Change::Added), 1, "{drift:#?}");
    assert_eq!(drift.entities[0].name, "lamp");
    let world = package
        .integrity
        .iter()
        .find(|check| check.key == "world_sha256")
        .expect("world_sha256 is recorded");
    assert!(!world.holds() && world.actual.is_some(), "{world:?}");
    let log = package
        .integrity
        .iter()
        .find(|check| check.key == "log_sha256")
        .expect("log_sha256 is recorded");
    assert!(!log.holds() && log.actual.is_none(), "{log:?}");
}

// The package module lives in the binary; the test needs it as a lib.
// Until it earns extraction (the whole point of this app), load via the
// same code path by including it.
// The UI reads op_summary and friends; the test target doesn't.
#[allow(dead_code)]
#[path = "../src/package.rs"]
mod package_lib;

// The diff module likewise: pure functions over JSON, included so the
// two sides of a fold can be compared here — and named `diff`, as in the
// binary, because the package module reads the head check through it.
#[allow(dead_code)]
#[path = "../src/diff.rs"]
mod diff;

fn crate_load(dir: &std::path::Path) -> package_lib::Package {
    package_lib::load(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
}
