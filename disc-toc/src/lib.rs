//! The demo disc's table of contents: one sector at a fixed LBA listing every
//! program on the disc.
//!
//! `mkdisc` writes it, the launcher reads it at boot. Going through the disc
//! rather than baking the list into the launcher breaks the circular
//! dependency the alternative creates: the launcher's own size shifts every
//! following LBA, so a launcher that embedded the table would need rebuilding
//! after the layout it changed.
//!
//! Fixed 2048-byte layout:
//!
//! ```text
//! 0x00  magic "PSXDEMO1"
//! 0x08  u32 entry count
//! 0x0C  u32 first CD-DA track the menu plays, 0 for none
//! 0x10  u32 how many consecutive tracks it cycles through
//! 0x14  music credit, NUL-padded ASCII
//! 0x44  beat grid, MAX_MENU_TRACKS x (u32 milli-BPM, u32 first-beat ms)
//! 0x84  menu track titles, MAX_MENU_TRACKS x MENU_TITLE_BYTES
//! 0x144 entries, ENTRY_BYTES each:
//!         0x00  name, NUL-padded ASCII
//!         0x18  u32 LBA of the program's PSX-EXE header sector
//!         0x1C  u32 sectors between disc LBA 0 and the program's image
//!         0x20  u32 CD-DA tracks belonging to programs ahead of this one
//!         0x24  English description, NUL-padded ASCII
//!         0x64  Italian description, NUL-padded ASCII
//! ```

#![no_std]

/// Sector holding the table. Files land from
/// `psx_iso::PLAYTEST_FIRST_FILE_LBA` (21) onwards and `SYSTEM.CNF` takes
/// that one, so the table is the second file and the boot EXE follows it.
/// Keeping the table ahead of the EXE means its LBA does not move when the
/// launcher grows.
pub const TOC_LBA: u32 = 22;

/// File name `mkdisc` gives the table in the ISO 9660 root directory.
pub const TOC_FILE_NAME: &str = "DEMOTOC.BIN";

/// Identifies a demo-disc table of contents.
pub const MAGIC: [u8; 8] = *b"PSXDEMO1";

/// Sectors the table occupies. Two, since the per-track titles and the two
/// descriptions per program stopped fitting in one.
pub const TOC_SECTORS: u32 = 2;

/// The whole table.
pub const TOC_BYTES: usize = 2048 * TOC_SECTORS as usize;

/// Bytes per entry.
pub const ENTRY_BYTES: usize = 164;

/// Bytes reserved for an entry's display name.
pub const NAME_BYTES: usize = 24;

/// Bytes reserved for each of an entry's two descriptions. The launcher wraps
/// them to two lines, so this is two screen-widths of the 8-pixel font less a
/// little slack.
pub const DESC_BYTES: usize = 64;

/// Bytes reserved for the music credit the menu prints. A licence that asks
/// for attribution is only satisfied if the attribution ships with the disc,
/// so it travels in the table beside the track number it refers to.
pub const CREDIT_BYTES: usize = 48;

/// Menu tracks the beat grid has room for.
pub const MAX_MENU_TRACKS: usize = 8;

/// Bytes reserved for each menu track's title, shown as "now playing".
pub const MENU_TITLE_BYTES: usize = 24;

const HEADER_BYTES: usize = 0x144;
const CREDIT_AT: usize = 0x14;
const BEATS_AT: usize = 0x44;
const TITLES_AT: usize = 0x84;

/// Entries that fit in one sector.
pub const MAX_ENTRIES: usize = (TOC_BYTES - HEADER_BYTES) / ENTRY_BYTES;

/// One program on the disc.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Display name, ASCII, truncated to [`NAME_BYTES`].
    pub name: [u8; NAME_BYTES],
    /// LBA of the program's PSX-EXE header sector, absolute on this disc.
    pub exe_lba: u32,
    /// Sectors between this disc's LBA 0 and the start of the program's own
    /// disc image. Every LBA the program was built to know is relative to
    /// that image, so this one number relocates all of them.
    pub lba_offset: u32,
    /// CD-DA tracks belonging to programs placed ahead of this one.
    pub cdda_track_base: u32,
    /// One-line description, English.
    pub desc_en: [u8; DESC_BYTES],
    /// One-line description, Italian.
    pub desc_it: [u8; DESC_BYTES],
}

fn fixed<const N: usize>(text: &str) -> [u8; N] {
    let mut out = [0u8; N];
    let src = text.as_bytes();
    let n = if src.len() > N { N } else { src.len() };
    out[..n].copy_from_slice(&src[..n]);
    out
}

fn trimmed(bytes: &[u8]) -> &str {
    let end = match bytes.iter().position(|&b| b == 0) {
        Some(i) => i,
        None => bytes.len(),
    };
    core::str::from_utf8(&bytes[..end]).unwrap_or("")
}

impl Entry {
    /// Build an entry, truncating `name` to [`NAME_BYTES`].
    pub fn new(name: &str, exe_lba: u32, lba_offset: u32, cdda_track_base: u32) -> Self {
        Entry {
            name: fixed(name),
            exe_lba,
            lba_offset,
            cdda_track_base,
            desc_en: [0; DESC_BYTES],
            desc_it: [0; DESC_BYTES],
        }
    }

    /// Attach the two descriptions, each truncated to [`DESC_BYTES`].
    pub fn described(mut self, english: &str, italian: &str) -> Self {
        self.desc_en = fixed(english);
        self.desc_it = fixed(italian);
        self
    }

    /// The name as a `str`, NUL padding stripped. Empty if not valid ASCII.
    pub fn name_str(&self) -> &str {
        trimmed(&self.name)
    }

    /// The English description, NUL padding stripped.
    pub fn desc_en_str(&self) -> &str {
        trimmed(&self.desc_en)
    }

    /// The Italian description, NUL padding stripped.
    pub fn desc_it_str(&self) -> &str {
        trimmed(&self.desc_it)
    }
}

/// What the table says apart from the programs themselves.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// How many entries follow.
    pub count: usize,
    /// First CD-DA track the menu plays, or 0 if the disc has none.
    pub menu_track: u32,
    /// How many consecutive tracks from `menu_track` it cycles through.
    pub menu_track_count: u32,
    /// Music credit, NUL-padded.
    pub credit: [u8; CREDIT_BYTES],
    /// Per menu track: tempo in thousandths of a BPM, and how far into the
    /// track its first beat falls, in milliseconds. Measured rather than
    /// assumed (see `tools/beatgrid.py`): a tempo out by 1 BPM drifts several
    /// beats over a four-minute track, which looks like a bug rather than an
    /// effect.
    pub beats: [(u32, u32); MAX_MENU_TRACKS],
    /// Title of each menu track, NUL-padded, for the menu to name what is
    /// playing. The permission was given per track, so the disc says which.
    pub titles: [[u8; MENU_TITLE_BYTES]; MAX_MENU_TRACKS],
}

impl Header {
    /// The credit as a `str`, NUL padding stripped.
    pub fn credit_str(&self) -> &str {
        trimmed(&self.credit)
    }

    /// Title of menu track `index`, NUL padding stripped. Empty when the
    /// disc did not name it.
    pub fn title(&self, index: usize) -> &str {
        self.titles.get(index).map_or("", |t| trimmed(t))
    }

    /// Milliseconds per beat for menu track `index`, and how far into the
    /// track the grid starts. `None` when that track has no measured tempo,
    /// which is the caller's cue to skip the beat-driven visuals rather than
    /// pulse at a guess.
    pub fn beat(&self, index: usize) -> Option<(u32, u32)> {
        let (milli_bpm, phase_ms) = *self.beats.get(index)?;
        if milli_bpm == 0 {
            return None;
        }
        // 60 s per minute, in ms, over beats per minute in thousandths.
        Some((60_000_000 / milli_bpm, phase_ms))
    }
}

/// Serialize `entries` into the table sector.
///
/// Returns `None` if there are more than [`MAX_ENTRIES`].
pub fn encode(
    entries: &[Entry],
    menu_track: u32,
    menu_track_count: u32,
    credit: &str,
    beats: &[(u32, u32)],
    titles: &[&str],
) -> Option<[u8; TOC_BYTES]> {
    if entries.len() > MAX_ENTRIES
        || beats.len() > MAX_MENU_TRACKS
        || titles.len() > MAX_MENU_TRACKS
    {
        return None;
    }
    let mut out = [0u8; TOC_BYTES];
    out[..8].copy_from_slice(&MAGIC);
    out[8..12].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    out[12..16].copy_from_slice(&menu_track.to_le_bytes());
    out[16..20].copy_from_slice(&menu_track_count.to_le_bytes());
    out[CREDIT_AT..CREDIT_AT + CREDIT_BYTES].copy_from_slice(&fixed::<CREDIT_BYTES>(credit));
    for (i, title) in titles.iter().enumerate() {
        let at = TITLES_AT + i * MENU_TITLE_BYTES;
        out[at..at + MENU_TITLE_BYTES].copy_from_slice(&fixed::<MENU_TITLE_BYTES>(title));
    }
    for (i, (milli_bpm, phase_ms)) in beats.iter().enumerate() {
        let at = BEATS_AT + i * 8;
        out[at..at + 4].copy_from_slice(&milli_bpm.to_le_bytes());
        out[at + 4..at + 8].copy_from_slice(&phase_ms.to_le_bytes());
    }
    for (i, entry) in entries.iter().enumerate() {
        let at = HEADER_BYTES + i * ENTRY_BYTES;
        out[at..at + NAME_BYTES].copy_from_slice(&entry.name);
        let n = at + NAME_BYTES;
        out[n..n + 4].copy_from_slice(&entry.exe_lba.to_le_bytes());
        out[n + 4..n + 8].copy_from_slice(&entry.lba_offset.to_le_bytes());
        out[n + 8..n + 12].copy_from_slice(&entry.cdda_track_base.to_le_bytes());
        let d = n + 12;
        out[d..d + DESC_BYTES].copy_from_slice(&entry.desc_en);
        out[d + DESC_BYTES..d + 2 * DESC_BYTES].copy_from_slice(&entry.desc_it);
    }
    Some(out)
}

/// Parse the table sector into `into`, returning everything that is not an
/// entry.
///
/// Returns `None` on a bad magic or an implausible count, which is how the
/// launcher tells "this disc has no table" from "this disc's table is empty".
pub fn decode(sector: &[u8; TOC_BYTES], into: &mut [Entry; MAX_ENTRIES]) -> Option<Header> {
    if sector[..8] != MAGIC {
        return None;
    }
    let count = u32::from_le_bytes([sector[8], sector[9], sector[10], sector[11]]) as usize;
    if count > MAX_ENTRIES {
        return None;
    }
    let mut credit = [0u8; CREDIT_BYTES];
    credit.copy_from_slice(&sector[CREDIT_AT..CREDIT_AT + CREDIT_BYTES]);
    let mut titles = [[0u8; MENU_TITLE_BYTES]; MAX_MENU_TRACKS];
    for (i, slot) in titles.iter_mut().enumerate() {
        let at = TITLES_AT + i * MENU_TITLE_BYTES;
        slot.copy_from_slice(&sector[at..at + MENU_TITLE_BYTES]);
    }
    let mut beats = [(0u32, 0u32); MAX_MENU_TRACKS];
    for (i, slot) in beats.iter_mut().enumerate() {
        let at = BEATS_AT + i * 8;
        let word = |a: usize| u32::from_le_bytes([sector[a], sector[a + 1], sector[a + 2], sector[a + 3]]);
        *slot = (word(at), word(at + 4));
    }
    let header = Header {
        count,
        beats,
        titles,
        menu_track: u32::from_le_bytes([sector[12], sector[13], sector[14], sector[15]]),
        menu_track_count: u32::from_le_bytes([sector[16], sector[17], sector[18], sector[19]]),
        credit,
    };
    for (i, slot) in into.iter_mut().enumerate().take(count) {
        let at = HEADER_BYTES + i * ENTRY_BYTES;
        let mut name = [0u8; NAME_BYTES];
        name.copy_from_slice(&sector[at..at + NAME_BYTES]);
        let n = at + NAME_BYTES;
        let word = |at: usize| {
            u32::from_le_bytes([sector[at], sector[at + 1], sector[at + 2], sector[at + 3]])
        };
        slot.name = name;
        slot.exe_lba = word(n);
        slot.lba_offset = word(n + 4);
        slot.cdda_track_base = word(n + 8);
        let d = n + 12;
        slot.desc_en.copy_from_slice(&sector[d..d + DESC_BYTES]);
        slot.desc_it.copy_from_slice(&sector[d + DESC_BYTES..d + 2 * DESC_BYTES]);
    }
    Some(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> [Entry; MAX_ENTRIES] {
        [Entry::new("", 0, 0, 0); MAX_ENTRIES]
    }

    #[test]
    fn round_trips_entries() {
        let entries = [
            Entry::new("CORTEX IGNITION", 4096, 4074, 0)
                .described("Original 3D action game", "Gioco d'azione 3D originale"),
            Entry::new("HALF-LIFE", 40960, 40938, 1),
        ];
        let sector = encode(
            &entries,
            30,
            4,
            "Music by Just Music",
            &[(176_000, 34), (175_000, 23)],
            &["KNUCKLE DUST", "RUSTED HAMMER"],
        )
        .expect("fits");
        let mut out = blank();
        let header = decode(&sector, &mut out).expect("decodes");
        assert_eq!(header.count, 2);
        assert_eq!(header.menu_track, 30);
        assert_eq!(header.menu_track_count, 4);
        assert_eq!(header.credit_str(), "Music by Just Music");
        // 176 BPM is 340 ms a beat, and the grid starts 34 ms in.
        assert_eq!(header.beat(0), Some((340, 34)));
        assert_eq!(header.beat(1), Some((342, 23)));
        assert_eq!(header.beat(2), None, "no measured tempo, no pulse");
        assert_eq!(header.title(0), "KNUCKLE DUST");
        assert_eq!(header.title(1), "RUSTED HAMMER");
        assert_eq!(header.title(2), "", "unnamed track");
        assert_eq!(out[0], entries[0]);
        assert_eq!(out[1], entries[1]);
        assert_eq!(out[0].name_str(), "CORTEX IGNITION");
        assert_eq!(out[1].lba_offset, 40938);
        assert_eq!(out[1].cdda_track_base, 1);
        assert_eq!(out[0].desc_en_str(), "Original 3D action game");
        assert_eq!(out[0].desc_it_str(), "Gioco d'azione 3D originale");
        assert_eq!(out[1].desc_en_str(), "", "an entry may have no description");
    }

    #[test]
    fn rejects_a_sector_that_is_not_a_toc() {
        let mut sector = encode(&[Entry::new("X", 1, 0, 0)], 0, 0, "", &[], &[]).expect("fits");
        sector[0] ^= 0xFF;
        assert_eq!(decode(&sector, &mut blank()), None);
    }

    #[test]
    fn rejects_more_entries_than_fit() {
        let too_many = [Entry::new("X", 1, 0, 0); MAX_ENTRIES + 1];
        assert!(encode(&too_many, 0, 0, "", &[], &[]).is_none());
    }

    #[test]
    fn truncates_an_overlong_description() {
        let entry = Entry::new("X", 0, 0, 0).described(&"e".repeat(80), &"i".repeat(80));
        assert_eq!(entry.desc_en_str().len(), DESC_BYTES);
        assert_eq!(entry.desc_it_str().len(), DESC_BYTES);
    }

    #[test]
    fn truncates_an_overlong_name() {
        let entry = Entry::new("0123456789012345678901234567890", 7, 0, 0);
        assert_eq!(entry.name_str().len(), NAME_BYTES);
    }
}
