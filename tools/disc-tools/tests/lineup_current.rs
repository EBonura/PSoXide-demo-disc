//! The newest lineups can be re-pressed from `release-inputs` alone.
//!
//! A lineup that names a file in a lane directory breaks the day that lane is
//! cleaned (the v0.50 Arcade, Celeste and Quake rows did, and the large images
//! under release-inputs themselves were deleted once). So the newest Public
//! and Private lineup must keep every input and every source mirror under the
//! inputs root and hash-gate each one. `disc-tools lineup check` does the
//! work; this runs it on the files that would be pressed next.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The inputs root the lineups name: `$RELEASE_INPUTS`, else the campaign's.
fn inputs_root() -> String {
    std::env::var("RELEASE_INPUTS")
        .unwrap_or_else(|_| "~/Desktop/repos/campaign-2026-10-15/release-inputs".into())
}

/// `release/lineup-v0.52.json` and `-private.json` with the highest version.
fn newest(private: bool) -> PathBuf {
    let suffix = if private { "-private.json" } else { ".json" };
    let mut best: Option<((u32, u32), PathBuf)> = None;
    for entry in fs::read_dir(repo_root().join("release")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(version) = name
            .strip_prefix("lineup-v")
            .and_then(|rest| rest.strip_suffix(suffix))
        else {
            continue;
        };
        // Only plain pressings: "0.52", not "0.47-private-diag" or "collection-v0.1".
        let Some((major, minor)) = version.split_once('.') else {
            continue;
        };
        let (Ok(major), Ok(minor)) = (major.parse::<u32>(), minor.parse::<u32>()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(seen, _)| (major, minor) > *seen) {
            best = Some(((major, minor), path));
        }
    }
    best.expect("a lineup file").1
}

fn check(lineup: PathBuf) {
    let output = Command::new(env!("CARGO_BIN_EXE_disc-tools"))
        .args(["lineup", "check", "--exists", "--hashes", "--lineup"])
        .arg(&lineup)
        .args(["--inputs-root", &inputs_root()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}: {}{}",
        lineup.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_newest_public_lineup_is_self_contained() {
    check(newest(false));
}

#[test]
fn the_newest_private_lineup_is_self_contained() {
    check(newest(true));
}
