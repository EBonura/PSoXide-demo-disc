//! The menu music is authored once (`make music`) and committed as four files
//! that must agree: the XA files, the song table the launcher compiles in, the
//! level-meter analysis and the titles in the Makefile. A re-authoring that
//! updates one and not the rest would press a disc that plays the wrong song
//! or none, so this fails first.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Bytes of one raw XA sector as `psx-audio-cook` writes it.
const XA_SECTOR_BYTES: u64 = 2336;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn songs_rs() -> String {
    fs::read_to_string(repo_root().join("audio/launcher/songs.rs")).expect("songs.rs")
}

/// The numbers inside `pub const NAME: [..] = [ ... ];`.
fn numbers(source: &str, name: &str) -> Vec<u64> {
    let pattern = Regex::new(&format!(r"pub const {name}: [^=]*= \[([^\]]*)\];")).unwrap();
    let body = &pattern
        .captures(source)
        .unwrap_or_else(|| panic!("{name} not in songs.rs"))[1];
    Regex::new(r"\d+")
        .unwrap()
        .find_iter(body)
        .map(|m| m.as_str().parse().unwrap())
        .collect()
}

#[test]
fn each_xa_file_is_as_long_as_the_song_table_says() {
    let source = songs_rs();
    let names = Regex::new(r#""([A-Z0-9]+\.XA)""#).unwrap();
    let names: Vec<&str> = names
        .captures_iter(&source)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    let sectors = numbers(&source, "XA_FILE_SECTORS");
    assert_eq!(names.len(), sectors.len());
    for (name, want) in names.iter().zip(sectors) {
        let size = fs::metadata(repo_root().join("audio/launcher").join(name))
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .len();
        assert_eq!(size, want * XA_SECTOR_BYTES, "{name} is not {want} sectors");
    }
}

#[test]
fn songs_analysis_and_titles_agree() {
    let songs = Regex::new(r"\((\d+), (\d+), (\d+)\)")
        .unwrap()
        .find_iter(
            songs_rs()
                .lines()
                .find(|l| l.starts_with("pub const XA_SONGS"))
                .expect("XA_SONGS"),
        )
        .count();
    let analysis = fs::read(repo_root().join("audio/launcher/menu.spectrum")).expect("spectrum");
    assert_eq!(&analysis[..8], b"PSXSPEC1");
    let analysed = u32::from_le_bytes(analysis[8..12].try_into().unwrap()) as usize;
    let makefile = fs::read_to_string(repo_root().join("Makefile")).unwrap();
    let titled = makefile.matches("--menu-title \"").count();
    assert_eq!(songs, analysed, "song table against meter analysis");
    assert_eq!(songs, titled, "song table against Makefile titles");
    assert!(songs <= 8, "the disc table holds eight songs");
}

#[test]
fn no_song_runs_longer_than_its_file() {
    let source = songs_rs();
    let sectors = numbers(&source, "XA_FILE_SECTORS");
    let songs_line = source
        .lines()
        .find(|l| l.starts_with("pub const XA_SONGS"))
        .unwrap();
    for c in Regex::new(r"\((\d+), (\d+), (\d+)\)")
        .unwrap()
        .captures_iter(songs_line)
    {
        let (file, ms): (usize, u64) = (c[1].parse().unwrap(), c[3].parse().unwrap());
        // A song of N milliseconds spans N * 75 / 1000 sectors of its file.
        assert!(ms * 75 / 1000 < sectors[file], "song of {ms} ms in file {file}");
    }
}
