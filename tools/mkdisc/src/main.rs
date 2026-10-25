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
//! 26+  SPECTRUM.BIN   level-meter data for the menu songs
//! ..   SHOTS.BIN      menu backdrops
//! ..   PSX.EXE        the launcher
//! ..   bare EXEs      programs with no disc data of their own
//! ..   MUSIC*.XA      the menu songs, XA-ADPCM files in the launcher's ISO
//! ..   game images    each game's whole data track, verbatim but re-addressed
//! ..   audio tracks   every game's CD-DA, in program order
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
mod wav;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use disc_toc::Entry;
use psx_chainloader::{check_payload, payload_extent, Conflict, Span, MIN_LOAD_ADDR, RAM_END};
use psx_iso::iso9660::{default_system_cnf, IsoBuilder, SECTOR_SIZE};
use psx_iso::SECTOR_BYTES;

/// The spans of RAM the chain loader occupies while it streams a program in:
/// its own blob and the stack it runs on (`chainload-layout`, which the loader
/// link and the launcher read too). A program whose payload would land on one
/// would be overwritten by the very code loading it, so the disc build refuses
/// it. The program may sit anywhere else, above the blob as well as below it.
fn reserved_spans() -> [Span; 2] {
    let [blob, stack] = chainload_layout::RESERVED;
    [Span::new(blob.0, blob.1), Span::new(stack.0, stack.1)]
}

/// A boot EXE must appear near the front of an imported data track. Ordinary
/// PSoXide images put it at the canonical playtest LBA; collection images may
/// place cached metadata and screenshots before it.
const IMAGE_BOOT_SCAN_SECTORS: usize = 4_096;

/// Frames of silence a CUE conventionally puts before an audio track.
const PREGAP_FRAMES: u32 = 150;

const EXE_MAGIC: &[u8; 8] = b"PS-X EXE";

/// Bytes of one raw XA sector as `psx-audio-cook` writes it: the 8-byte
/// subheader and the 2328 bytes that follow.
const XA_SECTOR_BYTES: usize = 2336;

/// FNV-1a-32 over `bytes`: the payload checksum the chain loader recomputes
/// over RAM before jumping. Must match the loader's implementation.
fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811C_9DC5;
    for &b in bytes {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Pre-analysed level-meter data for every menu song, end to end.
const SPECTRUM_FILE_NAME: &str = "SPECTRUM.BIN";

/// Magic of an analysis file (`mkdisc analyse`): the magic, a song count, one
/// u32 frame count per song, then the frames of every song end to end.
const ANALYSIS_MAGIC: &[u8; 8] = b"PSXSPEC1";

/// Cooked menu-backdrop screenshots, one after another on sector boundaries.
const SHOTS_FILE_NAME: &str = "SHOTS.BIN";

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
    /// `NAME=VERSION`, the version each program's own source declares.
    versions: Vec<(String, String)>,
    /// XA-ADPCM song files (`psx-audio-cook xa-pack`) the launcher's ISO
    /// carries for the menu to play, in file order.
    menu_xa: Vec<PathBuf>,
    /// The analysis file (`mkdisc analyse`) holding every menu song's meter.
    menu_spectrum: Option<PathBuf>,
    /// Attribution the menu prints for the music.
    credit: String,
    /// Per menu song, in song order: `(milli-BPM, first-beat ms)` from
    /// `disc-tools beatgrid`.
    menu_beats: Vec<(u32, u32)>,
    /// Per menu song, in the same order: the title the menu shows as
    /// "now playing". Their count is the number of songs.
    menu_titles: Vec<String>,
    /// Display names pressed onto the disc but held off the carousel until
    /// the cheat code reveals them.
    gates: Vec<String>,
    /// `(name, cooked blob)` menu backdrops, in slideshow order per name.
    shots: Vec<(String, PathBuf)>,
    /// Sectors the target blank holds. A layout over it is an error that names
    /// every program's share, rather than a burn that runs off the disc.
    max_sectors: Option<u32>,
    /// Zero-filled data sectors appended after the last program, so the outermost
    /// stretch of the pressing (where silicon reads are least reliable) holds nothing
    /// that matters. They count against `--max-sectors`.
    tail_filler: u32,
    /// Print the sector budget and stop before writing the image.
    dry_run: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut launcher = None;
    let mut out = None;
    let mut volume = String::from("PSXDEMO");
    let mut programs = Vec::new();
    let mut shared_cdda = Vec::new();
    let mut descriptions = Vec::new();
    let mut versions = Vec::new();
    let mut menu_xa: Vec<PathBuf> = Vec::new();
    let mut menu_spectrum: Option<PathBuf> = None;
    let mut credit = String::new();
    let mut menu_beats: Vec<(u32, u32)> = Vec::new();
    let mut menu_titles: Vec<String> = Vec::new();
    let mut gates: Vec<String> = Vec::new();
    let mut max_sectors = None;
    let mut tail_filler = 0u32;
    let mut dry_run = false;
    let mut shots: Vec<(String, PathBuf)> = Vec::new();

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
            "--version-of" => {
                let spec = it.next().ok_or("--version-of takes NAME=VERSION")?;
                let (name, version) = spec
                    .split_once('=')
                    .ok_or_else(|| format!("--version-of wants NAME=VERSION, got {spec:?}"))?;
                versions.push((name.to_string(), version.trim().to_string()));
            }
            "--gate" => gates.push(it.next().ok_or("--gate takes a display NAME".to_string())?),
            "--shot" => {
                let (name, path) = split(&it.next().ok_or("--shot takes NAME=path")?, "--shot")?;
                shots.push((name, path));
            }
            "--describe" => {
                let spec = it.next().ok_or("--describe takes NAME=ENGLISH|ITALIAN")?;
                let (name, text) = spec.split_once('=').ok_or_else(|| {
                    format!("--describe wants NAME=ENGLISH|ITALIAN, got {spec:?}")
                })?;
                let (english, italian) = text.split_once('|').ok_or_else(|| {
                    format!("--describe wants the two languages split by '|', got {text:?}")
                })?;
                descriptions.push((
                    name.to_string(),
                    english.trim().to_string(),
                    italian.trim().to_string(),
                ));
            }
            "--menu-xa" => menu_xa.push(PathBuf::from(
                it.next().ok_or("--menu-xa takes a path".to_string())?,
            )),
            "--menu-spectrum" => {
                menu_spectrum = Some(PathBuf::from(
                    it.next().ok_or("--menu-spectrum takes a path".to_string())?,
                ))
            }
            "--menu-cdda" => {
                return Err("--menu-cdda is gone: the menu songs are XA files now, give them \
                            with --menu-xa and their meter with --menu-spectrum"
                    .to_string())
            }
            "--credit" => credit = it.next().ok_or("--credit takes a string".to_string())?,
            "--menu-title" => {
                menu_titles.push(it.next().ok_or("--menu-title takes a string".to_string())?)
            }
            "--menu-beat" => {
                let spec = it.next().ok_or("--menu-beat takes MILLIBPM:PHASEMS")?;
                let (bpm, phase) = spec
                    .split_once(':')
                    .ok_or_else(|| format!("--menu-beat wants MILLIBPM:PHASEMS, got {spec:?}"))?;
                let parse = |v: &str, what: &str| {
                    v.parse::<u32>()
                        .map_err(|_| format!("--menu-beat {what} {v:?} is not a number"))
                };
                menu_beats.push((parse(bpm, "tempo")?, parse(phase, "phase")?));
            }
            "--max-sectors" => {
                let value = it.next().ok_or("--max-sectors takes a sector count")?;
                max_sectors = Some(value.parse::<u32>().map_err(|_| {
                    format!("--max-sectors wants a sector count, got {value:?}")
                })?);
            }
            "--tail-filler" => {
                let value = it.next().ok_or("--tail-filler takes a sector count")?;
                tail_filler = value.parse::<u32>().map_err(|_| {
                    format!("--tail-filler wants a sector count, got {value:?}")
                })?;
            }
            "--dry-run" => dry_run = true,
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
        versions,
        menu_xa,
        menu_spectrum,
        credit,
        menu_beats,
        menu_titles,
        gates,
        shots,
        max_sectors,
        tail_filler,
        dry_run,
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
         --gate        NAME stays off the carousel until the cheat code\n\
         --shot        NAME=<blob> adds a menu backdrop for the program; repeat\n\
        \x20             for a slideshow (cook the blob with disc-tools cook-shot)\n\
         --menu-xa     an XA song file (psx-audio-cook xa-pack) for the launcher's\n\
        \x20             ISO; repeat it for every file of the pack\n\
         --menu-spectrum  the level-meter file `mkdisc analyse` wrote for the songs\n\
         --credit      attribution the menu prints for the music\n\
         --menu-beat   MILLIBPM:PHASEMS per song, in song order, from\n\
        \x20             disc-tools beatgrid; drives the menu's beat pulse\n\
         --menu-title  title of each song, in song order, shown as now playing;\n\
        \x20             their count is the song count\n\
         --max-sectors the target blank's capacity; a layout over it is an error\n\
         --dry-run     print the sector budget and stop without writing the image"
    );
}

/// The bits of a PSX-EXE header the disc build cares about.
#[derive(Clone, Copy)]
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

/// Reject a program whose payload would be loaded on top of the chain loader.
///
/// The loader streams the payload in whole sectors and then jumps, so the
/// extent that matters is the loaded payload rounded up to sectors: `.bss` is
/// zeroed by the target's own `_start`, after the loader has stopped running.
/// This is the rule the loader applies on the console (`check_payload`), so the
/// two cannot disagree.
fn check_clear_of_loader(header: &ExeHeader, what: &Path) -> Result<(), String> {
    check_payload(header.load_addr, header.payload_bytes, &reserved_spans()).map_err(|conflict| {
        let extent = payload_extent(header.load_addr, header.payload_bytes)
            .map(|span| format!("{:#010X}..{:#010X}", span.start, span.end))
            .unwrap_or_else(|| String::from("past the address space"));
        match conflict {
            Conflict::BelowFloor => format!(
                "{}: loads at {:#010X}, below the {:#010X} floor",
                what.display(),
                header.load_addr,
                MIN_LOAD_ADDR
            ),
            Conflict::PastRam => format!(
                "{}: payload {extent} runs past the end of RAM ({:#010X})",
                what.display(),
                RAM_END
            ),
            Conflict::Reserved(span) => format!(
                "{}: payload {extent} overlaps the chain loader's {:#010X}..{:#010X}. \
                 Move the loader (chainload-layout) or shrink or relocate the program.",
                what.display(),
                span.start,
                span.end
            ),
        }
    })
}

/// One line per program for the budget output: where its payload loads and how
/// much room it leaves around the loader, so a pressing's log shows how close
/// each program is to the edge before one crosses it.
fn memory_map(rows: &[(String, ExeHeader)]) -> String {
    let [blob, _] = reserved_spans();
    let mut text = format!(
        "chain-load memory map (loader {:#010X}..{:#010X}, stack {:#010X}..{:#010X})\n",
        blob.start,
        blob.end,
        reserved_spans()[1].start,
        RAM_END
    );
    for (name, header) in rows {
        let Some(extent) = payload_extent(header.load_addr, header.payload_bytes) else {
            text.push_str(&format!("  {name:<20} (extent overflows)\n"));
            continue;
        };
        let margin = if extent.end <= blob.start {
            format!("{} B below the loader", blob.start - extent.end)
        } else if extent.start >= blob.end {
            format!("{} B above the loader", extent.start - blob.end)
        } else {
            String::from("OVERLAPS THE LOADER")
        };
        text.push_str(&format!(
            "  {name:<20} {:#010X}..{:#010X}  {margin}\n",
            extent.start, extent.end
        ));
    }
    text
}

/// Find the first valid chain-loadable PS-X EXE in an imported data track.
///
/// The old packer assumed every image used PSoXide's single-program LBA. A
/// collection has a table and cached screenshots before its launcher, so its
/// SYSTEM.CNF boot program legitimately lands later. Scanning raw user-data
/// sectors keeps both layouts relocatable without teaching the outer disc the
/// inner ISO directory structure.
fn find_image_boot_exe(data: &[u8], what: &Path) -> Result<(u32, ExeHeader), String> {
    let mut first_refusal: Option<String> = None;
    for (lba, sector) in data
        .chunks_exact(SECTOR_BYTES)
        .take(IMAGE_BOOT_SCAN_SECTORS)
        .enumerate()
    {
        let user = &sector[24..24 + SECTOR_SIZE];
        if !user.starts_with(EXE_MAGIC) {
            continue;
        }
        let Ok(header) = parse_exe_header(user, what) else {
            continue;
        };
        match check_clear_of_loader(&header, what) {
            Ok(()) => return Ok((lba as u32, header)),
            Err(reason) => first_refusal = first_refusal.or(Some(reason)),
        }
    }
    // A boot EXE that exists but cannot be loaded is the real failure; say why
    // instead of claiming there was none.
    Err(first_refusal.unwrap_or_else(|| {
        format!(
            "{}: no chain-loadable PSX-EXE in the first {IMAGE_BOOT_SCAN_SECTORS} data sectors",
            what.display()
        )
    }))
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
    let mut text =
        format!("FILE \"{bin_name}\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n");
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
/// Stamp each program with the version its own source declares.
///
/// The disc could always say which pressing it was, from `git describe` in the
/// launcher's header, but not which build of anything it carried. Eleven
/// programs come from eight repositories at eight different moments, so
/// "v0.16" answered a question nobody was asking when a single game looked
/// wrong.
fn apply_versions(
    entries: &mut [Entry],
    names: &[&str],
    versions: &[(String, String)],
) -> Result<(), String> {
    for (name, version) in versions {
        let at = names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| format!("--version-of names {name:?}, which is not on this disc"))?;
        if version.is_empty() {
            return Err(format!(
                "--version-of gives {name:?} an empty version. Omit the flag instead: a program \
                 that declares no version should not have the disc claim one."
            ));
        }
        if version.len() > disc_toc::VERSION_BYTES {
            return Err(format!(
                "{name}: version {version:?} is {} characters, {} over the {} the table holds",
                version.len(),
                version.len() - disc_toc::VERSION_BYTES,
                disc_toc::VERSION_BYTES
            ));
        }
        entries[at] = entries[at].versioned(version);
    }
    Ok(())
}

/// Mark gated programs hidden. An unknown name fails the build: a gate that
/// silently does not take would ship the one thing it existed to hold back.
fn apply_gates(entries: &mut [Entry], names: &[&str], gates: &[String]) -> Result<(), String> {
    for name in gates {
        let at = names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| format!("--gate names {name:?}, which is not on this disc"))?;
        entries[at] = entries[at].gated();
    }
    Ok(())
}

/// Point each entry at its slice of the shot region.
fn apply_shots(
    entries: &mut [Entry],
    names: &[&str],
    spans: &[(String, u8, u8)],
) -> Result<(), String> {
    for (name, first, count) in spans {
        let at = names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| format!("--shot names {name:?}, which is not on this disc"))?;
        entries[at] = entries[at].with_shots(*first, *count);
    }
    Ok(())
}

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

/// Red Book numbers tracks 1 to 99, and track 1 is the data track.
const MAX_TRACKS: u32 = 99;

/// Rows of `(name, sectors, CD-DA tracks)`, one per share of the image.
type Budget = [(String, u32, u32)];

fn budget_table(budget: &Budget, max_sectors: Option<u32>) -> String {
    let total: u32 = budget.iter().map(|(_, sectors, _)| sectors).sum();
    let tracks: u32 = 1 + budget.iter().map(|(_, _, tracks)| tracks).sum::<u32>();
    let mut out = String::from("sector budget:\n");
    for (name, sectors, audio) in budget {
        out.push_str(&format!(
            "  {name:<32} {sectors:>7} sectors  {:>6.1} MiB  {audio:>2} CD-DA\n",
            *sectors as f64 * SECTOR_BYTES as f64 / (1024.0 * 1024.0)
        ));
    }
    out.push_str(&format!(
        "  {:<32} {total:>7} sectors  {:>6.1} MiB  {tracks:>2} tracks  {}\n",
        "total",
        total as f64 * SECTOR_BYTES as f64 / (1024.0 * 1024.0),
        msf(total)
    ));
    if let Some(max) = max_sectors {
        let percent = total as f64 * 100.0 / max as f64;
        if total <= max {
            out.push_str(&format!(
                "  fits: {percent:.1}% of {max} sectors, {} spare\n",
                max - total
            ));
        } else {
            out.push_str(&format!(
                "  DOES NOT FIT: {percent:.1}% of {max} sectors, {} over\n",
                total - max
            ));
        }
    }
    out
}

fn check_budget(budget: &Budget, max_sectors: Option<u32>) -> Result<(), String> {
    let total: u32 = budget.iter().map(|(_, sectors, _)| sectors).sum();
    let tracks: u32 = 1 + budget.iter().map(|(_, _, tracks)| tracks).sum::<u32>();
    if tracks > MAX_TRACKS {
        return Err(format!(
            "{tracks} tracks is more than the {MAX_TRACKS} a CD can number; drop a program"
        ));
    }
    match max_sectors {
        Some(max) if total > max => Err(format!(
            "the layout needs {total} sectors ({}), {} more than the {max} of the target \
             blank. Drop a program or raise the target.",
            msf(total),
            total - max
        )),
        _ => Ok(()),
    }
}


/// Split an analysis file into its meter bytes and the frame count of each
/// song.
fn parse_analysis(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u32>), String> {
    if bytes.get(..8) != Some(ANALYSIS_MAGIC) {
        return Err("not a mkdisc analysis file (run `mkdisc analyse`)".into());
    }
    let word = |at: usize| -> Result<u32, String> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| "truncated".to_string())
    };
    let songs = word(8)? as usize;
    if songs > disc_toc::MAX_MENU_TRACKS {
        return Err(format!(
            "{songs} songs, the table holds {}",
            disc_toc::MAX_MENU_TRACKS
        ));
    }
    let mut frames = Vec::with_capacity(songs);
    for song in 0..songs {
        frames.push(word(12 + 4 * song)?);
    }
    let data_at = 12 + 4 * songs;
    let want = frames.iter().sum::<u32>() as usize * spectrum::BANDS;
    let data = bytes
        .get(data_at..)
        .filter(|d| d.len() == want)
        .ok_or_else(|| {
            format!(
                "the frame counts want {want} bytes of meter data, the file holds {}",
                bytes.len().saturating_sub(data_at)
            )
        })?;
    Ok((data.to_vec(), frames))
}

/// `mkdisc analyse OUT.spectrum SONG.wav...`: the level-meter animation of
/// every song, as one file `--menu-spectrum` reads. Run once when the music is
/// authored; the WAVs themselves stay out of the repository.
fn analyse_command(args: &[String]) -> Result<(), String> {
    let (out, songs) = args
        .split_first()
        .filter(|(_, songs)| !songs.is_empty())
        .ok_or("usage: mkdisc analyse OUT.spectrum SONG.wav...")?;
    if songs.len() > disc_toc::MAX_MENU_TRACKS {
        return Err(format!(
            "{} songs, the table holds {}",
            songs.len(),
            disc_toc::MAX_MENU_TRACKS
        ));
    }
    let mut frames = Vec::new();
    let mut data = Vec::new();
    for path in songs {
        let bytes = fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
        let wav = wav::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        let meter = spectrum::analyse(&wav.stereo_pcm, wav.rate);
        println!(
            "{path}: {} Hz, {} frames of meter",
            wav.rate,
            meter.len() / spectrum::BANDS
        );
        frames.push((meter.len() / spectrum::BANDS) as u32);
        data.extend_from_slice(&meter);
    }
    let mut file = Vec::new();
    file.extend_from_slice(ANALYSIS_MAGIC);
    file.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    for f in &frames {
        file.extend_from_slice(&f.to_le_bytes());
    }
    file.extend_from_slice(&data);
    fs::write(out, file).map_err(|e| format!("write {out}: {e}"))
}

fn run() -> Result<(), String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("analyse") {
        return analyse_command(&raw[1..]);
    }
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
    // The menu songs: XA files that ride in the launcher's own ISO, found by
    // the launcher through the root directory by name.
    let mut menu_xa: Vec<(String, Vec<u8>)> = Vec::new();
    for path in &args.menu_xa {
        let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if bytes.is_empty() || bytes.len() % XA_SECTOR_BYTES != 0 {
            return Err(format!(
                "{}: {} bytes is not a whole number of {XA_SECTOR_BYTES}-byte XA sectors; \
                 give it a file psx-audio-cook xa-pack wrote",
                path.display(),
                bytes.len()
            ));
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{}: no file name", path.display()))?
            .to_ascii_uppercase();
        menu_xa.push((name, bytes));
    }
    let song_count = args.menu_titles.len();
    if song_count > disc_toc::MAX_MENU_TRACKS {
        return Err(format!(
            "{song_count} --menu-title against the {} songs the table holds",
            disc_toc::MAX_MENU_TRACKS
        ));
    }
    if menu_xa.is_empty() != (song_count == 0) {
        return Err(
            "--menu-xa and --menu-title come together: the files hold the songs, the titles \
             name them"
                .to_string(),
        );
    }
    // The meter data was analysed when the music was authored (the source
    // WAVs are not in the repository), so it travels as a file of its own.
    let (spectrum_data, spectrum_frames) = match &args.menu_spectrum {
        Some(path) => {
            let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
            parse_analysis(&bytes).map_err(|e| format!("{}: {e}", path.display()))?
        }
        None => (Vec::new(), Vec::new()),
    };
    if !spectrum_frames.is_empty() && spectrum_frames.len() != song_count {
        return Err(format!(
            "{} has {} songs but there are {song_count} --menu-title",
            args.menu_spectrum.as_ref().unwrap().display(),
            spectrum_frames.len()
        ));
    }

    // Screenshots, cooked by disc-tools cook-shot. Grouped by program (in
    // first-appearance order, keeping each program's own slideshow order) and
    // padded to sector boundaries, so the launcher addresses shot `i` as
    // `shots_lba + i * SHOT_SECTORS` with nothing but the table in hand.
    let mut shot_groups: Vec<(String, Vec<Vec<u8>>)> = Vec::new();
    for (name, path) in &args.shots {
        let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if bytes.len() != disc_toc::SHOT_BYTES {
            return Err(format!(
                "--shot {}: {} bytes, but a cooked shot is exactly {} (CLUT then {}x{} \
                 pixels). Cook it with disc-tools cook-shot.",
                path.display(),
                bytes.len(),
                disc_toc::SHOT_BYTES,
                disc_toc::SHOT_W,
                disc_toc::SHOT_H
            ));
        }
        match shot_groups.iter_mut().find(|(n, _)| n == name) {
            Some((_, group)) => group.push(bytes),
            None => shot_groups.push((name.clone(), vec![bytes])),
        }
    }
    let total_shots: usize = shot_groups.iter().map(|(_, g)| g.len()).sum();
    if total_shots > disc_toc::MAX_SHOTS {
        return Err(format!(
            "{total_shots} --shot blobs is more than the {} the launcher's RAM cache holds",
            disc_toc::MAX_SHOTS
        ));
    }
    let mut shots_bin = Vec::new();
    let mut shot_spans: Vec<(String, u8, u8)> = Vec::new();
    for (name, group) in &shot_groups {
        let first = (shots_bin.len() / (disc_toc::SHOT_SECTORS as usize * SECTOR_SIZE)) as u8;
        shot_spans.push((name.clone(), first, group.len() as u8));
        for shot in group {
            shots_bin.extend_from_slice(shot);
            shots_bin.resize(shots_bin.len().next_multiple_of(SECTOR_SIZE), 0);
        }
    }

    let spectrum_lba = toc_lba + disc_toc::TOC_SECTORS;
    let spectrum_sectors = sectors_for(spectrum_data.len());
    let shots_lba = spectrum_lba + spectrum_sectors;
    let shots_sectors = sectors_for(shots_bin.len());
    let mut next_lba = shots_lba + shots_sectors + sectors_for(launcher.len());
    let mut entries: Vec<Option<Entry>> = vec![None; args.programs.len()];
    let mut iso_files = Vec::new();
    let mut map = Vec::new();

    let mut loaded: Vec<(String, ExeHeader)> = Vec::new();
    for (index, program) in args.programs.iter().enumerate() {
        let Source::Exe(path) = &program.source else {
            continue;
        };
        let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let header = parse_exe_header(&bytes, path)?;
        check_clear_of_loader(&header, path)?;
        loaded.push((program.name.clone(), header));
        let payload_end = 2048usize + header.payload_bytes as usize;
        if bytes.len() < payload_end {
            return Err(format!(
                "{}: header claims {} payload bytes but the file holds {}",
                path.display(),
                header.payload_bytes,
                bytes.len().saturating_sub(2048)
            ));
        }
        let fnv = fnv1a32(&bytes[2048..payload_end]);
        entries[index] = Some(Entry::new(&program.name, next_lba, 0, 0).with_payload_fnv(fnv));
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
        let (boot_exe_lba, header) = find_image_boot_exe(&image.data, path)?;
        check_clear_of_loader(&header, path)?;
        loaded.push((program.name.clone(), header));
        // Checksum the payload as the loader will read it: the 2048-byte
        // data windows of the raw sectors after the header sector.
        let mut remaining = header.payload_bytes as usize;
        let mut hash: u32 = 0x811C_9DC5;
        let mut sector = boot_exe_lba as usize + 1;
        while remaining > 0 {
            let at = sector * SECTOR_BYTES + 24;
            let take = remaining.min(SECTOR_SIZE);
            let data = image.data.get(at..at + take).ok_or_else(|| {
                format!("{}: image ends inside its boot EXE payload", path.display())
            })?;
            for &b in data {
                hash ^= b as u32;
                hash = hash.wrapping_mul(0x0100_0193);
            }
            remaining -= take;
            sector += 1;
        }
        images.push((index, path.clone(), image, hash, boot_exe_lba));
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
        if !shots_bin.is_empty() {
            builder.add_file(SHOTS_FILE_NAME, shots_bin.clone());
        }
        builder.add_file("PSX.EXE", launcher.clone());
        for (name, bytes) in &iso_files {
            builder.add_file(name, bytes.clone());
        }
        for (name, bytes) in &menu_xa {
            builder
                .add_xa_file(name, bytes.clone())
                .expect("lengths were checked when the files were read");
        }
        builder.build_bin()
    };

    // Sizing pass: the ISO's length fixes where the first image lands, and the
    // table has to name that. The table is exactly one sector either way, so
    // the second build comes out the same length.
    let iso_frames = (build_iso(vec![0u8; disc_toc::TOC_BYTES]).len() / SECTOR_BYTES) as u32;

    // Every sector the finished image will hold is known now, so the budget is
    // settled before hundreds of megabytes get written: an optional program
    // that does not fit fails here with each share named, not at the burner.
    let mut budget = vec![(
        String::from("launcher ISO (menu, bare EXEs, XA)"),
        iso_frames,
        0u32,
    )];
    for (index, _, image, _, _) in &images {
        budget.push((
            args.programs[*index].name.clone(),
            ((image.data.len() + image.audio_bytes.len()) / SECTOR_BYTES) as u32,
            image.audio.len() as u32,
        ));
    }
    if args.tail_filler != 0 {
        budget.push((String::from("tail filler (outer edge)"), args.tail_filler, 0));
    }
    print!("{}", memory_map(&loaded));
    print!("{}", budget_table(&budget, args.max_sectors));
    check_budget(&budget, args.max_sectors)?;
    if args.dry_run {
        return Ok(());
    }

    let mut image_lba = iso_frames;
    // The menu music is XA inside the launcher's ISO and owns no CD-DA track,
    // so the first game's audio is track 2 and every base counts only the
    // games' own tracks. Nothing on the disc reads a track by a fixed number
    // any more: the old "menu songs are tracks 2-5" convention is gone.
    let mut cdda_track_base = 0u32;
    for (index, path, image, payload_fnv, boot_exe_lba) in &images {
        let frames = (image.data.len() / SECTOR_BYTES) as u32;
        let program = &args.programs[*index];
        entries[*index] = Some(
            Entry::new(
                &program.name,
                image_lba + *boot_exe_lba,
                image_lba,
                cdda_track_base,
            )
            .with_payload_fnv(*payload_fnv),
        );
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
    apply_versions(&mut entries, &names, &args.versions)?;
    apply_gates(&mut entries, &names, &args.gates)?;
    apply_shots(&mut entries, &names, &shot_spans)?;
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
        if given != 0 && given != song_count {
            return Err(format!(
                "{given} {flag} against {song_count} songs: they pair up in order, so give one \
                 per song or none at all"
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
    if song_count != 0 && args.credit.is_empty() {
        return Err(
            "--menu-xa without --credit: the menu has nowhere to attribute the \
                    music, which is the one thing an attribution licence asks for"
                .to_string(),
        );
    }

    // The table's first field says which CD-DA track the menu plays; the menu
    // has none any more, so it is 0 and the song count is what the launcher
    // reads. (A table with its own song section is the pool design in the XA
    // migration plan; this reuses the fields that already exist.)
    let toc = disc_toc::encode(
        &entries,
        0,
        song_count as u32,
        &args.credit,
        &args.menu_beats,
        &args
            .menu_titles
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        if spectrum_data.is_empty() {
            0
        } else {
            spectrum_lba
        },
        &spectrum_frames,
        if shots_bin.is_empty() { 0 } else { shots_lba },
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
    for (_, _, image, _, _) in &images {
        place_data_track(&mut disc, &image.data, lba)?;
        lba += (image.data.len() / SECTOR_BYTES) as u32;
    }

    // The tail filler: a zero-data Form 1 sector (the ISO's own empty system-area sector 1,
    // already encoded with its EDC/ECC) repeated, re-addressed like every placed sector.
    if args.tail_filler != 0 {
        let template: Vec<u8> = disc[SECTOR_BYTES..2 * SECTOR_BYTES].to_vec();
        if template[0x18..0x818].iter().any(|b| *b != 0) {
            return Err("the ISO's system-area sector 1 is not empty: no filler template".to_string());
        }
        for _ in 0..args.tail_filler {
            let at = disc.len();
            disc.extend_from_slice(&template);
            rewrite_sector_msf(&mut disc[at..at + SECTOR_BYTES], (at / SECTOR_BYTES) as u32);
        }
    }

    // All CD-DA follows all data, each game's in program order.
    let mut placed_audio = Vec::new();
    for (_, _, image, _, _) in &images {
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
    if !spectrum_data.is_empty() {
        println!(
            "level meter: {} frames over {} track(s) at LBA {spectrum_lba} ({} KiB)",
            spectrum_frames.iter().sum::<u32>(),
            spectrum_frames.len(),
            spectrum_data.len() / 1024
        );
    }
    if !shots_bin.is_empty() {
        println!(
            "backdrops: {total_shots} shot(s) over {} program(s) at LBA {shots_lba} ({} KiB)",
            shot_spans.len(),
            shots_bin.len() / 1024
        );
    }
    if song_count != 0 {
        println!(
            "menu music: {song_count} song(s) in {} XA file(s), credited as {:?}",
            menu_xa.len(),
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

    fn row(name: &str, sectors: u32, tracks: u32) -> (String, u32, u32) {
        (name.to_string(), sectors, tracks)
    }

    #[test]
    fn a_layout_within_the_target_passes_the_budget() {
        let budget = [row("launcher", 100, 0), row("GAME", 899, 2)];
        assert!(check_budget(&budget, Some(1000)).is_ok());
        assert!(check_budget(&budget, Some(999)).is_ok());
        assert!(check_budget(&budget, None).is_ok());
        assert!(budget_table(&budget, Some(1000)).contains("1 spare"));
    }

    #[test]
    fn a_layout_over_the_target_names_the_overrun() {
        let budget = [row("launcher", 100, 0), row("GAME", 900, 2)];
        let error = check_budget(&budget, Some(999)).unwrap_err();
        assert!(error.contains("1000 sectors"), "{error}");
        assert!(error.contains("1 more than the 999"), "{error}");
        assert!(budget_table(&budget, Some(999)).contains("DOES NOT FIT"));
    }

    #[test]
    fn more_than_ninety_nine_tracks_is_an_error() {
        let fits = [row("launcher", 1, 0), row("GAME", 1, 98)];
        assert!(check_budget(&fits, None).is_ok());
        let over = [row("launcher", 1, 0), row("GAME", 1, 99)];
        assert!(check_budget(&over, None).unwrap_err().contains("100 tracks"));
    }

    #[test]
    fn accepts_a_program_that_clears_the_blob() {
        let h = header(0x8001_0000, 1_500_000);
        assert!(check_clear_of_loader(&h, Path::new("cortex")).is_ok());
    }

    #[test]
    fn accepts_the_half_life_intro_above_the_blob() {
        // 75,776 bytes at 0x801E0000, ending 0x801F2800: past the old blob
        // address, which is why it could not be pressed.
        let h = header(0x801E_0000, 75_776);
        assert!(check_clear_of_loader(&h, Path::new("half-life")).is_ok());
    }

    #[test]
    fn rejects_a_program_that_would_overwrite_the_blob() {
        let blob = chainload_layout::LOADER_BASE;
        let h = header(0x8001_0000, blob - 0x8001_0000 + 1);
        let err = check_clear_of_loader(&h, Path::new("huge")).unwrap_err();
        assert!(err.contains("overlaps the chain loader"), "{err}");
    }

    #[test]
    fn rejects_a_program_that_would_reach_the_stack() {
        let h = header(0x801E_0000, 0x1_9000);
        let err = check_clear_of_loader(&h, Path::new("tall")).unwrap_err();
        assert!(err.contains("overlaps the chain loader"), "{err}");
    }

    #[test]
    fn the_checked_extent_is_whole_sectors() {
        let blob = chainload_layout::LOADER_BASE;
        // One byte over a sector boundary writes the next whole sector.
        let h = header(blob - 2048, 2049);
        assert!(check_clear_of_loader(&h, Path::new("edge")).is_err());
        let h = header(blob - 2048, 2048);
        assert!(check_clear_of_loader(&h, Path::new("edge")).is_ok());
    }

    /// The boot EXE of every program on the v0.47 Private pressing plus the
    /// current Half-Life intro: `(name, load address, payload bytes)`, read from
    /// the pressed images. A change to `chainload-layout` that would strand any
    /// of them fails here instead of at the next pressing.
    const PRESSED_PROGRAMS: [(&str, u32, u32); 12] = [
        ("PSOXIDE ARCADE", 0x8001_0000, 110_592),
        ("CELESTE COLLECTION", 0x8001_0000, 587_776),
        ("NITROXIDE", 0x8001_0000, 1_363_968),
        ("VOXIDE", 0x8001_0000, 733_184),
        ("WIPEOUT PSOXIDE", 0x8001_0000, 546_816),
        ("HOLLOW KNIGHT", 0x8001_0000, 1_024_000),
        ("COUNTER-STRIKE", 0x8001_0000, 604_160),
        ("HALF-LIFE", 0x801E_0000, 75_776),
        ("OCARINA OF TIME", 0x8001_0000, 1_728_512),
        ("QUAKE SHAREWARE", 0x8001_0000, 559_104),
        ("CORTEX IGNITION", 0x8001_0000, 1_134_592),
        ("PSXCEL", 0x8001_0000, 260_096),
    ];

    #[test]
    fn every_pressed_program_clears_the_loader_and_the_stack() {
        for (name, load_addr, payload_bytes) in PRESSED_PROGRAMS {
            let h = header(load_addr, payload_bytes);
            check_clear_of_loader(&h, Path::new(name)).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn the_memory_map_names_each_program_and_flags_an_overlap() {
        let rows = vec![
            (String::from("CORTEX"), header(0x8001_0000, 0x0011_5000)),
            (String::from("HALF-LIFE"), header(0x801E_0000, 75_776)),
            (String::from("BAD"), header(0x801C_8000, 2048)),
        ];
        let map = memory_map(&rows);
        assert!(map.contains("CORTEX"), "{map}");
        assert!(map.contains("B below the loader"), "{map}");
        assert!(map.contains("B above the loader"), "{map}");
        assert!(map.contains("OVERLAPS THE LOADER"), "{map}");
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
    fn finds_a_collection_launcher_after_cached_disc_data() {
        let boot_lba = 136usize;
        let mut image = vec![0u8; (boot_lba + 2) * SECTOR_BYTES];
        let at = boot_lba * SECTOR_BYTES + 24;
        image[at..at + 8].copy_from_slice(EXE_MAGIC);
        image[at + 0x18..at + 0x1C].copy_from_slice(&0x8001_0000u32.to_le_bytes());
        image[at + 0x1C..at + 0x20].copy_from_slice(&4096u32.to_le_bytes());
        let (found, header) = find_image_boot_exe(&image, Path::new("arcade")).expect("boot");
        assert_eq!(found, boot_lba as u32);
        assert_eq!(header.load_addr, 0x8001_0000);
        assert_eq!(header.payload_bytes, 4096);
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
            assert_eq!(
                sector[0x0C..0x0F],
                expect[0x0C..0x0F],
                "sector {i} re-addressed"
            );
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
        apply_shared_cdda(&mut entries, &names, &[("PONG".into(), "GH-PSX".into())])
            .expect("both names exist");
        assert_eq!(entries[2].cdda_track_base, entries[0].cdda_track_base);
        assert_eq!(entries[1].cdda_track_base, 1, "others untouched");
    }

    #[test]
    fn sharing_with_a_program_that_is_not_on_the_disc_is_an_error() {
        let mut entries = [Entry::new("PONG", 0, 0, 0)];
        let err = apply_shared_cdda(&mut entries, &["PONG"], &[("PONG".into(), "GH-PSX".into())])
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
            &[(
                "VOXIDE".into(),
                "Voxel sandbox".into(),
                "Sandbox a voxel".into(),
            )],
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

    #[test]
    fn an_analysis_file_round_trips_its_songs() {
        let mut file = Vec::new();
        file.extend_from_slice(ANALYSIS_MAGIC);
        file.extend_from_slice(&2u32.to_le_bytes());
        file.extend_from_slice(&1u32.to_le_bytes());
        file.extend_from_slice(&2u32.to_le_bytes());
        file.extend(std::iter::repeat_n(7u8, 3 * spectrum::BANDS));
        let (data, frames) = parse_analysis(&file).expect("valid");
        assert_eq!(frames, vec![1, 2]);
        assert_eq!(data.len(), 3 * spectrum::BANDS);
    }

    #[test]
    fn an_analysis_file_that_disagrees_with_itself_is_an_error() {
        let mut file = Vec::new();
        file.extend_from_slice(ANALYSIS_MAGIC);
        file.extend_from_slice(&1u32.to_le_bytes());
        file.extend_from_slice(&5u32.to_le_bytes());
        file.extend(std::iter::repeat_n(0u8, 3));
        assert!(parse_analysis(&file).unwrap_err().contains("want"));
        assert!(parse_analysis(b"nonsense").is_err());
    }
}
