//! Split a pressed disc into the browser emulator's delivery set.
//!
//! The web build boots on the data track and streams the CD-DA tracks behind
//! it, so the disc ships as: the data portion gzipped, each audio track's file
//! extent FLAC-encoded (lossless; the browser decodes back to the exact
//! sectors), and a manifest naming the pieces with sizes and checksums.
//!
//! Every byte of the .bin lands in exactly one piece, and the tool proves it by
//! reassembling the pieces and comparing against the original before it will
//! write a manifest. A delivery that cannot round-trip is a bug here, not a
//! support ticket later.
//!
//! Usage:
//!   disc-tools web-delivery <disc.cue> <disc.bin> <outdir> [TITLE ...]
//!
//! Titles are per CD-DA track in disc order; missing ones fall back to
//! "TRACK NN". Requires the `flac` CLI.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use regex::Regex;
use serde_json::{json, Value};

use crate::disc::{msf_to_sector, SECTOR_BYTES};
use crate::util::{dumps, fnv1a32, Error, Result};

const SECTOR: usize = SECTOR_BYTES as usize;

/// `str.splitlines()`: Python also breaks on lone CR, VT, FF and a few
/// Unicode separators, which a cue written elsewhere could contain.
fn split_lines(text: &str) -> Vec<&str> {
    let is_break = |c: char| {
        matches!(
            c,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        )
    };
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if !is_break(c) {
            continue;
        }
        lines.push(&text[start..at]);
        start = at + c.len_utf8();
        if c == '\r' && matches!(chars.peek(), Some(&(_, '\n'))) {
            chars.next();
            start += 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// One cue track: its number, its kind (`MODE2/2352`, `AUDIO`) and the sector
/// inside the single BIN where its file extent begins.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CueTrack {
    number: u32,
    kind: String,
    start: u64,
}

/// Track list with the sector where each track's file extent begins (INDEX 00
/// when present, else INDEX 01), mirroring the emulator's cue loader.
fn parse_cue(text: &str) -> Result<Vec<CueTrack>> {
    let track = Regex::new(r"^TRACK (\d+) (\S+)")?;
    let index0 = Regex::new(r"^INDEX 00 (\S+)")?;
    let index1 = Regex::new(r"^INDEX 01 (\S+)")?;
    let mut tracks = Vec::new();
    let mut current: Option<(u32, String)> = None;
    let (mut at0, mut at1): (Option<u64>, Option<u64>) = (None, None);
    let mut push =
        |current: &Option<(u32, String)>, at0: Option<u64>, at1: Option<u64>| -> Result<()> {
            if let Some((number, kind)) = current {
                let Some(start) = at0.or(at1) else {
                    bail!("track {number} has no INDEX line");
                };
                tracks.push(CueTrack {
                    number: *number,
                    kind: kind.clone(),
                    start,
                });
            }
            Ok(())
        };
    for line in split_lines(text) {
        let line = line.trim();
        if let Some(m) = track.captures(line) {
            push(&current, at0, at1)?;
            current = Some((m[1].parse::<u32>()?, m[2].to_string()));
            (at0, at1) = (None, None);
        } else if let Some(m) = index0.captures(line) {
            at0 = Some(msf_to_sector(&m[1])?);
        } else if let Some(m) = index1.captures(line) {
            at1 = Some(msf_to_sector(&m[1])?);
        }
    }
    push(&current, at0, at1)?;
    Ok(tracks)
}

/// `data[from..to]` with Python's forgiving slice bounds.
fn slice(data: &[u8], from: usize, to: usize) -> &[u8] {
    let to = to.min(data.len());
    let from = from.min(to);
    &data[from..to]
}

/// A scratch file for the `flac` CLI, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        // Unique per call: the tests run several deliveries in one process.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("web-delivery-{}-{n}-{label}.cdda", std::process::id());
        Scratch(std::env::temp_dir().join(name))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn run_flac(args: &[&std::ffi::OsStr]) -> Result<()> {
    let status = Command::new("flac")
        .args(args)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| Error(format!("cannot run flac: {e}")))?;
    ensure!(status.success(), "flac failed ({status})");
    Ok(())
}

fn flac_encode(raw: &[u8], out: &Path) -> Result<()> {
    let scratch = Scratch::new("encode");
    fs::write(&scratch.0, raw)?;
    let flags = [
        "--totally-silent",
        "--force-raw-format",
        "--endian=little",
        "--sign=signed",
        "--channels=2",
        "--bps=16",
        "--sample-rate=44100",
        "-8",
        "-f",
        "-o",
    ];
    let mut args: Vec<&std::ffi::OsStr> = flags.iter().map(|f| std::ffi::OsStr::new(*f)).collect();
    args.push(out.as_os_str());
    args.push(scratch.0.as_os_str());
    run_flac(&args)
}

fn flac_decode(path: &Path) -> Result<Vec<u8>> {
    let scratch = Scratch::new("decode");
    let flags = [
        "--totally-silent",
        "-d",
        "--force-raw-format",
        "--endian=little",
        "--sign=signed",
        "-f",
        "-o",
    ];
    let mut args: Vec<&std::ffi::OsStr> = flags.iter().map(|f| std::ffi::OsStr::new(*f)).collect();
    args.push(scratch.0.as_os_str());
    args.push(path.as_os_str());
    run_flac(&args)?;
    Ok(fs::read(&scratch.0)?)
}

fn gzip_best(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(9));
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn gunzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(data).read_to_end(&mut out)?;
    Ok(out)
}

/// The manifest text twin: "data FILE GZ_BYTES RAW_BYTES FNV" then one
/// "track NUMBER FILE FLAC_BYTES RAW_BYTES FNV TITLE..." per track, for the
/// wasm side to parse without a JSON dependency.
fn manifest_lines(manifest: &Value) -> String {
    let field = |value: &Value, key: &str| match &value[key] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let data = &manifest["data"];
    let mut lines = vec![format!(
        "data {} {} {} {}",
        field(data, "file"),
        field(data, "gz_bytes"),
        field(data, "raw_bytes"),
        field(data, "fnv")
    )];
    for t in manifest["tracks"].as_array().into_iter().flatten() {
        lines.push(format!(
            "track {} {} {} {} {} {}",
            field(t, "number"),
            field(t, "file"),
            field(t, "flac_bytes"),
            field(t, "raw_bytes"),
            field(t, "fnv"),
            field(t, "title")
        ));
    }
    lines.join("\n") + "\n"
}

/// Floor division by a MiB, as `n // 1024 // 1024` is for a negative n too.
fn mebibytes(bytes: i64) -> i64 {
    bytes.div_euclid(1024).div_euclid(1024)
}

/// Build the delivery set in `outdir` from the cue text and the whole BIN.
/// Returns the one-line summary.
fn deliver(cue_text: &str, disc: &[u8], outdir: &Path, titles: &[String]) -> Result<String> {
    fs::create_dir_all(outdir)?;
    ensure!(
        disc.len().is_multiple_of(SECTOR),
        "bin is not whole raw sectors"
    );
    let tracks = parse_cue(cue_text)?;
    ensure!(
        tracks.first().is_some_and(|t| t.kind.starts_with("MODE2")),
        "track 1 must be the data track"
    );

    // File extents: each track runs to the next track's start, in sectors.
    let total_sectors = disc.len() / SECTOR;
    let mut starts: Vec<usize> = tracks.iter().map(|t| t.start as usize).collect();
    starts.push(total_sectors);

    let data_raw = slice(disc, 0, starts[1] * SECTOR);
    let data_gz = gzip_best(data_raw)?;
    fs::write(outdir.join("demo-data.bin.gz"), &data_gz)?;

    let mut manifest_tracks = Vec::new();
    let mut flac_total: i64 = 0;
    let audio = &tracks[1..];
    for (i, track) in audio.iter().enumerate() {
        ensure!(
            track.kind == "AUDIO",
            "track {} is {}, expected AUDIO",
            track.number,
            track.kind
        );
        let raw = slice(disc, starts[1 + i] * SECTOR, starts[2 + i] * SECTOR);
        let name = format!("track-{:02}.flac", track.number);
        flac_encode(raw, &outdir.join(&name))?;
        let title = titles
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("TRACK {:02}", track.number));
        let flac_bytes = fs::metadata(outdir.join(&name))?.len();
        flac_total += flac_bytes as i64;
        manifest_tracks.push(json!({
            "number": track.number,
            "title": title,
            "file": name,
            "flac_bytes": flac_bytes,
            "raw_bytes": raw.len(),
            "fnv": fnv1a32(raw),
        }));
    }
    let manifest = json!({
        "version": 1,
        "cue": "demo-disc.cue",
        "data": {
            "file": "demo-data.bin.gz",
            "gz_bytes": data_gz.len(),
            "raw_bytes": data_raw.len(),
            "fnv": fnv1a32(data_raw),
        },
        "tracks": manifest_tracks,
    });

    // The round trip: decompress + decode every piece and demand the disc back.
    let mut rebuilt = gunzip(&data_gz)?;
    for entry in manifest["tracks"].as_array().into_iter().flatten() {
        let file = entry["file"].as_str().unwrap_or_default();
        rebuilt.extend(flac_decode(&outdir.join(file))?);
    }
    ensure!(
        rebuilt == disc,
        "reassembled delivery differs from the pressed disc"
    );

    fs::write(outdir.join("web-manifest.txt"), manifest_lines(&manifest))?;
    fs::write(outdir.join("web-manifest.json"), dumps(&manifest, 1, false))?;
    let saved = disc.len() as i64 - data_gz.len() as i64 - flac_total;
    Ok(format!(
        "web delivery: data {} MiB -> {} MiB gz, {} track(s) to FLAC, {} MiB saved, round trip OK",
        mebibytes(data_raw.len() as i64),
        mebibytes(data_gz.len() as i64),
        audio.len(),
        mebibytes(saved)
    ))
}

pub fn run(args: &[String]) -> Result<i32> {
    ensure!(
        args.len() >= 3,
        "usage: disc-tools web-delivery <disc.cue> <disc.bin> <outdir> [TITLE ...]"
    );
    let cue = fs::read_to_string(&args[0]).map_err(|e| Error(format!("{}: {e}", args[0])))?;
    let disc = fs::read(&args[1]).map_err(|e| Error(format!("{}: {e}", args[1])))?;
    println!("{}", deliver(&cue, &disc, Path::new(&args[2]), &args[3..])?);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUE: &str = "FILE \"d.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n  TRACK 02 AUDIO\r\n    INDEX 00 00:00:20\r\n    INDEX 01 00:00:30\r\n  TRACK 03 AUDIO\r\n    INDEX 01 00:01:00\r\n";

    fn have_flac() -> bool {
        Command::new("flac")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// 20 data sectors, then two audio tracks of 12 and 9 sectors.
    fn synthetic_disc() -> (String, Vec<u8>) {
        let cue = "FILE \"d.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 00 00:00:20\n    INDEX 01 00:00:22\n  TRACK 03 AUDIO\n    INDEX 01 00:00:32\n";
        let mut disc = Vec::new();
        for i in 0..(20 + 12 + 9) * SECTOR {
            disc.push(((i * 7) ^ (i >> 5)) as u8);
        }
        (cue.to_string(), disc)
    }

    #[test]
    fn cue_uses_index_00_when_present_else_index_01() {
        let tracks = parse_cue(CUE).unwrap();
        assert_eq!(
            tracks,
            vec![
                CueTrack {
                    number: 1,
                    kind: "MODE2/2352".into(),
                    start: 0
                },
                CueTrack {
                    number: 2,
                    kind: "AUDIO".into(),
                    start: 20
                },
                CueTrack {
                    number: 3,
                    kind: "AUDIO".into(),
                    start: 75
                },
            ]
        );
    }

    #[test]
    fn splitlines_matches_python() {
        assert_eq!(split_lines("a\r\nb\rc\nd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_lines("a\n\nb\n"), vec!["a", "", "b"]);
        assert_eq!(split_lines(""), Vec::<&str>::new());
        assert_eq!(split_lines("x\u{2028}y"), vec!["x", "y"]);
    }

    #[test]
    fn manifest_text_twin_has_one_line_per_piece() {
        let manifest = json!({
            "version": 1, "cue": "demo-disc.cue",
            "data": {"file": "demo-data.bin.gz", "gz_bytes": 10, "raw_bytes": 99, "fnv": 7},
            "tracks": [{"number": 2, "title": "TWO WORDS", "file": "track-02.flac", "flac_bytes": 5, "raw_bytes": 50, "fnv": 9}],
        });
        assert_eq!(
            manifest_lines(&manifest),
            "data demo-data.bin.gz 10 99 7\ntrack 2 track-02.flac 5 50 9 TWO WORDS\n"
        );
    }

    #[test]
    fn mebibytes_floors_like_python() {
        assert_eq!(mebibytes(3 * 1024 * 1024 + 5), 3);
        assert_eq!(mebibytes(-1), -1);
        assert_eq!(mebibytes(0), 0);
    }

    #[test]
    fn slices_clamp_like_python() {
        let data = [1u8, 2, 3, 4];
        assert_eq!(slice(&data, 2, 99), &[3, 4]);
        assert_eq!(slice(&data, 9, 12), &[] as &[u8]);
        assert_eq!(slice(&data, 3, 1), &[] as &[u8]);
    }

    #[test]
    fn gzip_round_trips() {
        let data: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(gunzip(&gzip_best(&data).unwrap()).unwrap(), data);
    }

    #[test]
    fn delivery_round_trips_and_writes_both_manifests() {
        if !have_flac() {
            eprintln!("skipping: flac is not installed");
            return;
        }
        let (cue, disc) = synthetic_disc();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("web");
        let titles = vec!["ONE".to_string()];
        let summary = deliver(&cue, &disc, &out, &titles).unwrap();
        assert!(summary.ends_with("round trip OK"), "{summary}");
        let text = fs::read_to_string(out.join("web-manifest.txt")).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("data demo-data.bin.gz "));
        assert!(lines[0].contains(&format!(" {} ", 20 * SECTOR)));
        assert!(lines[1].starts_with("track 2 track-02.flac "));
        assert!(lines[1].ends_with(&format!(
            " {} {} ONE",
            12 * SECTOR,
            fnv1a32(&disc[20 * SECTOR..32 * SECTOR])
        )));
        assert!(lines[2].ends_with(" TRACK 03"));
        let json = fs::read_to_string(out.join("web-manifest.json")).unwrap();
        assert!(json.starts_with(
            "{\n \"version\": 1,\n \"cue\": \"demo-disc.cue\",\n \"data\": {\n  \"file\""
        ));
        assert!(!json.ends_with('\n'));
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tracks"][1]["raw_bytes"], 9 * SECTOR);
    }

    #[test]
    fn delivery_refuses_bad_discs() {
        let (cue, mut disc) = synthetic_disc();
        let dir = tempfile::tempdir().unwrap();
        disc.push(0);
        let error = deliver(&cue, &disc, &dir.path().join("a"), &[]).unwrap_err();
        assert!(error.0.contains("whole raw sectors"), "{error}");
        disc.pop();
        let audio_first = cue.replace("MODE2/2352", "AUDIO");
        let error = deliver(&audio_first, &disc, &dir.path().join("b"), &[]).unwrap_err();
        assert!(
            error.0.contains("track 1 must be the data track"),
            "{error}"
        );
        let data_second = cue.replacen("TRACK 03 AUDIO", "TRACK 03 MODE2/2352", 1);
        if have_flac() {
            let error = deliver(&data_second, &disc, &dir.path().join("c"), &[]).unwrap_err();
            assert!(error.0.contains("expected AUDIO"), "{error}");
        }
    }
}
