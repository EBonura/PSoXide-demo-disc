//! Compare every audio sector of a pressing with the cue it was taken from.
//!
//! The pressing owns the menu songs (tracks 2 to 5, from `--audio-dir`) and
//! then each program's own tracks, in the order the table's `cdda_track_base`
//! gives them. For each `--input NAME=CUE` that carries audio, the track base in
//! the table must be the running count, every source track must sit in the
//! pressing with the same pregap and length, and the sector bytes must hash
//! the same. Entries that carry no audio are named with `--no-audio` and must
//! have a base of zero. The pressing must have no audio track left over.
//!
//! Replaces `check_audio_relocation.py`, which hard coded one carousel order
//! and sibling checkout paths.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::args::Args;
use crate::disc::{image_for_cue, read_toc, SECTOR_BYTES};
use crate::util::{expand_home, hex, Error, Result};

/// The four menu songs, in the order the launcher plays them.
const MENU_SONGS: [&str; 4] = [
    "knuckle-dust",
    "rusted-hammer",
    "chainsaw-heart",
    "night-crawler",
];

#[derive(Debug, Clone)]
struct Track {
    number: u32,
    audio: bool,
    index00: Option<u64>,
    index01: u64,
    /// First sector of the track including its pregap.
    start: u64,
    /// One past the last sector.
    end: u64,
}

fn parse_cue(cue: &Path) -> Result<(PathBuf, Vec<Track>)> {
    let image = image_for_cue(cue)?;
    let text = fs::read_to_string(cue)?;
    let track_line = Regex::new(r"^\s*TRACK (\d+) (\S+)")?;
    let index_line = Regex::new(r"^\s*INDEX (00|01) (\d+):(\d+):(\d+)")?;
    let mut raw: Vec<(u32, bool, Option<u64>, Option<u64>)> = Vec::new();
    for line in text.lines() {
        if let Some(c) = track_line.captures(line) {
            raw.push((c[1].parse().unwrap_or(0), &c[2] == "AUDIO", None, None));
        } else if let Some(c) = index_line.captures(line) {
            let at = |i: usize| c[i].parse::<u64>().unwrap_or(0);
            let sector = at(2) * 4500 + at(3) * 75 + at(4);
            let last = raw
                .last_mut()
                .ok_or_else(|| Error("INDEX before any TRACK".into()))?;
            if &c[1] == "00" {
                last.2 = Some(sector);
            } else {
                last.3 = Some(sector);
            }
        }
    }
    let total = fs::metadata(&image)?.len() / SECTOR_BYTES;
    let mut tracks: Vec<Track> = Vec::new();
    for (i, (number, audio, i0, i1)) in raw.iter().enumerate() {
        ensure!(
            *number as usize == i + 1,
            "{}: tracks are not numbered in order",
            cue.display()
        );
        let index01 = i1.ok_or_else(|| Error(format!("track {number} has no INDEX 01")))?;
        tracks.push(Track {
            number: *number,
            audio: *audio,
            index00: *i0,
            index01,
            start: i0.unwrap_or(index01),
            end: 0,
        });
    }
    for i in 0..tracks.len() {
        tracks[i].end = if i + 1 < tracks.len() {
            tracks[i + 1].start
        } else {
            total
        };
        ensure!(
            tracks[i].start <= tracks[i].index01 && tracks[i].index01 < tracks[i].end,
            "{}: track {} has impossible indexes",
            cue.display(),
            tracks[i].number
        );
    }
    Ok((image, tracks))
}

/// sha256 of `[start, end)` sectors of an image.
fn digest_sectors(image: &Path, start: u64, end: u64) -> Result<String> {
    let mut stream = File::open(image)?;
    stream.seek(SeekFrom::Start(start * SECTOR_BYTES))?;
    let mut digest = Sha256::new();
    let mut remaining = (end - start) * SECTOR_BYTES;
    let mut chunk = vec![0u8; 1 << 20];
    while remaining > 0 {
        let want = remaining.min(chunk.len() as u64) as usize;
        stream
            .read_exact(&mut chunk[..want])
            .map_err(|e| Error(format!("{}: {e}", image.display())))?;
        digest.update(&chunk[..want]);
        remaining -= want as u64;
    }
    Ok(hex(&digest.finalize()))
}

pub fn run(args: &[String]) -> Result<i32> {
    let args = Args::parse(args, &["cue", "audio-dir", "input", "no-audio"], &[])?;
    let combined_cue = args.require_path("cue")?;
    let audio_dir = args.require_path("audio-dir")?;
    let (combined, tracks) = parse_cue(&combined_cue)?;
    let entries = read_toc(&combined)?;
    let base_of = |name: &str| -> Result<u32> {
        entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.cdda_track_base)
            .ok_or_else(|| Error(format!("{name}: not in the disc table")))
    };
    ensure!(
        tracks.len() > MENU_SONGS.len(),
        "the pressing has no room for the menu songs"
    );

    let mut report: Vec<Value> = Vec::new();
    for (offset, song) in MENU_SONGS.iter().enumerate() {
        let number = offset + 2;
        let track = &tracks[number - 1];
        ensure!(track.audio, "track {number} is not audio");
        let raw = fs::read(audio_dir.join(format!("{song}.cdda")))
            .map_err(|e| Error(format!("{song}.cdda: {e}")))?;
        let mut expected = raw.clone();
        expected.resize(
            raw.len().div_ceil(SECTOR_BYTES as usize) * SECTOR_BYTES as usize,
            0,
        );
        let want = hex(&Sha256::digest(&expected));
        let got = digest_sectors(&combined, track.index01, track.end)?;
        ensure!(want == got, "menu song {song} differs from track {number}");
        report.push(json!({"owner": "LAUNCHER", "track": number, "sha256": want}));
    }

    // (table base, name, cue) for every input that carries audio.
    let mut inputs: Vec<(u32, String, PathBuf)> = Vec::new();
    for spec in args.all("input") {
        let (name, cue) = spec
            .split_once('=')
            .ok_or_else(|| Error(format!("--input wants NAME=CUE, got {spec:?}")))?;
        inputs.push((base_of(name)?, name.to_string(), expand_home(cue)));
    }
    inputs.sort_by_key(|(base, name, _)| (*base, name.clone()));

    let mut base: u32 = MENU_SONGS.len() as u32;
    for (table_base, name, cue) in &inputs {
        let (source, original) = parse_cue(cue)?;
        if original.len() <= 1 {
            // A data-only image: the table still records some base for it, and
            // nothing reads CD-DA through it, so there is nothing to compare.
            continue;
        }
        ensure!(
            *table_base == base,
            "{name}: table track base {table_base}, expected {base}"
        );
        for st in &original[1..] {
            let index = base as usize + st.number as usize - 1;
            let track = tracks
                .get(index)
                .ok_or_else(|| Error(format!("{name}: no pressed track {}", index + 1)))?;
            ensure!(
                track.audio && st.audio,
                "{name}: source track {} is not audio on both sides",
                st.number
            );
            ensure!(
                track.index01 - track.start == st.index01 - st.start
                    && track.end - track.start == st.end - st.start,
                "{name}: track {} pregap or length moved",
                st.number
            );
            let hash = digest_sectors(&source, st.start, st.end)?;
            ensure!(
                hash == digest_sectors(&combined, track.start, track.end)?,
                "{name}: source track {} differs from pressed track {}",
                st.number,
                track.number
            );
            report.push(json!({
                "owner": name,
                "source_track": st.number,
                "track": track.number,
                "sectors": track.end - track.index01,
                "sha256_with_pregap": hash,
                "had_index00": st.index00.is_some(),
            }));
        }
        base += original.len() as u32 - 1;
    }
    for name in args.all("no-audio") {
        ensure!(
            base_of(name)? == 0,
            "{name}: expected track base 0 for a program without audio"
        );
    }
    ensure!(
        tracks.len() == base as usize + 1,
        "the pressing has {} tracks but the inputs account for {}",
        tracks.len(),
        base + 1
    );
    ensure!(
        report.len() == tracks.len() - 1,
        "{} reports for {} audio tracks",
        report.len(),
        tracks.len() - 1
    );
    let bases: serde_json::Map<String, Value> = entries
        .iter()
        .map(|e| (e.name.clone(), json!(e.cdda_track_base)))
        .collect();
    let out = json!({
        "cue": combined_cue.display().to_string(),
        "audio_tracks": report.len(),
        "track_bases": bases,
        "tracks": report,
        "result": "PASS",
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("disc-tools-audio-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_cue_gives_each_track_its_pregap_and_end() {
        let dir = temp_dir("cue");
        // 100 sectors: data 0..30, audio with a 4 sector pregap 30..70, audio 70..100.
        fs::write(dir.join("a.bin"), vec![0u8; 100 * SECTOR_BYTES as usize]).unwrap();
        let cue = "FILE \"a.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 00 00:00:30\n    INDEX 01 00:00:34\n  TRACK 03 AUDIO\n    INDEX 01 00:00:70\n";
        fs::write(dir.join("a.cue"), cue).unwrap();
        let (_, tracks) = parse_cue(&dir.join("a.cue")).unwrap();
        assert_eq!(tracks.len(), 3);
        assert!(!tracks[0].audio && tracks[1].audio);
        assert_eq!(
            (tracks[1].start, tracks[1].index01, tracks[1].end),
            (30, 34, 70)
        );
        assert_eq!((tracks[2].start, tracks[2].end), (70, 100));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn digest_covers_exactly_the_requested_sectors() {
        let dir = temp_dir("digest");
        let mut data = vec![0u8; 4 * SECTOR_BYTES as usize];
        data[SECTOR_BYTES as usize] = 7;
        fs::write(dir.join("d.bin"), &data).unwrap();
        let whole = digest_sectors(&dir.join("d.bin"), 0, 4).unwrap();
        let tail = digest_sectors(&dir.join("d.bin"), 1, 2).unwrap();
        let mut one = vec![0u8; SECTOR_BYTES as usize];
        one[0] = 7;
        assert_eq!(tail, hex(&Sha256::digest(&one)));
        assert_ne!(whole, tail);
        fs::remove_dir_all(&dir).unwrap();
    }
}
