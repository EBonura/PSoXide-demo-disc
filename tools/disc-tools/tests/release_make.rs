//! The Makefile has one way to press a disc, the lineup, and one target per
//! release channel.
//!
//! The non-lineup `make disc` path (every program rebuilt from the submodule
//! pins, Quake and Cortex pin gates, `cortex_bakes_only_the_current_project`)
//! was retired on 2026-10-29. These tests read the target list `make` itself
//! reports, so a target added back by accident, or a channel target lost, fails
//! here rather than at the next release.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every explicit target in the Makefile's rule database.
fn targets() -> BTreeSet<String> {
    let output = Command::new("make")
        .args(["-pqRr", "-f", "Makefile", "--no-print-directory"])
        .current_dir(repo_root())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    let mut found = BTreeSet::new();
    let mut in_files = false;
    for line in text.lines() {
        if line.starts_with("# Files") {
            in_files = true;
            continue;
        }
        if !in_files || line.starts_with('#') || line.starts_with('\t') || line.starts_with(' ') {
            continue;
        }
        if let Some((name, _)) = line.split_once(':') {
            if !name.is_empty()
                && !name.contains(['%', '=', ' ', '$', '/'])
                && !name.starts_with('.')
            {
                found.insert(name.to_string());
            }
        }
    }
    found
}

#[test]
fn the_non_lineup_path_is_gone() {
    let targets = targets();
    for retired in [
        "disc",
        "disc-only",
        "programs",
        "quake-programs",
        "quake-programs-verify",
        "quake-verify",
        "quake-repin",
        "quake-headless-check",
        "cortex-if-stale",
        "cortex-current-if-stale",
        "cortex-symbol-check",
        "cs-program",
        "hk-program",
        "release-frontend",
        "release-headless-check",
        "release-web",
        "itch",
        "check-locks",
        "sdk-coherence",
    ] {
        assert!(
            !targets.contains(retired),
            "{retired} is a retired target and is back in the Makefile"
        );
    }
}

#[test]
fn every_release_channel_has_its_target() {
    let targets = targets();
    for needed in [
        "lineup-disc",
        "lineup-prepare",
        "lineup-check",
        "lineup-budget",
        "release-inputs-check",
        "press-public",
        "press-private",
        "gates-public",
        "gates-private",
        "install-public",
        "install-private",
        "release-public-dry-run",
        "release-public",
        "check",
    ] {
        assert!(
            targets.contains(needed),
            "Makefile lost the {needed} target"
        );
    }
}

#[test]
fn the_wrapper_scripts_are_gone() {
    for script in ["release-public.sh", "tools/check-locks.sh"] {
        assert!(
            !repo_root().join(script).exists(),
            "{script} was replaced by Make targets"
        );
    }
}
