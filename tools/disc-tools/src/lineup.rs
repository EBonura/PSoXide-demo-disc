//! Press the disc from already built programs named in a lineup file.
//!
//! `make disc` rebuilds every program from this repo's submodules. A lineup
//! pressing instead takes each program exactly as it was built for the PS1
//! games library (or from a named build directory), because those are the
//! builds that were played and checked. The lineup file names every input with
//! its sha256, its source revision and the receipt of the build that made it;
//! nothing is pressed from a file whose hash moved.
//!
//!   prepare  verify every input, derive the few that need it into --out, and
//!            write --out/lineup.mk, which points the Makefile's program and
//!            version variables at them
//!   refresh  take the newest library build of every row that names one,
//!            copy it into the lineup inputs and rewrite its hashes
//!   receipt  after mkdisc: prove each pressed entry is its lineup input and
//!            write the release receipt and the component receipt
//!
//! Derivations, all byte-for-byte slices of a hashed input:
//!   data_track_of  the image's data track alone (its CD-DA is borrowed)
//!   boot_exe_of    the image's SYSTEM.CNF boot EXE, for a bare --game entry
//!   from_disc      a data image lifted out of an earlier pressing

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::args::Args;
use crate::disc::{
    self, cue_file_names, find_boot_exe, parse_exe_at, parse_toc, read_user_sector, SECTOR_BYTES,
};
use crate::receipt::{check_no_overlap, data_track_sectors, program_record};
use crate::util::{
    command_output, expand_home, file_record, fnv1a32, git, repo_root, resolve, sha256_file,
    write_receipt, Error, Result,
};

fn check_hash(path: &Path, expected: Option<&str>, what: &str) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = sha256_file(path)?;
    ensure!(
        actual == expected,
        "{what}: {} is {actual}, lineup says {expected}",
        path.display()
    );
    Ok(())
}

/// The single BIN a source cue names, resolved. Looser than the pressing's own
/// cue rules: a lineup input only has to name one file.
fn cue_bin(cue: &Path) -> Result<PathBuf> {
    let text = fs::read(cue)?;
    ensure!(text.is_ascii(), "{}: not ASCII", cue.display());
    let names = cue_file_names(&String::from_utf8_lossy(&text));
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    ensure!(
        unique.len() == 1,
        "{}: expected one FILE line",
        cue.display()
    );
    resolve(&cue.parent().unwrap_or(Path::new(".")).join(&names[0]))
}

fn slug(name: &str) -> String {
    let squash = Regex::new(r"[^a-z0-9]+").expect("static regex");
    squash
        .replace_all(&name.to_lowercase(), "-")
        .trim_matches('-')
        .to_string()
}

/// Read `size` bytes from an ISO9660 file extent.
fn read_extent(stream: &mut File, lba: u64, size: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    for i in 0..size.div_ceil(disc::USER_DATA_BYTES) as u64 {
        data.extend(read_user_sector(stream, lba + i)?);
    }
    data.truncate(size);
    Ok(data)
}

/// The file SYSTEM.CNF boots, read through the image's ISO9660 tree.
fn iso_boot_exe(image: &Path) -> Result<Vec<u8>> {
    let mut stream = File::open(image)?;
    let pvd = read_user_sector(&mut stream, 16)?;
    ensure!(
        &pvd[1..6] == b"CD001",
        "{}: no ISO9660 volume descriptor",
        image.display()
    );
    let root = &pvd[156..190];
    let listing = read_extent(
        &mut stream,
        disc::le32(root, 2) as u64,
        disc::le32(root, 10) as usize,
    )?;
    let mut files: BTreeMap<String, (u64, usize)> = BTreeMap::new();
    let mut at = 0;
    while at < listing.len() {
        let length = listing[at] as usize;
        if length == 0 {
            at = (at / 2048 + 1) * 2048;
            continue;
        }
        let name_len = listing[at + 32] as usize;
        let name = String::from_utf8_lossy(&listing[at + 33..at + 33 + name_len]).into_owned();
        let key = name.split(';').next().unwrap_or("").to_uppercase();
        files.insert(
            key,
            (
                disc::le32(&listing, at + 2) as u64,
                disc::le32(&listing, at + 10) as usize,
            ),
        );
        at += length;
    }
    let &(lba, size) = files
        .get("SYSTEM.CNF")
        .ok_or_else(|| Error(format!("{}: no SYSTEM.CNF", image.display())))?;
    let config = String::from_utf8_lossy(&read_extent(&mut stream, lba, size)?).into_owned();
    let boot = Regex::new(r"(?i)BOOT\s*=\s*cdrom:\\?([^;\s]+)")?;
    let Some(found) = boot.captures(&config) else {
        bail!("{}: SYSTEM.CNF names no BOOT file", image.display());
    };
    let &(lba, size) = files.get(&found[1].to_uppercase()).ok_or_else(|| {
        Error(format!(
            "{}: boot file {} is not in the root directory",
            image.display(),
            &found[1]
        ))
    })?;
    let exe = read_extent(&mut stream, lba, size)?;
    ensure!(
        exe.len() >= 8 && &exe[..8] == disc::PSX_EXE_MAGIC,
        "{}: boot file is not a PS-X EXE",
        image.display()
    );
    Ok(exe)
}

fn write_image(out: &Path, name: &str, source: &Path, start: u64, sectors: u64) -> Result<PathBuf> {
    let stem = slug(name);
    let bin = out.join(format!("{stem}.bin"));
    let mut input = File::open(source)?;
    input.seek(SeekFrom::Start(start))?;
    let mut output = File::create(&bin)?;
    let mut remaining = sectors * SECTOR_BYTES;
    let mut chunk = vec![0u8; 1 << 20];
    while remaining > 0 {
        let want = remaining.min(chunk.len() as u64) as usize;
        let n = input.read(&mut chunk[..want])?;
        ensure!(
            n > 0,
            "{}: ended before {sectors} sectors",
            source.display()
        );
        output.write_all(&chunk[..n])?;
        remaining -= n as u64;
    }
    write_cue(out, &stem)
}

fn write_cue(out: &Path, stem: &str) -> Result<PathBuf> {
    let cue = out.join(format!("{stem}.cue"));
    fs::write(
        &cue,
        format!("FILE \"{stem}.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n"),
    )?;
    Ok(cue)
}

fn load(path: &Path) -> Result<Value> {
    let document = crate::util::read_json(path)?;
    ensure!(
        document.get("schema") == Some(&json!(1)),
        "{}: unsupported lineup schema",
        path.display()
    );
    Ok(document)
}

fn text<'a>(row: &'a Value, key: &str) -> Result<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error(format!("'{key}'")))
}

fn check_source(row: &Value) -> Result<()> {
    let source = row.get("source").ok_or_else(|| Error("'source'".into()))?;
    let local = expand_home(text(source, "local")?);
    let revision = text(source, "revision")?;
    let result = Command::new("git")
        .arg("-C")
        .arg(&local)
        .args(["cat-file", "-e", &format!("{revision}^{{commit}}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| Error(format!("cannot run git: {e}")))?;
    ensure!(
        result.success(),
        "{}: {revision} is not a commit in {}",
        text(row, "name")?,
        local.display()
    );
    Ok(())
}

/// One input resolved to the file the Makefile will press.
fn prepare_row(row: &Value, out: &Path) -> Result<PathBuf> {
    let name = text(row, "name")?;
    let empty = Value::Object(Map::new());
    let derive = row.get("derive").unwrap_or(&empty);
    let hash = |key: &str| row.get(key).and_then(Value::as_str);
    check_source(row)?;
    let target = if let Some(cue) = row.get("cue").and_then(Value::as_str) {
        let cue = resolve(&expand_home(cue))?;
        check_hash(&cue, hash("cue_sha256"), name)?;
        check_hash(&cue_bin(&cue)?, hash("bin_sha256"), name)?;
        cue
    } else if let Some(source) = derive.get("data_track_of").and_then(Value::as_str) {
        let source = resolve(&expand_home(source))?;
        let image = cue_bin(&source)?;
        check_hash(&image, hash("bin_sha256"), name)?;
        let sectors = data_track_sectors(&source, &image)?;
        write_image(out, name, &image, 0, sectors)?
    } else if let Some(source) = derive.get("from_disc").and_then(Value::as_str) {
        let disc = resolve(&expand_home(source))?;
        check_hash(
            &disc,
            derive.get("disc_sha256").and_then(Value::as_str),
            name,
        )?;
        let lba = derive
            .get("image_lba")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error("'image_lba'".into()))?;
        let sectors = derive
            .get("sectors")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error("'sectors'".into()))?;
        write_image(out, name, &disc, lba * SECTOR_BYTES, sectors)?
    } else if let Some(source) = derive.get("boot_exe_of").and_then(Value::as_str) {
        let source = resolve(&expand_home(source))?;
        let image = cue_bin(&source)?;
        check_hash(&image, hash("bin_sha256"), name)?;
        let target = out.join(format!("{}.exe", slug(name)));
        fs::write(&target, iso_boot_exe(&image)?)?;
        target
    } else {
        bail!("{name}: no input");
    };
    if text(row, "kind")? == "image" {
        let exe = find_boot_exe(&cue_bin(&target)?)?;
        let fnv = fnv1a32(&exe.payload);
        if let Some(expected) = hash("payload_fnv1a32") {
            let expected = u32::from_str_radix(expected.trim_start_matches("0x"), 16)?;
            ensure!(
                expected == fnv,
                "{name}: payload FNV {fnv:#010x} != lineup {:#010x}",
                expected
            );
        }
    }
    Ok(target)
}

fn programs_of(lineup: &Value) -> Result<&Vec<Value>> {
    lineup
        .get("programs")
        .and_then(Value::as_array)
        .ok_or_else(|| Error("'programs'".into()))
}

fn prepare(args: &Args) -> Result<()> {
    let lineup_path = args.require_path("lineup")?;
    let lineup = load(&lineup_path)?;
    let out = crate::util::resolve_lenient(&args.require_path("out")?);
    fs::create_dir_all(&out)?;
    let lineup_name = lineup_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut lines = vec![format!(
        "# Generated by disc-tools lineup from {lineup_name}; do not edit."
    )];
    for row in programs_of(&lineup)? {
        let target = prepare_row(row, &out)?;
        lines.push(format!("{} := {}", text(row, "var")?, target.display()));
        if let Some(version_var) = row.get("version_var").and_then(Value::as_str) {
            lines.push(format!("{version_var} := {}", text(row, "version")?));
        }
        println!("{}: {}", text(row, "name")?, target.display());
    }
    fs::write(out.join("lineup.mk"), lines.join("\n") + "\n")?;
    println!(
        "lineup verified; variables in {}",
        out.join("lineup.mk").display()
    );
    Ok(())
}

fn mk_values(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for line in fs::read_to_string(path)?.lines() {
        if !line.starts_with('#') {
            if let Some((key, value)) = line.split_once(" := ") {
                values.insert(key.to_string(), value.to_string());
            }
        }
    }
    Ok(values)
}

/// The record a bare-EXE program leaves in the release receipt.
fn exe_record(target: &Path, combined: &Path, entry: &disc::TocEntry) -> Result<Value> {
    let name = &entry.name;
    let payload = fs::read(target)?;
    let (header, body) = payload.split_at(disc::USER_DATA_BYTES.min(payload.len()));
    let embedded = parse_exe_at(combined, entry.exe_lba as u64)?;
    ensure!(
        embedded.header == header
            && body.len() >= embedded.payload_bytes as usize
            && embedded.payload == body[..embedded.payload_bytes as usize],
        "{name}: embedded EXE differs from the lineup input"
    );
    let fnv = fnv1a32(&embedded.payload);
    ensure!(
        fnv == entry.payload_fnv,
        "{name}: payload FNV {fnv:#010x} != TOC {:#010x}",
        entry.payload_fnv
    );
    Ok(json!({
        "input": {"exe": file_record(target)?, "payload": {"fnv1a32": format!("0x{fnv:08x}"), "bytes": embedded.payload_bytes}},
        "embedded": {"exe_lba": entry.exe_lba, "version": entry.version, "flags": entry.flags},
    }))
}

fn receipt(args: &Args) -> Result<()> {
    let root = repo_root()?;
    let lineup_path = args.require_path("lineup")?;
    let lineup = load(&lineup_path)?;
    let values = mk_values(&args.require_path("mk")?)?;
    let cue = resolve(&args.require_path("cue")?)?;
    let combined = disc::image_for_cue(&cue)?;
    let entries = parse_toc(&disc::read_user_sectors(
        &combined,
        disc::TOC_LBA,
        disc::TOC_SECTORS,
    )?)?;
    let omitted = args.all("omit");
    let mut programs = Map::new();
    let mut ranges = Vec::new();
    let mut sources = Map::new();
    for row in programs_of(&lineup)? {
        let name = text(row, "name")?;
        if omitted.contains(&name) {
            ensure!(
                !entries.iter().any(|e| e.name == name),
                "{name} is on the disc but the pressing leaves it out"
            );
            continue;
        }
        let Some(entry) = entries.iter().find(|e| e.name == name) else {
            bail!("{name} is in the lineup but not on the disc");
        };
        let target = PathBuf::from(
            values
                .get(text(row, "var")?)
                .ok_or_else(|| Error(format!("{name}: no variable in lineup.mk")))?,
        );
        let record = if text(row, "kind")? == "image" {
            let record = program_record(&target, &combined, entry)?;
            let embedded = &record["embedded"];
            ranges.push((
                embedded["image_lba_start"].as_u64().unwrap_or(0),
                embedded["image_lba_end_exclusive"].as_u64().unwrap_or(0),
                name.to_string(),
            ));
            record
        } else {
            exe_record(&target, &combined, entry)?
        };
        let version = text(row, "version")?;
        ensure!(
            entry.version == version,
            "{name}: carousel says {:?}, lineup says {version:?}",
            entry.version
        );
        let derived_from = row.get("derive");
        let receipt_text = row.get("receipt").and_then(Value::as_str).unwrap_or("");
        let receipt_path = receipt_text
            .starts_with(['~', '/'])
            .then(|| expand_home(receipt_text));
        let build_receipt = match receipt_path.as_deref().filter(|p| p.is_file()) {
            Some(path) => file_record(path)?,
            None => row.get("receipt").cloned().unwrap_or(Value::Null),
        };
        let mut source = row["source"].as_object().cloned().unwrap_or_default();
        source.insert("kind".into(), json!("lineup-build"));
        sources.insert(name.to_string(), Value::Object(source.clone()));
        let mut entry_row = Map::new();
        entry_row.insert("source".into(), Value::Object(source));
        entry_row.insert("build".into(), row["build"].clone());
        entry_row.insert("build_receipt".into(), build_receipt);
        if let Some(derived) = derived_from {
            entry_row.insert("derived_from".into(), derived.clone());
        }
        for (key, value) in record.as_object().expect("object") {
            entry_row.insert(key.clone(), value.clone());
        }
        programs.insert(name.to_string(), Value::Object(entry_row));
    }
    check_no_overlap(&mut ranges).map_err(|e| {
        Error(e.0.replace("embedded image ranges overlap", "embedded images overlap"))
    })?;
    let lineup_names: Vec<&str> = programs_of(&lineup)?
        .iter()
        .filter_map(|r| r.get("name").and_then(Value::as_str))
        .collect();
    let mut extra: Vec<&str> = entries
        .iter()
        .map(|e| e.name.as_str())
        .filter(|n| !lineup_names.contains(n))
        .collect();
    extra.sort();
    ensure!(
        extra.is_empty(),
        "on the disc but not in the lineup: {}",
        extra.join(", ")
    );
    let frontend = args.require_path("frontend")?;
    let release = json!({
        "schema": crate::receipt::SCHEMA,
        "kind": "lineup",
        "lineup": file_record(&lineup_path)?,
        "build_command": args.require("build-command")?,
        "frontend": file_record(&frontend)?,
        "combined": {
            "cue": file_record(&cue)?,
            "bin": file_record(&combined)?,
            "sectors": fs::metadata(&combined)?.len() / SECTOR_BYTES,
        },
        "programs": programs,
    });
    write_receipt(&args.require_path("release-out")?, &release)?;
    write_components(
        &root,
        &lineup_path,
        &frontend,
        &cue,
        &combined,
        &sources,
        args,
    )?;
    println!(
        "{} programs receipted: {}",
        programs.len(),
        args.require("release-out")?
    );
    println!("components: {}", args.require("components-out")?);
    Ok(())
}

/// The component receipt beside the release receipt: which sources built the
/// launcher, which tuple the disc pins, and the toolchain that built it.
fn write_components(
    root: &Path,
    lineup_path: &Path,
    frontend: &Path,
    cue: &Path,
    combined: &Path,
    sources: &Map<String, Value>,
    args: &Args,
) -> Result<()> {
    let dirty = git(root, &["status", "--porcelain", "--untracked-files=normal"])?;
    let components = crate::util::read_json(&root.join("release-components.json"))?;
    let mut launcher_inputs = Map::new();
    for path in ["games/PSoXide-editor", "games/PSoXide-sdk"] {
        let head = git(&root.join(path), &["rev-parse", "HEAD"])?;
        let link = git(root, &["rev-parse", &format!("HEAD:{path}")])?;
        ensure!(head == link, "{path} is at {head}, the gitlink says {link}");
        launcher_inputs.insert(path.to_string(), json!({"revision": head}));
    }
    launcher_inputs["games/PSoXide-editor"][".components-receipt.json"] =
        file_record(&root.join("games/PSoXide-editor/.components-receipt.json"))?;
    let document = json!({
        "schema": 1,
        "kind": "lineup",
        "components": components["components"],
        "launcher_sources": launcher_inputs,
        "demo_revision": git(root, &["rev-parse", "HEAD"])?,
        "demo_describe": git(root, &["describe", "--tags", "--match", "v*", "--always", "--dirty"])?,
        "demo_tree_clean": dirty.is_empty(),
        "build_recipe": file_record(&root.join("Makefile"))?,
        "lineup": file_record(lineup_path)?,
        "programs": sources,
        "rustc": command_output("rustc", &["--version"], Some(root))?,
        "cargo": command_output("cargo", &["--version"], Some(root))?,
        "frontend": file_record(frontend)?,
        "cue": file_record(cue)?,
        "bin": file_record(combined)?,
    });
    write_receipt(&args.require_path("components-out")?, &document)
}

/// The commit a library receipt says its build came from, whichever of the
/// receipt shapes in use (`source.commit`, `source_commit`) it is written in.
fn receipt_commit(path: &Path) -> Option<String> {
    let document = crate::util::read_json(path).ok()?;
    document
        .pointer("/source/commit")
        .or_else(|| document.get("source_commit"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Copy `from` over `to` only when the bytes differ; true when it copied.
fn copy_if_changed(from: &Path, to: &Path) -> Result<bool> {
    if to.is_file() && sha256_file(from)? == sha256_file(to)? {
        return Ok(false);
    }
    // Remove first so a copy-on-write clone replaces the file, never rewrites it.
    if to.exists() {
        fs::remove_file(to)?;
    }
    fs::copy(from, to).map_err(|e| Error(format!("{}: {e}", to.display())))?;
    Ok(true)
}

/// Take the newest library build of every row that names one (`"library":
/// "<cue path under the library root>"`), copy it over the lineup's input,
/// and write its hashes into the lineup file, so the next `prepare` accepts it.
/// The revision and build text stay human-owned: a moved build whose receipt
/// names a different commit is reported as needing its text updated.
fn refresh(args: &Args) -> Result<()> {
    let lineup_path = args.require_path("lineup")?;
    let library = resolve(&expand_home(args.require("library")?))?;
    let mut lineup = load(&lineup_path)?;
    let only: Vec<&str> = args.all("only");
    let mut moved = 0;
    let programs = lineup
        .get_mut("programs")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| Error("'programs'".into()))?;
    for row in programs {
        let Some(relative) = row
            .get("library")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let name = text(row, "name")?.to_string();
        if !only.is_empty() && !only.contains(&name.as_str()) {
            continue;
        }
        let library_cue = resolve(&library.join(&relative))?;
        let library_bin = cue_bin(&library_cue)?;
        let input_cue = expand_home(text(row, "cue")?);
        let dir = input_cue
            .parent()
            .ok_or_else(|| Error(format!("{name}: input cue has no directory")))?
            .to_path_buf();
        fs::create_dir_all(&dir)?;
        let input_bin = dir.join(
            library_bin
                .file_name()
                .ok_or_else(|| Error(format!("{name}: library bin has no name")))?,
        );
        let old_bin = row
            .get("bin_sha256")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let new_bin = sha256_file(&library_bin)?;
        let new_cue = sha256_file(&library_cue)?;
        let copied_bin = copy_if_changed(&library_bin, &input_bin)?;
        let copied_cue = copy_if_changed(&library_cue, &input_cue)?;
        // The build receipt next to the library image, when there is one.
        let mut library_commit = None;
        if let Some(receipt) = row.get("receipt").and_then(Value::as_str) {
            let receipt = expand_home(receipt);
            if let (Some(file), Some(parent)) = (receipt.file_name(), library_cue.parent()) {
                let candidate = parent.join(file);
                if candidate.is_file() {
                    copy_if_changed(&candidate, &receipt)?;
                    library_commit = receipt_commit(&candidate);
                }
            }
        }
        row["bin_sha256"] = json!(new_bin);
        row["cue_sha256"] = json!(new_cue);
        if new_bin == old_bin {
            println!("{name}: unchanged ({})", &new_bin[..8]);
            continue;
        }
        moved += 1;
        println!(
            "{name}: {} -> {} (bin {}, cue {})",
            &old_bin[..old_bin.len().min(8)],
            &new_bin[..8],
            if copied_bin {
                "copied"
            } else {
                "already there"
            },
            if copied_cue {
                "copied"
            } else {
                "already there"
            },
        );
        let revision = row
            .pointer("/source/revision")
            .and_then(Value::as_str)
            .unwrap_or("");
        match library_commit {
            Some(commit) if !revision.starts_with(&commit) && !commit.starts_with(revision) => println!(
                "  UPDATE source.revision and build text: lineup says {}, library receipt says {commit}",
                &revision[..revision.len().min(8)]
            ),
            Some(_) => println!("  receipt commit matches source.revision; check the build text"),
            None => println!("  no readable receipt commit; check source.revision and build text"),
        }
    }
    if moved != 0 {
        let mut out = crate::util::dumps(&lineup, 2, false);
        out.push('\n');
        crate::util::write_atomic(&lineup_path, &out)?;
    }
    println!(
        "lineup refresh: {moved} input(s) moved{}",
        if moved != 0 {
            "; lineup file rewritten"
        } else {
            ""
        }
    );
    Ok(())
}

pub fn run(raw: &[String]) -> Result<i32> {
    let Some((command, rest)) = raw.split_first() else {
        bail!("usage: lineup prepare|refresh|receipt [options]");
    };
    match command.as_str() {
        "prepare" => prepare(&Args::parse(rest, &["lineup", "out"], &[])?)?,
        "refresh" => refresh(&Args::parse(rest, &["lineup", "library", "only"], &[])?)?,
        "receipt" => receipt(&Args::parse(
            rest,
            &[
                "lineup",
                "mk",
                "cue",
                "frontend",
                "build-command",
                "release-out",
                "omit",
                "components-out",
            ],
            &[],
        )?)?,
        other => bail!("unknown lineup command {other:?}"),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn refresh_copies_a_moved_library_build_and_rewrites_its_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let inputs = dir.path().join("inputs");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&inputs).unwrap();
        fs::write(library.join("g.bin"), b"new build").unwrap();
        fs::write(library.join("g.cue"), "FILE \"g.bin\" BINARY\n").unwrap();
        fs::write(inputs.join("g.bin"), b"old build").unwrap();
        fs::write(inputs.join("g.cue"), "FILE \"g.bin\" BINARY\n").unwrap();
        let lineup_path = dir.path().join("lineup.json");
        let lineup = json!({
            "schema": 1,
            "programs": [
                {
                    "name": "G", "library": "g.cue",
                    "cue": inputs.join("g.cue").display().to_string(),
                    "cue_sha256": "0", "bin_sha256": "old",
                    "source": {"revision": "abc"}
                },
                {"name": "OTHER", "cue": "x.cue", "bin_sha256": "keep"}
            ]
        });
        fs::write(&lineup_path, crate::util::dumps(&lineup, 2, false)).unwrap();
        let arguments = raw(&[
            "--lineup",
            lineup_path.to_str().unwrap(),
            "--library",
            library.to_str().unwrap(),
        ]);
        refresh(&Args::parse(&arguments, &["lineup", "library", "only"], &[]).unwrap()).unwrap();
        let after = crate::util::read_json(&lineup_path).unwrap();
        let rows = after["programs"].as_array().unwrap();
        assert_eq!(
            rows[0]["bin_sha256"],
            json!(sha256_file(&library.join("g.bin")).unwrap())
        );
        assert_eq!(
            rows[0]["cue_sha256"],
            json!(sha256_file(&library.join("g.cue")).unwrap())
        );
        assert_eq!(rows[1]["bin_sha256"], json!("keep"));
        assert_eq!(fs::read(inputs.join("g.bin")).unwrap(), b"new build");
        // Run again: nothing moved, the file is left alone.
        let before = fs::read_to_string(&lineup_path).unwrap();
        refresh(&Args::parse(&arguments, &["lineup", "library", "only"], &[]).unwrap()).unwrap();
        assert_eq!(fs::read_to_string(&lineup_path).unwrap(), before);
    }
}
