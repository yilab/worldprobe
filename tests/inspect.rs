//! The inspector inspects what the ecosystem actually ships: when the
//! sibling checkouts are present (the murky-swamp layout, or a local
//! clone arrangement), load their packages and prove the fold, the
//! history, and the state all read. CI with no siblings skips — the
//! packages' own repositories test them there; this test is for the
//! desk where they sit side by side.

use std::path::PathBuf;

fn sibling(relative: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
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

// The package module lives in the binary; the test needs it as a lib.
// Until it earns extraction (the whole point of this app), load via the
// same code path by including it.
// The UI reads op_summary and friends; the test target doesn't.
#[allow(dead_code)]
#[path = "../src/package.rs"]
mod package_lib;

fn crate_load(dir: &std::path::Path) -> package_lib::Package {
    package_lib::load(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
}
