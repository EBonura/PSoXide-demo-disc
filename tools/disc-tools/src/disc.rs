//! The pressed disc as the tools see it: raw 2352-byte sectors, the demo table
//! at LBA 22, PS-X EXE headers and one-BIN cue sheets.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::util::{fnv1a32, resolve, Error, Result};
use crate::{bail, ensure};

pub const SECTOR_BYTES: u64 = 2352;
pub const USER_DATA_AT: u64 = 24;
pub const USER_DATA_BYTES: usize = 2048;
pub const TOC_LBA: u64 = 22;
pub const TOC_SECTORS: u64 = 4;
pub const TOC_MAGIC: &[u8; 8] = b"PSXDEMO4";
pub const TOC_HEADER_BYTES: usize = 0x16C;
pub const TOC_ENTRY_BYTES: usize = 512;
pub const TOC_NAME_BYTES: usize = 24;
pub const TOC_DESC_BYTES: usize = 224;
pub const TOC_VERSION_AT: usize = 488;
pub const TOC_VERSION_BYTES: usize = 16;
pub const TOC_FLAGS_AT: usize = 504;
pub const TOC_MAX_ENTRIES: usize = (TOC_SECTORS as usize * USER_DATA_BYTES - TOC_HEADER_BYTES) / TOC_ENTRY_BYTES;
pub const FLAG_HIDDEN: u32 = 1;
pub const PSX_EXE_MAGIC: &[u8; 8] = b"PS-X EXE";
pub const BOOT_SCAN_SECTORS: u64 = 64;

/// One row of the demo table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    pub name: String,
    pub exe_lba: u32,
    pub image_lba: u32,
    pub cdda_track_base: u32,
    pub payload_fnv: u32,
    /// The English description, the first of the two text fields.
    pub description: String,
    pub version: String,
    pub flags: u32,
}

impl TocEntry {
    pub fn hidden(&self) -> bool {
        self.flags & FLAG_HIDDEN != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exe {
    pub lba: u64,
    pub pc: u32,
    pub load: u32,
    pub payload_bytes: u32,
    pub header: Vec<u8>,
    pub payload: Vec<u8>,
}

pub fn le32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// A NUL-terminated ASCII field.
pub fn text_field(data: &[u8]) -> Result<String> {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let slice = &data[..end];
    ensure!(slice.is_ascii(), "non-ASCII text in a disc table field");
    Ok(String::from_utf8_lossy(slice).into_owned())
}

/// The 2048 user bytes of one sector.
pub fn read_user_sector(stream: &mut File, lba: u64) -> Result<Vec<u8>> {
    stream.seek(SeekFrom::Start(lba * SECTOR_BYTES + USER_DATA_AT))?;
    let mut data = vec![0u8; USER_DATA_BYTES];
    let mut filled = 0;
    while filled < data.len() {
        let n = stream.read(&mut data[filled..])?;
        if n == 0 {
            bail!("disc ended at LBA {lba}");
        }
        filled += n;
    }
    Ok(data)
}

pub fn read_user_sectors(image: &Path, lba: u64, count: u64) -> Result<Vec<u8>> {
    let mut stream = File::open(image).map_err(|e| Error(format!("{}: {e}", image.display())))?;
    let mut out = Vec::with_capacity(count as usize * USER_DATA_BYTES);
    for sector in lba..lba + count {
        match read_user_sector(&mut stream, sector) {
            Ok(chunk) => out.extend(chunk),
            Err(_) => bail!("{}: ended while reading {count} user-data sectors at LBA {lba}", image.display()),
        }
    }
    Ok(out)
}

/// Decode the table from its four sectors.
pub fn parse_toc(raw: &[u8]) -> Result<Vec<TocEntry>> {
    ensure!(
        raw.len() == TOC_SECTORS as usize * USER_DATA_BYTES && &raw[..8] == TOC_MAGIC,
        "missing {} at combined LBA {TOC_LBA}",
        String::from_utf8_lossy(TOC_MAGIC)
    );
    let count = le32(raw, 8) as usize;
    ensure!(count != 0 && count <= TOC_MAX_ENTRIES, "invalid combined TOC entry count {count}");
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES;
        let row = &raw[at..at + TOC_ENTRY_BYTES];
        let name = text_field(&row[..TOC_NAME_BYTES])?;
        ensure!(!entries.iter().any(|e: &TocEntry| e.name == name), "duplicate combined TOC name: {name}");
        entries.push(TocEntry {
            exe_lba: le32(row, 24),
            image_lba: le32(row, 28),
            cdda_track_base: le32(row, 32),
            payload_fnv: le32(row, 36),
            description: text_field(&row[40..40 + TOC_DESC_BYTES])?,
            version: text_field(&row[TOC_VERSION_AT..TOC_VERSION_AT + TOC_VERSION_BYTES])?,
            flags: le32(row, TOC_FLAGS_AT),
            name,
        });
    }
    Ok(entries)
}

pub fn read_toc(image: &Path) -> Result<Vec<TocEntry>> {
    parse_toc(&read_user_sectors(image, TOC_LBA, TOC_SECTORS)?)
}

/// The PS-X EXE whose header sits at `lba`, with its whole payload.
pub fn parse_exe_at(image: &Path, lba: u64) -> Result<Exe> {
    let mut stream = File::open(image)?;
    let header = read_user_sector(&mut stream, lba)?;
    ensure!(&header[..8] == PSX_EXE_MAGIC, "no PS-X EXE at {} LBA {lba}", image.display());
    let pc = le32(&header, 0x10);
    let load = le32(&header, 0x18);
    let payload_bytes = le32(&header, 0x1C);
    ensure!(
        payload_bytes != 0 && payload_bytes as usize % USER_DATA_BYTES == 0,
        "invalid PS-X EXE payload size {payload_bytes}"
    );
    ensure!(
        load <= pc && (pc as u64) < load as u64 + payload_bytes as u64,
        "PS-X EXE PC {pc:#x} lies outside its payload"
    );
    let mut payload = Vec::with_capacity(payload_bytes as usize);
    for offset in 0..(payload_bytes as usize / USER_DATA_BYTES) as u64 {
        payload.extend(read_user_sector(&mut stream, lba + 1 + offset)?);
    }
    Ok(Exe { lba, pc, load, payload_bytes, header, payload })
}

/// The boot EXE of an imported data track: the first sector in the opening
/// run that carries the PS-X EXE magic.
pub fn find_boot_exe(image: &Path) -> Result<Exe> {
    let sectors = std::fs::metadata(image)?.len() / SECTOR_BYTES;
    let mut stream = File::open(image)?;
    for lba in 0..sectors.min(BOOT_SCAN_SECTORS) {
        if &read_user_sector(&mut stream, lba)?[..8] == PSX_EXE_MAGIC {
            return parse_exe_at(image, lba);
        }
    }
    bail!("no boot PS-X EXE in first {BOOT_SCAN_SECTORS} sectors: {}", image.display())
}

pub fn exe_fnv(exe: &Exe) -> u32 {
    fnv1a32(&exe.payload)
}

fn file_line() -> Regex {
    Regex::new(r#"(?m)^FILE "([^"\r\n]+)" BINARY$"#).expect("static regex")
}

/// Names on the cue's `FILE "x" BINARY` lines, in order, duplicates kept.
pub fn cue_file_names(text: &str) -> Vec<String> {
    file_line().captures_iter(text).map(|c| c[1].to_string()).collect()
}

/// The one BIN a cue names, required to sit beside it and to be a whole
/// number of raw sectors.
pub fn image_for_cue(cue: &Path) -> Result<PathBuf> {
    let cue = resolve(cue)?;
    ensure!(cue.is_file(), "cue is not a regular file: {}", cue.display());
    let text = std::fs::read(&cue)?;
    ensure!(text.is_ascii(), "cue is not ASCII: {}", cue.display());
    let text = String::from_utf8_lossy(&text).into_owned();
    let mut unique: Vec<String> = Vec::new();
    for name in cue_file_names(&text) {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    ensure!(unique.len() == 1, "cue must reference exactly one unique BIN: {}", cue.display());
    let relative = Path::new(&unique[0]);
    ensure!(
        !relative.is_absolute() && relative.components().count() == 1,
        "cue BIN must sit beside the cue: {}",
        cue.display()
    );
    let parent = cue.parent().unwrap_or(Path::new("."));
    let image = resolve(&parent.join(relative))?;
    ensure!(
        image.parent() == Some(parent) && image.is_file(),
        "cue BIN must resolve beside the cue: {}",
        cue.display()
    );
    let size = std::fs::metadata(&image)?.len();
    ensure!(size != 0 && size % SECTOR_BYTES == 0, "raw BIN size is not a positive whole sector: {}", image.display());
    Ok(image)
}

/// `mm:ss:ff` to a sector count.
pub fn msf_to_sector(msf: &str) -> Result<u64> {
    let parts: Vec<&str> = msf.split(':').collect();
    ensure!(parts.len() == 3, "bad MSF {msf:?}");
    let number = |s: &str| s.parse::<u64>().map_err(|_| Error(format!("bad MSF {msf:?}")));
    Ok((number(parts[0])? * 60 + number(parts[1])?) * 75 + number(parts[2])?)
}

#[cfg(test)]
pub mod testing {
    //! Builders for synthetic discs, shared by the tests of the other modules.
    use super::*;

    /// A raw image of `sectors` sectors whose user data carries `fill`.
    pub fn blank_image(sectors: usize) -> Vec<u8> {
        vec![0u8; sectors * SECTOR_BYTES as usize]
    }

    pub fn put_user(image: &mut [u8], lba: usize, offset: usize, data: &[u8]) {
        let at = lba * SECTOR_BYTES as usize + USER_DATA_AT as usize + offset;
        image[at..at + data.len()].copy_from_slice(data);
    }

    /// A table with the given `(name, exe_lba, image_lba, flags)` rows.
    pub fn toc_bytes(rows: &[(&str, u32, u32, u32)]) -> Vec<u8> {
        let mut toc = vec![0u8; TOC_SECTORS as usize * USER_DATA_BYTES];
        toc[..8].copy_from_slice(TOC_MAGIC);
        toc[8..12].copy_from_slice(&(rows.len() as u32).to_le_bytes());
        for (index, (name, exe, image, flags)) in rows.iter().enumerate() {
            let at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES;
            toc[at..at + name.len()].copy_from_slice(name.as_bytes());
            toc[at + 24..at + 28].copy_from_slice(&exe.to_le_bytes());
            toc[at + 28..at + 32].copy_from_slice(&image.to_le_bytes());
            toc[at + TOC_FLAGS_AT..at + TOC_FLAGS_AT + 4].copy_from_slice(&flags.to_le_bytes());
        }
        toc
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn toc_round_trips_names_flags_and_lbas() {
        let toc = toc_bytes(&[("CORTEX", 30, 8, 1), ("VOXIDE", 90, 70, 0)]);
        let entries = parse_toc(&toc).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].hidden());
        assert_eq!(entries[1].exe_lba, 90);
        assert_eq!(entries[1].image_lba, 70);
    }

    #[test]
    fn toc_rejects_bad_magic_count_and_duplicates() {
        let mut toc = toc_bytes(&[("A", 1, 1, 0)]);
        toc[0] = b'X';
        assert!(parse_toc(&toc).is_err());
        let mut toc = toc_bytes(&[("A", 1, 1, 0)]);
        toc[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse_toc(&toc).is_err());
        assert!(parse_toc(&toc_bytes(&[("A", 1, 1, 0), ("A", 2, 2, 0)])).is_err());
    }

    #[test]
    fn msf_converts() {
        assert_eq!(msf_to_sector("00:02:00").unwrap(), 150);
        assert!(msf_to_sector("1:2").is_err());
    }
}
