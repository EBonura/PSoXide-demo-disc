//! The architecture gate's receipt check (sdk/docs/GATE.md, G3) for every executable a disc takes.
//!
//! `mkdisc` calls `psx_iso` directly, so the SDK's `mkisopsx` check does not cover it. This
//! module is what GATE.md says a caller of `psx_iso` must do: read the executable, call
//! `psx_gate::press::check` before the bytes reach the image, and stop on an error.
//!
//! Which executables: the launcher, each `--game` EXE, and every PSX-EXE that starts a data
//! sector of each `--image` (a collection or a game with a second stage carries more than its
//! boot EXE: Half-Life has two, the Arcade four). Each executable is judged as the program that
//! `mkdisc` was given a name for (`NAME` of `--game` and `--image`), against the SDK's
//! `sdk/profile.toml` and that program's own `psx-gate.toml` (`--gate-file NAME=path`, which the
//! lineup resolves from the source mirror at the revision the image was built from).
//!
//! What it does not cover is written in GATE.md: a forged receipt, a program renamed to a listed
//! name, an executable that is not at the start of a sector of the data track (compressed, or
//! unpacked by a first stage from a pack), and any writer of a disc image other than `mkdisc`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use psx_gate::press::{self, Verdict};
use psx_gate::profile::Profile;
use psx_iso::SECTOR_BYTES;

use crate::{EXE_MAGIC, SECTOR_SIZE};

/// First byte of the user data in a raw Mode 2 sector.
const USER_AT: usize = 24;
/// Bytes of a PSX-EXE header sector.
const EXE_HEADER_BYTES: usize = SECTOR_SIZE;
/// Where PlayStation RAM starts and ends (KSEG0 addresses an executable may load at).
const RAM: std::ops::Range<u32> = 0x8000_0000..0x8020_0000;

/// What the caller gives: the SDK's profile, the program files and whether a diag image may go on.
pub struct Options {
    /// `sdk/profile.toml` of the SDK the launcher links.
    pub sdk_profile: PathBuf,
    /// This repository's `psx-gate.toml` (the launcher's own file).
    pub launcher_file: Option<PathBuf>,
    /// `(program name, psx-gate.toml)` of each game, from `--gate-file`.
    pub program_files: Vec<(String, PathBuf)>,
    /// A diag receipt is accepted (`--diag`): a development disc, never a shipping one.
    pub allow_diag: bool,
}

/// The check, with what it found so far.
pub struct Gate {
    sdk: Profile,
    allow_diag: bool,
    launcher: Option<Profile>,
    games: BTreeMap<String, Profile>,
    /// One line per executable that passed.
    pub passed: Vec<String>,
    /// One message per executable refused.
    pub refused: Vec<String>,
}

fn own_file(path: &Path) -> Result<Profile, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Profile::parse_program_file(&text, path)
}

impl Gate {
    /// # Errors
    /// The SDK profile or a program file cannot be read.
    pub fn new(options: &Options) -> Result<Self, String> {
        let sdk = Profile::read(&options.sdk_profile)?;
        let launcher = options.launcher_file.as_deref().map(own_file).transpose()?;
        let mut games = BTreeMap::new();
        for (name, path) in &options.program_files {
            games.insert(name.clone(), own_file(path)?);
        }
        Ok(Self {
            sdk,
            allow_diag: options.allow_diag,
            launcher,
            games,
            passed: Vec::new(),
            refused: Vec::new(),
        })
    }

    /// Judge one executable. `name` is the program's name on the disc (`LAUNCHER` for the
    /// launcher), `place` says where the bytes came from, for the message.
    pub fn check(&mut self, name: &str, place: &str, exe: &[u8]) {
        let own = if name == LAUNCHER {
            self.launcher.as_ref()
        } else {
            self.games.get(name)
        };
        match press::check(exe, name, self.allow_diag, &self.sdk, own) {
            Ok(verdict) => self.passed.push(format!("{name}: {place}: {}", describe(&verdict))),
            Err(message) => self.refused.push(format!("{message} ({place})")),
        }
    }

    /// Print what was judged; stop when anything was refused.
    ///
    /// # Errors
    /// The refusals, every one on its own line.
    pub fn finish(self) -> Result<(), String> {
        for line in &self.passed {
            println!("gate: {line}");
        }
        if self.refused.is_empty() {
            return Ok(());
        }
        Err(format!(
            "{} executable(s) refused by the architecture gate (sdk/docs/GATE.md); no disc written:\n  {}",
            self.refused.len(),
            self.refused.join("\n  ")
        ))
    }
}

/// What `check` calls the launcher.
pub const LAUNCHER: &str = "LAUNCHER";

fn describe(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Receipt(program) => format!("receipt ({program})"),
        Verdict::Diag(program) => format!("diag receipt ({program}), --diag given"),
        Verdict::Listed(program) => format!("no receipt, listed in its gate file ({program})"),
        _ => String::from("accepted"),
    }
}

/// Every PSX-EXE that starts a sector of a raw data track, as `(sector, executable)`. The bytes
/// are what the BIOS would read as the file: the header sector and `t_size` bytes after it. A
/// sector that begins with the magic but whose header cannot be an executable (load address
/// outside RAM, size zero or past the end of the track) is not one and is skipped.
pub fn executables_in(data: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let sectors = data.len() / SECTOR_BYTES;
    let user = |lba: usize| &data[lba * SECTOR_BYTES + USER_AT..lba * SECTOR_BYTES + USER_AT + SECTOR_SIZE];
    let mut found = Vec::new();
    for lba in 0..sectors {
        let head = user(lba);
        if !head.starts_with(EXE_MAGIC) {
            continue;
        }
        let word = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().expect("four bytes"));
        let (load, size) = (word(0x18), word(0x1C) as usize);
        let body_sectors = size.div_ceil(SECTOR_SIZE);
        if !RAM.contains(&load) || size == 0 || lba + 1 + body_sectors > sectors {
            continue;
        }
        let mut exe = Vec::with_capacity(EXE_HEADER_BYTES + size);
        exe.extend_from_slice(head);
        for s in 0..body_sectors {
            exe.extend_from_slice(user(lba + 1 + s));
        }
        exe.truncate(EXE_HEADER_BYTES + size);
        found.push((lba as u32, exe));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use psx_iso::receipt::{self, Kind};

    fn exe(load: u32, size: u32) -> Vec<u8> {
        let mut v = vec![0u8; EXE_HEADER_BYTES + size as usize];
        v[..8].copy_from_slice(EXE_MAGIC);
        v[0x18..0x1C].copy_from_slice(&load.to_le_bytes());
        v[0x1C..0x20].copy_from_slice(&size.to_le_bytes());
        v
    }

    /// A raw data track: the sectors of `files` one after another, each file padded to sectors.
    fn track(files: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for file in files {
            for chunk in file.chunks(SECTOR_SIZE) {
                let mut sector = vec![0u8; SECTOR_BYTES];
                sector[USER_AT..USER_AT + chunk.len()].copy_from_slice(chunk);
                out.extend_from_slice(&sector);
            }
        }
        out
    }

    fn profile(text: &str) -> Profile {
        Profile::parse_program_file(text, Path::new("psx-gate.toml")).unwrap()
    }

    fn gate(own: &[(&str, &str)]) -> Gate {
        Gate {
            sdk: Profile::parse("", Path::new("sdk/profile.toml")).unwrap(),
            allow_diag: false,
            launcher: None,
            games: own.iter().map(|(n, t)| (n.to_string(), profile(t))).collect(),
            passed: Vec::new(),
            refused: Vec::new(),
        }
    }

    #[test]
    fn finds_every_executable_of_a_track_and_skips_false_starts() {
        let mut first = exe(0x8001_0000, 4096);
        receipt::write(&mut first, "a", Kind::Shipping).unwrap();
        let second = exe(0x801E_0000, 2048);
        // Starts with the magic but loads outside RAM: not an executable.
        let not_one = exe(0x0000_0010, 2048);
        let data = track(&[&first, &[7u8; 5000], &second, &not_one]);
        let found = executables_in(&data);
        assert_eq!(found.iter().map(|(lba, _)| *lba).collect::<Vec<_>>(), [0, 6]);
        assert_eq!(found[0].1, first);
        assert_eq!(found[1].1, second);
    }

    #[test]
    fn a_header_that_runs_past_the_track_is_not_an_executable() {
        let data = track(&[&exe(0x8001_0000, 2048)[..EXE_HEADER_BYTES]]);
        assert!(executables_in(&data).is_empty());
    }

    #[test]
    fn a_program_without_a_receipt_is_refused_by_name() {
        let mut g = gate(&[]);
        g.check("HOLLOW KNIGHT", "image sector 22", &exe(0x8001_0000, 2048));
        let text = g.finish().unwrap_err();
        assert!(text.contains("HOLLOW KNIGHT"), "{text}");
        assert!(text.contains("no gate receipt"), "{text}");
        assert!(text.contains("image sector 22"), "{text}");
    }

    #[test]
    fn a_receipt_passes_and_a_change_after_the_gate_is_refused() {
        let mut gated = exe(0x8001_0000, 2048);
        receipt::write(&mut gated, "game", Kind::Shipping).unwrap();
        let mut g = gate(&[]);
        g.check("A", "exe", &gated);
        assert!(g.refused.is_empty());
        let mut patched = gated.clone();
        patched[3000] ^= 1;
        g.check("B", "exe", &patched);
        assert!(g.finish().unwrap_err().contains("changed"));
    }

    #[test]
    fn a_game_lists_its_own_unreceipted_image_and_a_stale_entry_is_refused() {
        let listed = "[[exception]]\nkind = \"no-receipt\"\nitem = \"-\"\nremove = \"W9\"\n";
        let mut plain = exe(0x8001_0000, 2048);
        let mut g = gate(&[("OLD GAME", listed)]);
        g.check("OLD GAME", "exe", &plain);
        assert!(g.refused.is_empty(), "{:?}", g.refused);
        // Another game does not borrow that entry.
        g.check("OTHER", "exe", &plain);
        assert_eq!(g.refused.len(), 1);
        // Once the game has a receipt, its entry must go.
        receipt::write(&mut plain, "old", Kind::Shipping).unwrap();
        let mut g = gate(&[("OLD GAME", listed)]);
        g.check("OLD GAME", "exe", &plain);
        assert!(g.finish().unwrap_err().contains("delete the entry"));
    }

    #[test]
    fn a_diag_receipt_needs_the_diag_flag() {
        let mut tap = exe(0x8001_0000, 2048);
        receipt::write(&mut tap, "launcher", Kind::Diag).unwrap();
        let mut g = gate(&[]);
        g.check(LAUNCHER, "PSX.EXE", &tap);
        assert!(g.finish().unwrap_err().contains("diagnostic"));
        let mut g = gate(&[]);
        g.allow_diag = true;
        g.check(LAUNCHER, "PSX.EXE", &tap);
        assert!(g.finish().is_ok());
    }
}
