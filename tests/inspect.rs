//! The inspector inspects what the ecosystem actually ships: when the
//! sibling checkouts are present (the murky-swamp layout, or a local
//! clone arrangement), load their packages and prove the fold, the
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
    let raised = diff_lib::diff_worlds(&world_at(&fork, None), &world_at(&fork, Some("e2")));
    assert_eq!(
        raised
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e.name.as_str(), e.change))
            .collect::<Vec<_>>(),
        vec![
            ("20", "checkpoint_flag", diff_lib::Change::Added),
            ("21", "banner", diff_lib::Change::Added),
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
        diff_lib::diff_worlds(&world_at(&fork, Some("e3")), &world_at(&fork, Some("e4")))
            .is_empty()
    );
    // The castle: main keeps the garden, the variant digs the moat.
    let castle = crate_load(&root.join("examples/forked-exploration"));
    let variant = diff_lib::diff_worlds(
        &world_at(&castle, Some("e3")),
        &world_at(&castle, Some("e5")),
    );
    assert_eq!(
        variant
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e.change))
            .collect::<Vec<_>>(),
        vec![
            ("102", diff_lib::Change::Removed),
            ("103", diff_lib::Change::Added),
        ]
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
        let head = package.head().unwrap_or_else(|| panic!("{pkg}: no head"));
        let written = serde_json::to_value(&package.manifest).unwrap();
        let diff = diff_lib::diff_worlds(&world_at(&package, Some(&head)), &written);
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

// The package module lives in the binary; the test needs it as a lib.
// Until it earns extraction (the whole point of this app), load via the
// same code path by including it.
// The UI reads op_summary and friends; the test target doesn't.
#[allow(dead_code)]
#[path = "../src/package.rs"]
mod package_lib;

// The diff module likewise: pure functions over JSON, included so the
// two sides of a fold can be compared here.
#[allow(dead_code)]
#[path = "../src/diff.rs"]
mod diff_lib;

fn crate_load(dir: &std::path::Path) -> package_lib::Package {
    package_lib::load(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
}
