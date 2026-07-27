//! Build the demo disc: one launcher, N chain-loadable programs, one image.
//!
//! Disc layout (LBAs are ISO 9660 sectors; `psx_iso` places files
//! sequentially from 21):
//!
//! ```text
//! 21  SYSTEM.CNF     boot record, points the BIOS at PSX.EXE
//! 22  DEMOTOC.BIN    the table the launcher reads (fixed LBA, see disc-toc)
//! 23+ PSX.EXE        the launcher
//! ..  <game>.EXE     one per program, chain-loaded by LBA
//! ```
//!
//! Usage:
//!
//! ```text
//! mkdisc --launcher <exe> --out <bin> [--volume ID] --game NAME=<exe> ...
//! ```
//!
//! `NAME` is what the menu shows; `=` splits it from the path, so names may
//! contain spaces but not `=`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use disc_toc::Entry;
use psx_iso::iso9660::{default_system_cnf, IsoBuilder, SECTOR_SIZE};

/// Where the chain-load blob runs (`loader/loader.ld`). A program whose
/// payload reaches this address would be overwritten by the very code loading
/// it, so the disc build refuses it.
const LOADER_BASE: u32 = 0x801F_0000;

/// Lowest legal PSX-EXE load address (`sdk/psoxide.ld`).
const MIN_LOAD_ADDR: u32 = 0x8001_0000;

const EXE_MAGIC: &[u8; 8] = b"PS-X EXE";

struct Game {
    name: String,
    path: PathBuf,
}

struct Args {
    launcher: PathBuf,
    out: PathBuf,
    volume: String,
    games: Vec<Game>,
}

fn parse_args() -> Result<Args, String> {
    let mut launcher = None;
    let mut out = None;
    let mut volume = String::from("PSXDEMO");
    let mut games = Vec::new();

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--launcher" => {
                launcher = Some(PathBuf::from(
                    it.next().ok_or("--launcher takes a path".to_string())?,
                ))
            }
            "--out" => {
                out = Some(PathBuf::from(
                    it.next().ok_or("--out takes a path".to_string())?,
                ))
            }
            "--volume" => volume = it.next().ok_or("--volume takes a string".to_string())?,
            "--game" => {
                let spec = it.next().ok_or("--game takes NAME=path".to_string())?;
                let (name, path) = spec
                    .split_once('=')
                    .ok_or_else(|| format!("--game wants NAME=path, got {spec:?}"))?;
                games.push(Game {
                    name: name.to_string(),
                    path: PathBuf::from(path),
                });
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    Ok(Args {
        launcher: launcher.ok_or("--launcher is required".to_string())?,
        out: out.ok_or("--out is required".to_string())?,
        volume,
        games,
    })
}

fn print_usage() {
    println!(
        "mkdisc --launcher <exe> --out <bin> [--volume ID] --game NAME=<exe> ...\n\n\
         Builds the PSoXide demo disc: the launcher boots, reads DEMOTOC.BIN,\n\
         and chain-loads whichever program the user picks."
    );
}

/// The bits of a PSX-EXE header the disc build cares about.
struct ExeHeader {
    load_addr: u32,
    payload_bytes: u32,
}

fn parse_exe_header(bytes: &[u8], what: &Path) -> Result<ExeHeader, String> {
    if bytes.len() < 2048 {
        return Err(format!("{}: shorter than a PSX-EXE header", what.display()));
    }
    if &bytes[..8] != EXE_MAGIC {
        return Err(format!("{}: not a PSX-EXE (bad magic)", what.display()));
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    Ok(ExeHeader {
        load_addr: word(0x18),
        payload_bytes: word(0x1C),
    })
}

/// Reject a program that would be loaded on top of the chain-load blob.
///
/// The blob streams the payload in and then jumps, so only the loaded payload
/// matters here: `.bss` is zeroed by the target's own `_start`, after the blob
/// has stopped running.
fn check_fits_below_loader(header: &ExeHeader, what: &Path) -> Result<(), String> {
    if header.load_addr < MIN_LOAD_ADDR {
        return Err(format!(
            "{}: loads at {:#010X}, below the {:#010X} floor",
            what.display(),
            header.load_addr,
            MIN_LOAD_ADDR
        ));
    }
    let end = header.load_addr.saturating_add(header.payload_bytes);
    if end > LOADER_BASE {
        return Err(format!(
            "{}: payload ends at {:#010X}, past the chain-load blob at {:#010X}. \
             Shrink the program or move LOADER_BASE (loader/loader.ld, launcher, mkdisc).",
            what.display(),
            end,
            LOADER_BASE
        ));
    }
    Ok(())
}

fn sectors_for(bytes: usize) -> u32 {
    bytes.div_ceil(SECTOR_SIZE) as u32
}

/// ISO 9660 short name for a menu entry: uppercase alphanumerics, truncated
/// to 8 characters, plus `.EXE`.
fn iso_name(display: &str, index: usize) -> String {
    let stem: String = display
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(6)
        .collect::<String>()
        .to_ascii_uppercase();
    if stem.is_empty() {
        format!("GAME{index:02}.EXE")
    } else {
        format!("{stem}{index:02}.EXE")
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    let launcher = fs::read(&args.launcher)
        .map_err(|e| format!("read {}: {e}", args.launcher.display()))?;
    parse_exe_header(&launcher, &args.launcher)?;

    // 21: SYSTEM.CNF. 22: the table. The table goes ahead of the launcher so
    // its LBA does not shift when the launcher grows.
    let system_cnf = default_system_cnf();
    let toc_lba = psx_iso::PLAYTEST_FIRST_FILE_LBA + sectors_for(system_cnf.len());
    if toc_lba != disc_toc::TOC_LBA {
        return Err(format!(
            "table of contents would land at LBA {toc_lba}, but the launcher \
             reads LBA {}. Adjust disc_toc::TOC_LBA.",
            disc_toc::TOC_LBA
        ));
    }

    // Place every program first: the table has to name their LBAs, and their
    // LBAs depend only on the sizes of what comes before them.
    let mut next_lba = toc_lba + 1 + sectors_for(launcher.len());
    let mut entries = Vec::new();
    let mut placed = Vec::new();
    let mut map = Vec::new();
    for (index, game) in args.games.iter().enumerate() {
        let bytes =
            fs::read(&game.path).map_err(|e| format!("read {}: {e}", game.path.display()))?;
        let header = parse_exe_header(&bytes, &game.path)?;
        check_fits_below_loader(&header, &game.path)?;

        entries.push(Entry::new(&game.name, next_lba));
        map.push(format!(
            "  LBA {next_lba:>7}  {:>6} KiB  {:<24} {}",
            bytes.len() / 1024,
            game.name,
            game.path.display()
        ));
        next_lba += sectors_for(bytes.len());
        placed.push((iso_name(&game.name, index), bytes));
    }

    let toc = disc_toc::encode(&entries).ok_or_else(|| {
        format!(
            "{} programs is more than the {} that fit in the table sector",
            entries.len(),
            disc_toc::MAX_ENTRIES
        )
    })?;

    let mut builder = IsoBuilder::new()
        .volume_id(&args.volume)
        .system_id("PLAYSTATION");
    builder.add_file("SYSTEM.CNF", system_cnf);
    builder.add_file(disc_toc::TOC_FILE_NAME, toc.to_vec());
    builder.add_file("PSX.EXE", launcher);
    for (name, bytes) in placed {
        builder.add_file(&name, bytes);
    }
    let image = builder.build_bin();

    fs::write(&args.out, &image).map_err(|e| format!("write {}: {e}", args.out.display()))?;
    let cue = args.out.with_extension("cue");
    let bin_name = args
        .out
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("output path has no file name")?;
    fs::write(
        &cue,
        format!("FILE \"{bin_name}\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n"),
    )
    .map_err(|e| format!("write {}: {e}", cue.display()))?;

    println!(
        "wrote {} ({} sectors, {:.1} MiB)",
        args.out.display(),
        image.len() / psx_iso::SECTOR_BYTES,
        image.len() as f64 / (1024.0 * 1024.0),
    );
    println!("wrote {}", cue.display());
    println!("table of contents at LBA {}:", disc_toc::TOC_LBA);
    for line in map {
        println!("{line}");
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mkdisc: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(load_addr: u32, payload: u32) -> ExeHeader {
        ExeHeader {
            load_addr,
            payload_bytes: payload,
        }
    }

    #[test]
    fn accepts_a_program_that_clears_the_blob() {
        let h = header(0x8001_0000, 1_500_000);
        assert!(check_fits_below_loader(&h, Path::new("cortex")).is_ok());
    }

    #[test]
    fn rejects_a_program_that_would_overwrite_the_blob() {
        let h = header(0x8001_0000, LOADER_BASE - 0x8001_0000 + 1);
        let err = check_fits_below_loader(&h, Path::new("huge")).unwrap_err();
        assert!(err.contains("chain-load blob"), "{err}");
    }

    #[test]
    fn parses_a_psx_exe_header() {
        let mut bytes = vec![0u8; 2048];
        bytes[..8].copy_from_slice(EXE_MAGIC);
        bytes[0x18..0x1C].copy_from_slice(&0x8001_0000u32.to_le_bytes());
        bytes[0x1C..0x20].copy_from_slice(&4096u32.to_le_bytes());
        let h = parse_exe_header(&bytes, Path::new("x")).expect("valid");
        assert_eq!(h.load_addr, 0x8001_0000);
        assert_eq!(h.payload_bytes, 4096);
    }

    #[test]
    fn rejects_a_file_that_is_not_a_psx_exe() {
        let bytes = vec![0u8; 2048];
        assert!(parse_exe_header(&bytes, Path::new("x")).is_err());
    }

    #[test]
    fn iso_names_stay_within_8_3() {
        let name = iso_name("Magikaaaaarp Pong", 7);
        assert_eq!(name, "MAGIKA07.EXE");
        assert_eq!(iso_name("!!!", 3), "GAME03.EXE");
    }
}
