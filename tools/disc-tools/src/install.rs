//! `disc-tools install`: put a pressing into the games library.
//!
//!   install --dist <dir> --name <disc name> --library <dir> --lineup <json>
//!           --edition Public|Private [--allow-dirty]
//!
//! The pressing is `<dist>/<name>.{bin,cue,components.json,release-receipt.json}`.
//! Each file is copied beside its destination under a `.tmp-` name (an APFS
//! clone, so it costs no disk), its sha256 is compared with the pressing's, and
//! only then are all five renamed over the library's copy together with a
//! provenance file naming the demo-disc commit and every lineup input. Nothing
//! else in the library is touched: older pressings are listed, not removed.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::args::Args;
use crate::disc::SECTOR_BYTES;
use crate::util::{dumps, git, read_json, repo_root, sha256_file, write_atomic, Error, Result};

const EXTENSIONS: [&str; 4] = ["bin", "cue", "components.json", "release-receipt.json"];

/// `2026-10-27T17:25:53Z` for a count of seconds since the Unix epoch.
fn iso_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// The provenance file: which commit pressed it, from which lineup, and every
/// program with the revision it was built from. The hardware tests row is
/// listed only when the pressing carried it.
fn provenance(
    game: &str,
    lineup_name: &str,
    commit: &str,
    branch: &str,
    bin_sha256: &str,
    cue_sha256: &str,
    sectors: u64,
    built: &str,
    lineup: &Value,
) -> Result<Value> {
    let programs = lineup
        .get("programs")
        .and_then(Value::as_array)
        .ok_or_else(|| Error("lineup has no 'programs'".into()))?;
    let inputs: Vec<Value> = programs
        .iter()
        .filter(|row| row.get("name").and_then(Value::as_str) != Some("HARDWARE TESTS"))
        .map(|row| {
            let field = |path: &[&str]| {
                let mut node = row;
                for key in path {
                    node = node.get(key).unwrap_or(&Value::Null);
                }
                node.clone()
            };
            json!({
                "name": field(&["name"]),
                "version": field(&["version"]),
                "bin_sha256": field(&["bin_sha256"]),
                "repository": field(&["source", "repository"]),
                "source_commit": field(&["source", "revision"]),
                "ref": field(&["source", "ref"]),
            })
        })
        .collect();
    Ok(json!({
        "game": game,
        "repository": "EBonura/PSoXide-demo-disc",
        "branch": branch,
        "source_commit": commit,
        "lineup": lineup_name,
        "bin_sha256": bin_sha256,
        "cue_sha256": cue_sha256,
        "sectors": sectors,
        "built": built,
        "build": "make lineup-disc from the landed main (see the release receipt beside the image). Every input is under the release inputs folder.",
        "inputs": inputs,
    }))
}

/// Copy `from` to `to`, as an APFS clone where the volume allows it.
fn clone_file(from: &Path, to: &Path) -> Result<()> {
    let cloned = Command::new("cp")
        .arg("-c")
        .arg(from)
        .arg(to)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !cloned {
        fs::copy(from, to)?;
    }
    Ok(())
}

/// Other demo-disc pressings of the same edition already in the library.
fn older_pressings(library: &Path, name: &str, private: bool) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(library) else {
        return found;
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = file.strip_prefix("PSoXide Demo Disc v") else {
            continue;
        };
        if !file.ends_with(".bin") || file.starts_with(&format!("{name}.")) {
            continue;
        }
        if file.contains(" Private.") == private {
            found.push(rest.trim_end_matches(".bin").to_string());
        }
    }
    found.sort();
    found
}

fn install(args: &Args) -> Result<()> {
    let dist = args.require_path("dist")?;
    let name = args.require("name")?;
    let library = args.require_path("library")?;
    let lineup_path = args.require_path("lineup")?;
    let edition = args.require("edition")?;
    ensure!(
        edition == "Public" || edition == "Private",
        "--edition is Public or Private"
    );
    ensure!(
        library.is_dir(),
        "library {} is not a directory",
        library.display()
    );

    let root = repo_root()?;
    let dirty = git(&root, &["status", "--porcelain", "--untracked-files=no"])?;
    ensure!(
        dirty.is_empty() || args.has("allow-dirty"),
        "the demo disc tree is not clean; commit (or pass --allow-dirty) so the provenance names a real commit"
    );
    let commit = git(&root, &["rev-parse", "--short=12", "HEAD"])?;
    let branch = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;

    // Copy and verify everything before replacing anything.
    let mut staged = Vec::new();
    for extension in EXTENSIONS {
        let source = dist.join(format!("{name}.{extension}"));
        ensure!(source.is_file(), "{} is missing", source.display());
        let temporary = library.join(format!(".tmp-{name}.{extension}"));
        let _ = fs::remove_file(&temporary);
        clone_file(&source, &temporary)?;
        let (want, got) = (sha256_file(&source)?, sha256_file(&temporary)?);
        if want != got {
            let _ = fs::remove_file(&temporary);
            bail!("{name}.{extension}: the copy hashes to {got}, the pressing to {want}");
        }
        staged.push((extension, temporary));
    }
    let bin = &staged[0].1;
    let sectors = fs::metadata(bin)?.len() / SECTOR_BYTES;
    let lineup = read_json(&lineup_path)?;
    let lineup_name = format!(
        "release/{}",
        lineup_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error(e.to_string()))?
        .as_secs();
    let document = provenance(
        &format!("{} ({edition})", name.trim_end_matches(" Private")),
        &lineup_name,
        &commit,
        &branch,
        &sha256_file(bin)?,
        &sha256_file(&staged[1].1)?,
        sectors,
        &iso_utc(seconds),
        &lineup,
    )?;
    let temporary = library.join(format!(".tmp-{name}.provenance.json"));
    write_atomic(&temporary, &(dumps(&document, 2, false) + "\n"))?;
    staged.push(("provenance.json", temporary));

    for (extension, temporary) in &staged {
        fs::rename(temporary, library.join(format!("{name}.{extension}")))?;
    }
    println!(
        "installed {name}: bin {} sectors {sectors} commit {commit}",
        sha256_file(&library.join(format!("{name}.bin")))?
    );
    let older = older_pressings(&library, name, edition == "Private");
    if !older.is_empty() {
        println!(
            "older {edition} pressings still in {}: {} (the library keeps the newest only; move them to the Trash)",
            library.display(),
            older.join(", ")
        );
    }
    Ok(())
}

pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(
        raw,
        &["dist", "name", "library", "lineup", "edition"],
        &["allow-dirty"],
    )?;
    install(&args)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_come_out_right() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_792_000_000), "2026-10-14T17:46:40Z");
        // Leap day.
        assert_eq!(iso_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn provenance_lists_every_input_but_the_hardware_tests() {
        let lineup = json!({"programs": [
            {"name": "G", "version": "1", "bin_sha256": "aa",
             "source": {"repository": "r", "revision": "abc", "ref": "main"}},
            {"name": "HARDWARE TESTS", "version": "2", "bin_sha256": "bb", "source": {}}
        ]});
        let document = provenance(
            "Disc (Public)",
            "release/l.json",
            "c0ffee",
            "main",
            "b",
            "c",
            7,
            "t",
            &lineup,
        )
        .unwrap();
        let inputs = document["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0]["source_commit"], json!("abc"));
        assert_eq!(document["sectors"], json!(7));
    }

    #[test]
    fn older_pressings_are_found_by_edition_and_never_include_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "PSoXide Demo Disc v0.51.bin",
            "PSoXide Demo Disc v0.52.bin",
            "PSoXide Demo Disc v0.51 Private.bin",
            "PSoXide Demo Disc v0.52 Private.bin",
            "Quake (quake-psx).bin",
        ] {
            fs::write(dir.path().join(file), b"x").unwrap();
        }
        assert_eq!(
            older_pressings(dir.path(), "PSoXide Demo Disc v0.52", false),
            ["0.51"]
        );
        assert_eq!(
            older_pressings(dir.path(), "PSoXide Demo Disc v0.52 Private", true),
            ["0.51 Private"]
        );
    }
}
