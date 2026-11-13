//! Which program is running, read from what the machine does and not from anything a program
//! prints.
//!
//! A shipping image carries no debug text (the SDK compiles it out, and a disc that ships may
//! not build it back in), so the gates cannot wait for a line such as "launcher: booted". They
//! read the emulator instead, through logs it already writes for an untouched shipping disc:
//!
//! * the **executables on the disc**: every PSX-EXE that starts a data sector, with its load
//!   address, size and entry point from the header and, when there is one, the gate receipt
//!   (`psoxide-pgo apply` writes it at 0x400; the program's name is in it);
//! * **words of RAM** (`--route-watch-u32`): the emulator reads them without touching the guest,
//!   at each route tick. A program is *resident* when a few words of its text, at the addresses
//!   they have in the executable, read back as the disc has them;
//! * **PC samples** (`--pc-sample-window-log`) and the **display flips** of the route log.
//!
//! The programs all load low in RAM, so an address range does not say who is running. The words
//! do: for each executable, [`probes`] picks addresses at which that executable's word differs
//! from the word of every other executable on the disc that covers the address (and is not
//! zero). A program that is not that executable cannot have all of them, whatever else is
//! in RAM, because by construction no other executable on the disc has them there; the check
//! fails when pointed at another program's entry.
//!
//! "The launcher booted": its words are resident before the launching press, the PC was sampled
//! in its range, and the display flipped many times (the menu is animating).
//! "It chain-loaded X": after the launching press one of X's executables becomes resident (an
//! image may hold several: Half-Life's intro hands over to its game), the launcher's words are
//! never resident again (no return to the menu), the display keeps flipping, and the PC was
//! sampled in X's code.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::disc::{le32, PSX_EXE_MAGIC, SECTOR_BYTES, USER_DATA_AT, USER_DATA_BYTES};
use crate::replay::{parse_int, Csv, Row};
use crate::util::{Error, Result};

/// Words of text one probe set holds.
const PROBE_WORDS: usize = 4;
/// Probe addresses are looked for this far past the entry point.
const PROBE_SPAN: u32 = 0x1_0000;
/// Probe addresses stay this far apart, so one edited function cannot hide them all.
const PROBE_GAP: u32 = 0x80;
/// Display flips the menu shows before the launching press.
pub(crate) const MENU_MIN_FLIPS: usize = 20;
/// Display flips a program shows after it became resident.
pub(crate) const RUN_MIN_FLIPS: usize = 10;
/// Where a PSX-EXE may load.
const RAM: std::ops::Range<u32> = 0x8000_0000..0x8020_0000;
/// Where the gate receipt sits in the header, and its size.
const RECEIPT_AT: usize = 0x400;
const RECEIPT_BYTES: usize = 0x60;

/// What the header's receipt says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Receipt {
    Missing,
    /// A receipt that matches the executable: the program it names.
    Shipping(String),
    Diag(String),
    /// A receipt whose digest is not this executable's.
    Mismatch,
}

impl Receipt {
    pub(crate) fn describe(&self) -> String {
        match self {
            Receipt::Missing => "no receipt".into(),
            Receipt::Shipping(program) => format!("receipt {program}"),
            Receipt::Diag(program) => format!("diag receipt {program}"),
            Receipt::Mismatch => "receipt does not match".into(),
        }
    }
}

/// One PSX-EXE found on the disc.
#[derive(Debug, Clone)]
pub(crate) struct DiscExe {
    pub(crate) lba: u64,
    /// Index of the table row it belongs to; `None` for the launcher's own sectors.
    pub(crate) owner: Option<usize>,
    pub(crate) load: u32,
    pub(crate) pc: u32,
    pub(crate) payload: Vec<u8>,
    pub(crate) receipt: Receipt,
}

impl DiscExe {
    pub(crate) fn end(&self) -> u32 {
        self.load + self.payload.len() as u32
    }

    pub(crate) fn contains(&self, pc: u64) -> bool {
        u64::from(self.load) <= pc && pc < u64::from(self.end())
    }

    fn word_at(&self, address: u32) -> Option<u32> {
        let at = address.checked_sub(self.load)? as usize;
        let bytes = self.payload.get(at..at + 4)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

/// The receipt of an executable, checked against its bytes (`psx-iso`'s `receipt::verify`: SHA-256
/// of the header and `t_size` payload bytes with the receipt area zeroed).
fn receipt_of(header: &[u8], payload: &[u8]) -> Receipt {
    let area = &header[RECEIPT_AT..RECEIPT_AT + RECEIPT_BYTES];
    if &area[..8] != b"PSXGATE1" {
        return Receipt::Missing;
    }
    let mut sha = Sha256::new();
    sha.update(&header[..RECEIPT_AT]);
    sha.update([0u8; RECEIPT_BYTES]);
    sha.update(&header[RECEIPT_AT + RECEIPT_BYTES..]);
    sha.update(payload);
    if area[8..40] != sha.finalize()[..] {
        return Receipt::Mismatch;
    }
    let len = usize::from(area[41]);
    let Some(name) = area.get(42..42 + len) else {
        return Receipt::Mismatch;
    };
    let name = String::from_utf8_lossy(name).into_owned();
    if area[40] == 0 {
        Receipt::Shipping(name)
    } else {
        Receipt::Diag(name)
    }
}

/// Every PSX-EXE that starts a data sector of `image` (the pressed disc), in disc order, each
/// owned by the table row whose programs start at or before it. An executable ahead of the
/// first program is the launcher's. A sector that begins with the magic but whose header cannot
/// be an executable (load address outside RAM, no payload, a payload past the end of the disc)
/// is skipped, as `mkdisc` skips it.
pub(crate) fn scan(image: &Path, rows: &[Row]) -> Result<Vec<DiscExe>> {
    let mut stream = File::open(image).map_err(|e| Error(format!("{}: {e}", image.display())))?;
    let sectors = stream.metadata()?.len() / SECTOR_BYTES;
    let mut starts: Vec<(u64, usize)> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let start = if row.image_lba != 0 { row.image_lba } else { row.exe_lba };
            (u64::from(start), index)
        })
        .collect();
    starts.sort_unstable();
    let mut found = Vec::new();
    let mut head = [0u8; 8];
    for lba in 0..sectors {
        stream.seek(SeekFrom::Start(lba * SECTOR_BYTES + USER_DATA_AT))?;
        if stream.read_exact(&mut head).is_err() || &head != PSX_EXE_MAGIC {
            continue;
        }
        let mut header = vec![0u8; USER_DATA_BYTES];
        stream.seek(SeekFrom::Start(lba * SECTOR_BYTES + USER_DATA_AT))?;
        stream.read_exact(&mut header)?;
        let (pc, load, size) = (le32(&header, 0x10), le32(&header, 0x18), le32(&header, 0x1C));
        let body = u64::from(size).div_ceil(USER_DATA_BYTES as u64);
        if !RAM.contains(&load) || size == 0 || lba + 1 + body > sectors {
            continue;
        }
        let mut payload = Vec::with_capacity(size as usize);
        let mut sector = vec![0u8; USER_DATA_BYTES];
        for next in 0..body {
            stream.seek(SeekFrom::Start((lba + 1 + next) * SECTOR_BYTES + USER_DATA_AT))?;
            stream.read_exact(&mut sector)?;
            payload.extend_from_slice(&sector);
        }
        payload.truncate(size as usize);
        let owner = starts.iter().rev().find(|(start, _)| *start <= lba).map(|(_, i)| *i);
        let receipt = receipt_of(&header, &payload);
        found.push(DiscExe { lba, owner, load, pc, payload, receipt });
    }
    Ok(found)
}

/// Words that tell executable `which` from every other executable on the disc: `(address, word)`
/// pairs from its text after the entry point. At each address no other executable has this word
/// (one that does not cover the address cannot), and the word is not zero.
///
/// # Errors
/// Fewer than [`PROBE_WORDS`] such words exist: the executable cannot be told from the others.
pub(crate) fn probes(exes: &[DiscExe], which: usize) -> Result<Vec<(u32, u32)>> {
    let this = &exes[which];
    let mut found: Vec<(u32, u32)> = Vec::new();
    let first = this.pc.max(this.load) & !3;
    let last = this.end().min(first.saturating_add(PROBE_SPAN));
    let mut address = first;
    while address + 4 <= last && found.len() < PROBE_WORDS {
        let word = this.word_at(address).unwrap_or(0);
        let apart = found.last().is_none_or(|(at, _)| address - at >= PROBE_GAP);
        let unique = exes
            .iter()
            .enumerate()
            .all(|(index, other)| index == which || other.word_at(address) != Some(word));
        if word != 0 && apart && unique {
            found.push((address, word));
        }
        address += 4;
    }
    ensure!(
        found.len() == PROBE_WORDS,
        "the executable at LBA {} cannot be told from the others on the disc: only {} of {PROBE_WORDS} \
         distinguishing words in its first {PROBE_SPAN:#x} bytes",
        this.lba,
        found.len()
    );
    Ok(found)
}

/// The emulator arguments that read `probes` (`--route-watch-u32` once per address).
pub(crate) fn watch_args(sets: &[&[(u32, u32)]]) -> Vec<String> {
    let mut addresses: Vec<u32> = sets.iter().flat_map(|set| set.iter().map(|(a, _)| *a)).collect();
    addresses.sort_unstable();
    addresses.dedup();
    addresses
        .into_iter()
        .flat_map(|address| ["--route-watch-u32".to_string(), format!("{address:#010x}")])
        .collect()
}

/// The emulator arguments for the PC samples this module reads.
pub(crate) fn pc_window_args(path: &Path) -> Vec<String> {
    vec![
        "--pc-sample-window-log".into(),
        path.display().to_string(),
        "--pc-sample-window-ticks".into(),
        "10".into(),
        "--pc-sample-instructions".into(),
        "4096".into(),
    ]
}

/// One row of the route log: a tick, whether the display flipped, the watched words.
#[derive(Debug, Clone)]
pub(crate) struct Tick {
    pub(crate) tick: u64,
    pub(crate) flipped: bool,
    words: Vec<(u32, u32)>,
}

impl Tick {
    fn resident(&self, probes: &[(u32, u32)]) -> bool {
        probes.iter().all(|probe| self.words.contains(probe))
    }
}

/// The route log, with the words at `addresses`.
pub(crate) fn read_route(path: &Path, addresses: &[u32]) -> Result<Vec<Tick>> {
    let csv = Csv::read(path)?;
    let mut ticks = Vec::with_capacity(csv.rows.len());
    for row in &csv.rows {
        let mut words = Vec::with_capacity(addresses.len());
        for &address in addresses {
            let value = parse_int(csv.required(row, &format!("ram_{address:08x}"))?, 10)?;
            words.push((address, value as u32));
        }
        ticks.push(Tick {
            tick: parse_int(csv.required(row, "route_tick")?, 10)?,
            flipped: csv.required(row, "display_start_changed")?.trim() == "1",
            words,
        });
    }
    Ok(ticks)
}

/// PC samples in `ranges` from route tick `from` on, from the window log.
pub(crate) fn pc_samples_in(
    path: &Path,
    from: u64,
    contains: impl Fn(u64) -> bool,
) -> Result<u64> {
    let csv = Csv::read(path)?;
    let mut total = 0;
    for row in &csv.rows {
        let tick = parse_int(csv.required(row, "window_start_tick")?, 10)?;
        let pc = parse_int(csv.required(row, "pc")?, 16)?;
        if tick + 10 > from && contains(pc) {
            total += parse_int(csv.required(row, "samples")?, 10)?;
        }
    }
    Ok(total)
}

/// What "the launcher booted" needs: before route tick `before` its words were resident, the
/// display flipped [`MENU_MIN_FLIPS`] times, and the PC was sampled in its code (`sampled`).
pub(crate) fn launcher_booted(
    ticks: &[Tick],
    launcher: &[(u32, u32)],
    before: u64,
    sampled: u64,
) -> Result<u64> {
    let menu: Vec<&Tick> = ticks.iter().filter(|t| t.tick <= before).collect();
    ensure!(
        menu.iter().any(|t| t.resident(launcher)),
        "the launcher's code is not in RAM before the launching press (route tick {before})"
    );
    let flips = menu.iter().filter(|t| t.flipped).count();
    ensure!(
        flips >= MENU_MIN_FLIPS,
        "the menu's display flipped {flips} times before the launching press, expected {MENU_MIN_FLIPS}"
    );
    ensure!(sampled != 0, "the PC was never sampled in the launcher's code");
    Ok(flips as u64)
}

/// What "it chain-loaded" found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chained {
    /// Which of the candidates became resident.
    pub(crate) candidate: usize,
    /// The route tick it was first seen.
    pub(crate) tick: u64,
    pub(crate) flips: usize,
}

/// After route tick `after`, one of `candidates` (the probes of each executable the program may
/// run) becomes resident; from then on the launcher's words are never resident again and the
/// display flips at least [`RUN_MIN_FLIPS`] times.
pub(crate) fn chain_loaded(
    ticks: &[Tick],
    launcher: &[(u32, u32)],
    candidates: &[Vec<(u32, u32)>],
    after: u64,
) -> Result<Chained> {
    let seen = ticks.iter().filter(|t| t.tick > after).find_map(|t| {
        candidates
            .iter()
            .position(|candidate| t.resident(candidate))
            .map(|candidate| (candidate, t.tick))
    });
    let Some((candidate, tick)) = seen else {
        bail!("no executable of the program became resident after route tick {after}");
    };
    let later: Vec<&Tick> = ticks.iter().filter(|t| t.tick > tick).collect();
    // The launcher's code may still be in RAM when the program is first seen (Half-Life's intro
    // loads high, above it, and loads its game over it later). What cannot happen is the launcher
    // coming back: its words gone from RAM and then resident again.
    let mut gone = false;
    for t in &later {
        if !t.resident(launcher) {
            gone = true;
        } else if gone {
            bail!("the launcher's code came back after the program was loaded (route tick {})", t.tick);
        }
    }
    let flips = later.iter().filter(|t| t.flipped).count();
    ensure!(
        flips >= RUN_MIN_FLIPS,
        "the display flipped {flips} times after the program was loaded, expected {RUN_MIN_FLIPS}"
    );
    Ok(Chained { candidate, tick, flips })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(lba: u64, load: u32, words: &[u32]) -> DiscExe {
        DiscExe {
            lba,
            owner: Some(0),
            load,
            pc: load,
            payload: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
            receipt: Receipt::Missing,
        }
    }

    /// `count` words, spaced so probes may sit `PROBE_GAP` apart.
    fn words(seed: u32, count: usize) -> Vec<u32> {
        (0..count as u32).map(|i| (i + 1).wrapping_mul(0x9E37_79B1) ^ seed).collect()
    }

    fn tick(n: u64, flipped: bool, words: &[(u32, u32)]) -> Tick {
        Tick { tick: n, flipped, words: words.to_vec() }
    }

    #[test]
    fn probes_differ_from_every_other_executable_at_their_address() {
        let launcher = exe(1, 0x8001_0000, &words(1, 256));
        let game = exe(2, 0x8001_0000, &words(2, 256));
        let all = [launcher, game];
        for which in 0..2 {
            let set = probes(&all, which).unwrap();
            assert_eq!(set.len(), PROBE_WORDS);
            for (address, word) in set {
                assert_ne!(word, 0);
                assert_ne!(all[1 - which].word_at(address), Some(word));
            }
        }
    }

    #[test]
    fn an_executable_that_is_a_copy_of_another_cannot_be_told_apart() {
        let a = exe(1, 0x8001_0000, &words(5, 256));
        let b = a.clone();
        let error = probes(&[a, b], 0).unwrap_err().0;
        assert!(error.contains("cannot be told from the others"), "{error}");
    }

    #[test]
    fn a_range_that_overlaps_another_is_told_by_its_words() {
        let low = exe(1, 0x8001_0000, &words(1, 4096));
        // Loads higher, covers part of `low`'s range; its words there are its own.
        let high = exe(2, 0x8001_2000, &words(9, 4096));
        let set = probes(&[low.clone(), high.clone()], 1).unwrap();
        // None of them is a word `low` holds at that address.
        assert!(set.iter().all(|(a, w)| low.word_at(*a) != Some(*w)));
    }

    #[test]
    fn the_launcher_boots_when_its_words_are_resident_and_the_menu_animates() {
        let launcher = vec![(0x8001_0000, 7), (0x8001_0100, 8)];
        let ticks: Vec<Tick> = (0..100).map(|n| tick(n, n % 2 == 0, &launcher)).collect();
        assert_eq!(launcher_booted(&ticks, &launcher, 80, 3).unwrap(), 41);
        // A frozen menu, a missing launcher and no PC samples each fail.
        let frozen: Vec<Tick> = (0..100).map(|n| tick(n, false, &launcher)).collect();
        assert!(launcher_booted(&frozen, &launcher, 80, 3).is_err());
        let wrong: Vec<Tick> = (0..100).map(|n| tick(n, true, &[(0x8001_0000, 1), (0x8001_0100, 8)])).collect();
        assert!(launcher_booted(&wrong, &launcher, 80, 3).is_err());
        assert!(launcher_booted(&ticks, &launcher, 80, 0).is_err());
    }

    #[test]
    fn a_chain_load_needs_the_program_resident_the_launcher_gone_and_the_display_moving() {
        let launcher = vec![(0x8001_0000, 7)];
        let program = vec![(0x8001_0000, 100)];
        let other = vec![(0x8001_0000, 200)];
        let mut ticks: Vec<Tick> = (0..50).map(|n| tick(n, true, &launcher)).collect();
        ticks.extend((50..120).map(|n| tick(n, n % 2 == 0, &program)));
        let found = chain_loaded(&ticks, &launcher, &[other.clone(), program.clone()], 40).unwrap();
        assert_eq!((found.candidate, found.tick), (1, 50));
        // Pointed at the wrong program's entry the check fails.
        assert!(chain_loaded(&ticks, &launcher, &[other.clone()], 40).is_err());
        // A return to the launcher fails it, even after the program ran.
        let mut returned = ticks.clone();
        returned.extend((120..130).map(|n| tick(n, true, &launcher)));
        assert!(chain_loaded(&returned, &launcher, &[program.clone()], 40)
            .unwrap_err()
            .0
            .contains("came back"));
        // A program that never moves the display fails it.
        let still: Vec<Tick> = (0..120).map(|n| tick(n, false, if n < 50 { &launcher } else { &program })).collect();
        assert!(chain_loaded(&still, &launcher, &[program], 40).unwrap_err().0.contains("flipped"));
    }

    #[test]
    fn a_program_that_loads_above_the_launcher_leaves_it_resident_until_it_loads_over_it() {
        // Half-Life: the intro is resident at 0x801E0000 while the launcher's words are still at
        // 0x80010000; the game then loads over them. That is not a return to the launcher.
        let launcher = vec![(0x8001_0000, 7)];
        let intro = vec![(0x801E_0000, 50)];
        let game = vec![(0x8001_0000, 99)];
        let mut ticks: Vec<Tick> = (0..50).map(|n| tick(n, true, &launcher)).collect();
        ticks.extend((50..80).map(|n| tick(n, n % 2 == 0, &[(0x8001_0000, 7), (0x801E_0000, 50)])));
        ticks.extend((80..140).map(|n| tick(n, n % 2 == 0, &[(0x8001_0000, 99)])));
        let found = chain_loaded(&ticks, &launcher, &[intro.clone(), game.clone()], 40).unwrap();
        assert_eq!((found.candidate, found.tick), (0, 50));
        // The launcher's words back after the game was loaded still fail.
        ticks.extend((140..150).map(|n| tick(n, true, &launcher)));
        assert!(chain_loaded(&ticks, &launcher, &[intro, game], 40).is_err());
    }

    #[test]
    fn the_receipt_names_the_program_and_a_changed_executable_does_not_match() {
        let mut header = vec![0u8; USER_DATA_BYTES];
        let payload = vec![5u8; 2048];
        assert_eq!(receipt_of(&header, &payload), Receipt::Missing);
        header[RECEIPT_AT..RECEIPT_AT + 8].copy_from_slice(b"PSXGATE1");
        header[RECEIPT_AT + 40] = 0;
        header[RECEIPT_AT + 41] = 4;
        header[RECEIPT_AT + 42..RECEIPT_AT + 46].copy_from_slice(b"game");
        let mut sha = Sha256::new();
        let mut zeroed = header.clone();
        zeroed[RECEIPT_AT..RECEIPT_AT + RECEIPT_BYTES].fill(0);
        sha.update(&zeroed);
        sha.update(&payload);
        header[RECEIPT_AT + 8..RECEIPT_AT + 40].copy_from_slice(&sha.finalize());
        assert_eq!(receipt_of(&header, &payload), Receipt::Shipping("game".into()));
        let mut changed = payload.clone();
        changed[1] ^= 1;
        assert_eq!(receipt_of(&header, &changed), Receipt::Mismatch);
    }
}
