//! The fail-closed checks on the Quake payload: the clean checkouts, the cue
//! and bin, the provenance sidecar, and the combined disc it is pressed into.
//! Every check either proves its piece of the contract or stops the build.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::json::parse_without_duplicate_keys;
use crate::disc::{self, text_field};
use crate::util::{git, is_hex, resolve_lenient, sha256_file, Error, Result};

pub const SECTOR_BYTES: u64 = disc::SECTOR_BYTES;
pub const BOOT_EXE_LBA: u64 = 22;
pub const PSOXIDE_REV_FILE: &str = "host/quake-build/main.rs";
pub const REDISTRIBUTION_STATUS: &str =
    "owner-approved public non-commercial release of canonical Quake 1.06 \
     shareware payload, 2026-08-25";
pub const QUAKE_PROVENANCE_SCHEMA: i128 = 1;
pub const GUEST_STAGE_SCHEMA: i128 = 1;
pub const SHAREWARE_PAK_SHA256: &str =
    "35a9c55e5e5a284a159ad2a62e0e8def23d829561fe2f54eb402dbc0a9a946af";
pub const SHAREWARE_PAK_BYTES: i128 = 18_689_235;
pub const PSOXIDE_SOURCE_KIND: &str = "local_checkout";
const PSOXIDE_SOURCE_KINDS: [&str; 2] = [PSOXIDE_SOURCE_KIND, "pinned_hydration"];
pub const BUILD_PROFILE: &str = "release";
const QUAKE_ENTRY_NAME: &str = "QUAKE SHAREWARE";

/// What a passing `verify` proves, and what the receipt then records.
#[derive(Debug, Clone)]
pub struct Verified {
    pub source_revision: String,
    pub declared_psoxide_revision: String,
    pub psoxide_revision: String,
    pub programs_psoxide_revision: String,
    pub provenance: PathBuf,
    pub provenance_sha256: String,
    pub cue: PathBuf,
    pub bin: PathBuf,
    pub exe: PathBuf,
    pub cue_sha256: String,
    pub bin_sha256: String,
    pub exe_sha256: String,
    pub cue_bytes: u64,
    pub bin_bytes: u64,
    pub exe_bytes: u64,
    pub psoxide_source_kind: String,
    pub pak0_sha256: String,
    pub pak0_bytes: i128,
    pub guest_stage_schema: i128,
    pub guest_recipe_sha256: String,
    pub rust_toolchain_sha256: String,
    pub rustc_version: String,
    pub cargo_version: String,
    pub profile: String,
    pub features: Vec<String>,
}

/// The Quake row of the combined disc's demo table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeTocEntry {
    pub exe_lba: u32,
    pub lba_offset: u32,
    pub cdda_track_base: u32,
    pub payload_fnv: u32,
    pub version: String,
    pub description: String,
}

/// The pins and paths `verify` and `receipt` share.
#[derive(Debug, Clone, Default)]
pub struct VerifyInputs {
    pub source: PathBuf,
    pub psoxide: PathBuf,
    pub programs_psoxide: Option<PathBuf>,
    pub programs_psoxide_stamp: PathBuf,
    pub cue: PathBuf,
    pub provenance: PathBuf,
    pub expected_revision: String,
    pub expected_psoxide_revision: String,
    pub expected_programs_psoxide_revision: Option<String>,
    pub expected_provenance_sha256: String,
    pub expected_cue_sha256: String,
    pub expected_bin_sha256: String,
    pub expected_exe_sha256: String,
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("static regex"))
}

/// A value that must be a whole lowercase hex string of the right length.
pub fn require_hex(value: &str, length: usize, label: &str) -> Result<String> {
    let normal = value.trim();
    ensure!(
        is_hex(normal, length),
        "{label} must be a full lowercase hexadecimal value"
    );
    Ok(normal.to_string())
}

pub fn require_revision(value: &str, label: &str) -> Result<String> {
    require_hex(value, 40, label)
}

pub fn require_digest(value: &str, label: &str) -> Result<String> {
    require_hex(value, 64, label)
}

pub fn require_object<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| Error(format!("{label} must be a JSON object")))
}

pub fn require_string<'a>(value: &'a Value, label: &str) -> Result<&'a str> {
    match value.as_str() {
        Some(text) if !text.is_empty() => Ok(text),
        _ => bail!("{label} must be a non-empty string"),
    }
}

/// An integer, not a float and not a boolean (Python's `type(x) is int`).
pub fn require_integer(value: &Value, label: &str) -> Result<i128> {
    match value {
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => Ok(i128::from(i)),
            (None, Some(u)) => Ok(i128::from(u)),
            _ => bail!("{label} must be an integer"),
        },
        _ => bail!("{label} must be an integer"),
    }
}

pub fn member<'a>(
    container: &'a serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<&'a Value> {
    container
        .get(key)
        .ok_or_else(|| Error(format!("{label} is missing required field {key:?}")))
}

/// A string field, required to be present, non-empty and a lowercase hex
/// value of `length`. `label` names it in the message.
fn hex_member(
    container: &serde_json::Map<String, Value>,
    key: &str,
    parent: &str,
    length: usize,
    label: &str,
) -> Result<String> {
    let text = require_string(member(container, key, parent)?, &format!("{parent}.{key}"))?;
    require_hex(text, length, label)
}

/// Python's text-mode reads turn `\r\n` and a lone `\r` into `\n`. The
/// stamp, the cue and the source declaration are all read that way, so a file
/// saved with Windows line endings is still one clean line per line.
pub fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// `resolve(strict=True)` whose only soft failure is "not there": other I/O
/// errors keep their own text, so a permissions problem is not misreported.
fn resolve_existing(path: &Path, missing: impl FnOnce() -> String) -> Result<PathBuf> {
    match path.canonicalize() {
        Ok(real) => Ok(real),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error(missing())),
        Err(e) => Err(Error(format!("{}: {e}", path.display()))),
    }
}

/// The checkout must be the repository root, at exactly the expected commit,
/// with nothing modified or untracked. Returns the resolved root and the
/// revision.
pub fn verify_clean_checkout(
    source: &Path,
    expected: &str,
    label: &str,
) -> Result<(PathBuf, String)> {
    let source = resolve_existing(source, || {
        format!("{label} checkout does not exist: {}", source.display())
    })?;
    ensure!(
        source.is_dir(),
        "{label} checkout is not a directory: {}",
        source.display()
    );
    let top = resolve_lenient(Path::new(&git(&source, &["rev-parse", "--show-toplevel"])?));
    ensure!(
        top == source,
        "{label} checkout must name the repository root: {} != {}",
        source.display(),
        top.display()
    );
    let revision = git(&source, &["rev-parse", "--verify", "HEAD^{commit}"])?.to_lowercase();
    ensure!(
        revision == expected,
        "{label} checkout revision mismatch: expected {expected}, got {revision}"
    );
    let dirty = git(
        &source,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )?;
    ensure!(
        dirty.is_empty(),
        "{label} checkout is dirty; exact revision provenance is false"
    );
    Ok((source, revision))
}

/// The one `const PSOXIDE_REV` the Quake build crate declares: the SDK
/// revision Quake was built against, which the disc must be pressed with.
pub fn declared_psoxide_revision(source: &Path) -> Result<String> {
    static LINE: OnceLock<Regex> = OnceLock::new();
    static DECLARATION: OnceLock<Regex> = OnceLock::new();
    let path = source.join(PSOXIDE_REV_FILE);
    let text = std::fs::read_to_string(&path)
        .map(|text| universal_newlines(&text))
        .map_err(|e| {
            Error(format!(
                "cannot read Quake PSOXIDE_REV declaration {}: {e}",
                path.display()
            ))
        })?;
    let line = regex(
        &LINE,
        r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+PSOXIDE_REV\b[^\n]*$",
    );
    let lines: Vec<&str> = line.find_iter(&text).map(|m| m.as_str()).collect();
    ensure!(
        lines.len() == 1,
        "{}: expected exactly one PSOXIDE_REV const declaration, found {}",
        path.display(),
        lines.len()
    );
    let declaration = regex(
        &DECLARATION,
        r#"(?m)\A(?:^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+PSOXIDE_REV\s*:\s*&str\s*=\s*"([^"]*)"\s*;\s*$)\z"#,
    );
    let Some(found) = declaration.captures(lines[0]) else {
        bail!(
            "{}: malformed PSOXIDE_REV const declaration",
            path.display()
        );
    };
    require_revision(&found[1], "Quake PSOXIDE_REV")
}

/// The stamp `make disc` writes: the SDK revision the ordinary programs were
/// really built with. One line, newline terminated, and equal to the checkout.
pub fn verify_programs_revision_stamp(stamp: &Path, expected: &str) -> Result<String> {
    let bytes = match std::fs::read(stamp) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
            "ordinary-program SDK revision stamp does not exist: {}; run 'make disc'",
            stamp.display()
        ),
        Err(e) => bail!(
            "cannot read ordinary-program SDK revision stamp {}: {e}",
            stamp.display()
        ),
    };
    ensure!(
        bytes.is_ascii(),
        "cannot read ordinary-program SDK revision stamp {}: not ASCII",
        stamp.display()
    );
    let text = universal_newlines(&String::from_utf8_lossy(&bytes));
    // Exactly one line: a body with no line break in it, ended by one "\n".
    // These are the separators Python's splitlines() honours in ASCII.
    let body = text.strip_suffix('\n');
    let one_line =
        body.is_some_and(|b| !b.contains(['\n', '\r', '\x0b', '\x0c', '\x1c', '\x1d', '\x1e']));
    ensure!(
        one_line,
        "ordinary-program SDK revision stamp is malformed: {}",
        stamp.display()
    );
    let revision = require_revision(body.unwrap_or(""), "ordinary-program SDK revision stamp")?;
    ensure!(
        revision == expected,
        "ordinary-program SDK revision mismatch: stamp has {revision}, but PSoXide checkout is {expected}; \
         run 'make disc'"
    );
    Ok(revision)
}

/// The components of a relative path the way Python's `Path` sees them: empty
/// and `.` parts vanish, `..` stays. `FILE "./x.bin"` is therefore one part.
fn path_parts(name: &str) -> Vec<&str> {
    name.split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

/// The one bin a cue names. It has to sit beside the cue, and when
/// `data_only` the cue must be exactly one MODE2/2352 data track at 00:00:00,
/// which is what the pinned Quake input is.
pub fn cue_bin(cue: &Path, data_only: bool) -> Result<PathBuf> {
    static FILE: OnceLock<Regex> = OnceLock::new();
    static TRACK: OnceLock<Regex> = OnceLock::new();
    static INDEX: OnceLock<Regex> = OnceLock::new();
    let cue = resolve_existing(cue, || format!("cue does not exist: {}", cue.display()))?;
    ensure!(
        cue.is_file(),
        "cue is not a regular file: {}",
        cue.display()
    );
    let bytes = std::fs::read(&cue)
        .map_err(|e| Error(format!("cannot read ASCII cue {}: {e}", cue.display())))?;
    ensure!(
        bytes.is_ascii(),
        "cannot read ASCII cue {}: not ASCII",
        cue.display()
    );
    let text = universal_newlines(&String::from_utf8_lossy(&bytes));

    let files: Vec<String> = regex(&FILE, r#"(?mi)^\s*FILE\s+"([^"]+)"\s+BINARY\s*$"#)
        .captures_iter(&text)
        .map(|c| c[1].to_string())
        .collect();
    ensure!(
        files.len() == 1,
        "{}: expected exactly one quoted FILE ... BINARY line, found {}",
        cue.display(),
        files.len()
    );
    let parts = path_parts(&files[0]);
    ensure!(
        !files[0].starts_with('/') && parts.len() == 1 && parts[0] != "..",
        "{}: FILE must name one bin beside the cue",
        cue.display()
    );
    let parent = cue.parent().unwrap_or(Path::new("/")).to_path_buf();
    let bin_path = parent.join(parts[0]);
    let resolved_bin = resolve_existing(&bin_path, || {
        format!("cue bin does not exist: {}", bin_path.display())
    })?;
    ensure!(
        resolved_bin.parent() == Some(parent.as_path()) && resolved_bin.is_file(),
        "{}: FILE must resolve to a regular bin beside the cue",
        cue.display()
    );

    let tracks: Vec<(String, String)> =
        regex(&TRACK, r"(?mi)^\s*TRACK\s+([0-9]{2})\s+([^\s]+)\s*$")
            .captures_iter(&text)
            .map(|c| (c[1].to_string(), c[2].to_uppercase()))
            .collect();
    let data_track = ("01".to_string(), "MODE2/2352".to_string());
    ensure!(
        tracks.first() == Some(&data_track),
        "{}: first track must be TRACK 01 MODE2/2352",
        cue.display()
    );
    ensure!(
        !data_only || tracks == [data_track],
        "{}: pinned Quake input must contain one data track only",
        cue.display()
    );
    let indices: Vec<(String, String)> = regex(
        &INDEX,
        r"(?mi)^\s*INDEX\s+([0-9]{2})\s+([0-9]{2}:[0-9]{2}:[0-9]{2})\s*$",
    )
    .captures_iter(&text)
    .map(|c| (c[1].to_string(), c[2].to_string()))
    .collect();
    ensure!(
        !data_only || indices == [("01".to_string(), "00:00:00".to_string())],
        "{}: pinned Quake input must start at INDEX 01 00:00:00",
        cue.display()
    );
    Ok(resolved_bin)
}

/// The `QUAKE SHAREWARE` row of the combined disc, held against the pinned
/// Quake revision: where its image sits, what version the menu shows.
pub fn quake_toc_entry(demo_bin: &Path, expected_revision: &str) -> Result<QuakeTocEntry> {
    let toc = disc::read_user_sectors(demo_bin, disc::TOC_LBA, disc::TOC_SECTORS)?;
    ensure!(
        &toc[..8] == disc::TOC_MAGIC,
        "{}: no {} table at LBA {}",
        demo_bin.display(),
        String::from_utf8_lossy(disc::TOC_MAGIC),
        disc::TOC_LBA
    );
    let count = disc::le32(&toc, 8) as usize;
    ensure!(
        count != 0 && count <= disc::TOC_MAX_ENTRIES,
        "{}: invalid demo table entry count {count}",
        demo_bin.display()
    );

    let mut matches = Vec::new();
    for index in 0..count {
        let at = disc::TOC_HEADER_BYTES + index * disc::TOC_ENTRY_BYTES;
        let entry = &toc[at..at + disc::TOC_ENTRY_BYTES];
        if text_field(&entry[..disc::TOC_NAME_BYTES])? != QUAKE_ENTRY_NAME {
            continue;
        }
        let numbers = disc::TOC_NAME_BYTES;
        let description_at = numbers + 16;
        let version_at = description_at + 2 * disc::TOC_DESC_BYTES;
        matches.push(QuakeTocEntry {
            exe_lba: disc::le32(entry, numbers),
            lba_offset: disc::le32(entry, numbers + 4),
            cdda_track_base: disc::le32(entry, numbers + 8),
            payload_fnv: disc::le32(entry, numbers + 12),
            version: text_field(&entry[version_at..version_at + disc::TOC_VERSION_BYTES])?,
            description: text_field(&entry[description_at..description_at + disc::TOC_DESC_BYTES])?,
        });
    }
    ensure!(
        matches.len() == 1,
        "{}: expected exactly one {QUAKE_ENTRY_NAME} table entry, found {}",
        demo_bin.display(),
        matches.len()
    );
    let entry = matches.remove(0);
    ensure!(
        u64::from(entry.lba_offset) >= disc::TOC_LBA + disc::TOC_SECTORS,
        "{}: Quake image overlaps the demo ISO",
        demo_bin.display()
    );
    ensure!(
        u64::from(entry.exe_lba) == u64::from(entry.lba_offset) + BOOT_EXE_LBA,
        "{}: Quake EXE LBA {} does not match image offset {} + boot LBA {BOOT_EXE_LBA}",
        demo_bin.display(),
        entry.exe_lba,
        entry.lba_offset
    );
    let expected_version = format!("q{}", &expected_revision[..7]);
    ensure!(
        entry.version == expected_version,
        "{}: Quake menu version is {:?}, expected {expected_version:?}",
        demo_bin.display(),
        entry.version
    );
    ensure!(
        entry.payload_fnv != 0,
        "{}: Quake loader payload checksum is zero",
        demo_bin.display()
    );
    ensure!(
        entry.description.to_lowercase().contains("shareware"),
        "{}: Quake description does not identify the shareware release",
        demo_bin.display()
    );
    Ok(entry)
}

/// Read into `buffer` until it is full or the file ends; returns how many
/// bytes arrived.
fn read_fully(stream: &mut File, buffer: &mut [u8]) -> Result<usize> {
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

/// The Quake bin sits inside the combined disc sector for sector. The only
/// bytes allowed to differ are the three MSF address bytes of each sector
/// header, because mkdisc rewrites them for the relocated position.
pub fn verify_embedded_image(demo_bin: &Path, quake_bin: &Path, lba_offset: u32) -> Result<u64> {
    let sectors = std::fs::metadata(quake_bin)?.len() / SECTOR_BYTES;
    let mut source = File::open(quake_bin)?;
    let mut combined = File::open(demo_bin)?;
    combined.seek(SeekFrom::Start(u64::from(lba_offset) * SECTOR_BYTES))?;
    let mut expected = vec![0u8; SECTOR_BYTES as usize];
    let mut actual = vec![0u8; SECTOR_BYTES as usize];
    for sector in 0..sectors {
        read_fully(&mut source, &mut expected)?;
        let got = read_fully(&mut combined, &mut actual)?;
        ensure!(
            got == actual.len(),
            "{}: embedded Quake image ends at sector {sector} of {sectors}",
            demo_bin.display()
        );
        ensure!(
            expected[..12] == actual[..12] && expected[15..] == actual[15..],
            "{}: embedded Quake sector {sector} differs outside relocated MSF bytes",
            demo_bin.display()
        );
    }
    Ok(sectors)
}

/// One artifact the sidecar vouches for, once its file, size and hash have
/// all been checked.
struct Artifact {
    path: PathBuf,
    sha256: String,
    bytes: u64,
}

/// A sidecar `artifacts.<key>` record: its file has to sit beside the cue
/// (and be the expected one), and its recorded size and SHA-256 have to match
/// both the bytes on disk and the pin the Makefile holds.
fn verify_artifact(
    artifacts: &serde_json::Map<String, Value>,
    key: &str,
    artifact_dir: &Path,
    expected_path: &Path,
    expected_sha256: &str,
) -> Result<Artifact> {
    let label = format!("provenance artifacts.{key}");
    let record = require_object(member(artifacts, key, "provenance artifacts")?, &label)?;
    let name = require_string(member(record, "file", &label)?, &format!("{label}.file"))?;
    let parts = path_parts(name);
    ensure!(
        !name.starts_with('/') && parts.len() == 1 && parts[0] != "..",
        "{label}.file must be one basename"
    );
    let candidate = artifact_dir.join(parts[0]);
    let path = resolve_existing(&candidate, || {
        format!("{label} does not exist: {}", candidate.display())
    })?;
    ensure!(
        path.parent() == Some(artifact_dir) && path.is_file(),
        "{label}.file must resolve beside the Quake cue"
    );
    ensure!(
        path == expected_path,
        "{label}.file names {:?}, expected {:?}",
        file_name(&path),
        file_name(expected_path)
    );

    let recorded_sha256 = hex_member(record, "sha256", &label, 64, &format!("{label}.sha256"))?;
    let recorded_bytes =
        require_integer(member(record, "bytes", &label)?, &format!("{label}.bytes"))?;
    let actual_bytes = std::fs::metadata(&path)?.len();
    ensure!(
        recorded_bytes == i128::from(actual_bytes),
        "{label} byte-size mismatch: sidecar has {recorded_bytes}, actual is {actual_bytes}"
    );
    let actual_sha256 = sha256_file(&path)?;
    ensure!(
        recorded_sha256 == actual_sha256,
        "{label} SHA-256 mismatch: sidecar has {recorded_sha256}, actual is {actual_sha256}"
    );
    ensure!(
        actual_sha256 == expected_sha256,
        "Quake {key} SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
    );
    Ok(Artifact {
        path,
        sha256: actual_sha256,
        bytes: actual_bytes,
    })
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Everything the sidecar proved, for `verify_quake` to fold into `Verified`.
struct Sidecar {
    path: PathBuf,
    sha256: String,
    cue: Artifact,
    bin: Artifact,
    exe: Artifact,
    source_kind: String,
    pak0_sha256: String,
    pak0_bytes: i128,
    guest_stage_schema: i128,
    guest_recipe_sha256: String,
    rust_toolchain_sha256: String,
    rustc_version: String,
    cargo_version: String,
    profile: String,
    features: Vec<String>,
}

/// The pins `verify_provenance` holds the sidecar against.
struct Pins<'a> {
    source_revision: &'a str,
    psoxide_revision: &'a str,
    provenance_sha256: &'a str,
    cue_sha256: &'a str,
    bin_sha256: &'a str,
    exe_sha256: &'a str,
}

/// A `tree_clean` claim must be literally `true`.
fn require_clean(parent: &serde_json::Map<String, Value>, label: &str, what: &str) -> Result<()> {
    ensure!(
        member(parent, "tree_clean", label)? == &Value::Bool(true),
        "provenance must record a clean {what} source tree"
    );
    Ok(())
}

/// Read `provenance`, require it to be the sidecar beside the cue, parse it
/// strictly and hold every claim in it against the checkouts and the pins.
fn verify_provenance(
    provenance: &Path,
    cue: &Path,
    bin_path: &Path,
    pins: &Pins,
) -> Result<Sidecar> {
    let resolved = resolve_existing(provenance, || {
        format!(
            "Quake provenance sidecar does not exist: {}",
            provenance.display()
        )
    })?;
    ensure!(
        resolved.is_file(),
        "Quake provenance sidecar is not a file: {}",
        resolved.display()
    );
    let expected_sidecar = cue.with_extension("provenance.json");
    ensure!(
        resolved == expected_sidecar,
        "Quake provenance sidecar must be beside the cue and named {:?}",
        file_name(&expected_sidecar)
    );
    let raw = std::fs::read(&resolved)?;
    let cannot_parse = |why: String| {
        Error(format!(
            "cannot parse Quake provenance sidecar {}: {why}",
            resolved.display()
        ))
    };
    ensure!(
        raw.is_ascii(),
        "cannot parse Quake provenance sidecar {}: not ASCII",
        resolved.display()
    );
    let text = String::from_utf8_lossy(&raw).into_owned();
    let document = parse_without_duplicate_keys(&text)?.map_err(cannot_parse)?;
    let root = require_object(&document, "Quake provenance")?;
    let schema = require_integer(
        member(root, "schema", "Quake provenance")?,
        "provenance schema",
    )?;
    ensure!(
        schema == QUAKE_PROVENANCE_SCHEMA,
        "unsupported Quake provenance schema {schema}; expected {QUAKE_PROVENANCE_SCHEMA}"
    );

    let quake_source = require_object(
        member(root, "quake_source", "Quake provenance")?,
        "provenance quake_source",
    )?;
    let recorded = hex_member(
        quake_source,
        "revision",
        "provenance quake_source",
        40,
        "provenance Quake revision",
    )?;
    ensure!(
        recorded == pins.source_revision,
        "provenance Quake revision mismatch: expected {}, got {recorded}",
        pins.source_revision
    );
    require_clean(quake_source, "provenance quake_source", "Quake")?;

    let psoxide = require_object(
        member(root, "psoxide", "Quake provenance")?,
        "provenance psoxide",
    )?;
    let recorded = hex_member(
        psoxide,
        "revision",
        "provenance psoxide",
        40,
        "provenance PSoXide revision",
    )?;
    ensure!(
        recorded == pins.psoxide_revision,
        "provenance PSoXide revision mismatch: expected {}, got {recorded}",
        pins.psoxide_revision
    );
    require_clean(psoxide, "provenance psoxide", "PSoXide")?;
    let source_kind = require_string(
        member(psoxide, "source_kind", "provenance psoxide")?,
        "provenance psoxide.source_kind",
    )?;
    ensure!(
        PSOXIDE_SOURCE_KINDS.contains(&source_kind),
        "provenance PSoXide source kind must describe a clean reproducible checkout ({}), got {source_kind:?}",
        sorted_kinds()
    );

    let shareware = require_object(
        member(root, "shareware", "Quake provenance")?,
        "provenance shareware",
    )?;
    let pak0_sha256 = hex_member(
        shareware,
        "pak0_sha256",
        "provenance shareware",
        64,
        "provenance shareware PAK0 SHA-256",
    )?;
    let pak0_bytes = require_integer(
        member(shareware, "pak0_bytes", "provenance shareware")?,
        "provenance shareware.pak0_bytes",
    )?;
    ensure!(
        pak0_sha256 == SHAREWARE_PAK_SHA256 && pak0_bytes == SHAREWARE_PAK_BYTES,
        "provenance does not identify the canonical Quake 1.06 shareware PAK0"
    );
    let build = require_object(
        member(root, "build", "Quake provenance")?,
        "provenance build",
    )?;
    let guest_stage_schema = require_integer(
        member(build, "guest_stage_schema", "provenance build")?,
        "provenance build.guest_stage_schema",
    )?;
    ensure!(
        guest_stage_schema == GUEST_STAGE_SCHEMA,
        "unsupported guest-stage schema {guest_stage_schema}; expected {GUEST_STAGE_SCHEMA}"
    );
    let guest_recipe_sha256 = hex_member(
        build,
        "guest_recipe_sha256",
        "provenance build",
        64,
        "provenance guest recipe SHA-256",
    )?;
    let rust_toolchain_sha256 = hex_member(
        build,
        "rust_toolchain_sha256",
        "provenance build",
        64,
        "provenance Rust toolchain SHA-256",
    )?;
    let rustc_version = require_string(
        member(build, "rustc_version", "provenance build")?,
        "provenance build.rustc_version",
    )?;
    let cargo_version = require_string(
        member(build, "cargo_version", "provenance build")?,
        "provenance build.cargo_version",
    )?;
    ensure!(
        rustc_version.starts_with("rustc ") && cargo_version.starts_with("cargo "),
        "provenance build must contain verbose rustc and cargo identities"
    );
    let profile = require_string(
        member(build, "profile", "provenance build")?,
        "provenance build.profile",
    )?;
    let features = match member(build, "features", "provenance build")? {
        Value::Array(items) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(|f| f.as_str().map(str::to_string))
            .collect::<Vec<_>>(),
        _ => bail!("provenance build.features must be a string array"),
    };
    ensure!(
        profile == BUILD_PROFILE && features.is_empty(),
        "shipping provenance must record release profile with no Cargo features"
    );

    let artifacts = require_object(
        member(root, "artifacts", "Quake provenance")?,
        "provenance artifacts",
    )?;
    let artifact_dir = resolve_lenient(cue.parent().unwrap_or(Path::new("/")));
    let cue_artifact = verify_artifact(artifacts, "cue", &artifact_dir, cue, pins.cue_sha256)?;
    let bin_artifact = verify_artifact(artifacts, "bin", &artifact_dir, bin_path, pins.bin_sha256)?;
    let exe_artifact = verify_artifact(
        artifacts,
        "exe",
        &artifact_dir,
        &cue.with_extension("exe"),
        pins.exe_sha256,
    )?;
    let provenance_sha256 = sha256_file(&resolved)?;
    ensure!(
        provenance_sha256 == pins.provenance_sha256,
        "Quake provenance SHA-256 mismatch: expected {}, got {provenance_sha256}",
        pins.provenance_sha256
    );
    Ok(Sidecar {
        path: resolved,
        sha256: provenance_sha256,
        cue: cue_artifact,
        bin: bin_artifact,
        exe: exe_artifact,
        source_kind: source_kind.to_string(),
        pak0_sha256,
        pak0_bytes,
        guest_stage_schema,
        guest_recipe_sha256,
        rust_toolchain_sha256,
        rustc_version: rustc_version.to_string(),
        cargo_version: cargo_version.to_string(),
        profile: profile.to_string(),
        features,
    })
}

/// The accepted source kinds, sorted, for the refusal message.
fn sorted_kinds() -> String {
    let mut kinds = PSOXIDE_SOURCE_KINDS;
    kinds.sort_unstable();
    kinds.join(", ")
}

/// Why a component lookup failed: the file or key was not usable (reworded as
/// "missing or invalid"), or it was fine and the contents disagree (a verdict).
enum Fault {
    Invalid(String),
    Mismatch(String),
}

impl From<String> for Fault {
    fn from(message: String) -> Self {
        Fault::Invalid(message)
    }
}

fn lookup<'a>(value: &'a Value, key: &str) -> std::result::Result<&'a Value, Fault> {
    match value {
        Value::Object(map) => map
            .get(key)
            .ok_or_else(|| Fault::Invalid(format!("missing key {key:?}"))),
        _ => Err(Fault::Invalid(format!(
            "cannot look up {key:?} in a non-object"
        ))),
    }
}

fn read_lock(path: &Path) -> std::result::Result<Value, Fault> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| Fault::Invalid(format!("{}: {e}", path.display())))
}

/// The sidecar's recorded component tuple has to be the one the Quake source
/// locks, the editor entry has to be the selected checkout, and the SDK and
/// emulator must agree with the editor's own lock.
fn verify_component_provenance(
    source: &Path,
    editor: &Path,
    provenance: &Path,
    revision: &str,
) -> Result<()> {
    let check = || -> std::result::Result<(), Fault> {
        let expected = lookup(
            &read_lock(&source.join("components.lock.json"))?,
            "components",
        )?
        .clone();
        let nested = lookup(
            &read_lock(&editor.join("components.lock.json"))?,
            "components",
        )?
        .clone();
        let recorded = lookup(&read_lock(provenance)?, "psoxide")?.clone();
        let repository = lookup(&recorded, "repository")?;
        if repository != "EBonura/PSoXide-editor" || lookup(&recorded, "components")? != &expected {
            return Err(Fault::Mismatch(
                "Quake component provenance differs from its source lock".into(),
            ));
        }
        let pinned = lookup(&expected, "editor")?;
        if lookup(pinned, "revision")? != revision || lookup(pinned, "repository")? != repository {
            return Err(Fault::Mismatch(
                "Quake editor component differs from the selected checkout".into(),
            ));
        }
        for name in ["sdk", "emulator"] {
            if lookup(&expected, name)? != lookup(&nested, name)? {
                return Err(Fault::Mismatch(format!(
                    "Quake {name} component differs from the selected editor lock"
                )));
            }
        }
        Ok(())
    };
    match check() {
        Ok(()) => Ok(()),
        Err(Fault::Mismatch(message)) => Err(Error(message)),
        Err(Fault::Invalid(why)) => bail!("missing or invalid Quake component provenance: {why}"),
    }
}

/// Verify the whole Quake contract, in the order that fails soonest and most
/// cheaply: the pins' shape, the three checkouts, the program stamp, the cue
/// and bin, the sidecar, the component provenance.
pub fn verify_quake(inputs: &VerifyInputs) -> Result<Verified> {
    let expected_revision = require_revision(&inputs.expected_revision, "expected revision")?;
    let expected_psoxide = require_revision(
        &inputs.expected_psoxide_revision,
        "expected PSoXide revision",
    )?;
    let programs_psoxide = inputs
        .programs_psoxide
        .clone()
        .unwrap_or_else(|| inputs.psoxide.clone());
    let expected_programs = require_revision(
        inputs
            .expected_programs_psoxide_revision
            .as_deref()
            .unwrap_or(&expected_psoxide),
        "expected ordinary-program PSoXide revision",
    )?;
    let provenance_pin = require_digest(
        &inputs.expected_provenance_sha256,
        "expected provenance SHA-256",
    )?;
    let cue_pin = require_digest(&inputs.expected_cue_sha256, "expected cue SHA-256")?;
    let bin_pin = require_digest(&inputs.expected_bin_sha256, "expected bin SHA-256")?;
    let exe_pin = require_digest(&inputs.expected_exe_sha256, "expected exe SHA-256")?;

    let (source, revision) =
        verify_clean_checkout(&inputs.source, &expected_revision, "Quake source")?;
    let declared_revision = declared_psoxide_revision(&source)?;
    let (_, psoxide_revision) =
        verify_clean_checkout(&inputs.psoxide, &expected_psoxide, "PSoXide")?;
    ensure!(
        declared_revision == psoxide_revision,
        "Quake PSOXIDE_REV mismatch: source declares {declared_revision}, but disc checkout is {psoxide_revision}"
    );
    let (_, programs_revision) = verify_clean_checkout(
        &programs_psoxide,
        &expected_programs,
        "ordinary-program PSoXide",
    )?;
    let programs_psoxide_revision =
        verify_programs_revision_stamp(&inputs.programs_psoxide_stamp, &programs_revision)?;

    let resolved_cue = resolve_existing(&inputs.cue, || {
        format!("cue does not exist: {}", inputs.cue.display())
    })?;
    let resolved_bin = cue_bin(&resolved_cue, true)?;
    let bin_bytes = std::fs::metadata(&resolved_bin)?.len();
    ensure!(
        bin_bytes != 0 && bin_bytes % SECTOR_BYTES == 0,
        "Quake bin is {bin_bytes} bytes, not a non-empty whole number of {SECTOR_BYTES}-byte sectors"
    );
    let mut stream = File::open(&resolved_bin)?;
    stream.seek(SeekFrom::Start(
        BOOT_EXE_LBA * SECTOR_BYTES + disc::USER_DATA_AT,
    ))?;
    let mut magic = [0u8; 8];
    let got = read_fully(&mut stream, &mut magic)?;
    ensure!(
        &magic[..got] == disc::PSX_EXE_MAGIC,
        "Quake bin has no PS-X EXE at the mkdisc boot LBA {BOOT_EXE_LBA}"
    );

    let pins = Pins {
        source_revision: &revision,
        psoxide_revision: &psoxide_revision,
        provenance_sha256: &provenance_pin,
        cue_sha256: &cue_pin,
        bin_sha256: &bin_pin,
        exe_sha256: &exe_pin,
    };
    let sidecar = verify_provenance(&inputs.provenance, &resolved_cue, &resolved_bin, &pins)?;
    verify_component_provenance(
        &source,
        &inputs.psoxide,
        &inputs.provenance,
        &psoxide_revision,
    )?;
    Ok(Verified {
        source_revision: revision,
        declared_psoxide_revision: declared_revision,
        psoxide_revision,
        programs_psoxide_revision,
        provenance: sidecar.path,
        provenance_sha256: sidecar.sha256,
        cue: sidecar.cue.path,
        bin: sidecar.bin.path,
        exe: sidecar.exe.path,
        cue_sha256: sidecar.cue.sha256,
        bin_sha256: sidecar.bin.sha256,
        exe_sha256: sidecar.exe.sha256,
        cue_bytes: sidecar.cue.bytes,
        bin_bytes: sidecar.bin.bytes,
        exe_bytes: sidecar.exe.bytes,
        psoxide_source_kind: sidecar.source_kind,
        pak0_sha256: sidecar.pak0_sha256,
        pak0_bytes: sidecar.pak0_bytes,
        guest_stage_schema: sidecar.guest_stage_schema,
        guest_recipe_sha256: sidecar.guest_recipe_sha256,
        rust_toolchain_sha256: sidecar.rust_toolchain_sha256,
        rustc_version: sidecar.rustc_version,
        cargo_version: sidecar.cargo_version,
        profile: sidecar.profile,
        features: sidecar.features,
    })
}
