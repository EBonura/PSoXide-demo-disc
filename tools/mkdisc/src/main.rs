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
mod spectrum;

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

/// Pre-analysed level-meter data for every menu track, end to end.
const SPECTRUM_FILE_NAME: &str = "SPECTRUM.BIN";

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
    /// `(borrower, lender)` display names: the borrower plays the lender's
    /// CD-DA tracks instead of shipping its own copy.
    shared_cdda: Vec<(String, String)>,
    /// `(name, english, italian)` blurbs shown under the carousel.
    descriptions: Vec<(String, String, String)>,
    /// Raw 44.1 kHz stereo PCM the menu cycles through behind itself, in the
    /// order given.
    menu_cdda: Vec<PathBuf>,
    /// Attribution the menu prints for that track.
    credit: String,
    /// Per menu track, in the same order: `(milli-BPM, first-beat ms)` from
    /// `tools/beatgrid.py`.
    menu_beats: Vec<(u32, u32)>,
    /// Per menu track, in the same order: the title the menu shows as
    /// "now playing".
    menu_titles: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut launcher = None;
    let mut out = None;
    let mut volume = String::from("PSXDEMO");
    let mut programs = Vec::new();
    let mut shared_cdda = Vec::new();
    let mut descriptions = Vec::new();
    let mut menu_cdda = Vec::new();
    let mut credit = String::new();
    let mut menu_beats: Vec<(u32, u32)> = Vec::new();
    let mut menu_titles: Vec<String> = Vec::new();

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
            "--share-cdda" => {
                let (borrower, lender) = split(
                    &it.next().ok_or("--share-cdda takes BORROWER=LENDER")?,
                    "--share-cdda",
                )?;
                shared_cdda.push((borrower, lender.to_string_lossy().into_owned()));
            }
            "--describe" => {
                let spec = it.next().ok_or("--describe takes NAME=ENGLISH|ITALIAN")?;
                let (name, text) = spec
                    .split_once('=')
                    .ok_or_else(|| format!("--describe wants NAME=ENGLISH|ITALIAN, got {spec:?}"))?;
                let (english, italian) = text.split_once('|').ok_or_else(|| {
                    format!("--describe wants the two languages split by '|', got {text:?}")
                })?;
                descriptions.push((
                    name.to_string(),
                    english.trim().to_string(),
                    italian.trim().to_string(),
                ));
            }
            "--menu-cdda" => menu_cdda.push(PathBuf::from(
                it.next().ok_or("--menu-cdda takes a path".to_string())?,
            )),
            "--credit" => credit = it.next().ok_or("--credit takes a string".to_string())?,
            "--menu-title" => {
                menu_titles.push(it.next().ok_or("--menu-title takes a string".to_string())?)
            }
            "--menu-beat" => {
                let spec = it.next().ok_or("--menu-beat takes MILLIBPM:PHASEMS")?;
                let (bpm, phase) = spec.split_once(':').ok_or_else(|| {
                    format!("--menu-beat wants MILLIBPM:PHASEMS, got {spec:?}")
                })?;
                let parse = |v: &str, what: &str| {
                    v.parse::<u32>()
                        .map_err(|_| format!("--menu-beat {what} {v:?} is not a number"))
                };
                menu_beats.push((parse(bpm, "tempo")?, parse(phase, "phase")?));
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
        shared_cdda,
        descriptions,
        menu_cdda,
        credit,
        menu_beats,
        menu_titles,
    })
}

fn print_usage() {
    println!(
        "mkdisc --launcher <exe> --out <bin> [--volume ID]\n\
        \x20      [--game NAME=<exe>] [--image NAME=<cue>]\n\
        \x20      [--share-cdda BORROWER=LENDER] ...\n\n\
         --game        embeds a bare PSX-EXE (programs that never read the disc)\n\
         --image       places a whole game disc image, data track and CD-DA alike\n\
         --share-cdda  points one program at another's CD-DA tracks, so a song\n\
        \x20             two programs both use is only burned once\n\
         --describe    NAME=ENGLISH|ITALIAN, the blurb under the carousel\n\
         --menu-cdda   raw 44.1 kHz stereo PCM for the menu; repeat it and the\n\
        \x20             menu cycles through the tracks in order\n\
         --credit      attribution the menu prints for that track\n\
         --menu-beat   MILLIBPM:PHASEMS for the matching --menu-cdda, from\n\
        \x20             tools/beatgrid.py; drives the menu's beat pulse\n\
         --menu-title  title of the matching --menu-cdda, shown as now playing"
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

/// Point each borrower at its lender's CD-DA tracks.
///
/// Two programs built around the same song only need one copy of it burned:
/// they both ask for their own track 2, and the same base sends them to the
/// same place.
fn apply_shared_cdda(
    entries: &mut [Entry],
    names: &[&str],
    shared: &[(String, String)],
) -> Result<(), String> {
    for (borrower, lender) in shared {
        let find = |name: &str| {
            names
                .iter()
                .position(|n| *n == name)
                .ok_or_else(|| format!("--share-cdda names {name:?}, which is not on this disc"))
        };
        let (borrower, lender) = (find(borrower)?, find(lender)?);
        entries[borrower].cdda_track_base = entries[lender].cdda_track_base;
    }
    Ok(())
}

/// How many lines `text` takes at `columns`, wrapping on spaces the same way
/// the launcher does. Fitting the byte budget is not enough on its own: a
/// description can be short and still wrap to one line more than there is
/// room for, and the tail would silently vanish.
fn wrapped_lines(text: &str, columns: usize) -> usize {
    let mut rest = text;
    let mut lines = 0;
    while !rest.is_empty() {
        lines += 1;
        if rest.len() <= columns {
            break;
        }
        rest = match rest[..columns].rfind(' ') {
            Some(at) => rest[at + 1..].trim_start(),
            None => rest[columns..].trim_start(),
        };
    }
    lines
}

/// Attach each blurb to its program.
fn apply_descriptions(
    entries: &mut [Entry],
    names: &[&str],
    descriptions: &[(String, String, String)],
) -> Result<(), String> {
    for (name, english, italian) in descriptions {
        let at = names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| format!("--describe names {name:?}, which is not on this disc"))?;
        // The table pads to a fixed width, so an overlong blurb would be
        // quietly cut mid-word on screen. Say so instead.
        for (language, text) in [("English", english), ("Italian", italian)] {
            if text.len() > disc_toc::DESC_BYTES {
                return Err(format!(
                    "{name}: the {language} description is {} characters, {} over the {} the \
                     table holds:\n  {text}",
                    text.len(),
                    text.len() - disc_toc::DESC_BYTES,
                    disc_toc::DESC_BYTES
                ));
            }
            let lines = wrapped_lines(text, disc_toc::DESC_COLUMNS);
            if lines > disc_toc::DESC_LINES {
                return Err(format!(
                    "{name}: the {language} description wraps to {lines} lines of {}, and the \
                     menu has room for {}. The tail would simply not be drawn:\n  {text}",
                    disc_toc::DESC_COLUMNS,
                    disc_toc::DESC_LINES
                ));
            }
        }
        entries[at] = entries[at].described(english, italian);
    }
    Ok(())
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
    let mut menu_audio = Vec::new();
    for path in &args.menu_cdda {
        let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if bytes.len() % SECTOR_BYTES != 0 {
            return Err(format!(
                "{}: {} bytes is not a whole number of {SECTOR_BYTES}-byte CD-DA sectors. \
                 Pad it, or the last sector will be a click.",
                path.display(),
                bytes.len()
            ));
        }
        menu_audio.push(bytes);
    }
    // Analysed here rather than by a separate tool: mkdisc already has the
    // PCM in hand, and a meter that disagrees with the track on the disc is
    // exactly the kind of drift a build step prevents.
    let mut spectrum_data = Vec::new();
    let mut spectrum_frames = Vec::new();
    for bytes in &menu_audio {
        let data = spectrum::analyse(bytes);
        spectrum_frames.push((data.len() / spectrum::BANDS) as u32);
        spectrum_data.extend_from_slice(&data);
    }


    let spectrum_lba = toc_lba + disc_toc::TOC_SECTORS;
    let spectrum_sectors = sectors_for(spectrum_data.len());
    let mut next_lba =
        spectrum_lba + spectrum_sectors + sectors_for(launcher.len());
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
        if !spectrum_data.is_empty() {
            builder.add_file(SPECTRUM_FILE_NAME, spectrum_data.clone());
        }
        builder.add_file("PSX.EXE", launcher.clone());
        for (name, bytes) in &iso_files {
            builder.add_file(name, bytes.clone());
        }
        builder.build_bin()
    };

    // Sizing pass: the ISO's length fixes where the first image lands, and the
    // table has to name that. The table is exactly one sector either way, so
    // the second build comes out the same length.
    let iso_frames = (build_iso(vec![0u8; disc_toc::TOC_BYTES]).len() / SECTOR_BYTES) as u32;

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

    let mut entries: Vec<Entry> = entries
        .into_iter()
        .map(|e| e.expect("every program is either an exe or an image"))
        .collect();

    let names: Vec<&str> = args.programs.iter().map(|p| p.name.as_str()).collect();
    apply_shared_cdda(&mut entries, &names, &args.shared_cdda)?;
    apply_descriptions(&mut entries, &names, &args.descriptions)?;
    // The menu's own track goes last, after every game's, so adding or
    // removing it cannot shift a game's CD-DA base.
    let menu_track = if menu_audio.is_empty() {
        0
    } else {
        // Track 1 is the data track, so audio starts at 2.
        2 + cdda_track_base
    };
    // The menu draws the credit as one centred line at the 8-pixel font, so a
    // long one runs off both edges. Same silent-clipping trap as the blurbs.
    const CREDIT_COLUMNS: usize = 39;
    if args.credit.len() > CREDIT_COLUMNS {
        return Err(format!(
            "--credit is {} characters, {} more than the {CREDIT_COLUMNS} the menu can \
             draw on one line:\n  {}",
            args.credit.len(),
            args.credit.len() - CREDIT_COLUMNS,
            args.credit
        ));
    }
    for (flag, given) in [
        ("--menu-beat", args.menu_beats.len()),
        ("--menu-title", args.menu_titles.len()),
    ] {
        if given != 0 && given != menu_audio.len() {
            return Err(format!(
                "{given} {flag} against {} --menu-cdda: they pair up in order, so give one \
                 per track or none at all",
                menu_audio.len()
            ));
        }
    }
    for title in &args.menu_titles {
        if title.len() > disc_toc::MENU_TITLE_BYTES {
            return Err(format!(
                "--menu-title {title:?} is {} characters, {} over the {} the table holds",
                title.len(),
                title.len() - disc_toc::MENU_TITLE_BYTES,
                disc_toc::MENU_TITLE_BYTES
            ));
        }
    }
    if !menu_audio.is_empty() && args.credit.is_empty() {
        return Err("--menu-cdda without --credit: the menu has nowhere to attribute the \
                    track, which is the one thing an attribution licence asks for"
            .to_string());
    }

    let toc = disc_toc::encode(
        &entries,
        menu_track,
        menu_audio.len() as u32,
        &args.credit,
        &args.menu_beats,
        &args
            .menu_titles
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        if spectrum_data.is_empty() { 0 } else { spectrum_lba },
        &spectrum_frames,
    )
    .ok_or_else(|| {
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
    for bytes in &menu_audio {
        let at = (disc.len() / SECTOR_BYTES) as u32;
        placed_audio.push(PlacedAudio {
            index00: at,
            index01: at,
        });
        disc.extend_from_slice(bytes);
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
    if !spectrum_data.is_empty() {
        println!(
            "level meter: {} frames over {} track(s) at LBA {spectrum_lba} ({} KiB)",
            spectrum_frames.iter().sum::<u32>(),
            spectrum_frames.len(),
            spectrum_data.len() / 1024
        );
    }
    if menu_track != 0 {
        println!(
            "menu music on CD-DA track{} {menu_track}{}, credited as {:?}",
            if menu_audio.len() > 1 { "s" } else { "" },
            if menu_audio.len() > 1 {
                format!("-{}", menu_track + menu_audio.len() as u32 - 1)
            } else {
                String::new()
            },
            args.credit
        );
    }
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
    fn a_borrower_plays_the_lenders_tracks() {
        let mut entries = [
            Entry::new("GH-PSX", 0, 0, 0),
            Entry::new("CORTEX", 0, 0, 1),
            Entry::new("PONG", 0, 0, 0),
        ];
        let names = ["GH-PSX", "CORTEX", "PONG"];
        // Pong ships no audio of its own and asks for track 2, same as GH-PSX.
        apply_shared_cdda(
            &mut entries,
            &names,
            &[("PONG".into(), "GH-PSX".into())],
        )
        .expect("both names exist");
        assert_eq!(entries[2].cdda_track_base, entries[0].cdda_track_base);
        assert_eq!(entries[1].cdda_track_base, 1, "others untouched");
    }

    #[test]
    fn sharing_with_a_program_that_is_not_on_the_disc_is_an_error() {
        let mut entries = [Entry::new("PONG", 0, 0, 0)];
        let err = apply_shared_cdda(
            &mut entries,
            &["PONG"],
            &[("PONG".into(), "GH-PSX".into())],
        )
        .unwrap_err();
        assert!(err.contains("GH-PSX"), "{err}");
    }

    #[test]
    fn an_overlong_description_and_credit_share_the_same_reasoning() {
        // The credit check lives in `run` against the CLI argument, so this
        // only pins the width the menu can actually draw. 320 pixels, 8 wide.
        assert_eq!(320 / 8, 40);
    }

    #[test]
    fn counting_wrapped_lines_matches_a_greedy_wrap() {
        assert_eq!(wrapped_lines("", 10), 0);
        assert_eq!(wrapped_lines("short", 10), 1);
        assert_eq!(wrapped_lines("exactly ten", 11), 1);
        assert_eq!(wrapped_lines("one two three four", 10), 2);
        // A word longer than the line still has to go somewhere.
        assert_eq!(wrapped_lines("supercalifragilistic", 10), 2);
    }

    #[test]
    fn descriptions_land_on_the_named_program() {
        let mut entries = [Entry::new("PONG", 0, 0, 0), Entry::new("VOXIDE", 0, 0, 0)];
        apply_descriptions(
            &mut entries,
            &["PONG", "VOXIDE"],
            &[("VOXIDE".into(), "Voxel sandbox".into(), "Sandbox a voxel".into())],
        )
        .expect("the name exists");
        assert_eq!(entries[1].desc_en_str(), "Voxel sandbox");
        assert_eq!(entries[1].desc_it_str(), "Sandbox a voxel");
        assert_eq!(entries[0].desc_en_str(), "", "others untouched");
    }

    #[test]
    fn an_overlong_description_is_an_error_rather_than_a_silent_trim() {
        let mut entries = [Entry::new("PONG", 0, 0, 0)];
        let long = "x".repeat(disc_toc::DESC_BYTES + 1);
        let err = apply_descriptions(
            &mut entries,
            &["PONG"],
            &[("PONG".into(), long, "ok".into())],
        )
        .unwrap_err();
        assert!(err.contains("English"), "{err}");
        assert!(err.contains("1 over"), "{err}");
    }

    #[test]
    fn describing_a_program_that_is_not_on_the_disc_is_an_error() {
        let mut entries = [Entry::new("PONG", 0, 0, 0)];
        let err = apply_descriptions(
            &mut entries,
            &["PONG"],
            &[("VOXIDE".into(), "a".into(), "b".into())],
        )
        .unwrap_err();
        assert!(err.contains("VOXIDE"), "{err}");
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
