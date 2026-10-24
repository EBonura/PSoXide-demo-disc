//! The browser emulator's lazy delivery of a disc whose audio is XA, so the
//! whole disc is one data track.
//!
//! The split delivery (`web`) gzips the data track and makes the page download
//! all of it before the first frame. That is the right trade while the music is
//! CD-DA, but an XA disc keeps its music inside the data track, and a data
//! track of hundreds of megabytes cannot be a boot gate. Here the raw image is
//! served as is and the page reads it with HTTP Range requests, a few sectors
//! at a time, as the guest asks for them.
//!
//! What the page needs to do that without reading the image first is computed
//! once, at press time, in a single pass over the image:
//!
//! - the game-image hash (FNV-1a 64 over every byte), which input tapes and
//!   save states key on;
//! - bytes 12..20 of every sector (the header the drive reports after a seek),
//!   gzipped as a sidecar, so a seek answers before its sector has arrived.
//!
//! A hosting file size cap splits the image into numbered parts of a fixed size,
//! a whole number of the emulator's 32-sector read chunks, so a chunk read stays
//! inside one file. The tool then reads every file it wrote back and demands the
//! hash it computed from the source.
//!
//! Manifest line, read by the emulator's `web_stream::parse_manifest` (the
//! sidecar and the optional part size are described there too):
//!
//! ```text
//! image FILE BYTES GAME_IMAGE_HASH HDR_FILE HDR_RAW_BYTES HDR_FNV32 BOOT_SECTORS [PART_BYTES]
//! ```
//!
//! With `PART_BYTES` the image is the files `FILE.001`, `FILE.002`, ... of
//! that many bytes each, the last one shorter.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;

use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};

use crate::disc::SECTOR_BYTES;
use crate::util::{dumps, fnv1a32, Error, Result};

const SECTOR: usize = SECTOR_BYTES as usize;
/// Sectors per chunk the emulator reads the image in (`web_disc::CHUNK_SECTORS`).
const CHUNK_BYTES: u64 = 32 * SECTOR_BYTES;
/// Sectors fetched up front. The demo disc's launcher reads about 0.91 MB
/// (390 sectors) before its first frame, measured on the Aug-26 disc, so 410
/// covers it with a little to spare; the emulator rounds up to a whole chunk.
pub const DEFAULT_BOOT_SECTORS: u64 = 410;
/// Sectors per read while streaming through the image.
const PASS_SECTORS: usize = 892;

/// FNV-1a 64 as `emulator_core::GameImageHasher` computes it, fed in pieces.
#[derive(Clone, Copy)]
struct GameImageHasher(u64);

impl GameImageHasher {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn update(&mut self, bytes: &[u8]) {
        let mut hash = self.0;
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = hash;
    }
}

/// Bytes per file for an image of `len` bytes under a cap of `limit` bytes per
/// file (exclusive, as the host refuses a file at the cap): 0 when the image
/// fits in one file, else the largest whole number of read chunks that does.
pub fn part_bytes(len: u64, limit: u64) -> Result<u64> {
    if len < limit {
        return Ok(0);
    }
    let part = (limit - 1) / CHUNK_BYTES * CHUNK_BYTES;
    ensure!(part > 0, "file cap {limit} is smaller than one read chunk");
    Ok(part)
}

/// The image's file names: `name` itself, or `name.001`... when split.
fn file_names(name: &str, len: u64, part: u64) -> Vec<String> {
    if part == 0 {
        return vec![name.to_string()];
    }
    (1..=len.div_ceil(part))
        .map(|n| format!("{name}.{n:03}"))
        .collect()
}

/// Writes a byte stream into consecutive files of `part` bytes (one file when
/// `part` is 0).
struct PartWriter<'a> {
    dir: &'a Path,
    names: Vec<String>,
    part: u64,
    index: usize,
    room: u64,
    file: Option<File>,
}

impl<'a> PartWriter<'a> {
    fn new(dir: &'a Path, names: Vec<String>, part: u64) -> PartWriter<'a> {
        PartWriter {
            dir,
            names,
            part,
            index: 0,
            room: 0,
            file: None,
        }
    }

    fn write(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            if self.room == 0 {
                let name = self
                    .names
                    .get(self.index)
                    .ok_or_else(|| Error("more image bytes than part files".into()))?;
                self.file = Some(File::create(self.dir.join(name))?);
                self.index += 1;
                self.room = if self.part == 0 { u64::MAX } else { self.part };
            }
            let take = (bytes.len() as u64).min(self.room) as usize;
            self.file
                .as_mut()
                .expect("a file is open")
                .write_all(&bytes[..take])?;
            self.room -= take as u64;
            bytes = &bytes[take..];
        }
        Ok(())
    }

    fn finish(self) -> Result<()> {
        ensure!(
            self.index == self.names.len(),
            "fewer image bytes than part files"
        );
        Ok(())
    }
}

/// What a lazy delivery wrote, for the summary and the manifests.
struct Delivered {
    name: String,
    len: u64,
    hash: u64,
    part: u64,
    files: Vec<String>,
    hdr_file: String,
    hdr_gz_bytes: usize,
    hdr_raw_bytes: usize,
    hdr_fnv: u32,
    boot_sectors: u64,
}

impl Delivered {
    fn manifest_line(&self) -> String {
        let mut line = format!(
            "image {} {} {} {} {} {} {}",
            self.name,
            self.len,
            self.hash,
            self.hdr_file,
            self.hdr_raw_bytes,
            self.hdr_fnv,
            self.boot_sectors
        );
        if self.part != 0 {
            line += &format!(" {}", self.part);
        }
        line + "\n"
    }

    fn manifest_json(&self) -> Value {
        json!({
            "version": 1,
            "image": {
                "file": self.name,
                "files": self.files,
                "bytes": self.len,
                "part_bytes": self.part,
                "game_image_hash": self.hash,
                "boot_sectors": self.boot_sectors,
            },
            "headers": {
                "file": self.hdr_file,
                "gz_bytes": self.hdr_gz_bytes,
                "raw_bytes": self.hdr_raw_bytes,
                "fnv": self.hdr_fnv,
            },
        })
    }
}

/// Stage the lazy delivery of `image` in `outdir`: the image (split if it must
/// be), the header sidecar `<name stem>.hdr.gz`, `web-manifest.txt` and its JSON
/// twin. `limit` is the host's per-file cap in bytes; `name` is the image's
/// served file name. Returns the one-line summary.
pub fn deliver(
    image: &Path,
    outdir: &Path,
    name: &str,
    limit: u64,
    boot_sectors: u64,
) -> Result<String> {
    let len = fs::metadata(image)
        .map_err(|e| Error(format!("{}: {e}", image.display())))?
        .len();
    ensure!(len > 0, "{}: empty image", image.display());
    ensure!(
        len.is_multiple_of(SECTOR_BYTES),
        "{}: not whole {SECTOR}-byte sectors",
        image.display()
    );
    ensure!(boot_sectors > 0, "boot sectors must be at least 1");
    fs::create_dir_all(outdir)?;
    let part = part_bytes(len, limit)?;
    let files = file_names(name, len, part);
    // Never write over the file being read: a press that copies the image into
    // the delivery directory first would otherwise truncate its own source.
    let source = fs::metadata(image)?;
    for file in files.iter().chain(std::iter::once(&format!(
        "{}.hdr.gz",
        name.strip_suffix(".bin").unwrap_or(name)
    ))) {
        if let Ok(existing) = fs::metadata(outdir.join(file)) {
            ensure!(
                !(existing.dev() == source.dev() && existing.ino() == source.ino()),
                "{file} in the output directory is the source image; read it from elsewhere"
            );
        }
    }

    // One pass: hash, collect the sector headers, write the image files.
    let mut reader = BufReader::with_capacity(1 << 20, File::open(image)?);
    let mut hasher = GameImageHasher::new();
    let mut headers = Vec::with_capacity((len / SECTOR_BYTES) as usize * 8);
    let mut writer = PartWriter::new(outdir, files.clone(), part);
    let mut buf = vec![0u8; SECTOR * PASS_SECTORS];
    loop {
        let mut n = 0;
        while n < buf.len() {
            let got = reader.read(&mut buf[n..])?;
            if got == 0 {
                break;
            }
            n += got;
        }
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        for sector in buf[..n].chunks_exact(SECTOR) {
            headers.extend_from_slice(&sector[12..20]);
        }
        writer.write(&buf[..n])?;
    }
    writer.finish()?;

    let stem = name.strip_suffix(".bin").unwrap_or(name);
    let hdr_file = format!("{stem}.hdr.gz");
    let mut gz = GzEncoder::new(Vec::new(), Compression::new(9));
    gz.write_all(&headers)?;
    let gz = gz.finish()?;
    fs::write(outdir.join(&hdr_file), &gz)?;

    let delivered = Delivered {
        name: name.to_string(),
        len,
        hash: hasher.0,
        part,
        files,
        hdr_file,
        hdr_gz_bytes: gz.len(),
        hdr_raw_bytes: headers.len(),
        hdr_fnv: fnv1a32(&headers),
        boot_sectors,
    };
    verify(outdir, &delivered, &headers)?;

    fs::write(outdir.join("web-manifest.txt"), delivered.manifest_line())?;
    fs::write(
        outdir.join("web-manifest.json"),
        dumps(&delivered.manifest_json(), 1, false),
    )?;
    Ok(format!(
        "web lazy delivery: {} MiB as {} file(s), header sidecar {} KiB gz, boot {} sectors, round trip OK",
        len / 1024 / 1024,
        delivered.files.len(),
        delivered.hdr_gz_bytes / 1024,
        boot_sectors
    ))
}

/// Read back what was written: every image file at its size, their
/// concatenation at the source's hash, and the sidecar back to the headers.
fn verify(outdir: &Path, d: &Delivered, headers: &[u8]) -> Result<()> {
    let mut hasher = GameImageHasher::new();
    let mut total = 0u64;
    for (i, file) in d.files.iter().enumerate() {
        let path = outdir.join(file);
        let size = fs::metadata(&path)?.len();
        let last = i + 1 == d.files.len();
        ensure!(
            d.part == 0 || last || size == d.part,
            "{file}: {size} bytes, expected {}",
            d.part
        );
        ensure!(
            d.part == 0 || size <= d.part,
            "{file}: {size} bytes is over the part size {}",
            d.part
        );
        let mut reader = BufReader::with_capacity(1 << 20, File::open(&path)?);
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        total += size;
    }
    ensure!(
        total == d.len,
        "written parts total {total}, image is {}",
        d.len
    );
    ensure!(
        hasher.0 == d.hash,
        "written image differs from the pressed disc"
    );
    let mut back = Vec::new();
    flate2::read::GzDecoder::new(fs::read(outdir.join(&d.hdr_file))?.as_slice())
        .read_to_end(&mut back)?;
    ensure!(back == headers, "header sidecar does not round trip");
    Ok(())
}

/// What the compact delivery wrote, from the packer's report.
struct Compact {
    stem: String,
    index_file: String,
    index_bytes: u64,
    index_fnv: u32,
    payload_file: String,
    payload_bytes: u64,
    part: u64,
    image_bytes: u64,
    hash: u64,
    chunks: u64,
    boot_chunks: u64,
    boot_bytes: u64,
    kinds: String,
}

impl Compact {
    fn from_report(stem: &str, report: &HashMap<String, String>) -> Result<Compact> {
        let text = |key: &str| -> Result<String> {
            report
                .get(key)
                .cloned()
                .ok_or_else(|| Error(format!("webdisc did not report {key}")))
        };
        let number = |key: &str| -> Result<u64> {
            text(key)?
                .parse()
                .map_err(|_| Error(format!("webdisc reported a non-number for {key}")))
        };
        Ok(Compact {
            stem: stem.to_string(),
            index_file: text("index_file")?,
            index_bytes: number("index_bytes")?,
            index_fnv: number("index_fnv32")? as u32,
            payload_file: text("payload_file")?,
            payload_bytes: number("payload_bytes")?,
            part: number("part_bytes")?,
            image_bytes: number("image_bytes")?,
            hash: number("image_hash")?,
            chunks: number("chunks")?,
            boot_chunks: number("boot_chunks")?,
            boot_bytes: number("boot_bytes")?,
            kinds: text("kinds")?,
        })
    }

    /// The payload's file names: the file itself, or `.001`... when split.
    fn payload_files(&self) -> Vec<String> {
        if self.part == 0 {
            return vec![self.payload_file.clone()];
        }
        (1..=self.payload_bytes.div_ceil(self.part))
            .map(|n| format!("{}.{n:03}", self.payload_file))
            .collect()
    }

    /// `compact INDEX_FILE INDEX_BYTES INDEX_FNV32 PAYLOAD_FILE BOOT_BYTES PART_BYTES`,
    /// read by the emulator's `web_stream::parse_manifest` and by the page's
    /// early prefetch in `index.html`.
    fn manifest_line(&self) -> String {
        format!(
            "compact {} {} {} {} {} {}\n",
            self.index_file,
            self.index_bytes,
            self.index_fnv,
            self.payload_file,
            self.boot_bytes,
            self.part
        )
    }

    fn manifest_json(&self) -> Value {
        json!({
            "version": 2,
            "compact": {
                "name": self.stem,
                "index": {
                    "file": self.index_file,
                    "bytes": self.index_bytes,
                    "fnv": self.index_fnv,
                },
                "payload": {
                    "file": self.payload_file,
                    "files": self.payload_files(),
                    "bytes": self.payload_bytes,
                    "part_bytes": self.part,
                },
                "image_bytes": self.image_bytes,
                "game_image_hash": self.hash,
                "chunks": self.chunks,
                "boot_chunks": self.boot_chunks,
                "boot_bytes": self.boot_bytes,
                "sector_kinds": self.kinds,
            },
        })
    }
}

/// Run `webdisc pack` (the emulator checkout's `psx-webdisc` tool) and read its
/// `key value` report. The tool reads every file it wrote back and compares
/// each sector with the image, and exits nonzero on any difference.
fn run_pack(
    webdisc: &Path,
    image: &Path,
    outdir: &Path,
    stem: &str,
    extra: &[String],
) -> Result<HashMap<String, String>> {
    let output = Command::new(webdisc)
        .arg("pack")
        .arg(image)
        .arg(outdir)
        .arg(stem)
        .args(extra)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| Error(format!("cannot run {}: {e}", webdisc.display())))?;
    ensure!(
        output.status.success(),
        "webdisc pack failed ({})",
        output.status
    );
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect())
}

/// Stage the compact delivery of `image` in `outdir`: `<stem>.pxi` (index),
/// `<stem>.pxw` (payload, in numbered parts if it would reach `limit`),
/// `web-manifest.txt` and its JSON twin. `webdisc` is the packer binary built
/// from the emulator checkout the page is built from, so the page and the
/// format always agree. Returns the one-line summary.
pub fn deliver_compact(
    webdisc: &Path,
    image: &Path,
    outdir: &Path,
    stem: &str,
    limit: u64,
    boot_sectors: u64,
) -> Result<String> {
    let len = fs::metadata(image)
        .map_err(|e| Error(format!("{}: {e}", image.display())))?
        .len();
    ensure!(len > 0, "{}: empty image", image.display());
    ensure!(
        len.is_multiple_of(SECTOR_BYTES),
        "{}: not whole {SECTOR}-byte sectors",
        image.display()
    );
    ensure!(boot_sectors > 0, "boot sectors must be at least 1");
    fs::create_dir_all(outdir)?;
    // Never write over the file being read.
    let source = fs::metadata(image)?;
    for file in [format!("{stem}.pxi"), format!("{stem}.pxw")] {
        if let Ok(existing) = fs::metadata(outdir.join(&file)) {
            ensure!(
                !(existing.dev() == source.dev() && existing.ino() == source.ino()),
                "{file} in the output directory is the source image; read it from elsewhere"
            );
        }
    }
    let boot = boot_sectors.to_string();
    let mut args = vec!["--boot-sectors".to_string(), boot];
    let mut report = run_pack(webdisc, image, outdir, stem, &args)?;
    let mut delivered = Compact::from_report(stem, &report)?;
    if delivered.payload_bytes >= limit {
        // Too big for one file on this host: pack again in numbered parts.
        fs::remove_file(outdir.join(&delivered.payload_file))?;
        args.extend(["--part-bytes".to_string(), (limit - 1).to_string()]);
        report = run_pack(webdisc, image, outdir, stem, &args)?;
        delivered = Compact::from_report(stem, &report)?;
    }
    ensure!(
        delivered.image_bytes == len,
        "webdisc packed {} bytes of a {len} byte image",
        delivered.image_bytes
    );
    let index = fs::read(outdir.join(&delivered.index_file))?;
    ensure!(
        index.len() as u64 == delivered.index_bytes && fnv1a32(&index) == delivered.index_fnv,
        "the index on disk is not the one webdisc reported"
    );
    let mut written = 0;
    for file in delivered.payload_files() {
        let size = fs::metadata(outdir.join(&file))
            .map_err(|e| Error(format!("{file}: {e}")))?
            .len();
        ensure!(size < limit, "{file}: {size} bytes is over the host cap {limit}");
        written += size;
    }
    ensure!(
        written == delivered.payload_bytes,
        "payload files total {written} bytes, expected {}",
        delivered.payload_bytes
    );
    fs::write(outdir.join("web-manifest.txt"), delivered.manifest_line())?;
    fs::write(
        outdir.join("web-manifest.json"),
        dumps(&delivered.manifest_json(), 1, false),
    )?;
    Ok(format!(
        "web compact delivery: {} MiB image as {} MiB payload in {} file(s), index {} KiB, boot {} chunks ({} KiB), {}, round trip OK",
        len / 1024 / 1024,
        delivered.payload_bytes / 1024 / 1024,
        delivered.payload_files().len(),
        delivered.index_bytes / 1024,
        delivered.boot_chunks,
        delivered.boot_bytes / 1024,
        delivered.kinds
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(sectors: usize) -> Vec<u8> {
        (0..sectors * SECTOR)
            .map(|i| ((i * 7) ^ (i >> 5)) as u8)
            .collect()
    }

    fn write_disc(dir: &Path, sectors: usize) -> std::path::PathBuf {
        let path = dir.join("source.bin");
        fs::write(&path, disc(sectors)).unwrap();
        path
    }

    #[test]
    fn hasher_matches_the_emulators_fnv_1a_64() {
        // Published FNV-1a 64 vectors.
        let mut h = GameImageHasher::new();
        assert_eq!(h.0, 0xcbf2_9ce4_8422_2325);
        h.update(b"a");
        assert_eq!(h.0, 0xaf63_dc4c_8601_ec8c);
        let mut h = GameImageHasher::new();
        h.update(b"foobar");
        assert_eq!(h.0, 0x8594_4171_f739_67e8);
        // Feeding in pieces is feeding at once.
        let mut split = GameImageHasher::new();
        split.update(b"foo");
        split.update(b"bar");
        assert_eq!(split.0, h.0);
    }

    #[test]
    fn part_size_is_whole_chunks_under_the_cap() {
        assert_eq!(part_bytes(10, 200_000_000).unwrap(), 0);
        assert_eq!(part_bytes(199_999_999, 200_000_000).unwrap(), 0);
        let part = part_bytes(236_100_816, 200_000_000).unwrap();
        assert_eq!(part % CHUNK_BYTES, 0);
        assert!(part < 200_000_000 && part + CHUNK_BYTES >= 200_000_000);
        assert_eq!(part, 199_976_448);
        // A cap an image exactly meets splits it, as the host refuses that.
        assert_eq!(
            part_bytes(CHUNK_BYTES * 3, CHUNK_BYTES * 3).unwrap(),
            CHUNK_BYTES * 2
        );
        assert!(part_bytes(10, 10).is_err());
    }

    #[test]
    fn names_are_numbered_only_when_split() {
        assert_eq!(file_names("d.bin", 100, 0), vec!["d.bin"]);
        assert_eq!(
            file_names("d.bin", 250, 100),
            vec!["d.bin.001", "d.bin.002", "d.bin.003"]
        );
        assert_eq!(
            file_names("d.bin", 200, 100),
            vec!["d.bin.001", "d.bin.002"]
        );
    }

    #[test]
    fn one_file_delivery_writes_the_image_sidecar_and_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_disc(dir.path(), 70);
        let out = dir.path().join("web");
        let summary = deliver(&src, &out, "demo-disc.bin", 200_000_000, 410).unwrap();
        assert!(summary.ends_with("round trip OK"), "{summary}");
        assert_eq!(fs::read(out.join("demo-disc.bin")).unwrap(), disc(70));
        let text = fs::read_to_string(out.join("web-manifest.txt")).unwrap();
        let mut hasher = GameImageHasher::new();
        hasher.update(&disc(70));
        let headers: Vec<u8> = disc(70)
            .chunks_exact(SECTOR)
            .flat_map(|s| s[12..20].to_vec())
            .collect();
        assert_eq!(
            text,
            format!(
                "image demo-disc.bin {} {} demo-disc.hdr.gz {} {} 410\n",
                70 * SECTOR,
                hasher.0,
                70 * 8,
                fnv1a32(&headers)
            )
        );
        let json: Value =
            serde_json::from_str(&fs::read_to_string(out.join("web-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(json["image"]["files"][0], "demo-disc.bin");
        assert_eq!(json["image"]["part_bytes"], 0);
    }

    #[test]
    fn a_split_delivery_numbers_its_parts_and_names_the_part_size() {
        let dir = tempfile::tempdir().unwrap();
        // Three chunks and a bit, under a cap that makes a part two chunks.
        let sectors = 32 * 3 + 5;
        let src = write_disc(dir.path(), sectors);
        let out = dir.path().join("web");
        let limit = CHUNK_BYTES * 2 + 1;
        deliver(&src, &out, "demo-disc.bin", limit, 410).unwrap();
        let part = (CHUNK_BYTES * 2) as usize;
        let whole = disc(sectors);
        assert_eq!(
            fs::read(out.join("demo-disc.bin.001")).unwrap(),
            whole[..part]
        );
        assert_eq!(
            fs::read(out.join("demo-disc.bin.002")).unwrap(),
            whole[part..]
        );
        assert!(!out.join("demo-disc.bin").exists());
        let text = fs::read_to_string(out.join("web-manifest.txt")).unwrap();
        assert!(text.trim_end().ends_with(&format!(" 410 {part}")), "{text}");
        for file in fs::read_dir(&out).unwrap() {
            assert!(file.unwrap().metadata().unwrap().len() < limit);
        }
    }

    #[test]
    fn an_image_that_fills_its_parts_exactly_has_no_empty_last_part() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_disc(dir.path(), 64);
        let out = dir.path().join("web");
        deliver(&src, &out, "d.bin", CHUNK_BYTES + 1, 410).unwrap();
        assert!(out.join("d.bin.001").is_file() && out.join("d.bin.002").is_file());
        assert!(!out.join("d.bin.003").exists());
    }

    #[test]
    fn never_overwrites_its_own_source() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("web");
        fs::create_dir(&out).unwrap();
        let src = out.join("demo-disc.bin");
        fs::write(&src, disc(40)).unwrap();
        let error = deliver(&src, &out, "demo-disc.bin", 200_000_000, 410).unwrap_err();
        assert!(error.0.contains("is the source image"), "{error}");
        // A hard link to the source is the same file under another name.
        let link = out.join("demo-disc.bin");
        fs::remove_file(&link).unwrap();
        let real = write_disc(dir.path(), 40);
        fs::hard_link(&real, &link).unwrap();
        assert!(deliver(&real, &out, "demo-disc.bin", 200_000_000, 410).is_err());
        assert_eq!(fs::read(&real).unwrap(), disc(40));
    }

    #[test]
    fn refuses_images_that_are_not_whole_sectors() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("bad.bin");
        fs::write(&src, vec![0u8; SECTOR + 1]).unwrap();
        let error = deliver(&src, &dir.path().join("o"), "d.bin", 200_000_000, 410).unwrap_err();
        assert!(error.0.contains("whole"), "{error}");
        fs::write(&src, b"").unwrap();
        assert!(deliver(&src, &dir.path().join("o"), "d.bin", 200_000_000, 410).is_err());
    }

    // ----- compact delivery ---------------------------------------------------

    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for `webdisc pack`: writes a payload of `payload` bytes (one
    /// file, or numbered parts when told a part size) and reports like the real
    /// one. The real tool is tested in the emulator repository.
    fn fake_webdisc(dir: &Path, payload: u64, image_bytes: u64) -> std::path::PathBuf {
        let path = dir.join("webdisc");
        let script = format!(
            r#"#!/bin/sh
image="$2"; out="$3"; name="$4"; part=0
while [ $# -gt 0 ]; do [ "$1" = --part-bytes ] && part="$2"; shift; done
printf 'INDEX' > "$out/$name.pxi"
if [ "$part" = 0 ]; then
  head -c {payload} /dev/zero > "$out/$name.pxw"
else
  left={payload}; n=1
  while [ $left -gt 0 ]; do
    take=$left; [ $take -gt $part ] && take=$part
    head -c $take /dev/zero > "$out/$name.pxw.00$n"
    left=$((left - take)); n=$((n + 1))
  done
fi
echo "index_file $name.pxi"
echo "index_bytes 5"
echo "index_fnv32 {fnv}"
echo "payload_file $name.pxw"
echo "payload_bytes {payload}"
echo "part_bytes $part"
echo "image_bytes {image_bytes}"
echo "image_hash 99"
echo "chunks 7"
echo "boot_chunks 2"
echo "boot_bytes 1234"
echo "kinds form1=1 form2=2 form2_no_edc=0 verbatim=0"
"#,
            fnv = fnv1a32(b"INDEX")
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_compact_delivery_that_fits_is_one_file_and_one_manifest_line() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_disc(dir.path(), 70);
        let tool = fake_webdisc(dir.path(), 5000, (70 * SECTOR) as u64);
        let out = dir.path().join("web");
        let summary = deliver_compact(&tool, &src, &out, "demo-disc", 200_000_000, 410).unwrap();
        assert!(summary.ends_with("round trip OK"), "{summary}");
        let text = fs::read_to_string(out.join("web-manifest.txt")).unwrap();
        assert_eq!(
            text,
            format!("compact demo-disc.pxi 5 {} demo-disc.pxw 1234 0\n", fnv1a32(b"INDEX"))
        );
        let json: Value =
            serde_json::from_str(&fs::read_to_string(out.join("web-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(json["compact"]["payload"]["files"][0], "demo-disc.pxw");
        assert_eq!(json["compact"]["game_image_hash"], 99);
    }

    #[test]
    fn a_compact_payload_over_the_cap_is_repacked_in_parts() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_disc(dir.path(), 70);
        let tool = fake_webdisc(dir.path(), 5000, (70 * SECTOR) as u64);
        let out = dir.path().join("web");
        deliver_compact(&tool, &src, &out, "demo-disc", 3000, 410).unwrap();
        assert!(!out.join("demo-disc.pxw").exists());
        assert_eq!(fs::metadata(out.join("demo-disc.pxw.001")).unwrap().len(), 2999);
        assert_eq!(fs::metadata(out.join("demo-disc.pxw.002")).unwrap().len(), 2001);
        let text = fs::read_to_string(out.join("web-manifest.txt")).unwrap();
        assert!(text.trim_end().ends_with(" demo-disc.pxw 1234 2999"), "{text}");
    }

    #[test]
    fn a_compact_delivery_refuses_a_packer_that_packed_another_image() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_disc(dir.path(), 70);
        let tool = fake_webdisc(dir.path(), 5000, 1234 * SECTOR as u64);
        let error = deliver_compact(&tool, &src, &dir.path().join("web"), "d", 200_000_000, 410)
            .unwrap_err();
        assert!(error.0.contains("packed"), "{error}");
        assert!(deliver_compact(
            &dir.path().join("no-such-tool"),
            &src,
            &dir.path().join("web2"),
            "d",
            200_000_000,
            410
        )
        .is_err());
    }
}
