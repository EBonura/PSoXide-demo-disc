//! Prove that what is about to be uploaded is the public edition and nothing
//! else.
//!
//!     disc-tools public-only [--allow TEXT]... [--same-disc] [--standalone TEXT] PATH...
//!
//! Every PATH is a staged file or a directory of them (the itch.io download
//! folder, the HTML5 folder, the Celeste package). The Half-Life, Hollow
//! Knight, Counter-Strike, WipEout and Ocarina of Time programs exist only on the private
//! edition, which stays in the local library; this refuses to bless a package
//! that carries any trace of them:
//!
//! * a disc image (`*.bin`, or the lazy web delivery's `*.bin.001`, `.002`, ...
//!   read as one image) must show exactly the public menu in its table of
//!   contents, with nothing hidden, and its ISO 9660 directory listing must
//!   not name any of the four;
//! * every staged file, every disc image byte and the decompressed web header
//!   sidecar are searched, case-insensitively, for the four programs' names
//!   and repository names;
//! * no staged file may be a component receipt, release receipt or provenance
//!   record, which name the pressed programs;
//! * with `--same-disc`, every disc image found must be byte-identical to the
//!   others (the download BIN and the web parts are one pressing).
//!
//! `--standalone TEXT` is for a single-game package (the Celeste disc): its
//! images are not the demo disc, so the table of contents is replaced by two
//! rules, the ISO holds only SYSTEM.CNF and PSX.EXE, and the image contains TEXT.
//!
//! `--allow TEXT` excuses a search hit whose surrounding 48 bytes contain TEXT
//! (case-insensitive), and each excused hit is printed. It exists for text that
//! belongs to a public program (a splash tagline), not for the programs.
//! Exit 0 when clean, 1 when anything fails.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::args::Args;
use crate::deploy::{parse_public_toc, PUBLIC_GAMES};
use crate::disc::{SECTOR_BYTES, TOC_LBA, TOC_SECTORS, USER_DATA_AT, USER_DATA_BYTES};
use crate::util::{hex, Error, Result};

/// What may not appear anywhere in a public file. Lower case; matched
/// case-insensitively.
const FORBIDDEN: &[&str] = &[
    "half-life",
    "half life",
    "halflife",
    "hollow knight",
    "hollowknight",
    "counter-strike",
    "counter strike",
    "counterstrike",
    "wipeout",
    "hl-psx",
    "hk-psx",
    "cs-psx",
    "hlgame",
    "hkpsx",
    "cspsx",
    "cstrike",
    "ocarina",
    "oot-psx",
];

/// File names that are receipts of a pressing, which list every program.
const RECEIPT_MARKERS: &[&str] = &[
    "components.json",
    "release-receipt",
    "provenance",
    "lineup",
    ".private",
];

const CONTEXT: usize = 48;

/// One disc image, possibly in numbered parts, read as a single byte range.
struct Image {
    label: String,
    parts: Vec<(PathBuf, u64)>,
}

impl Image {
    fn len(&self) -> u64 {
        self.parts.iter().map(|(_, n)| n).sum()
    }

    fn read_at(&self, mut offset: u64, buf: &mut [u8]) -> Result<()> {
        let mut filled = 0;
        for (path, size) in &self.parts {
            if offset >= *size {
                offset -= size;
                continue;
            }
            let mut file =
                File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
            file.seek(SeekFrom::Start(offset))?;
            let take = ((*size - offset) as usize).min(buf.len() - filled);
            file.read_exact(&mut buf[filled..filled + take])?;
            filled += take;
            offset = 0;
            if filled == buf.len() {
                return Ok(());
            }
        }
        bail!("{}: ended before {} bytes", self.label, buf.len())
    }

    fn user_sector(&self, lba: u64) -> Result<Vec<u8>> {
        let mut data = vec![0u8; USER_DATA_BYTES];
        self.read_at(lba * SECTOR_BYTES + USER_DATA_AT, &mut data)?;
        Ok(data)
    }

    fn user_bytes(&self, lba: u64, count: u64) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(count as usize * USER_DATA_BYTES);
        for sector in lba..lba + count {
            out.extend(self.user_sector(sector)?);
        }
        Ok(out)
    }

    /// Stream the image in 8 MiB chunks to `visit(offset, chunk)`.
    fn stream(&self, mut visit: impl FnMut(u64, &[u8])) -> Result<()> {
        let mut offset = 0u64;
        for (path, _) in &self.parts {
            let mut file =
                File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
            let mut chunk = vec![0u8; 8 << 20];
            loop {
                let n = file.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                visit(offset, &chunk[..n]);
                offset += n as u64;
            }
        }
        Ok(())
    }

    fn sha256(&self) -> Result<String> {
        let mut hasher = Sha256::new();
        self.stream(|_, chunk| hasher.update(chunk))?;
        Ok(hex(&hasher.finalize()))
    }
}

/// All matches of `FORBIDDEN` in a stream, across chunk boundaries, with the
/// bytes around each. Offsets are into the whole stream.
struct Scanner {
    carry: Vec<u8>,
    carry_at: u64,
    hits: Vec<(u64, &'static str, String)>,
}

impl Scanner {
    fn new() -> Scanner {
        Scanner {
            carry: Vec::new(),
            carry_at: 0,
            hits: Vec::new(),
        }
    }

    fn feed(&mut self, at: u64, chunk: &[u8]) {
        let overlap = FORBIDDEN.iter().map(|t| t.len()).max().unwrap_or(0) + CONTEXT;
        let mut data = std::mem::take(&mut self.carry);
        let base = if data.is_empty() { at } else { self.carry_at };
        data.extend_from_slice(chunk);
        let lower = data.to_ascii_lowercase();
        // Matches that begin inside the carried tail were already reported by
        // the previous pass; start after it.
        let skip = (at.saturating_sub(base)) as usize;
        let skip = skip.saturating_sub(FORBIDDEN.iter().map(|t| t.len()).max().unwrap_or(0));
        for i in skip..lower.len() {
            if !matches!(lower[i], b'h' | b'c' | b'w') {
                continue;
            }
            for token in FORBIDDEN {
                let t = token.as_bytes();
                if lower[i..].starts_with(t) {
                    // A match that starts in the old tail is a repeat only if it
                    // ended inside it; one that straddles the join is new.
                    if base + (i as u64) + (t.len() as u64) <= at {
                        continue;
                    }
                    let from = i.saturating_sub(CONTEXT);
                    let to = (i + t.len() + CONTEXT).min(data.len());
                    let shown: String = data[from..to]
                        .iter()
                        .map(|&b| {
                            if (0x20..0x7f).contains(&b) {
                                b as char
                            } else {
                                '.'
                            }
                        })
                        .collect();
                    self.hits.push((base + i as u64, token, shown));
                }
            }
        }
        let keep = overlap.min(data.len());
        self.carry_at = base + (data.len() - keep) as u64;
        self.carry = data[data.len() - keep..].to_vec();
    }
}

fn scan_bytes(data: &[u8]) -> Vec<(u64, &'static str, String)> {
    let mut scanner = Scanner::new();
    scanner.feed(0, data);
    scanner.hits
}

/// Every path in the image's ISO 9660 directory tree.
fn iso_listing(image: &Image) -> Result<Vec<String>> {
    let pvd = image.user_sector(16)?;
    ensure!(
        pvd[0] == 1 && &pvd[1..6] == b"CD001",
        "{}: no ISO 9660 primary volume descriptor at LBA 16",
        image.label
    );
    let root = &pvd[156..190];
    let (extent, size) = (le32(root, 2), le32(root, 10));
    let mut found = Vec::new();
    walk_directory(image, extent as u64, size as usize, "", &mut found, 0)?;
    Ok(found)
}

fn le32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn walk_directory(
    image: &Image,
    extent: u64,
    size: usize,
    prefix: &str,
    found: &mut Vec<String>,
    depth: usize,
) -> Result<()> {
    ensure!(depth < 16, "{}: ISO directory nesting too deep", image.label);
    let sectors = size.div_ceil(USER_DATA_BYTES) as u64;
    let data = image.user_bytes(extent, sectors)?;
    let mut at = 0usize;
    while at < size {
        let length = data[at] as usize;
        if length == 0 {
            // Records do not cross a sector; skip to the next one.
            at = (at / USER_DATA_BYTES + 1) * USER_DATA_BYTES;
            continue;
        }
        ensure!(
            at + length <= data.len() && length >= 34,
            "{}: bad ISO directory record",
            image.label
        );
        let record = &data[at..at + length];
        let name_len = record[32] as usize;
        let raw = &record[33..33 + name_len];
        let is_dir = record[25] & 2 != 0;
        at += length;
        if raw == [0] || raw == [1] {
            continue; // . and ..
        }
        let mut name = String::from_utf8_lossy(raw).into_owned();
        if let Some(stripped) = name.strip_suffix(";1") {
            name = stripped.to_string();
        }
        let path = format!("{prefix}/{name}");
        found.push(path.clone());
        if is_dir {
            walk_directory(
                image,
                le32(record, 2) as u64,
                le32(record, 10) as usize,
                &path,
                found,
                depth + 1,
            )?;
        }
    }
    Ok(())
}

/// Disc images under the given files and directories: single `*.bin` files and
/// numbered `*.bin.NNN` sets grouped by prefix.
fn collect(paths: &[PathBuf]) -> Result<(Vec<PathBuf>, Vec<Image>)> {
    let mut files = Vec::new();
    for path in paths {
        walk(path, &mut files)?;
    }
    files.sort();
    let mut images: Vec<Image> = Vec::new();
    let mut part_sets: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for file in &files {
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned());
        let Some(name) = name else { continue };
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".bin") {
            images.push(Image {
                label: file.display().to_string(),
                parts: vec![(file.clone(), fs::metadata(file)?.len())],
            });
        } else if let Some((stem, tail)) = lower.rsplit_once('.') {
            if stem.ends_with(".bin") && !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())
            {
                let key = format!(
                    "{}/{}",
                    file.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                    stem
                );
                match part_sets.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, list)) => list.push(file.clone()),
                    None => part_sets.push((key, vec![file.clone()])),
                }
            }
        }
    }
    for (key, mut list) in part_sets {
        list.sort();
        let mut parts = Vec::new();
        for part in &list {
            parts.push((part.clone(), fs::metadata(part)?.len()));
        }
        images.push(Image {
            label: format!("{key}.NNN ({} parts)", list.len()),
            parts,
        });
    }
    Ok((files, images))
}

fn walk(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let meta = fs::metadata(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            walk(&entry?.path(), out)?;
        }
    } else {
        out.push(path.to_path_buf());
    }
    Ok(())
}

struct Report {
    lines: Vec<String>,
    failures: usize,
}

impl Report {
    fn ok(&mut self, text: String) {
        self.lines.push(format!("ok    {text}"));
    }
    fn note(&mut self, text: String) {
        self.lines.push(format!("note  {text}"));
    }
    fn fail(&mut self, text: String) {
        self.failures += 1;
        self.lines.push(format!("FAIL  {text}"));
    }
}

fn excused(shown: &str, allow: &[String]) -> bool {
    let lower = shown.to_ascii_lowercase();
    allow.iter().any(|a| lower.contains(&a.to_ascii_lowercase()))
}

fn judge_hits(
    report: &mut Report,
    what: &str,
    hits: Vec<(u64, &'static str, String)>,
    allow: &[String],
) {
    let mut clean = true;
    for (offset, token, shown) in hits {
        if excused(&shown, allow) {
            report.note(format!(
                "{what}: excused '{token}' at byte {offset}: ...{shown}..."
            ));
        } else {
            clean = false;
            report.fail(format!(
                "{what}: '{token}' at byte {offset}: ...{shown}..."
            ));
        }
    }
    if clean {
        report.ok(format!("{what}: none of the four programs named"));
    }
}

fn check_image(
    report: &mut Report,
    image: &Image,
    allow: &[String],
    standalone: Option<&str>,
) -> Result<()> {
    let label = &image.label;
    if let Some(expect) = standalone {
        // A single-game disc: one boot EXE and nothing else, and it is that game.
        let mut whole = Vec::new();
        image.stream(|_, chunk| whole.extend_from_slice(chunk))?;
        let lower = whole.to_ascii_lowercase();
        let wanted = expect.to_ascii_lowercase();
        if lower.windows(wanted.len()).any(|w| w == wanted.as_bytes()) {
            report.ok(format!("{label}: is the standalone game ('{expect}' found)"));
        } else {
            report.fail(format!("{label}: '{expect}' not found in the image"));
        }
        match iso_listing(image) {
            Ok(paths) if paths == ["/SYSTEM.CNF", "/PSX.EXE"] => {
                report.ok(format!("{label}: ISO holds SYSTEM.CNF and PSX.EXE only"))
            }
            Ok(paths) => report.fail(format!("{label}: ISO holds {}", paths.join(" "))),
            Err(error) => report.fail(format!("{label}: ISO listing: {error}")),
        }
    } else {
        // Table of contents: exactly the public programs, none hidden.
        let raw = image.user_bytes(TOC_LBA, TOC_SECTORS)?;
        match parse_public_toc(&raw) {
            Ok(toc) => {
                if toc.menu == PUBLIC_GAMES && toc.visible == PUBLIC_GAMES.len() - 1 && toc.claimed == toc.visible
                {
                    report.ok(format!(
                        "{label}: table of contents is the public menu ({})",
                        toc.menu.join(", ")
                    ));
                } else {
                    report.fail(format!(
                        "{label}: table of contents is not the public menu: claimed {}, visible {}, menu {}",
                        toc.claimed,
                        toc.visible,
                        toc.menu.join(", ")
                    ));
                }
            }
            Err(error) => report.fail(format!("{label}: table of contents: {error}")),
        }
    }
    // ISO directory listing.
    match iso_listing(image) {
        Ok(paths) => {
            let bad: Vec<&String> = paths
                .iter()
                .filter(|p| {
                    let lower = p.to_ascii_lowercase();
                    FORBIDDEN.iter().any(|t| lower.contains(t))
                })
                .collect();
            report.note(format!("{label}: ISO listing {}", paths.join(" ")));
            if bad.is_empty() {
                report.ok(format!("{label}: ISO directory names none of the four"));
            } else {
                for path in bad {
                    report.fail(format!("{label}: ISO path {path}"));
                }
            }
        }
        Err(error) => report.fail(format!("{label}: ISO listing: {error}")),
    }
    // Every byte.
    let mut scanner = Scanner::new();
    image.stream(|at, chunk| scanner.feed(at, chunk))?;
    judge_hits(
        report,
        &format!("{label} ({} bytes, all of it)", image.len()),
        scanner.hits,
        allow,
    );
    Ok(())
}

fn check_file(report: &mut Report, file: &Path, allow: &[String]) -> Result<()> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    // Disc images are checked as images.
    if lower.ends_with(".bin") || lower.rsplit_once('.').is_some_and(|(s, t)| {
        s.ends_with(".bin") && !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())
    }) {
        return Ok(());
    }
    if RECEIPT_MARKERS.iter().any(|m| lower.contains(m)) {
        report.fail(format!(
            "{}: a pressing receipt is not part of a public package",
            file.display()
        ));
        return Ok(());
    }
    if FORBIDDEN.iter().any(|t| lower.contains(t)) {
        report.fail(format!("{}: the file name names a private program", file.display()));
        return Ok(());
    }
    let mut data = fs::read(file)?;
    let mut what = file.display().to_string();
    if lower.ends_with(".gz") {
        let mut inflated = Vec::new();
        if flate2::read::GzDecoder::new(&data[..])
            .read_to_end(&mut inflated)
            .is_ok()
        {
            data = inflated;
            what = format!("{what} (decompressed)");
        }
    }
    judge_hits(report, &what, scan_bytes(&data), allow);
    Ok(())
}

pub fn run(args: &[String]) -> Result<i32> {
    let args = Args::parse(args, &["allow", "standalone"], &["same-disc"])?;
    ensure!(!args.positional.is_empty(), "name the staged files or folders to check");
    let allow: Vec<String> = args.all("allow").iter().map(|s| s.to_string()).collect();
    let paths: Vec<PathBuf> = args.positional.iter().map(PathBuf::from).collect();
    let (files, images) = collect(&paths)?;
    let mut report = Report {
        lines: Vec::new(),
        failures: 0,
    };
    report.note(format!(
        "{} file(s), {} disc image(s) under {}",
        files.len(),
        images.len(),
        args.positional.join(", ")
    ));
    for file in &files {
        check_file(&mut report, file, &allow)?;
    }
    let mut digests = Vec::new();
    for image in &images {
        check_image(&mut report, image, &allow, args.get("standalone"))?;
        digests.push((image.label.clone(), image.sha256()?));
    }
    if args.has("same-disc") {
        match digests.split_first() {
            Some((first, rest)) if !rest.is_empty() => {
                if rest.iter().all(|d| d.1 == first.1) {
                    report.ok(format!(
                        "all {} disc images are one pressing, sha256 {}",
                        digests.len(),
                        first.1
                    ));
                } else {
                    for (label, digest) in &digests {
                        report.fail(format!("disc images differ: {label} sha256 {digest}"));
                    }
                }
            }
            _ => report.fail("--same-disc needs at least two disc images".to_string()),
        }
    }
    for (label, digest) in &digests {
        report.note(format!("{label} sha256 {digest}"));
    }
    for line in &report.lines {
        println!("{line}");
    }
    if report.failures == 0 {
        println!("PUBLIC ONLY: clean ({} file(s), {} image(s))", files.len(), images.len());
        Ok(0)
    } else {
        println!("NOT PUBLIC ONLY: {} failure(s)", report.failures);
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_names_across_a_chunk_boundary() {
        let mut scanner = Scanner::new();
        scanner.feed(0, b"xxxx Hollow ");
        scanner.feed(12, b"Knight yyyy");
        assert_eq!(scanner.hits.len(), 1);
        assert_eq!(scanner.hits[0].1, "hollow knight");
        assert_eq!(scanner.hits[0].0, 5);
    }

    #[test]
    fn does_not_report_a_name_twice() {
        let mut scanner = Scanner::new();
        scanner.feed(0, b"WIPEOUT");
        scanner.feed(7, b" tail text of some length");
        assert_eq!(scanner.hits.len(), 1);
    }

    #[test]
    fn an_excused_hit_is_not_a_failure() {
        let hits = scan_bytes(b"NOW WITH 2MB OF RAM!ALSO TRY WIPEOUT!RUNS ON REAL HARDWARE!");
        assert_eq!(hits.len(), 1);
        let mut report = Report { lines: Vec::new(), failures: 0 };
        judge_hits(&mut report, "tagline", hits.clone(), &["also try wipeout".to_string()]);
        assert_eq!(report.failures, 0);
        judge_hits(&mut report, "tagline", hits, &[]);
        assert_eq!(report.failures, 1);
    }

    #[test]
    fn clean_bytes_pass() {
        assert!(scan_bytes(b"Celeste Quake VoXide NitroXide PSXcel Cortex Arcade").is_empty());
    }
}
