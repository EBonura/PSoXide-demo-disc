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
//! 0x0C  u32 reserved
//! 0x10  entries, ENTRY_BYTES each:
//!         0x00  name, NUL-padded ASCII
//!         0x18  u32 LBA of the program's PSX-EXE header sector
//!         0x1C  u32 reserved
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

/// One sector.
pub const TOC_BYTES: usize = 2048;

/// Bytes per entry.
pub const ENTRY_BYTES: usize = 32;

/// Bytes reserved for an entry's display name.
pub const NAME_BYTES: usize = 24;

const HEADER_BYTES: usize = 0x10;

/// Entries that fit in one sector.
pub const MAX_ENTRIES: usize = (TOC_BYTES - HEADER_BYTES) / ENTRY_BYTES;

/// One program on the disc.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Display name, ASCII, truncated to [`NAME_BYTES`].
    pub name: [u8; NAME_BYTES],
    /// LBA of the program's PSX-EXE header sector.
    pub exe_lba: u32,
}

impl Entry {
    /// Build an entry, truncating `name` to [`NAME_BYTES`].
    pub fn new(name: &str, exe_lba: u32) -> Self {
        let mut bytes = [0u8; NAME_BYTES];
        let src = name.as_bytes();
        let n = if src.len() > NAME_BYTES {
            NAME_BYTES
        } else {
            src.len()
        };
        bytes[..n].copy_from_slice(&src[..n]);
        Entry {
            name: bytes,
            exe_lba,
        }
    }

    /// The name as a `str`, NUL padding stripped. Empty if not valid ASCII.
    pub fn name_str(&self) -> &str {
        let end = match self.name.iter().position(|&b| b == 0) {
            Some(i) => i,
            None => NAME_BYTES,
        };
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

/// Serialize `entries` into the table sector.
///
/// Returns `None` if there are more than [`MAX_ENTRIES`].
pub fn encode(entries: &[Entry]) -> Option<[u8; TOC_BYTES]> {
    if entries.len() > MAX_ENTRIES {
        return None;
    }
    let mut out = [0u8; TOC_BYTES];
    out[..8].copy_from_slice(&MAGIC);
    out[8..12].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    for (i, entry) in entries.iter().enumerate() {
        let at = HEADER_BYTES + i * ENTRY_BYTES;
        out[at..at + NAME_BYTES].copy_from_slice(&entry.name);
        out[at + NAME_BYTES..at + NAME_BYTES + 4].copy_from_slice(&entry.exe_lba.to_le_bytes());
    }
    Some(out)
}

/// Parse the table sector into `into`, returning the entry count.
///
/// Returns `None` on a bad magic or an implausible count, which is how the
/// launcher tells "this disc has no table" from "this disc's table is empty".
pub fn decode(sector: &[u8; TOC_BYTES], into: &mut [Entry; MAX_ENTRIES]) -> Option<usize> {
    if sector[..8] != MAGIC {
        return None;
    }
    let count = u32::from_le_bytes([sector[8], sector[9], sector[10], sector[11]]) as usize;
    if count > MAX_ENTRIES {
        return None;
    }
    for (i, slot) in into.iter_mut().enumerate().take(count) {
        let at = HEADER_BYTES + i * ENTRY_BYTES;
        let mut name = [0u8; NAME_BYTES];
        name.copy_from_slice(&sector[at..at + NAME_BYTES]);
        let lba_at = at + NAME_BYTES;
        slot.name = name;
        slot.exe_lba = u32::from_le_bytes([
            sector[lba_at],
            sector[lba_at + 1],
            sector[lba_at + 2],
            sector[lba_at + 3],
        ]);
    }
    Some(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> [Entry; MAX_ENTRIES] {
        [Entry::new("", 0); MAX_ENTRIES]
    }

    #[test]
    fn round_trips_entries() {
        let entries = [
            Entry::new("CORTEX IGNITION", 4096),
            Entry::new("HALF-LIFE", 40960),
        ];
        let sector = encode(&entries).expect("fits");
        let mut out = blank();
        assert_eq!(decode(&sector, &mut out), Some(2));
        assert_eq!(out[0], entries[0]);
        assert_eq!(out[1], entries[1]);
        assert_eq!(out[0].name_str(), "CORTEX IGNITION");
    }

    #[test]
    fn rejects_a_sector_that_is_not_a_toc() {
        let mut sector = encode(&[Entry::new("X", 1)]).expect("fits");
        sector[0] ^= 0xFF;
        assert_eq!(decode(&sector, &mut blank()), None);
    }

    #[test]
    fn rejects_more_entries_than_fit() {
        let too_many = [Entry::new("X", 1); MAX_ENTRIES + 1];
        assert!(encode(&too_many).is_none());
    }

    #[test]
    fn truncates_an_overlong_name() {
        let entry = Entry::new("0123456789012345678901234567890", 7);
        assert_eq!(entry.name_str().len(), NAME_BYTES);
    }
}
