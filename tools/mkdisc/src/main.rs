//! Build the demo disc: one launcher, N chain-loadable programs, one image.
//!
//! A program that ships on its own disc has that disc's geometry baked in:
//! pack LBAs, CD-DA track numbers. Rather than re-cook every program, `mkdisc`
//! drops each game's *existing* disc image onto the big one unchanged and
//! records two numbers per game -- how far its image moved, and how many CD-DA
//! tracks sit ahead of it. The loader hands those to the game's `_start`, and
//! `psx_io::disc_base` applies them inside the SDK's disc entry points. Nothing
//! in a game's own source knows any of this happened.
//!
//! Disc layout:
//!
//! ```text
//! 21   SYSTEM.CNF     boot record, points the BIOS at PSX.EXE
//! 22   DEMOTOC.BIN    the table the launcher reads (fixed LBA, see disc-toc)
//! 23+  PSX.EXE        the launcher
//! ..   bare EXEs      programs with no disc data of their own
//! ..   game images    each game's whole data track, verbatim but re-addressed
//! ..   audio tracks   every game's CD-DA, in the same order
//! ```
//!
//! Usage:
//!
//! ```text
//! mkdisc --launcher <exe> --out <bin> [--volume ID]
//!        [--game NAME=<exe>] [--image NAME=<cue>] ...
//! ```
//!
//! `--game` embeds a bare PSX-EXE, for programs that never touch the disc
//! after boot. `--image` takes a whole `.cue`, for programs that stream data
//! or play CD-DA. `NAME` is what the menu shows; `=` splits it from the path.

mod cue;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use disc_toc::Entry;
use psx_iso::iso9660::{default_system_cnf, IsoBuilder, SECTOR_SIZE};
use psx_iso::SECTOR_BYTES;

/// Where the chain-load blob runs (`loader/loader.ld`). A program whose
/// payload reaches this address would be overwritten by the very code loading
/// it, so the disc build refuses it.
const LOADER_BASE: u32 = 0x801F_0000;

/// Lowest legal PSX-EXE load address (`sdk/psoxide.ld`).
const MIN_LOAD_ADDR: u32 = 0x8001_0000;

/// Every PSoXide disc puts its boot EXE here (`psx_iso::PLAYTEST_BOOT_EXE_START_LBA`).
/// Checked against the magic when an image is placed, so an image that breaks
/// the convention fails the build instead of booting into noise.
const IMAGE_BOOT_EXE_LBA: u32 = psx_iso::PLAYTEST_BOOT_EXE_START_LBA;

/// Frames of silence a CUE conventionally puts before an audio track.
const PREGAP_FRAMES: u32 = 150;

const EXE_MAGIC: &[u8; 8] = b"PS-X EXE";

enum Source {
    /// A bare PSX-EXE, embedded as an ISO file.
    Exe(PathBuf),
    /// A whole disc image, placed verbatim.
    Image(PathBuf),
}

struct Program {
    name: String,
    source: Source,
}

struct Args {
    launcher: PathBuf,
    out: PathBuf,
    volume: String,
    programs: Vec<Program>,
}

fn parse_args() -> Result<Args, String> {
    let mut launcher = None;
    let mut out = None;
    let mut volume = String::from("PSXDEMO");
    let mut programs = Vec::new();

    let split = |spec: &str, flag: &str| -> Result<(String, PathBuf), String> {
        let (name, path) = spec
            .split_once('=')
            .ok_or_else(|| format!("{flag} wants NAME=path, got {spec:?}"))?;
        Ok((name.to_string(), PathBuf::from(path)))
    };

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
                let (name, path) = split(&it.next().ok_or("--game takes NAME=path")?, "--game")?;
                programs.push(Program {
                    name,
                    source: Source::Exe(path),
                });
            }
            "--image" => {
                let (name, path) = split(&it.next().ok_or("--image takes NAME=path")?, "--image")?;
                programs.push(Program {
                    name,
                    source: Source::Image(path),
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
        programs,
    })
}

fn print_usage() {
    println!(
        "mkdisc --launcher <exe> --out <bin> [--volume ID]\n\
        \x20      [--game NAME=<exe>] [--image NAME=<cue>] ...\n\n\
         --game  embeds a bare PSX-EXE (programs that never read the disc)\n\
         --image places a whole game disc image, data track and CD-DA alike"
    );
}

/// The bits of a PSX-EXE header the disc build cares about.
struct ExeHeader {
    load_addr: u32,
    payload_bytes: u32,
}

fn parse_exe_header(bytes: &[u8], what: &Path) -> Result<ExeHeader, String> {
    if bytes.len() < SECTOR_SIZE {
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
/// to 6 characters, plus a 2-digit index and `.EXE`.
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

/// Rewrite a relocated data sector's BCD MSF header so the drive's seeks land
/// where the sector actually is.
///
/// Safe to do in place: Mode 2 Form 1 ECC is computed with these four bytes
/// zeroed and the EDC covers `0x10..0x818`, so neither depends on them (see
/// `encode_mode2_form1_edc_ecc` in `psx-iso`).
fn rewrite_sector_msf(sector: &mut [u8], lba: u32) {
    let bcd = |v: u32| (((v / 10) << 4) | (v % 10)) as u8;
    let absolute = lba + PREGAP_FRAMES;
    sector[0x0C] = bcd(absolute / (60 * 75));
    sector[0x0D] = bcd((absolute / 75) % 60);
    sector[0x0E] = bcd(absolute % 75);
}

/// Place `data` (a whole data track, raw 2352-byte sectors) at `lba`,
/// re-addressing every sector on the way.
fn place_data_track(image: &mut Vec<u8>, data: &[u8], lba: u32) -> Result<(), String> {
    if data.len() % SECTOR_BYTES != 0 {
        return Err(format!(
            "data track is {} bytes, not a whole number of {SECTOR_BYTES}-byte sectors",
            data.len()
        ));
    }
    for (index, sector) in data.chunks(SECTOR_BYTES).enumerate() {
        let at = image.len();
        image.extend_from_slice(sector);
        rewrite_sector_msf(&mut image[at..at + SECTOR_BYTES], lba + index as u32);
    }
    Ok(())
}

fn msf(frames: u32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        frames / (60 * 75),
        (frames / 75) % 60,
        frames % 75
    )
}

/// One audio track's final position on the demo disc.
struct PlacedAudio {
    index00: u32,
    index01: u32,
}

fn write_cue(path: &Path, bin_name: &str, audio: &[PlacedAudio]) -> Result<(), String> {
    let mut text = format!("FILE \"{bin_name}\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n");
    for (index, track) in audio.iter().enumerate() {
        let number = index + 2;
        text.push_str(&format!("  TRACK {number:02} AUDIO\n"));
        if track.index00 != track.index01 {
            text.push_str(&format!("    INDEX 00 {}\n", msf(track.index00)));
        }
        text.push_str(&format!("    INDEX 01 {}\n", msf(track.index01)));
    }
    fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// A game image sliced out of its own disc, ready to be re-placed.
struct LoadedImage {
    data: Vec<u8>,
    audio_bytes: Vec<u8>,
    audio: Vec<cue::AudioTrack>,
}

fn load_image(path: &Path) -> Result<LoadedImage, String> {
    let sheet = cue::parse(path)?;
    let bytes = fs::read(&sheet.bin).map_err(|e| format!("read {}: {e}", sheet.bin.display()))?;
    let split = sheet.data_frames as usize * SECTOR_BYTES;
    if bytes.len() < split {
        return Err(format!(
            "{}: claims {} data frames but the BIN only holds {}",
            path.display(),
            sheet.data_frames,
            bytes.len() / SECTOR_BYTES
        ));
    }
    let (data, audio_bytes) = bytes.split_at(split);
    Ok(LoadedImage {
        data: data.to_vec(),
        audio_bytes: audio_bytes.to_vec(),
        audio: sheet.audio,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    let launcher =
        fs::read(&args.launcher).map_err(|e| format!("read {}: {e}", args.launcher.display()))?;
    parse_exe_header(&launcher, &args.launcher)?;

    let system_cnf = default_system_cnf();
    let toc_lba = psx_iso::PLAYTEST_FIRST_FILE_LBA + sectors_for(system_cnf.len());
    if toc_lba != disc_toc::TOC_LBA {
        return Err(format!(
            "table of contents would land at LBA {toc_lba}, but the launcher \
             reads LBA {}. Adjust disc_toc::TOC_LBA.",
            disc_toc::TOC_LBA
        ));
    }

    // Bare EXEs ride inside the launcher's own ISO, so their LBAs follow from
    // the file order. Whole images are appended after the ISO ends, which the
    // ISO's own length decides -- so build the ISO first, then place them.
    let mut next_lba = toc_lba + 1 + sectors_for(launcher.len());
    let mut entries: Vec<Option<Entry>> = vec![None; args.programs.len()];
    let mut iso_files = Vec::new();
    let mut map = Vec::new();

    for (index, program) in args.programs.iter().enumerate() {
        let Source::Exe(path) = &program.source else {
            continue;
        };
        let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let header = parse_exe_header(&bytes, path)?;
        check_fits_below_loader(&header, path)?;
        entries[index] = Some(Entry::new(&program.name, next_lba, 0, 0));
        map.push((
            next_lba,
            bytes.len(),
            program.name.clone(),
            String::from("exe"),
        ));
        next_lba += sectors_for(bytes.len());
        iso_files.push((iso_name(&program.name, index), bytes));
    }

    // Load every image up front: their audio has to be concatenated after all
    // the data, and the track base for each depends on the ones before it.
    let mut images = Vec::new();
    for (index, program) in args.programs.iter().enumerate() {
        let Source::Image(path) = &program.source else {
            continue;
        };
        let image = load_image(path)?;
        let boot_at = IMAGE_BOOT_EXE_LBA as usize * SECTOR_BYTES;
        let boot_sector = image
            .data
            .get(boot_at + 24..boot_at + 24 + SECTOR_SIZE)
            .ok_or_else(|| format!("{}: data track is too short to hold a boot EXE", path.display()))?;
        let header = parse_exe_header(boot_sector, path).map_err(|_| {
            format!(
                "{}: no PSX-EXE at LBA {IMAGE_BOOT_EXE_LBA}. mkdisc places images \
                 verbatim and expects PSoXide's boot layout.",
                path.display()
            )
        })?;
        check_fits_below_loader(&header, path)?;
        images.push((index, path.clone(), image));
    }

    let build_iso = |toc: Vec<u8>| {
        let mut builder = IsoBuilder::new()
            .volume_id(&args.volume)
            .system_id("PLAYSTATION");
        builder.add_file("SYSTEM.CNF", system_cnf.clone());
        builder.add_file(disc_toc::TOC_FILE_NAME, toc);
        builder.add_file("PSX.EXE", launcher.clone());
        for (name, bytes) in &iso_files {
            builder.add_file(name, bytes.clone());
        }
        builder.build_bin()
    };

    // Sizing pass: the ISO's length fixes where the first image lands, and the
    // table has to name that. The table is exactly one sector either way, so
    // the second build comes out the same length.
    let iso_frames = (build_iso(vec![0u8; SECTOR_SIZE]).len() / SECTOR_BYTES) as u32;

    let mut image_lba = iso_frames;
    let mut cdda_track_base = 0u32;
    for (index, path, image) in &images {
        let frames = (image.data.len() / SECTOR_BYTES) as u32;
        let program = &args.programs[*index];
        entries[*index] = Some(Entry::new(
            &program.name,
            image_lba + IMAGE_BOOT_EXE_LBA,
            image_lba,
            cdda_track_base,
        ));
        map.push((
            image_lba,
            image.data.len(),
            program.name.clone(),
            format!(
                "image, {} CD-DA track(s) from {}",
                image.audio.len(),
                cdda_track_base + 2
            ),
        ));
        let _ = path;
        image_lba += frames;
        cdda_track_base += image.audio.len() as u32;
    }

    let entries: Vec<Entry> = entries
        .into_iter()
        .map(|e| e.expect("every program is either an exe or an image"))
        .collect();
    let toc = disc_toc::encode(&entries).ok_or_else(|| {
        format!(
            "{} programs is more than the {} that fit in the table sector",
            entries.len(),
            disc_toc::MAX_ENTRIES
        )
    })?;

    let mut disc = build_iso(toc.to_vec());
    if (disc.len() / SECTOR_BYTES) as u32 != iso_frames {
        return Err("ISO length changed between the sizing and final builds".to_string());
    }

    let mut lba = iso_frames;
    for (_, _, image) in &images {
        place_data_track(&mut disc, &image.data, lba)?;
        lba += (image.data.len() / SECTOR_BYTES) as u32;
    }

    // All audio follows all data, in program order.
    let mut placed_audio = Vec::new();
    for (_, _, image) in &images {
        let base = (disc.len() / SECTOR_BYTES) as u32;
        for track in &image.audio {
            placed_audio.push(PlacedAudio {
                index00: base + track.index00,
                index01: base + track.index01,
            });
        }
        disc.extend_from_slice(&image.audio_bytes);
    }

    fs::write(&args.out, &disc).map_err(|e| format!("write {}: {e}", args.out.display()))?;
    let cue_path = args.out.with_extension("cue");
    let bin_name = args
        .out
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("output path has no file name")?;
    write_cue(&cue_path, bin_name, &placed_audio)?;

    let frames = (disc.len() / SECTOR_BYTES) as u32;
    println!(
        "wrote {} ({frames} sectors, {}, {:.1} MiB)",
        args.out.display(),
        msf(frames),
        disc.len() as f64 / (1024.0 * 1024.0),
    );
    println!("wrote {}", cue_path.display());
    println!(
        "{} program(s), {} CD-DA track(s), table of contents at LBA {}:",
        entries.len(),
        placed_audio.len(),
        disc_toc::TOC_LBA
    );
    map.sort_by_key(|(lba, ..)| *lba);
    for (lba, bytes, name, kind) in map {
        println!("  LBA {lba:>7}  {:>7} KiB  {name:<26} {kind}", bytes / 1024);
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
        let mut bytes = vec![0u8; SECTOR_SIZE];
        bytes[..8].copy_from_slice(EXE_MAGIC);
        bytes[0x18..0x1C].copy_from_slice(&0x8001_0000u32.to_le_bytes());
        bytes[0x1C..0x20].copy_from_slice(&4096u32.to_le_bytes());
        let h = parse_exe_header(&bytes, Path::new("x")).expect("valid");
        assert_eq!(h.load_addr, 0x8001_0000);
        assert_eq!(h.payload_bytes, 4096);
    }

    #[test]
    fn rejects_a_file_that_is_not_a_psx_exe() {
        let bytes = vec![0u8; SECTOR_SIZE];
        assert!(parse_exe_header(&bytes, Path::new("x")).is_err());
    }

    #[test]
    fn iso_names_stay_within_8_3() {
        assert_eq!(iso_name("Magikaaaaarp Pong", 7), "MAGIKA07.EXE");
        assert_eq!(iso_name("!!!", 3), "GAME03.EXE");
    }

    #[test]
    fn relocated_sectors_carry_their_new_address() {
        // Absolute time is LBA + 150 frames, BCD-encoded.
        let msf_at = |lba: u32| {
            let mut sector = vec![0u8; SECTOR_BYTES];
            rewrite_sector_msf(&mut sector, lba);
            [sector[0x0C], sector[0x0D], sector[0x0E]]
        };
        assert_eq!(msf_at(0), [0x00, 0x02, 0x00]);
        // 4350 + 150 = 4500 frames = exactly one minute.
        assert_eq!(msf_at(4350), [0x01, 0x00, 0x00]);
        // BCD, not hex: 59 seconds is 0x59.
        assert_eq!(msf_at(4349), [0x00, 0x59, 0x74]);
    }

    #[test]
    fn placing_a_track_re_addresses_every_sector_and_keeps_the_payload() {
        let mut data = vec![0u8; 3 * SECTOR_BYTES];
        for (i, sector) in data.chunks_mut(SECTOR_BYTES).enumerate() {
            sector[24] = i as u8 + 1;
        }
        let mut image = Vec::new();
        place_data_track(&mut image, &data, 1000).expect("aligned");
        assert_eq!(image.len(), data.len());
        for (i, sector) in image.chunks(SECTOR_BYTES).enumerate() {
            assert_eq!(sector[24], i as u8 + 1, "payload survived");
            let mut expect = vec![0u8; SECTOR_BYTES];
            rewrite_sector_msf(&mut expect, 1000 + i as u32);
            assert_eq!(sector[0x0C..0x0F], expect[0x0C..0x0F], "sector {i} re-addressed");
        }
    }

    #[test]
    fn rejects_a_track_that_is_not_whole_sectors() {
        let mut image = Vec::new();
        assert!(place_data_track(&mut image, &vec![0u8; 100], 0).is_err());
    }

    #[test]
    fn cue_times_are_absolute_on_the_finished_disc() {
        let dir = std::env::temp_dir().join("mkdisc-cue-out");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.cue");
        write_cue(
            &path,
            "demo.bin",
            &[
                PlacedAudio {
                    index00: 75 * 60,
                    index01: 75 * 62,
                },
                PlacedAudio {
                    index00: 75 * 90,
                    index01: 75 * 90,
                },
            ],
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("TRACK 02 AUDIO\n    INDEX 00 01:00:00\n    INDEX 01 01:02:00"));
        // No pregap means no INDEX 00 line.
        assert!(text.contains("TRACK 03 AUDIO\n    INDEX 01 01:30:00"));
    }
}
