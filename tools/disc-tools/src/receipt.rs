//! Create or verify a fail-closed receipt for a combined demo-disc pressing.
//!
//! A receipt names every program on the disc, the exact input it was pressed
//! from and the clean source revision that built it, and proves the embedded
//! copy matches the input sector for sector. `verify` rebuilds the document
//! from the same inputs and refuses any difference.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::args::Args;
use crate::disc::{
    self, find_boot_exe, image_for_cue, parse_exe_at, parse_toc, Exe, TocEntry, SECTOR_BYTES,
    USER_DATA_BYTES,
};
use crate::util::{
    file_record, fnv1a32, git, hex, is_digest, is_revision, resolve, sha256_bytes, sha256_file,
    write_receipt, Error, Result,
};

pub const SCHEMA: &str = "psoxide-combined-release-v1";
/// Every private pressing carries these; the receipt must cover all of them.
pub const CORE_PROGRAMS: [&str; 2] = ["CORTEX IGNITION", "QUAKE SHAREWARE"];
/// HL=1, CS=1 and HK=1 each add one. The receipt covers exactly the ones
/// pressed, and a pressed one it does not cover is an error, never a silent
/// omission.
pub const OPTIONAL_PROGRAMS: [&str; 3] = ["HALF-LIFE", "COUNTER-STRIKE", "HOLLOW KNIGHT"];
const HL_COOK_MANIFEST: &str = "data/.hlpsx-cook.json";
const HL_COOK_SCHEMA: u64 = 1;
const TREE_DOMAIN: &[u8] = b"hl-psx-tree-v1\0";
const PSOXIDE_SKIPPED_DIRECTORIES: [&str; 11] = [
    ".git",
    "target",
    "build",
    "dist",
    "baked",
    "cooked",
    "node_modules",
    "captures",
    "graphify-out",
    ".web",
    "__pycache__",
];

/// One hash over a set of files: each contributes its relative path and size,
/// then its bytes, so a renamed or resized file changes the digest.
pub fn tree_sha256(root: &Path, files: &[PathBuf]) -> Result<String> {
    let unique: BTreeSet<&PathBuf> = files.iter().collect();
    let mut digest = Sha256::new();
    digest.update(TREE_DOMAIN);
    for relative in unique {
        let path = root.join(relative);
        ensure!(
            path.is_file(),
            "provenance input is missing: {}",
            path.display()
        );
        let encoded = posix(relative);
        digest.update((encoded.len() as u64).to_le_bytes());
        digest.update(encoded.as_bytes());
        digest.update(fs::metadata(&path)?.len().to_le_bytes());
        let mut stream = File::open(&path)?;
        let mut chunk = vec![0u8; 1 << 20];
        loop {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            digest.update(&chunk[..n]);
        }
    }
    Ok(hex(&digest.finalize()))
}

fn posix(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every regular file under `root`, relative, directories and names visited
/// in sorted order (like `os.walk`, which never descends into symlinks).
pub fn recursive_files(root: &Path, skip_directories: &[&str]) -> Result<Vec<PathBuf>> {
    fn walk(root: &Path, dir: &Path, skip: &[&str], out: &mut Vec<PathBuf>) -> Result<()> {
        let mut directories = Vec::new();
        let mut names = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if fs::metadata(entry.path())
                .map(|m| m.is_dir())
                .unwrap_or(false)
            {
                if !skip.contains(&name.as_str()) {
                    directories.push((name, entry.file_type()?.is_symlink()));
                }
            } else {
                names.push(name);
            }
        }
        directories.sort();
        names.sort();
        for name in names {
            let path = dir.join(&name);
            if path.is_file() {
                out.push(path.strip_prefix(root).expect("under root").to_path_buf());
            }
        }
        for (name, symlink) in directories {
            if !symlink {
                walk(root, &dir.join(name), skip, out)?;
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, skip_directories, &mut files)?;
    Ok(files)
}

/// Hash of every tracked file of a git checkout.
pub fn source_tree_sha256(source: &Path) -> Result<String> {
    let listed = crate::util::git_bytes(source, &["ls-files", "-z"])?;
    let files: Vec<PathBuf> = listed
        .split(|&b| b == 0)
        .filter(|raw| !raw.is_empty())
        .map(|raw| PathBuf::from(String::from_utf8_lossy(raw).into_owned()))
        .collect();
    tree_sha256(source, &files)
}

pub fn psoxide_tree_sha256(psoxide: &Path) -> Result<String> {
    let files: Vec<PathBuf> = recursive_files(psoxide, &PSOXIDE_SKIPPED_DIRECTORIES)?
        .into_iter()
        .filter(|path| path.file_name().map(|n| n != ".DS_Store").unwrap_or(true))
        .collect();
    tree_sha256(psoxide, &files)
}

pub fn psoxide_revision_from_marker(marker: &str) -> Result<String> {
    if let Some(rest) = marker.strip_prefix("git:") {
        let revision = rest.to_lowercase();
        ensure!(
            is_revision(&revision),
            "invalid git revision in hydrated PSoXide marker"
        );
        return Ok(revision);
    }
    if let Some(rest) = marker.strip_prefix("local:") {
        let source = resolve(Path::new(rest))?;
        let revision = git(&source, &["rev-parse", "--verify", "HEAD^{commit}"])?.to_lowercase();
        ensure!(
            is_revision(&revision),
            "invalid revision for local hydrated PSoXide source"
        );
        ensure!(
            git(
                &source,
                &["status", "--porcelain=v1", "--untracked-files=normal"]
            )?
            .is_empty(),
            "local hydrated PSoXide source is dirty and cannot identify a release: {}",
            source.display()
        );
        return Ok(revision);
    }
    bail!("unsupported hydrated PSoXide marker: {marker:?}")
}

pub fn cooked_tree_sha256(source: &Path) -> Result<String> {
    let data = source.join("data");
    let files: Vec<PathBuf> = recursive_files(&data, &[])?
        .into_iter()
        .filter(|path| path != Path::new(".hlpsx-cook.json"))
        .collect();
    tree_sha256(&data, &files)
}

/// `NAME=/path` as given on the command line.
pub fn split_assignment(value: &str, label: &str) -> Result<(String, PathBuf)> {
    let Some((name, raw)) = value.split_once('=') else {
        bail!("{label} must be NAME=/absolute/path: {value:?}");
    };
    ensure!(
        !name.is_empty() && !raw.is_empty(),
        "invalid {label}: {value:?}"
    );
    Ok((name.to_string(), PathBuf::from(raw)))
}

fn quoted(names: &[String]) -> String {
    format!(
        "[{}]",
        names
            .iter()
            .map(|n| format!("'{n}'"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The receipted programs in canonical order: every core one plus the optional
/// ones given, and nothing else.
pub fn pressed_programs(names: &[String], label: &str) -> Result<Vec<String>> {
    let given: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let missing: Vec<String> = CORE_PROGRAMS
        .iter()
        .filter(|n| !given.contains(**n))
        .map(|n| n.to_string())
        .collect();
    let mut extra: Vec<String> = given
        .iter()
        .filter(|n| !CORE_PROGRAMS.contains(n) && !OPTIONAL_PROGRAMS.contains(n))
        .map(|n| n.to_string())
        .collect();
    extra.sort();
    if !missing.is_empty() || !extra.is_empty() {
        bail!(
            "{label} names must be {:?} plus any of {:?}; missing={} extra={}",
            CORE_PROGRAMS,
            OPTIONAL_PROGRAMS,
            quoted(&missing),
            quoted(&extra)
        );
    }
    Ok(CORE_PROGRAMS
        .iter()
        .chain(OPTIONAL_PROGRAMS.iter())
        .filter(|n| given.contains(**n))
        .map(|n| n.to_string())
        .collect())
}

pub fn check_toc_covered(entries: &[TocEntry], names: &[String]) -> Result<()> {
    let present: BTreeSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    let missing: Vec<String> = names
        .iter()
        .filter(|n| !present.contains(n.as_str()))
        .cloned()
        .collect();
    ensure!(
        missing.is_empty(),
        "combined TOC is missing required programs: {}",
        quoted(&missing)
    );
    let uncovered: Vec<String> = OPTIONAL_PROGRAMS
        .iter()
        .filter(|n| present.contains(**n) && !names.iter().any(|m| m == **n))
        .map(|n| n.to_string())
        .collect();
    ensure!(
        uncovered.is_empty(),
        "combined TOC carries programs the receipt does not cover: {}",
        quoted(&uncovered)
    );
    Ok(())
}

pub fn exact_assignments(values: &[&str], label: &str) -> Result<Vec<(String, PathBuf)>> {
    let mut decoded: Vec<(String, PathBuf)> = Vec::new();
    for value in values {
        let (name, path) = split_assignment(value, label)?;
        ensure!(
            !decoded.iter().any(|(n, _)| *n == name),
            "duplicate {label} for {name}"
        );
        decoded.push((name, path));
    }
    let names: Vec<String> = decoded.iter().map(|(n, _)| n.clone()).collect();
    pressed_programs(&names, label)?;
    Ok(decoded)
}

pub fn source_record(source: &Path) -> Result<Value> {
    let source = resolve(source)?;
    ensure!(
        source.is_dir(),
        "source is not a directory: {}",
        source.display()
    );
    let top = resolve(Path::new(&git(&source, &["rev-parse", "--show-toplevel"])?))?;
    ensure!(
        top == source,
        "source must name its repository root: {} != {}",
        source.display(),
        top.display()
    );
    let revision = git(&source, &["rev-parse", "--verify", "HEAD^{commit}"])?.to_lowercase();
    ensure!(
        is_revision(&revision),
        "invalid source revision for {}: {revision:?}",
        source.display()
    );
    let dirty = git(
        &source,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )?;
    ensure!(
        dirty.is_empty(),
        "source is dirty and cannot identify a release: {}",
        source.display()
    );
    Ok(json!({
        "path": source.display().to_string(),
        "kind": "clean-git-commit",
        "revision": revision,
        "tree_clean": true,
    }))
}

fn string_field<'a>(document: &'a Value, key: &str) -> &'a str {
    document.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The provenance a Half-Life cook wrote beside its assets, held against the
/// clean source, the hydrated PSoXide tree and the cooked files as they are now.
pub fn hl_cook_record(source: &Path, source_row: &Value) -> Result<Value> {
    let path = source.join(HL_COOK_MANIFEST);
    let text = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "Half-Life cooked provenance is missing: {}; run a fresh full asset cook",
            path.display()
        ),
        Err(error) => return Err(error.into()),
    };
    let document: Value = match std::str::from_utf8(&text)
        .ok()
        .filter(|t| t.is_ascii())
        .map(serde_json::from_str::<Value>)
    {
        Some(Ok(document)) => document,
        Some(Err(error)) => bail!(
            "invalid Half-Life cooked provenance: {}: {error}",
            path.display()
        ),
        None => bail!(
            "invalid Half-Life cooked provenance: {}: not ASCII",
            path.display()
        ),
    };
    let required = [
        "schema",
        "hl_psx_revision",
        "hl_psx_tree_sha256",
        "psoxide_source",
        "psoxide_revision",
        "psoxide_tree_sha256",
        "half_life_input_sha256",
        "cooked_tree_sha256",
    ];
    let keys: BTreeSet<&str> = document
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    ensure!(
        document.is_object() && keys == required.iter().copied().collect(),
        "Half-Life cooked provenance has unexpected fields: {}",
        path.display()
    );
    ensure!(
        document["schema"].as_u64() == Some(HL_COOK_SCHEMA),
        "unsupported Half-Life cooked provenance schema: {}",
        document["schema"]
    );
    for field in [
        "hl_psx_tree_sha256",
        "psoxide_tree_sha256",
        "half_life_input_sha256",
        "cooked_tree_sha256",
    ] {
        ensure!(
            is_digest(string_field(&document, field)),
            "Half-Life cooked provenance has invalid {field}"
        );
    }
    ensure!(
        document["hl_psx_revision"] == source_row["revision"],
        "Half-Life cooked provenance revision does not match the clean source"
    );
    ensure!(
        string_field(&document, "hl_psx_tree_sha256") == source_tree_sha256(source)?,
        "Half-Life cooked provenance source tree does not match the clean source"
    );
    let psoxide = source.join(".psoxide");
    let marker = psoxide.join(".psoxide-source");
    let marker_text = match fs::read_to_string(&marker) {
        Ok(text) => text.trim().to_string(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "Half-Life hydrated PSoXide marker is missing: {}",
                marker.display()
            )
        }
        Err(error) => return Err(error.into()),
    };
    ensure!(
        string_field(&document, "psoxide_source") == marker_text,
        "Half-Life cooked provenance PSoXide source does not match its hydrated tree"
    );
    ensure!(
        string_field(&document, "psoxide_tree_sha256") == psoxide_tree_sha256(&psoxide)?,
        "Half-Life cooked provenance PSoXide tree does not match its hydrated tree"
    );
    ensure!(
        string_field(&document, "psoxide_revision") == psoxide_revision_from_marker(&marker_text)?,
        "Half-Life cooked provenance PSoXide revision does not match its source"
    );
    ensure!(
        string_field(&document, "cooked_tree_sha256") == cooked_tree_sha256(source)?,
        "Half-Life cooked provenance digest does not match the current cooked assets"
    );
    let revision = string_field(&document, "psoxide_revision");
    ensure!(
        is_revision(revision) || revision.starts_with("tree:"),
        "Half-Life cooked provenance has invalid PSoXide revision"
    );
    Ok(json!({
        "manifest": file_record(&path)?,
        "document": document,
        "verified": true,
    }))
}

/// How many sectors of a source image the combined disc embeds: the whole
/// image for a data-only cue, otherwise up to the first audio pregap, since
/// the audio tracks are re-laid-out in the combined image.
pub fn data_track_sectors(cue: &Path, image: &Path) -> Result<u64> {
    let bytes = fs::read(cue)?;
    ensure!(bytes.is_ascii(), "cue is not ASCII: {}", cue.display());
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let track = Regex::new(r"(?m)^\s*TRACK\s+(\d+)\s+([^\s]+)\s*$")?;
    let tracks: Vec<(u32, String)> = track
        .captures_iter(&text)
        .map(|c| Ok((c[1].parse::<u32>()?, c[2].to_uppercase())))
        .collect::<Result<_>>()?;
    ensure!(
        tracks.first().map(|t| (t.0, t.1.as_str())) == Some((1, "MODE2/2352")),
        "first CUE track must be TRACK 01 MODE2/2352: {}",
        cue.display()
    );
    let total = fs::metadata(image)?.len() / SECTOR_BYTES;
    if tracks.len() == 1 {
        return Ok(total);
    }
    ensure!(
        (tracks[1].0, tracks[1].1.as_str()) == (2, "AUDIO"),
        "second CUE track must be TRACK 02 AUDIO: {}",
        cue.display()
    );
    let body_pattern = Regex::new(r"(?m)^\s*TRACK\s+02\s+AUDIO\s*$")?;
    let Some(start) = body_pattern.find(&text) else {
        bail!("cannot locate TRACK 02 body: {}", cue.display());
    };
    let rest = &text[start.end()..];
    let next = Regex::new(r"(?m)^\s*TRACK\s+\d+")?;
    let body = &rest[..next.find(rest).map(|m| m.start()).unwrap_or(rest.len())];
    let index_zero = Regex::new(r"(?m)^\s*INDEX\s+00\s+(\d+):(\d+):(\d+)\s*$")?;
    let Some(found) = index_zero.captures(body) else {
        bail!(
            "audio-bearing CUE has no TRACK 02 INDEX 00: {}",
            cue.display()
        );
    };
    let (minute, second, frame) = (
        found[1].parse::<u64>()?,
        found[2].parse::<u64>()?,
        found[3].parse::<u64>()?,
    );
    ensure!(
        second < 60 && frame < 75,
        "invalid CUE timestamp at TRACK 02 INDEX 00: {}",
        cue.display()
    );
    let sectors = (minute * 60 + second) * 75 + frame;
    ensure!(
        sectors > 0 && sectors <= total,
        "invalid data-track sector count {sectors} for {}",
        cue.display()
    );
    Ok(sectors)
}

/// Compare the source image with the copy embedded at `image_lba`, sector by
/// sector. mkdisc rewrites only the absolute MSF bytes (12..15) of a sector
/// when it relocates an image, so those are the only bytes allowed to differ.
pub fn verify_embedded_image(
    combined: &Path,
    source: &Path,
    image_lba: u64,
    sectors: u64,
) -> Result<u64> {
    let combined_sectors = fs::metadata(combined)?.len() / SECTOR_BYTES;
    ensure!(
        image_lba + sectors <= combined_sectors,
        "embedded image range [{image_lba}, {}) exceeds disc",
        image_lba + sectors
    );
    let mut expected_stream = File::open(source)?;
    let mut actual_stream = File::open(combined)?;
    actual_stream.seek(SeekFrom::Start(image_lba * SECTOR_BYTES))?;
    const BATCH: u64 = 2048;
    let sector = SECTOR_BYTES as usize;
    let mut done = 0u64;
    while done < sectors {
        let take = BATCH.min(sectors - done) as usize;
        let mut expected = vec![0u8; take * sector];
        let mut actual = vec![0u8; take * sector];
        let got = read_up_to(&mut expected_stream, &mut expected)?;
        let have = read_up_to(&mut actual_stream, &mut actual)?;
        for index in 0..take {
            let (e, a) = (
                &expected[index * sector..(index + 1) * sector],
                &actual[index * sector..(index + 1) * sector],
            );
            let whole = (index + 1) * sector <= got && (index + 1) * sector <= have;
            if !whole || e[..12] != a[..12] || e[15..] != a[15..] {
                bail!(
                    "embedded sector {} differs outside relocated MSF bytes",
                    done as usize + index
                );
            }
        }
        done += take as u64;
    }
    Ok(sectors)
}

fn read_up_to(stream: &mut File, buffer: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let n = stream.read(&mut buffer[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

fn hex32(value: u32) -> String {
    format!("0x{value:08x}")
}

/// What one program's input image and its embedded copy prove.
pub fn program_record(cue: &Path, combined: &Path, entry: &TocEntry) -> Result<Value> {
    let cue = resolve(cue)?;
    let image = image_for_cue(&cue)?;
    let sectors = data_track_sectors(&cue, &image)?;
    let input_exe = find_boot_exe(&image)?;
    verify_embedded_image(combined, &image, entry.image_lba as u64, sectors)?;
    ensure!(
        entry.exe_lba as u64 == entry.image_lba as u64 + input_exe.lba,
        "{}: TOC EXE LBA {} != image LBA {} + input boot LBA {}",
        entry.name,
        entry.exe_lba,
        entry.image_lba,
        input_exe.lba
    );
    let embedded_exe = parse_exe_at(combined, entry.exe_lba as u64)?;
    ensure!(
        embedded_exe.header == input_exe.header && embedded_exe.payload == input_exe.payload,
        "{}: embedded PS-X EXE differs from input image",
        entry.name
    );
    let payload_fnv = fnv1a32(&input_exe.payload);
    ensure!(
        payload_fnv == entry.payload_fnv,
        "{}: payload FNV {payload_fnv:#010x} != TOC {:#010x}",
        entry.name,
        entry.payload_fnv
    );
    let mut exe_bytes = input_exe.header.clone();
    exe_bytes.extend(&input_exe.payload);
    let payload_sectors = input_exe.payload_bytes as u64 / USER_DATA_BYTES as u64;
    let total_sectors = fs::metadata(&image)?.len() / SECTOR_BYTES;
    let image_lba = entry.image_lba as u64;
    let exe_lba = entry.exe_lba as u64;
    Ok(json!({
        "input": {
            "cue": file_record(&cue)?,
            "bin": file_record(&image)?,
            "bin_total_sectors": total_sectors,
            "data_track_sectors": sectors,
            "exe": {"bytes": exe_bytes.len(), "sha256": sha256_bytes(&exe_bytes)},
            "payload": {
                "bytes": input_exe.payload_bytes,
                "sha256": sha256_bytes(&input_exe.payload),
                "fnv1a32": hex32(payload_fnv),
                "pc": hex32(input_exe.pc),
                "load": hex32(input_exe.load),
            },
        },
        "embedded": {
            "image_lba_start": image_lba,
            "image_lba_end_exclusive": image_lba + sectors,
            "image_sectors": sectors,
            "exe_lba": exe_lba,
            "payload_lba_start": exe_lba + 1,
            "payload_lba_end_exclusive": exe_lba + 1 + payload_sectors,
            "version": entry.version,
            "flags": entry.flags,
        },
    }))
}

/// Embedded image ranges `(start, end_exclusive, name)` must not overlap.
pub fn check_no_overlap(ranges: &mut [(u64, u64, String)]) -> Result<()> {
    ranges.sort();
    for pair in ranges.windows(2) {
        ensure!(
            pair[1].0 >= pair[0].1,
            "embedded image ranges overlap: {} and {}",
            pair[0].2,
            pair[1].2
        );
    }
    Ok(())
}

fn embedded_range(record: &Value, name: &str) -> (u64, u64, String) {
    let embedded = &record["embedded"];
    (
        embedded["image_lba_start"].as_u64().unwrap_or(0),
        embedded["image_lba_end_exclusive"].as_u64().unwrap_or(0),
        name.to_string(),
    )
}

pub fn build_document(
    combined_cue: &Path,
    frontend: &Path,
    build_command: &str,
    programs: &[(String, PathBuf)],
    sources: &[(String, PathBuf)],
) -> Result<Value> {
    ensure!(
        !build_command.is_empty() && !build_command.contains('\n') && !build_command.contains('\r'),
        "build command must be one non-empty line"
    );
    let combined_cue = resolve(combined_cue)?;
    let combined = image_for_cue(&combined_cue)?;
    let entries = parse_toc(&disc::read_user_sectors(
        &combined,
        disc::TOC_LBA,
        disc::TOC_SECTORS,
    )?)?;
    let program_names: Vec<String> = programs.iter().map(|(n, _)| n.clone()).collect();
    let names = pressed_programs(&program_names, "program")?;
    let source_names: BTreeSet<&str> = sources.iter().map(|(n, _)| n.as_str()).collect();
    ensure!(
        source_names == names.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        "every receipted program needs exactly one source"
    );
    check_toc_covered(&entries, &names)?;
    let mut rows = Map::new();
    let mut ranges = Vec::new();
    for name in &names {
        let entry = entries
            .iter()
            .find(|e| &e.name == name)
            .expect("covered above");
        let cue = &programs.iter().find(|(n, _)| n == name).expect("named").1;
        let record = program_record(cue, &combined, entry)?;
        let source = resolve(&sources.iter().find(|(n, _)| n == name).expect("named").1)?;
        let source_row = source_record(&source)?;
        let mut row = Map::new();
        row.insert("source".into(), source_row.clone());
        for (key, value) in record.as_object().expect("object") {
            row.insert(key.clone(), value.clone());
        }
        if name == "HALF-LIFE" {
            row.insert(
                "cooked_assets".into(),
                hl_cook_record(&source, &source_row)?,
            );
        }
        ranges.push(embedded_range(&record, name));
        rows.insert(name.clone(), Value::Object(row));
    }
    check_no_overlap(&mut ranges)?;
    Ok(json!({
        "schema": SCHEMA,
        "build_command": build_command,
        "frontend": file_record(frontend)?,
        "combined": {
            "cue": file_record(&combined_cue)?,
            "bin": file_record(&combined)?,
            "sectors": fs::metadata(&combined)?.len() / SECTOR_BYTES,
        },
        "programs": rows,
    }))
}

fn verify_file_record(path: &Path, expected: &Value, label: &str) -> Result<()> {
    ensure!(path.is_file(), "{label} is missing: {}", path.display());
    ensure!(
        Some(fs::metadata(path)?.len()) == expected.get("bytes").and_then(Value::as_u64),
        "{label} size no longer matches the receipt: {}",
        path.display()
    );
    ensure!(
        expected.get("sha256").and_then(Value::as_str) == Some(sha256_file(path)?.as_str()),
        "{label} SHA-256 no longer matches the receipt: {}",
        path.display()
    );
    Ok(())
}

fn malformed(what: &str) -> Error {
    Error(format!("malformed receipt: missing {what}"))
}

fn member<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| malformed(&format!("'{key}'")))
}

fn basename(value: &Value) -> Result<String> {
    let text = value.as_str().ok_or_else(|| malformed("'path'"))?;
    Ok(Path::new(text)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default())
}

/// Verify a sealed combined image without requiring deleted build inputs.
///
/// The ordinary verifier remains the stronger source-provenance gate. This
/// mode is for a deliberately archived burn candidate: it verifies the exact
/// CUE/BIN hashes and then independently checks every embedded TOC entry and
/// executable payload against the receipt.
pub fn verify_sealed_document(receipt_path: &Path, expected: &Value) -> Result<()> {
    let dir = receipt_path.parent().unwrap_or(Path::new("."));
    let combined_row = member(expected, "combined")?;
    let cue_row = member(combined_row, "cue")?;
    let bin_row = member(combined_row, "bin")?;
    let cue = dir.join(basename(member(cue_row, "path")?)?);
    let image = dir.join(basename(member(bin_row, "path")?)?);
    verify_file_record(&cue, cue_row, "combined CUE")?;
    verify_file_record(&image, bin_row, "combined BIN")?;
    ensure!(
        image_for_cue(&cue)? == resolve(&image)?,
        "combined CUE does not reference the sealed BIN beside it"
    );
    let sectors = fs::metadata(&image)?.len() / SECTOR_BYTES;
    ensure!(
        combined_row.get("sectors").and_then(Value::as_u64) == Some(sectors),
        "combined sector count no longer matches the receipt"
    );
    let entries = parse_toc(&disc::read_user_sectors(
        &image,
        disc::TOC_LBA,
        disc::TOC_SECTORS,
    )?)?;
    let programs = member(expected, "programs")?
        .as_object()
        .ok_or_else(|| malformed("'programs'"))?;
    let names = pressed_programs(&programs.keys().cloned().collect::<Vec<_>>(), "program")?;
    check_toc_covered(&entries, &names)?;
    let mut ranges = Vec::new();
    for name in &names {
        let row = &programs[name];
        let missing = |what: &str| Error(format!("malformed receipt for {name}: missing {what}"));
        let embedded = row.get("embedded").ok_or_else(|| missing("'embedded'"))?;
        let recorded_exe = row
            .get("input")
            .and_then(|i| i.get("exe"))
            .ok_or_else(|| missing("'exe'"))?;
        let recorded_payload = row
            .get("input")
            .and_then(|i| i.get("payload"))
            .ok_or_else(|| missing("'payload'"))?;
        let source = row.get("source").ok_or_else(|| missing("'source'"))?;
        let entry = entries.iter().find(|e| &e.name == name).expect("covered");
        let exact = [
            ("exe_lba", Value::from(entry.exe_lba)),
            ("image_lba_start", Value::from(entry.image_lba)),
            ("version", Value::from(entry.version.clone())),
            ("flags", Value::from(entry.flags)),
        ];
        for (key, actual) in exact {
            ensure!(
                embedded.get(key) == Some(&actual),
                "{name}: sealed TOC {key} no longer matches receipt"
            );
        }
        let start = embedded["image_lba_start"]
            .as_u64()
            .ok_or_else(|| missing("'image_lba_start'"))?;
        let end = embedded["image_lba_end_exclusive"]
            .as_u64()
            .ok_or_else(|| missing("'image_lba_end_exclusive'"))?;
        let image_sectors = embedded["image_sectors"]
            .as_u64()
            .ok_or_else(|| missing("'image_sectors'"))?;
        ensure!(
            end == start + image_sectors && end <= sectors,
            "{name}: invalid sealed image range"
        );
        ranges.push((start, end, name.clone()));
        let exe = parse_exe_at(&image, entry.exe_lba as u64)?;
        verify_sealed_exe(name, &exe, recorded_exe, recorded_payload)?;
        ensure!(
            embedded.get("payload_lba_start").and_then(Value::as_u64)
                == Some(entry.exe_lba as u64 + 1),
            "{name}: invalid sealed payload start"
        );
        ensure!(
            embedded
                .get("payload_lba_end_exclusive")
                .and_then(Value::as_u64)
                == Some(
                    entry.exe_lba as u64 + 1 + exe.payload_bytes as u64 / USER_DATA_BYTES as u64
                ),
            "{name}: invalid sealed payload end"
        );
        ensure!(
            source.get("kind").and_then(Value::as_str) == Some("clean-git-commit")
                && source.get("tree_clean").and_then(Value::as_bool) == Some(true),
            "{name}: receipt lacks clean source provenance"
        );
        ensure!(
            is_revision(source.get("revision").and_then(Value::as_str).unwrap_or("")),
            "{name}: invalid recorded source revision"
        );
        if name == "HALF-LIFE" {
            verify_sealed_cook(name, row, source)?;
        }
    }
    check_no_overlap(&mut ranges)
}

fn verify_sealed_exe(
    name: &str,
    exe: &Exe,
    recorded_exe: &Value,
    recorded_payload: &Value,
) -> Result<()> {
    let mut exe_bytes = exe.header.clone();
    exe_bytes.extend(&exe.payload);
    ensure!(
        recorded_exe.get("bytes").and_then(Value::as_u64) == Some(exe_bytes.len() as u64),
        "{name}: embedded EXE size no longer matches receipt"
    );
    ensure!(
        recorded_exe.get("sha256").and_then(Value::as_str)
            == Some(sha256_bytes(&exe_bytes).as_str()),
        "{name}: embedded EXE SHA-256 no longer matches receipt"
    );
    let actual = json!({
        "bytes": exe.payload_bytes,
        "sha256": sha256_bytes(&exe.payload),
        "fnv1a32": hex32(fnv1a32(&exe.payload)),
        "pc": hex32(exe.pc),
        "load": hex32(exe.load),
    });
    ensure!(
        &actual == recorded_payload,
        "{name}: embedded payload no longer matches receipt"
    );
    Ok(())
}

fn verify_sealed_cook(name: &str, row: &Value, source: &Value) -> Result<()> {
    let cooked = row.get("cooked_assets").ok_or_else(|| {
        Error(format!(
            "{name}: sealed receipt lacks cooked-asset provenance"
        ))
    })?;
    let (Some(manifest_file), Some(manifest)) = (cooked.get("manifest"), cooked.get("document"))
    else {
        bail!("{name}: sealed receipt lacks cooked-asset provenance");
    };
    ensure!(
        cooked.get("verified") == Some(&Value::Bool(true)),
        "{name}: cooked-asset provenance was not verified"
    );
    ensure!(
        manifest_file
            .get("bytes")
            .and_then(Value::as_u64)
            .is_some_and(|b| b > 0)
            && is_digest(
                manifest_file
                    .get("sha256")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            ),
        "{name}: invalid sealed cooked manifest record"
    );
    ensure!(
        manifest.get("schema").and_then(Value::as_u64) == Some(HL_COOK_SCHEMA)
            && manifest.get("hl_psx_revision") == source.get("revision"),
        "{name}: cooked manifest does not match source"
    );
    for field in [
        "hl_psx_tree_sha256",
        "psoxide_tree_sha256",
        "half_life_input_sha256",
        "cooked_tree_sha256",
    ] {
        ensure!(
            is_digest(manifest.get(field).and_then(Value::as_str).unwrap_or("")),
            "{name}: invalid sealed cooked {field}"
        );
    }
    Ok(())
}

fn create(args: &Args) -> Result<()> {
    let programs = exact_assignments(&args.all("program"), "program")?;
    let sources = exact_assignments(&args.all("source"), "source")?;
    ensure!(
        !programs.is_empty() && !sources.is_empty(),
        "--program and --source are required"
    );
    let document = build_document(
        &args.require_path("combined-cue")?,
        &args.require_path("frontend")?,
        args.require("build-command")?,
        &programs,
        &sources,
    )?;
    let out = args.require_path("out")?;
    write_receipt(&out, &document)?;
    println!(
        "release receipt: {}",
        crate::util::resolve_lenient(&out).display()
    );
    println!(
        "combined cue SHA-256: {}",
        document["combined"]["cue"]["sha256"].as_str().unwrap_or("")
    );
    println!(
        "combined bin SHA-256: {}",
        document["combined"]["bin"]["sha256"].as_str().unwrap_or("")
    );
    Ok(())
}

fn verify(args: &Args) -> Result<()> {
    let receipt_path = resolve(&args.require_path("receipt")?)?;
    let text = fs::read(&receipt_path)?;
    ensure!(
        text.is_ascii(),
        "receipt is not ASCII: {}",
        receipt_path.display()
    );
    let expected: Value = serde_json::from_slice(&text)?;
    ensure!(
        expected.get("schema").and_then(Value::as_str) == Some(SCHEMA),
        "unsupported receipt schema: {}",
        expected
            .get("schema")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "None".into())
    );
    if args.has("sealed") {
        verify_sealed_document(&receipt_path, &expected)?;
        println!(
            "sealed release receipt verified: {}",
            receipt_path.display()
        );
        return Ok(());
    }
    let program_rows = member(&expected, "programs")?
        .as_object()
        .ok_or_else(|| malformed("'programs'"))?;
    let names = pressed_programs(&program_rows.keys().cloned().collect::<Vec<_>>(), "program")?;
    let mut programs = Vec::new();
    let mut sources = Vec::new();
    for name in &names {
        let row = &program_rows[name];
        let cue = row
            .pointer("/input/cue/path")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("'path'"))?;
        let source = row
            .pointer("/source/path")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("'path'"))?;
        programs.push((name.clone(), PathBuf::from(cue)));
        sources.push((name.clone(), PathBuf::from(source)));
    }
    let actual = build_document(
        Path::new(
            member(member(member(&expected, "combined")?, "cue")?, "path")?
                .as_str()
                .ok_or_else(|| malformed("'path'"))?,
        ),
        Path::new(
            member(member(&expected, "frontend")?, "path")?
                .as_str()
                .ok_or_else(|| malformed("'path'"))?,
        ),
        member(&expected, "build_command")?
            .as_str()
            .ok_or_else(|| malformed("'build_command'"))?,
        &programs,
        &sources,
    )?;
    ensure!(
        actual == expected,
        "receipt no longer matches its sources or artifacts"
    );
    println!("release receipt verified: {}", receipt_path.display());
    Ok(())
}

pub fn run(raw: &[String]) -> Result<i32> {
    let Some((command, rest)) = raw.split_first() else {
        bail!("usage: release-receipt create|verify [options]");
    };
    match command.as_str() {
        "create" => {
            let args = Args::parse(
                rest,
                &[
                    "combined-cue",
                    "frontend",
                    "build-command",
                    "program",
                    "source",
                    "out",
                ],
                &[],
            )?;
            create(&args)?;
        }
        "verify" => verify(&Args::parse(rest, &["receipt"], &["sealed"])?)?,
        other => bail!("unknown release-receipt command {other:?}"),
    }
    Ok(0)
}

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;
