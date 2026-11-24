//! `disc-tools members`: what the disc gate needs to know about each program on a pressed image,
//! written as `members.json` beside the image, with each program's link map copied under
//! `maps/`.
//!
//! The order is the disc table's, which is the order of the launcher's shelf, so a member's
//! `index` is the shelf position the journeys press Right to reach. For each member the file
//! records the executable that runs it (`exe_sha256`, `payload_sha256`, load and entry address),
//! the link map's name, the address of its VBlank counter, four words of its text that no other
//! program on the disc (nor the launcher) holds at that address, and, for programs that publish
//! one, the title state the lineup row names.
//!
//! Everything comes from the pressed image, its release receipt and the lineup file; nothing is
//! named here by hand:
//!
//! * the image range of a member is its receipt's `embedded` record;
//! * the executable whose map is kept is the one the member's build record names as the map's
//!   own (`artifacts.exe`), which must be inside the member's image range: on a disc whose boot
//!   program is not the game (a scene core's boot scene), the table's executable is the boot
//!   one. A record without it keeps the older rule: the table's, unless the record names
//!   executables (`files` entries ending in `.exe`) and the table's is not one of them, in which
//!   case it is the executable inside the member's image range that is (Half-Life's intro hands
//!   over to a second executable, the game);
//! * the map is the one the member's build record (the row's `receipt`, a provenance file) names
//!   under `artifacts.map.file`, looked up beside the record and then in each `--library`
//!   directory, and checked against the record's hash;
//! * a title probe is the row's optional `title_probe`: `{"sym": .., "op": "ge"|"eq", "value": ..}`.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::args::Args;
use crate::disc::{self, Exe, FLAG_HIDDEN, PSX_EXE_MAGIC};
use crate::util::{
    dumps, expand_home, read_json, sha256_bytes, sha256_file, write_atomic, Error, Result,
};

/// Words of text recorded for each member.
const WORDS: usize = 4;
/// Words are drawn from this many sample points of the payload, after the first 64 words.
const SAMPLES: usize = 97;
/// The first words of a payload (startup code, shared by every program) are never sampled.
const SKIP_WORDS: usize = 64;

/// One PS-X EXE found on the disc, with where it starts.
struct Found {
    lba: u64,
    exe: Exe,
}

impl Found {
    /// The executable as a file: header sector plus payload.
    fn whole_sha(&self) -> String {
        let mut whole = self.exe.header.clone();
        whole.extend_from_slice(&self.exe.payload);
        sha256_bytes(&whole)
    }

    /// The payload word the executable holds at `addr`, if it covers it.
    fn word_at(&self, addr: u32) -> Option<u32> {
        let offset = addr.checked_sub(self.exe.load)? as usize;
        let at = self.exe.payload.get(offset..offset + 4)?;
        Some(u32::from_le_bytes([at[0], at[1], at[2], at[3]]))
    }
}

/// Every PS-X EXE that starts a sector in `lba..end`.
fn executables_in(image: &Path, lba: u64, end: u64) -> Result<Vec<Found>> {
    let mut stream = File::open(image).map_err(|e| Error(format!("{}: {e}", image.display())))?;
    let mut found = Vec::new();
    for at in lba..end {
        if &disc::read_user_sector(&mut stream, at)?[..8] == PSX_EXE_MAGIC {
            found.push(Found {
                lba: at,
                exe: disc::parse_exe_at(image, at)?,
            });
        }
    }
    Ok(found)
}

/// A member's lineup row, receipt record and build record, as far as this tool reads them.
struct Source<'a> {
    row: &'a Value,
    receipt: &'a Value,
    provenance: Value,
    provenance_dir: PathBuf,
}

/// The first sector past a member's image, and the first of it: an image program's receipt
/// records the range, a bare executable occupies its header sector and payload.
fn lba_range(receipt: &Value, entry: &disc::TocEntry) -> Result<(u64, u64)> {
    let embedded = &receipt["embedded"];
    match (
        embedded["image_lba_start"].as_u64(),
        embedded["image_lba_end_exclusive"].as_u64(),
    ) {
        (Some(start), Some(end)) => Ok((start, end)),
        _ => Ok((entry.exe_lba as u64, entry.exe_lba as u64 + 1)),
    }
}

/// `path` with its release-inputs prefix replaced by `inputs`, when it is not there as written.
fn under_inputs(path: &str, inputs: Option<&Path>) -> PathBuf {
    let plain = expand_home(path);
    if plain.is_file() {
        return plain;
    }
    let Some(root) = inputs else {
        return plain;
    };
    let parts: Vec<_> = plain.components().collect();
    match parts
        .iter()
        .position(|part| part.as_os_str() == "release-inputs")
    {
        Some(at) => parts[at + 1..]
            .iter()
            .fold(root.to_path_buf(), |dir, part| dir.join(part)),
        None => plain,
    }
}

/// The member's build record: the row's `receipt` path, as a provenance file.
fn provenance_of(
    row: &Value,
    inputs: Option<&Path>,
    library: &[PathBuf],
) -> Result<(Value, PathBuf)> {
    let name = row["name"].as_str().unwrap_or("?");
    let Some(text) = row["receipt"]
        .as_str()
        .filter(|t| t.starts_with(['~', '/']))
    else {
        bail!("{name}: the lineup row has no receipt path (its build record)");
    };
    let mut path = under_inputs(text, inputs);
    if !path.is_file() {
        let file = path.file_name().map(PathBuf::from).unwrap_or_default();
        match library
            .iter()
            .map(|dir| dir.join(&file))
            .find(|p| p.is_file())
        {
            Some(found) => path = found,
            None => bail!("{name}: build record {} not found", path.display()),
        }
    }
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    Ok((read_json(&path)?, dir))
}

/// The link map the build record names: its file and the directory it was found in.
fn locate_map(name: &str, source: &Source, library: &[PathBuf]) -> Result<PathBuf> {
    let map = &source.provenance["artifacts"]["map"];
    let Some(file) = map["file"].as_str() else {
        bail!("{name}: the build record names no map (artifacts.map.file)");
    };
    let mut roots = vec![source.provenance_dir.clone()];
    roots.extend(library.iter().cloned());
    let Some(path) = roots.iter().map(|dir| dir.join(file)).find(|p| p.is_file()) else {
        bail!(
            "{name}: map {file:?} not found beside its build record or in --library \
             (copy the install's .map next to the provenance file)"
        );
    };
    if let Some(expected) = map["sha256"].as_str() {
        let actual = sha256_file(&path)?;
        ensure!(
            actual == expected,
            "{name}: {} hashes to {actual}, its build record says {expected}",
            path.display()
        );
    }
    Ok(path)
}

/// The address of `symbol` in a GNU-style link map line:
/// `<address> <size> <align> <section index> <name>`.
fn symbol_address(map: &str, symbol: &str) -> Option<String> {
    let pattern = format!(
        r"(?m)^\s*([0-9a-f]{{8}})\s+[0-9a-f]{{8}}\s+[0-9a-f]+\s+\d+\s+{}$",
        regex::escape(symbol)
    );
    let found = Regex::new(&pattern).ok()?.captures(map)?;
    Some(format!("0x{}", &found[1]))
}

/// The title state a row publishes: its symbol's address in the map, the comparison and value.
fn title_probe(name: &str, row: &Value, map: &str) -> Result<Value> {
    let Some(probe) = row.get("title_probe") else {
        return Ok(Value::Null);
    };
    let (Some(symbol), Some(op), Some(value)) = (
        probe["sym"].as_str(),
        probe["op"].as_str().filter(|op| matches!(*op, "ge" | "eq")),
        probe["value"].as_u64(),
    ) else {
        bail!("{name}: title_probe needs sym, op (ge or eq) and value");
    };
    let Some(addr) = symbol_address(map, symbol) else {
        bail!("{name}: title_probe symbol {symbol} is not in the map");
    };
    Ok(json!({"sym": symbol, "addr": addr, "op": op, "value": value}))
}

/// Up to four words of `exe`'s text that differ from what the launcher and every other
/// executable on the disc hold at the same address (zero words are never used, nor a word
/// already taken).
fn signature_words(exe: &Found, launcher: &Exe, others: &[Found]) -> Vec<(u32, u32)> {
    let launcher = Found {
        lba: u64::MAX,
        exe: launcher.clone(),
    };
    let count = exe.exe.payload.len() / 4;
    let step = (count / SAMPLES).max(1);
    let mut words: Vec<(u32, u32)> = Vec::new();
    for k in (SKIP_WORDS..count).step_by(step) {
        let addr = exe.exe.load + 4 * k as u32;
        let Some(word) = exe.word_at(addr) else {
            continue;
        };
        let taken = launcher.word_at(addr) == Some(word)
            || others
                .iter()
                .any(|other| other.lba != exe.lba && other.word_at(addr) == Some(word));
        if word != 0 && !taken && words.iter().all(|(_, w)| *w != word) {
            words.push((addr, word));
        }
        if words.len() == WORDS {
            break;
        }
    }
    words
}

/// The record of every member of `image`, in shelf order, plus the maps to copy: `(source, name)`.
fn members(
    image: &Path,
    receipt: &Value,
    lineup: &Value,
    inputs: Option<&Path>,
    library: &[PathBuf],
) -> Result<(Vec<Value>, Vec<(PathBuf, String)>)> {
    let entries: Vec<_> = disc::read_toc(image)?
        .into_iter()
        .filter(|e| e.flags & FLAG_HIDDEN == 0)
        .collect();
    let rows = lineup["programs"]
        .as_array()
        .ok_or_else(|| Error("lineup: 'programs'".into()))?;
    let mut sources = Vec::new();
    for entry in &entries {
        let row = rows
            .iter()
            .find(|r| r["name"].as_str() == Some(&entry.name))
            .ok_or_else(|| {
                Error(format!(
                    "{} is on the disc but not in the lineup",
                    entry.name
                ))
            })?;
        let program = &receipt["programs"][&entry.name];
        ensure!(
            program.is_object(),
            "{} is on the disc but not in the release receipt",
            entry.name
        );
        let (provenance, provenance_dir) = provenance_of(row, inputs, library)?;
        sources.push(Source {
            row,
            receipt: program,
            provenance,
            provenance_dir,
        });
    }

    // Every executable inside a member's range, for the words no other program may hold.
    let mut others = Vec::new();
    let mut ranges = Vec::new();
    for (entry, source) in entries.iter().zip(&sources) {
        let (start, end) = lba_range(source.receipt, entry)?;
        let found = executables_in(image, start, end)?;
        ranges.push((start, end));
        others.extend(found);
    }

    // The launcher's executable is the first one on the disc, ahead of every member's range.
    let first_member = ranges.iter().map(|(start, _)| *start).min().unwrap_or(0);
    let launcher = executables_in(image, 0, first_member)?
        .into_iter()
        .next()
        .ok_or_else(|| Error(format!("no launcher executable before LBA {first_member}")))?
        .exe;

    let mut out = Vec::new();
    let mut copies = Vec::new();
    let mut taken_maps = BTreeSet::new();
    for (index, (entry, source)) in entries.iter().zip(&sources).enumerate() {
        let name = entry.name.as_str();
        let listed: BTreeSet<&str> = source.provenance["files"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(file, _)| file.ends_with(".exe"))
            .filter_map(|(_, sha)| sha.as_str())
            .collect();
        let table_exe = others
            .iter()
            .find(|f| f.lba == entry.exe_lba as u64)
            .ok_or_else(|| Error(format!("{name}: no executable at LBA {}", entry.exe_lba)))?;
        let named = source.provenance["artifacts"]["exe"]["sha256"].as_str();
        let chosen = if let Some(sha) = named {
            let (start, end) = ranges[index];
            others
                .iter()
                .filter(|f| (f.lba >= start && f.lba < end) || f.lba == table_exe.lba)
                .find(|f| f.whole_sha() == sha)
                .ok_or_else(|| {
                    Error(format!(
                        "{name}: the build record names executable {} (artifacts.exe), which is \
                         not on the image in the member's range",
                        &sha[..sha.len().min(12)]
                    ))
                })?
        } else if listed.is_empty() || listed.contains(table_exe.whole_sha().as_str()) {
            table_exe
        } else {
            let (start, end) = ranges[index];
            others
                .iter()
                .filter(|f| f.lba >= start && f.lba < end && f.lba != table_exe.lba)
                .find(|f| listed.contains(f.whole_sha().as_str()))
                .unwrap_or(table_exe)
        };

        let map_path = locate_map(name, source, library)?;
        let map_name = map_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error(format!("{name}: bad map path")))?
            .to_string();
        let map_text = String::from_utf8_lossy(&fs::read(&map_path)?).into_owned();
        let Some(vblank) = symbol_address(&map_text, "__psx_rt_vblank_count") else {
            bail!(
                "{name}: __psx_rt_vblank_count is not in {}",
                map_path.display()
            );
        };
        let words = signature_words(chosen, &launcher, &others);
        ensure!(
            words.len() == WORDS,
            "{name}: only {} signature words differ from every other executable",
            words.len()
        );
        if taken_maps.insert(map_name.clone()) {
            copies.push((map_path, map_name.clone()));
        }
        let mut record = Map::new();
        record.insert(
            "title_probe".into(),
            title_probe(name, source.row, &map_text)?,
        );
        record.insert("index".into(), json!(index));
        record.insert("others_checked".into(), json!(others.len() - 1));
        record.insert("id".into(), json!(name));
        record.insert("exe_lba".into(), json!(entry.exe_lba));
        record.insert(
            "payload_sha256".into(),
            json!(sha256_bytes(&chosen.exe.payload)),
        );
        record.insert("exe_sha256".into(), json!(chosen.whole_sha()));
        record.insert("map".into(), json!(format!("maps/{map_name}")));
        record.insert("load".into(), json!(format!("{:#x}", chosen.exe.load)));
        record.insert("pc".into(), json!(format!("{:#x}", chosen.exe.pc)));
        record.insert("vblank".into(), json!(vblank));
        record.insert(
            "words".into(),
            Value::Array(
                words
                    .iter()
                    .map(|(a, w)| json!([format!("{a:#x}"), format!("{w:#x}")]))
                    .collect(),
            ),
        );
        record.insert("bin_sha256".into(), source.row["bin_sha256"].clone());
        out.push(Value::Object(record));
    }
    Ok((out, copies))
}

/// The ready state an edition section names (`ready = SYMBOL ge|eq VALUE`), resolved in `map`.
fn ready_probe(name: &str, text: &str, map: &str) -> Result<Value> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [symbol, op, value] = parts[..] else {
        bail!("[{name}] ready wants `SYMBOL ge|eq VALUE`, got {text:?}");
    };
    ensure!(
        matches!(op, "ge" | "eq"),
        "[{name}] ready: op {op} is not ge or eq"
    );
    let value = match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => value.parse(),
    }
    .map_err(|_| Error(format!("[{name}] ready: {value} is not a number")))?;
    let Some(addr) = symbol_address(map, symbol) else {
        bail!("[{name}] ready symbol {symbol} is not in its map");
    };
    Ok(json!({"sym": symbol, "addr": addr, "op": op, "value": value}))
}

/// The members of a scene disc (`SCENES.TBL` at LBA 22): the ranges from the pressing's
/// `<image>.layout.json`, the boot exe at each range's LBA 23, its map, ready state and presses
/// from the edition. The ready state is the program's first playable screen; `title_probe`
/// carries it too, so the journeys that read that field keep working.
fn scene_members(
    image: &Path,
    entries: &[crate::edition::Entry],
) -> Result<(Vec<Value>, Vec<(PathBuf, String)>)> {
    let layout = read_json(&image.with_extension("layout.json"))?;
    let rows = layout["rows"]
        .as_array()
        .ok_or_else(|| Error("layout.json: 'rows'".into()))?;
    let toc: Vec<_> = disc::read_toc(image)?
        .into_iter()
        .filter(|e| e.flags & FLAG_HIDDEN == 0)
        .collect();
    let mut others = Vec::new();
    let mut ranges = Vec::new();
    for entry in &toc {
        let row = rows
            .iter()
            .find(|r| r["name"].as_str() == Some(&entry.name))
            .ok_or_else(|| Error(format!("{} is on the disc but not in its layout", entry.name)))?;
        let (Some(start), Some(end)) = (
            row["lba_start"].as_u64(),
            row["lba_end_exclusive"].as_u64(),
        ) else {
            bail!("{}: layout row without its range", entry.name);
        };
        others.extend(executables_in(image, start, end)?);
        ranges.push((start, end));
    }
    let first_member = ranges.iter().map(|(start, _)| *start).min().unwrap_or(0);
    let launcher = executables_in(image, 0, first_member)?
        .into_iter()
        .next()
        .ok_or_else(|| Error(format!("no launcher executable before LBA {first_member}")))?
        .exe;
    let mut out = Vec::new();
    let mut copies = Vec::new();
    for (index, (entry, (start, _))) in toc.iter().zip(&ranges).enumerate() {
        let name = entry.name.as_str();
        let section = entries
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| Error(format!("{name} is on the disc but not in the edition")))?;
        // The program the core enters: the member's boot exe at its own LBA 23.
        let boot = start + 23;
        ensure!(
            entry.exe_lba as u64 == boot,
            "{name}: the table of contents says LBA {}, the scene layout {boot}",
            entry.exe_lba
        );
        let chosen = others
            .iter()
            .find(|f| f.lba == boot)
            .ok_or_else(|| Error(format!("{name}: no executable at LBA {boot}")))?;
        let map_path = PathBuf::from(
            section
                .get("map")
                .ok_or_else(|| Error(format!("[{name}] needs `map =` (its boot exe's link map)")))?,
        );
        let map_text = String::from_utf8_lossy(&fs::read(&map_path)?).into_owned();
        let map_name = map_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error(format!("{name}: bad map path")))?
            .to_string();
        let Some(vblank) = symbol_address(&map_text, "__psx_rt_vblank_count") else {
            bail!("{name}: __psx_rt_vblank_count is not in {}", map_path.display());
        };
        let words = signature_words(chosen, &launcher, &others);
        ensure!(
            words.len() == WORDS,
            "{name}: only {} signature words differ from every other executable",
            words.len()
        );
        let ready = match section.get("ready") {
            Some(text) => ready_probe(name, text, &map_text)?,
            None => Value::Null,
        };
        copies.push((map_path.clone(), map_name.clone()));
        let image_path = section.get("image").map(PathBuf::from);
        let bin_sha = match image_path {
            Some(cue) => json!(sha256_file(&disc::image_for_cue(&cue)?)?),
            None => Value::Null,
        };
        let mut record = Map::new();
        record.insert("title_probe".into(), ready.clone());
        record.insert("ready".into(), ready);
        record.insert("press".into(), json!(section.get("press").unwrap_or("")));
        record.insert("index".into(), json!(index));
        record.insert("others_checked".into(), json!(others.len() - 1));
        record.insert("id".into(), json!(name));
        record.insert("exe_lba".into(), json!(entry.exe_lba));
        record.insert("payload_sha256".into(), json!(sha256_bytes(&chosen.exe.payload)));
        record.insert("exe_sha256".into(), json!(chosen.whole_sha()));
        record.insert("map".into(), json!(format!("maps/{map_name}")));
        record.insert("load".into(), json!(format!("{:#x}", chosen.exe.load)));
        record.insert("pc".into(), json!(format!("{:#x}", chosen.exe.pc)));
        record.insert("vblank".into(), json!(vblank));
        record.insert(
            "words".into(),
            Value::Array(
                words
                    .iter()
                    .map(|(a, w)| json!([format!("{a:#x}"), format!("{w:#x}")]))
                    .collect(),
            ),
        );
        record.insert("bin_sha256".into(), bin_sha);
        out.push(Value::Object(record));
    }
    Ok((out, copies))
}

pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(
        raw,
        &[
            "image", "receipt", "lineup", "inputs", "out", "library", "edition", "programs",
        ],
        &[],
    )?;
    let image = args.require_path("image")?;
    let out = args.require_path("out")?;
    let (records, copies) = if let Some(edition) = args.path("edition") {
        // A scene disc: the edition names each program's map and first playable screen.
        let entries = crate::edition::load(args.path("programs").as_deref(), &edition)
            .map_err(Error)?;
        scene_members(&image, &entries)?
    } else {
        let receipt = read_json(&args.require_path("receipt")?)?;
        let lineup = read_json(&args.require_path("lineup")?)?;
        let inputs = args.path("inputs");
        let library: Vec<PathBuf> = args.all("library").into_iter().map(expand_home).collect();
        members(&image, &receipt, &lineup, inputs.as_deref(), &library)?
    };
    fs::create_dir_all(out.join("maps"))?;
    for (source, name) in &copies {
        fs::copy(source, out.join("maps").join(name))?;
    }
    write_atomic(
        &out.join("members.json"),
        &(dumps(&Value::Array(records.clone()), 1, false) + "\n"),
    )?;
    println!(
        "{} members, {} maps: {}",
        records.len(),
        copies.len(),
        out.join("members.json").display()
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SECTOR: usize = 2352;

    /// A raw image: user data written at the CD-ROM offset of each sector.
    struct Image {
        sectors: Vec<Vec<u8>>,
    }

    impl Image {
        fn new(count: usize) -> Image {
            Image {
                sectors: vec![vec![0u8; 2048]; count],
            }
        }

        fn put(&mut self, lba: usize, data: &[u8]) {
            for (i, chunk) in data.chunks(2048).enumerate() {
                self.sectors[lba + i][..chunk.len()].copy_from_slice(chunk);
            }
        }

        /// A PS-X EXE of `words` payload words (one sector each 512), every word `base + index`.
        fn exe(&mut self, lba: usize, load: u32, sectors: usize, base: u32) -> Vec<u8> {
            let mut header = vec![0u8; 2048];
            header[..8].copy_from_slice(PSX_EXE_MAGIC);
            header[0x10..0x14].copy_from_slice(&load.to_le_bytes());
            header[0x18..0x1c].copy_from_slice(&load.to_le_bytes());
            header[0x1c..0x20].copy_from_slice(&((sectors * 2048) as u32).to_le_bytes());
            let mut payload = Vec::new();
            for k in 0..sectors * 512 {
                payload.extend((base + k as u32).to_le_bytes());
            }
            self.put(lba, &header);
            self.put(lba + 1, &payload);
            [header, payload].concat()
        }

        fn toc(&mut self, rows: &[(&str, usize, u32)]) {
            let mut raw = vec![0u8; 4 * 2048];
            raw[..8].copy_from_slice(disc::TOC_MAGIC);
            raw[8..12].copy_from_slice(&(rows.len() as u32).to_le_bytes());
            for (i, (name, lba, flags)) in rows.iter().enumerate() {
                let at = disc::TOC_HEADER_BYTES + i * disc::TOC_ENTRY_BYTES;
                raw[at..at + name.len()].copy_from_slice(name.as_bytes());
                raw[at + 24..at + 28].copy_from_slice(&(*lba as u32).to_le_bytes());
                raw[at + disc::TOC_FLAGS_AT..at + disc::TOC_FLAGS_AT + 4]
                    .copy_from_slice(&flags.to_le_bytes());
            }
            self.put(disc::TOC_LBA as usize, &raw);
        }

        fn write(&self, path: &Path) {
            let mut file = File::create(path).unwrap();
            for sector in &self.sectors {
                file.write_all(&[0u8; 24]).unwrap();
                file.write_all(sector).unwrap();
                file.write_all(&vec![0u8; SECTOR - 24 - 2048]).unwrap();
            }
        }
    }

    const MAP: &str = "  80010000 00000010 4 1 text\n  8001f000 00000004 4 2 __psx_rt_vblank_count\n  8001e000 00000004 4 2 gamecycle\n";

    struct World {
        dir: tempfile::TempDir,
        image: PathBuf,
        receipt: Value,
        lineup: Value,
    }

    /// Launcher at LBA 0, A (bare exe) at 40, B (an image with an intro and a game) at 60..80.
    fn world(hidden_c: bool) -> World {
        let dir = tempfile::tempdir().unwrap();
        let mut disc = Image::new(120);
        // Words are `base + k`: A and B's game overlap the launcher's addresses and differ in value.
        disc.exe(0, 0x8001_0000, 1, 1000);
        disc.exe(40, 0x8001_0000, 2, 5000);
        disc.exe(60, 0x8001_0000, 1, 9000); // B's intro
        let game = disc.exe(70, 0x8001_0000, 2, 20000);
        disc.exe(90, 0x8001_0000, 1, 30000); // C, hidden or not
        let mut rows = vec![("A", 40usize, 0u32), ("B", 60, 0)];
        rows.push(("C", 90, u32::from(hidden_c)));
        disc.toc(&rows);
        let image = dir.path().join("disc.bin");
        disc.write(&image);

        let receipt = json!({"programs": {
            "A": {"embedded": {"exe_lba": 40}},
            "B": {"embedded": {"exe_lba": 60, "image_lba_start": 60, "image_lba_end_exclusive": 80}},
            "C": {"embedded": {"exe_lba": 90}},
        }});
        let record = |dir: &Path, name: &str, files: Value| {
            let map = dir.join(format!("{name}.map"));
            fs::write(&map, MAP).unwrap();
            let provenance = json!({
                "artifacts": {"map": {"file": format!("{name}.map"), "sha256": sha256_file(&map).unwrap()}},
                "files": files,
            });
            fs::write(
                dir.join(format!("{name}.provenance.json")),
                provenance.to_string(),
            )
            .unwrap();
        };
        let inputs = dir.path().join("release-inputs");
        fs::create_dir_all(&inputs).unwrap();
        record(&inputs, "A", json!({}));
        record(&inputs, "B", json!({"b.exe": sha256_bytes(&game)}));
        record(&inputs, "C", json!({}));
        let row = |name: &str, extra: Value| {
            let mut row = json!({
                "name": name,
                "bin_sha256": sha256_bytes(name.as_bytes()),
                "receipt": format!("~/gone/release-inputs/{name}.provenance.json"),
            });
            row.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            row
        };
        let lineup = json!({"programs": [
            row("A", json!({"title_probe": {"sym": "gamecycle", "op": "ge", "value": 100}})),
            row("B", json!({})),
            row("C", json!({})),
        ]});
        World {
            dir,
            image,
            receipt,
            lineup,
        }
    }

    fn inputs(world: &World) -> PathBuf {
        world.dir.path().join("release-inputs")
    }

    #[test]
    fn lists_members_in_table_order_with_maps_probes_and_hashes() {
        let w = world(false);
        let (records, copies) =
            members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap();
        let ids: Vec<_> = records.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["A", "B", "C"]);
        assert_eq!(copies.len(), 3);
        assert_eq!(records[0]["map"], "maps/A.map");
        assert_eq!(records[0]["vblank"], "0x8001f000");
        assert_eq!(
            records[0]["title_probe"],
            json!({"sym": "gamecycle", "addr": "0x8001e000", "op": "ge", "value": 100})
        );
        assert_eq!(records[1]["title_probe"], Value::Null);
        assert_eq!(records[0]["bin_sha256"], sha256_bytes(b"A"));
        assert_eq!(records[0]["words"].as_array().unwrap().len(), 4);
        assert_eq!(records[0]["load"], "0x80010000");
    }

    #[test]
    fn signature_words_differ_from_the_launcher_and_every_other_program() {
        let w = world(false);
        let (records, _) =
            members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap();
        // Every executable here holds `base + index` at index k; A's words are 5000 + k, so none
        // may equal 1000 + k, 9000 + k, 20000 + k or 30000 + k at the same address.
        for pair in records[0]["words"].as_array().unwrap() {
            let addr = u32::from_str_radix(pair[0].as_str().unwrap().trim_start_matches("0x"), 16)
                .unwrap();
            let word = u32::from_str_radix(pair[1].as_str().unwrap().trim_start_matches("0x"), 16)
                .unwrap();
            let k = (addr - 0x8001_0000) / 4;
            assert_eq!(word, 5000 + k);
            assert!(k >= 64);
        }
    }

    #[test]
    fn a_build_record_naming_the_second_executable_picks_it() {
        let w = world(false);
        let (records, _) =
            members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap();
        // B's table executable is the intro at 60; its record names the game at 70.
        assert_eq!(records[1]["exe_lba"], 60);
        let game = disc::parse_exe_at(&w.image, 70).unwrap();
        assert_eq!(records[1]["payload_sha256"], sha256_bytes(&game.payload));
        let first = disc::parse_exe_at(&w.image, 40).unwrap();
        assert_eq!(records[0]["payload_sha256"], sha256_bytes(&first.payload));
    }

    #[test]
    fn a_record_naming_the_maps_executable_picks_it_over_a_listed_boot_executable() {
        let w = world(false);
        // B's record lists both executables (a boot scene and the game, as a core-hosted disc's
        // keep dir holds them) and names the game as the map's own.
        let intro = disc::parse_exe_at(&w.image, 60).unwrap();
        let game = disc::parse_exe_at(&w.image, 70).unwrap();
        let whole = |e: &Exe| {
            let mut v = e.header.clone();
            v.extend_from_slice(&e.payload);
            sha256_bytes(&v)
        };
        let path = inputs(&w).join("B.provenance.json");
        let mut record: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        record["files"] = json!({"boot.exe": whole(&intro), "game.exe": whole(&game)});
        // Without artifacts.exe the listed table executable (the boot one) is taken.
        fs::write(&path, record.to_string()).unwrap();
        let (records, _) =
            members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap();
        assert_eq!(records[1]["payload_sha256"], sha256_bytes(&intro.payload));
        // With it, the game.
        record["artifacts"]["exe"] = json!({"file": "game.exe", "sha256": whole(&game)});
        fs::write(&path, record.to_string()).unwrap();
        let (records, _) =
            members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap();
        assert_eq!(records[1]["payload_sha256"], sha256_bytes(&game.payload));
        // An executable the image does not carry is refused, never replaced by the table's.
        record["artifacts"]["exe"]["sha256"] = json!("00".repeat(32));
        fs::write(&path, record.to_string()).unwrap();
        let err = members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap_err();
        assert!(err.0.contains("artifacts.exe"), "{}", err.0);
    }

    #[test]
    fn hidden_table_rows_are_not_shelf_members() {
        let w = world(true);
        let (records, _) =
            members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn a_map_that_is_not_the_recorded_one_is_refused() {
        let w = world(false);
        fs::write(inputs(&w).join("A.map"), "changed").unwrap();
        let error = members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap_err();
        assert!(error.0.contains("its build record says"), "{error}");
    }

    #[test]
    fn a_missing_map_asks_for_the_install_s_map_beside_its_record() {
        let w = world(false);
        fs::remove_file(inputs(&w).join("B.map")).unwrap();
        let error = members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap_err();
        assert!(error.0.contains("B: map \"B.map\" not found"), "{error}");
    }

    #[test]
    fn a_member_missing_from_the_lineup_is_refused() {
        let mut w = world(false);
        w.lineup["programs"].as_array_mut().unwrap().pop();
        let error = members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap_err();
        assert!(
            error.0.contains("C is on the disc but not in the lineup"),
            "{error}"
        );
    }

    #[test]
    fn a_title_probe_symbol_missing_from_the_map_is_refused() {
        let mut w = world(false);
        w.lineup["programs"][0]["title_probe"]["sym"] = json!("nowhere");
        let error = members(&w.image, &w.receipt, &w.lineup, Some(&inputs(&w)), &[]).unwrap_err();
        assert!(error.0.contains("nowhere is not in the map"), "{error}");
    }

    #[test]
    fn run_writes_members_json_and_the_maps() {
        let w = world(false);
        let out = w.dir.path().join("out");
        let receipt = w.dir.path().join("receipt.json");
        let lineup = w.dir.path().join("lineup.json");
        fs::write(&receipt, w.receipt.to_string()).unwrap();
        fs::write(&lineup, w.lineup.to_string()).unwrap();
        let raw: Vec<String> = [
            "--image",
            w.image.to_str().unwrap(),
            "--receipt",
            receipt.to_str().unwrap(),
            "--lineup",
            lineup.to_str().unwrap(),
            "--inputs",
            inputs(&w).to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(run(&raw).unwrap(), 0);
        let written = read_json(&out.join("members.json")).unwrap();
        assert_eq!(written.as_array().unwrap().len(), 3);
        assert!(out.join("maps/A.map").is_file());
    }

    #[test]
    fn the_private_lineup_names_the_restored_games_title_probes() {
        let root = crate::util::repo_root().unwrap();
        let lineup = read_json(&root.join("release/lineup-v0.59-private.json")).unwrap();
        let probes: Vec<(&str, &Value)> = lineup["programs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row.get("title_probe").is_some())
            .map(|row| (row["name"].as_str().unwrap(), &row["title_probe"]))
            .collect();
        let names: Vec<&str> = probes.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["360X", "QD UP"]);
        assert_eq!(probes[0].1["sym"], "gamecycle");
        assert_eq!(probes[1].1["sym"], "cursor");
    }
}
