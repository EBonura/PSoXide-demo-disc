//! The newest lineups can be re-pressed from `release-inputs` alone.
//!
//! A lineup that names a file in a lane directory breaks the day that lane is
//! cleaned (the v0.50 Arcade, Celeste and Quake rows did, and the large images
//! under release-inputs themselves were deleted once). So the newest Public
//! and Private lineup must keep every input and every source mirror under the
//! inputs root and hash-gate each one. `disc-tools lineup check` does the
//! work; this runs it on the files that would be pressed next.
//!
//! The newest lineup of each edition gets the full `--exists --hashes` check when
//! its inputs are present under the inputs root. A lineup whose inputs are all
//! gone (a pressing whose images were cleaned away for good) only gets the
//! structural check, and the test prints which inputs are missing. It still
//! fails when an input is present and its hash differs, and when only some of a
//! lineup's programs are present (a half-cleaned root is a defect, not history).
//! `RETIRED` names the lineups whose images are known to be gone for good.

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

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Lineups whose inputs were deleted for good and that nothing will restore. They get the
/// structural check only, and the missing inputs are printed. Add a file here only with the
/// reason, and never edit the lineup itself (it is the record of what was pressed).
const RETIRED: &[(&str, &str)] = &[(
    "lineup-v0.53.json",
    "the v0.52 disc, NitroXide.bin and Quake bin are gone; see release/README.txt (v0.53 inputs)",
)];

#[derive(Debug, PartialEq)]
enum Verdict {
    /// Every input present and every hash held.
    Full,
    /// No input present, or retired: structure only. Carries the missing lines.
    StructureOnly(Vec<String>),
}

fn disc_tools(args: &[&str], lineup: &std::path::Path, root: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_disc-tools"))
        .arg("lineup")
        .arg("check")
        .args(args)
        .arg("--lineup")
        .arg(lineup)
        .args(["--inputs-root", root])
        .output()
        .unwrap()
}

fn program_names(lineup: &std::path::Path) -> BTreeSet<String> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(lineup).unwrap()).unwrap();
    value["programs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap().to_string())
        .collect()
}

/// Judge one lineup against `root`. `retired` lineups skip the presence rule.
fn judge(lineup: &std::path::Path, root: &str, retired: bool) -> Result<Verdict, String> {
    let structure = disc_tools(&[], lineup, root);
    if !structure.status.success() {
        return Err(format!(
            "{}: structure: {}{}",
            lineup.display(),
            String::from_utf8_lossy(&structure.stdout),
            String::from_utf8_lossy(&structure.stderr)
        ));
    }
    let full = disc_tools(&["--exists", "--hashes"], lineup, root);
    if full.status.success() {
        return Ok(Verdict::Full);
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&full.stdout),
        String::from_utf8_lossy(&full.stderr)
    );
    let Some((_, list)) = text.split_once("lineup inputs are missing:") else {
        // Present and wrong (a hash, a cue): never excused.
        return Err(format!("{}: {text}", lineup.display()));
    };
    let lines: Vec<String> = list.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let absent: BTreeSet<String> = lines
        .iter()
        .filter_map(|l| l.split_once(": ").map(|(n, _)| n.to_string()))
        .collect();
    let all = program_names(lineup);
    if retired || absent == all {
        return Ok(Verdict::StructureOnly(lines));
    }
    Err(format!(
        "{}: only some inputs are present under {root}; missing for {:?}:\n  {}",
        lineup.display(),
        absent,
        lines.join("\n  ")
    ))
}

fn check(lineup: PathBuf) {
    let name = lineup.file_name().unwrap().to_string_lossy().into_owned();
    let retired = RETIRED.iter().find(|(file, _)| *file == name);
    match judge(&lineup, &inputs_root(), retired.is_some()) {
        Ok(Verdict::Full) => {}
        Ok(Verdict::StructureOnly(missing)) => eprintln!(
            "{name}: inputs absent under the inputs root, structure checked only{}\n  {}",
            retired.map(|(_, why)| format!(" (retired: {why})")).unwrap_or_default(),
            missing.join("\n  ")
        ),
        Err(problem) => panic!("{problem}"),
    }
}

#[test]
fn the_newest_public_lineup_is_self_contained() {
    check(newest(false));
}

#[test]
fn the_newest_private_lineup_is_self_contained() {
    check(newest(true));
}

/// A two-program lineup under a temporary root. Returns (root, lineup file).
fn fixture(present: &[bool; 2], bad_hash: bool) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("src/mirror.git")).unwrap();
    let mut rows = Vec::new();
    for (index, here) in present.iter().enumerate() {
        let bin = format!("input{index}.bin");
        let cue = root.join(format!("input{index}.cue"));
        let data = vec![index as u8 + 1; 64];
        let mut digest = format!("{:x}", Sha256::digest(&data));
        if bad_hash {
            digest = "0".repeat(64);
        }
        let cue_text = format!("FILE \"{bin}\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n");
        let cue_digest = format!("{:x}", Sha256::digest(cue_text.as_bytes()));
        if *here {
            fs::write(root.join(&bin), &data).unwrap();
            fs::write(&cue, &cue_text).unwrap();
        }
        rows.push(serde_json::json!({
            "name": format!("PROGRAM {index}"), "var": "X", "version_var": "V", "kind": "image",
            "bin_sha256": digest, "cue": cue.to_string_lossy(), "cue_sha256": cue_digest,
            "source": {"local": root.join("src/mirror.git").to_string_lossy(), "revision": "0"},
        }));
    }
    let lineup = root.join("lineup.json");
    fs::write(&lineup, serde_json::to_vec(&serde_json::json!({"schema": 1, "programs": rows})).unwrap()).unwrap();
    (dir, lineup)
}

fn root_of(lineup: &std::path::Path) -> String {
    lineup.parent().unwrap().to_string_lossy().into_owned()
}

#[test]
fn a_lineup_with_every_input_present_gets_the_full_check() {
    let (_dir, lineup) = fixture(&[true, true], false);
    assert_eq!(judge(&lineup, &root_of(&lineup), false), Ok(Verdict::Full));
}

#[test]
fn a_lineup_with_no_input_present_is_checked_for_structure_and_names_what_is_missing() {
    let (_dir, lineup) = fixture(&[false, false], false);
    match judge(&lineup, &root_of(&lineup), false) {
        Ok(Verdict::StructureOnly(missing)) => {
            assert!(missing.iter().any(|l| l.starts_with("PROGRAM 0:")));
            assert!(missing.iter().any(|l| l.starts_with("PROGRAM 1:")));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_lineup_with_only_some_inputs_present_fails() {
    let (_dir, lineup) = fixture(&[true, false], false);
    let error = judge(&lineup, &root_of(&lineup), false).unwrap_err();
    assert!(error.contains("only some inputs are present"), "{error}");
    assert!(error.contains("PROGRAM 1"), "{error}");
}

#[test]
fn a_retired_lineup_with_some_inputs_present_is_structure_only() {
    let (_dir, lineup) = fixture(&[true, false], false);
    assert!(matches!(
        judge(&lineup, &root_of(&lineup), true),
        Ok(Verdict::StructureOnly(_))
    ));
}

#[test]
fn a_present_input_with_a_wrong_hash_fails_even_when_retired() {
    let (_dir, lineup) = fixture(&[true, true], true);
    for retired in [false, true] {
        let error = judge(&lineup, &root_of(&lineup), retired).unwrap_err();
        assert!(error.contains("lineup says"), "{error}");
    }
}

#[test]
fn a_lineup_that_leans_on_a_path_outside_the_root_fails_the_structure_check() {
    let (_dir, lineup) = fixture(&[true, true], false);
    let error = judge(&lineup, "/nonexistent-inputs-root", false).unwrap_err();
    assert!(error.contains("structure"), "{error}");
}
