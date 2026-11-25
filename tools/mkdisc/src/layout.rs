//! Where the disc's data sits against the radius the console reads reliably,
//! and the outer-root aliases that point at files inside member images.
//!
//! # The safe-read limit
//!
//! The owner's console (SCPH-9002) read the burned v0.59 Private disc of
//! 2026-11-25 (CD-R burned at 10x) cleanly at or inside disc time 74:40. 360X's
//! film, streamed from LBA 335,881 (74:40) to 337,915 (75:07), played choppy
//! with one corrupt frame, and QD Up, from 75:07 on, stopped on the core's
//! read-failure screen (0x13). The limit is the start of that marginal band.
//!
//! The earlier radius sweep of 2026-10-19 (hardware tests v2.8, CD RADIUS
//! SWEEP, SDK `engine/examples/hardware-tests/src/radius.rs`) read every
//! 64-sector group cleanly up to 77:28 on a different burn; v0.58, with
//! tables at 78:13, never reached gameplay. The newer burn reads worse, so the
//! limit follows it: what a burn reads depends on the blank and the burn, and
//! the lowest edge measured on this console is the one that counts.
//!
//! Disc time is printed with LBA 0 at 00:02:00 (`frames = lba + 150`, seconds
//! truncated), so 74:40:00 is LBA `74 * 4500 + 40 * 75 - 150` = 335,850.
//!
//! # The rule
//!
//! Every sector a program needs to reach and play its game must lie below
//! [`SAFE_READ_END_LBA`]. Past it the disc may only hold an *edge-tolerant
//! tail*: a member image's trailing files (declared per member with
//! `--edge-tail`) whose bad read cannot hang or corrupt the program (streamed
//! music the program never waits on), and the zero filler. The builder keeps
//! the carousel order for placement when that already passes; otherwise it
//! moves the member with the longest declared tail to the end, and refuses the
//! layout if that still leaves needed data past the limit.

use std::fmt::Write as _;

use psx_iso::{Disc, SECTOR_BYTES};

/// First LBA the console is not known to read reliably: disc time 74:40:00,
/// where the v0.59 Private burn of 2026-11-25 started reading marginally (see
/// the module documentation).
pub const SAFE_READ_END_LBA: u32 = 335_850;

/// The sweep's clock: LBA 0 is disc time 00:02:00.
const PREGAP_FRAMES: u32 = 150;

/// A file inside a member image: name and extent relative to the image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageFile {
    pub path: String,
    pub lba: u32,
    pub sectors: u32,
    pub bytes: u32,
}

/// Every file of an image's ISO 9660 tree, by first sector.
pub fn image_files(data: &[u8]) -> Result<Vec<ImageFile>, String> {
    let disc = Disc::from_bin(data.to_vec());
    let mut files: Vec<ImageFile> = disc
        .list_files()
        .map_err(|e| format!("cannot list the image's files: {e:?}"))?
        .into_iter()
        .map(|f| ImageFile {
            sectors: f.size.div_ceil(2048).max(1),
            lba: f.lba,
            bytes: f.size,
            path: f.path,
        })
        .collect();
    files.sort_by_key(|f| f.lba);
    Ok(files)
}

/// What fills the end of a row past its needed data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tail {
    /// Everything in the row is needed.
    None,
    /// Declared edge-tolerant files, from the first of them to the image end.
    Tolerant(Vec<String>),
    /// Zero sectors nothing reads.
    Filler,
}

/// One stretch of the disc, in placement order.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: String,
    pub start: u32,
    pub sectors: u32,
    /// Sectors from `start` that a program needs; the rest is the tail.
    pub needed: u32,
    pub tail: Tail,
    /// The row's files, for naming what crosses the limit (empty for rows
    /// that are not member images).
    pub files: Vec<ImageFile>,
}

impl Row {
    fn needed_end(&self) -> u32 {
        self.start + self.needed
    }
    fn end(&self) -> u32 {
        self.start + self.sectors
    }
}

/// Where the declared edge-tolerant tail of an image starts (sectors from the
/// image start). The declared files must run contiguously to the image end,
/// so nothing else hides in the tail.
pub fn tail_start(
    member: &str,
    files: &[ImageFile],
    image_sectors: u32,
    declared: &[String],
) -> Result<u32, String> {
    let mut picked: Vec<&ImageFile> = Vec::new();
    for name in declared {
        let file = files
            .iter()
            .find(|f| f.path.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("--edge-tail {member}={name}: the image has no such file"))?;
        picked.push(file);
    }
    picked.sort_by_key(|f| f.lba);
    let Some(first) = picked.first() else {
        return Ok(image_sectors);
    };
    let mut at = first.lba;
    for file in &picked {
        if file.lba != at {
            return Err(format!(
                "--edge-tail {member}: {} starts at image sector {}, but the declared tail \
                 reached only {at}; the tail must be the declared files back to back",
                file.path, file.lba
            ));
        }
        at += file.sectors;
    }
    if let Some(intruder) = files
        .iter()
        .find(|f| f.lba >= first.lba && !picked.iter().any(|p| p.path == f.path))
    {
        return Err(format!(
            "--edge-tail {member}: {} at image sector {} sits inside the declared tail and \
             is not declared edge-tolerant",
            intruder.path, intruder.lba
        ));
    }
    if at != image_sectors {
        return Err(format!(
            "--edge-tail {member}: the declared files end at image sector {at}, the image at \
             {image_sectors}; the tail must run to the image end"
        ));
    }
    Ok(first.lba)
}

/// Placement order of the images: the carousel order when the rule passes
/// with it, else the same with the member that has the longest tolerant tail
/// moved to the end. `rows_for(order)` lays the rows out for an order.
pub fn choose_order(
    count: usize,
    tail_sectors: &[u32],
    rows_for: impl Fn(&[usize]) -> Vec<Row>,
    limit: u32,
) -> Vec<usize> {
    let natural: Vec<usize> = (0..count).collect();
    if check(&rows_for(&natural), limit).is_ok() {
        return natural;
    }
    let best = (0..count)
        .filter(|&i| tail_sectors[i] != 0)
        .max_by_key(|&i| (tail_sectors[i], i));
    match best {
        Some(last) => {
            let mut order: Vec<usize> = natural.into_iter().filter(|&i| i != last).collect();
            order.push(last);
            order
        }
        None => natural,
    }
}

/// Refuse a layout with needed data at or past `limit`, naming each member,
/// the file and where it ends.
pub fn check(rows: &[Row], limit: u32) -> Result<(), String> {
    let mut faults = Vec::new();
    for row in rows {
        if row.needed_end() <= limit {
            continue;
        }
        // Every needed file that reaches past the limit, largest first (the ones that matter
        // are the big data files, not the boot record that happens to sit there too).
        let mut crossing: Vec<&ImageFile> = row
            .files
            .iter()
            .filter(|f| f.lba < row.needed && row.start + f.lba + f.sectors > limit)
            .collect();
        crossing.sort_by_key(|f| (std::cmp::Reverse(f.sectors), f.lba));
        const NAMED: usize = 4;
        let mut what: Vec<String> = crossing
            .iter()
            .take(NAMED)
            .map(|f| {
                format!(
                    "{} at LBA {}..{} ({}..{})",
                    f.path,
                    row.start + f.lba,
                    row.start + f.lba + f.sectors,
                    clock(row.start + f.lba),
                    clock(row.start + f.lba + f.sectors)
                )
            })
            .collect();
        if crossing.len() > NAMED {
            what.push(format!("and {} more files", crossing.len() - NAMED));
        }
        if what.is_empty() {
            what.push(format!(
                "its data at LBA {}..{} ({}..{})",
                row.start.max(limit),
                row.needed_end(),
                clock(row.start.max(limit)),
                clock(row.needed_end())
            ));
        }
        faults.push(format!("  {}: {}", row.name, what.join("; ")));
    }
    if faults.is_empty() {
        return Ok(());
    }
    Err(format!(
        "data a program needs ends past the safe-read limit LBA {limit} ({}), where the \
         console's reads are not reliable (v0.59 Private burn 2026-11-25):\n{}\nDeclare a member's \
         trailing music edge-tolerant (--edge-tail, only where a bad read cannot hang the \
         game), or make room.",
        clock(limit),
        faults.join("\n")
    ))
}

/// Disc time the way the radius sweep prints it: `MM:SS`, LBA 0 = 00:02.
pub fn clock(lba: u32) -> String {
    let frames = lba + PREGAP_FRAMES;
    format!("{:02}:{:02}", frames / 4_500, frames % 4_500 / 75)
}

/// The layout table and the margin to the limit.
pub fn table(rows: &[Row], limit: u32) -> String {
    let mut out = format!(
        "disc layout against the safe-read limit LBA {limit} ({}, v0.59 Private burn 2026-11-25):\n",
        clock(limit)
    );
    let _ = writeln!(
        out,
        "  {:<26} {:>15}  {:>11}  {:>16}  tail",
        "member", "LBA", "time", "needed end"
    );
    for row in rows {
        let tail = match &row.tail {
            Tail::None => String::from("none"),
            Tail::Filler => format!("filler {} sectors", row.sectors),
            Tail::Tolerant(files) => format!(
                "edge-tolerant {} ({} sectors)",
                files.join("+"),
                row.sectors - row.needed
            ),
        };
        let needed = if row.needed == 0 {
            String::from("-")
        } else {
            format!("{} {}", row.needed_end(), clock(row.needed_end()))
        };
        let _ = writeln!(
            out,
            "  {:<26} {:>7}..{:<7}  {}..{}  {:>16}  {tail}",
            row.name,
            row.start,
            row.end(),
            clock(row.start),
            clock(row.end()),
            needed
        );
    }
    let needed_end = rows.iter().map(Row::needed_end).max().unwrap_or(0);
    let end = rows.iter().map(Row::end).max().unwrap_or(0);
    let past = end.saturating_sub(limit);
    let _ = writeln!(
        out,
        "  needed data ends at LBA {needed_end} ({}): {} sectors {} the limit; disc ends at \
         LBA {end} ({}), {past} sectors past it",
        clock(needed_end),
        limit.abs_diff(needed_end),
        if needed_end <= limit {
            "inside"
        } else {
            "PAST"
        },
        clock(end)
    );
    out
}

/// The same table as JSON, for the release receipt.
pub fn json(rows: &[Row], limit: u32, aliases: &[PlacedAlias]) -> String {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut out = format!(
        "{{\n  \"schema\": 1,\n  \"safe_read_end_lba\": {limit},\n  \"safe_read_end_time\": {},\n  \
         \"source\": \"v0.59 Private burn on the SCPH-9002, 2026-11-25: clean to disc time 74:40 (LBA 0 = 00:02:00)\",\n  \"rows\": [\n",
        quote(&clock(limit))
    );
    for (i, row) in rows.iter().enumerate() {
        let (kind, files) = match &row.tail {
            Tail::None => ("none", Vec::new()),
            Tail::Filler => ("filler", Vec::new()),
            Tail::Tolerant(files) => ("edge-tolerant", files.clone()),
        };
        let files: Vec<String> = files.iter().map(|f| quote(f)).collect();
        let _ = write!(
            out,
            "    {{\"name\": {}, \"lba_start\": {}, \"lba_end_exclusive\": {}, \"needed_end_exclusive\": {}, \
             \"time_start\": {}, \"time_end\": {}, \"tail\": {}, \"tail_files\": [{}]}}{}\n",
            quote(&row.name),
            row.start,
            row.end(),
            row.needed_end(),
            quote(&clock(row.start)),
            quote(&clock(row.end())),
            quote(kind),
            files.join(", "),
            if i + 1 == rows.len() { "" } else { "," }
        );
    }
    let needed_end = rows.iter().map(Row::needed_end).max().unwrap_or(0);
    let _ = write!(
        out,
        "  ],\n  \"needed_end_exclusive\": {needed_end},\n  \"margin_sectors\": {},\n  \"aliases\": [\n",
        i64::from(limit) - i64::from(needed_end)
    );
    for (i, alias) in aliases.iter().enumerate() {
        let _ = write!(
            out,
            "    {{\"name\": {}, \"member\": {}, \"file\": {}, \"lba\": {}, \"sectors\": {}, \"bytes\": {}, \
             \"replaces_menu_xa\": {}}}{}\n",
            quote(&alias.outer),
            quote(&alias.member),
            quote(&alias.inner),
            alias.lba,
            alias.sectors,
            alias.bytes,
            alias.replaces_menu_xa,
            if i + 1 == aliases.len() { "" } else { "," }
        );
    }
    out.push_str("  ]\n}\n");
    out
}

/// An outer-root name pointing at a file inside a member image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub outer: String,
    pub member: String,
    pub inner: String,
}

impl Alias {
    /// `OUTER=MEMBER:FILE`.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let bad = || format!("--alias wants OUTER=MEMBER:FILE, got {spec:?}");
        let (outer, rest) = spec.split_once('=').ok_or_else(bad)?;
        let (member, inner) = rest.rsplit_once(':').ok_or_else(bad)?;
        if outer.is_empty() || member.is_empty() || inner.is_empty() {
            return Err(bad());
        }
        Ok(Self {
            outer: outer.to_ascii_uppercase(),
            member: member.to_string(),
            inner: inner.to_string(),
        })
    }
}

/// An alias with its extent on the finished disc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedAlias {
    pub outer: String,
    pub member: String,
    pub inner: String,
    pub lba: u32,
    pub sectors: u32,
    pub bytes: u32,
    pub replaces_menu_xa: bool,
}

/// Whether a raw XA file (2336-byte sectors: subheader and 2328 bytes, as
/// `--menu-xa` takes it) holds the same sectors as `sectors` raw 2352-byte
/// image sectors: the subheader and user data of each, Form 1 or Form 2 by
/// its own submode (the EDC/ECC is recomputed at placement, so it is left out).
pub fn xa_matches_image(raw: &[u8], image: &[u8]) -> Result<(), String> {
    const XA: usize = 2336;
    if raw.len() % XA != 0 || image.len() % SECTOR_BYTES != 0 {
        return Err("not whole sectors".into());
    }
    if raw.len() / XA != image.len() / SECTOR_BYTES {
        return Err(format!(
            "{} sectors against {}",
            raw.len() / XA,
            image.len() / SECTOR_BYTES
        ));
    }
    for (index, (a, b)) in raw
        .chunks_exact(XA)
        .zip(image.chunks_exact(SECTOR_BYTES))
        .enumerate()
    {
        let form2 = a[2] & 0x20 != 0;
        let span = if form2 { 8 + 2324 } else { 8 + 2048 };
        if a[..span] != b[16..16 + span] {
            return Err(format!("sector {index} differs"));
        }
    }
    Ok(())
}

/// Point the root directory record `name` of the finished image at `lba`
/// with `bytes` as its size, then protect the directory sector again.
pub fn patch_root_record(disc: &mut [u8], name: &str, lba: u32, bytes: u32) -> Result<(), String> {
    let user = |sector: u32| sector as usize * SECTOR_BYTES + 24;
    let pvd = user(16);
    if disc.get(pvd + 1..pvd + 6) != Some(b"CD001") {
        return Err("no primary volume descriptor at LBA 16".into());
    }
    let root_record = pvd + 156;
    let root_lba = u32::from_le_bytes(disc[root_record + 2..root_record + 6].try_into().unwrap());
    let root_size =
        u32::from_le_bytes(disc[root_record + 10..root_record + 14].try_into().unwrap());
    if root_size as usize > 2048 {
        return Err("the root directory spans more than one sector".into());
    }
    let ident = format!("{};1", name.to_ascii_uppercase());
    let base = user(root_lba);
    let mut at = 0usize;
    while at < root_size as usize {
        let len = disc[base + at] as usize;
        if len == 0 {
            break;
        }
        let id_len = disc[base + at + 32] as usize;
        let id = &disc[base + at + 33..base + at + 33 + id_len];
        if id == ident.as_bytes() {
            let record = base + at;
            disc[record + 2..record + 6].copy_from_slice(&lba.to_le_bytes());
            disc[record + 6..record + 10].copy_from_slice(&lba.to_be_bytes());
            disc[record + 10..record + 14].copy_from_slice(&bytes.to_le_bytes());
            disc[record + 14..record + 18].copy_from_slice(&bytes.to_be_bytes());
            let sector = root_lba as usize * SECTOR_BYTES;
            psx_iso::iso9660::protect_mode2_form1(&mut disc[sector..sector + SECTOR_BYTES]);
            return Ok(());
        }
        at += len;
    }
    Err(format!("the root directory has no record {ident}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, lba: u32, sectors: u32) -> ImageFile {
        ImageFile {
            path: path.into(),
            lba,
            sectors,
            bytes: sectors * 2048,
        }
    }

    fn row(name: &str, start: u32, sectors: u32, needed: u32, files: Vec<ImageFile>) -> Row {
        Row {
            name: name.into(),
            start,
            sectors,
            needed,
            tail: if needed == sectors {
                Tail::None
            } else {
                Tail::Tolerant(vec!["MUSIC1.XA".into()])
            },
            files,
        }
    }

    #[test]
    fn the_limit_is_the_v059_burns_74_40() {
        assert_eq!(SAFE_READ_END_LBA, 74 * 4_500 + 40 * 75 - 150);
        assert_eq!(clock(SAFE_READ_END_LBA), "74:40");
        assert_eq!(clock(0), "00:02");
    }

    #[test]
    fn a_tail_must_be_the_declared_files_to_the_image_end() {
        let files = vec![
            file("PSX.EXE", 22, 380),
            file("WORLD.PAK", 1024, 48),
            file("MUSIC1.XA", 1072, 12_392),
        ];
        assert_eq!(
            tail_start("V", &files, 13_464, &["MUSIC1.XA".into()]),
            Ok(1072)
        );
        assert!(tail_start("V", &files, 13_464, &["WORLD.PAK".into()]).is_err());
        assert!(tail_start("V", &files, 13_500, &["MUSIC1.XA".into()]).is_err());
        assert!(tail_start("V", &files, 13_464, &["NOPE.XA".into()]).is_err());
        assert_eq!(tail_start("V", &files, 13_464, &[]), Ok(13_464));
        let two = vec![file("A.XA", 10, 5), file("B.XA", 15, 5)];
        assert_eq!(
            tail_start("Q", &two, 20, &["B.XA".into(), "A.XA".into()]),
            Ok(10)
        );
    }

    #[test]
    fn needed_data_past_the_limit_is_refused_by_name() {
        let qd = row(
            "QD UP",
            350_851,
            2_493,
            2_493,
            vec![file("QDUP.DAT", 1024, 1_469)],
        );
        let err = check(&[qd], SAFE_READ_END_LBA).unwrap_err();
        assert!(
            err.contains("QD UP: QDUP.DAT at LBA 351875..353344 (78:13..78:33)"),
            "{err}"
        );
        let vox = row(
            "VOXIDE",
            330_000,
            13_464,
            1_072,
            vec![file("MUSIC1.XA", 1072, 12_392)],
        );
        assert!(check(&[vox], SAFE_READ_END_LBA).is_ok());
        let edge = row("X", SAFE_READ_END_LBA - 10, 10, 10, Vec::new());
        assert!(check(&[edge.clone()], SAFE_READ_END_LBA).is_ok());
        let over = Row {
            sectors: 11,
            needed: 11,
            ..edge
        };
        assert!(check(&[over], SAFE_READ_END_LBA).is_err());
    }

    #[test]
    fn the_carousel_order_stays_when_it_passes_and_the_longest_tail_goes_last_otherwise() {
        // Three images of 100 sectors from LBA 0; the second has a 60-sector tail, the first a 30.
        let sizes = [100u32, 100, 100];
        let tails = [30u32, 60, 0];
        let lay = |limit_rows: &[usize]| {
            let mut at = 0;
            limit_rows
                .iter()
                .map(|&i| {
                    let r = row(
                        &format!("P{i}"),
                        at,
                        sizes[i],
                        sizes[i] - tails[i],
                        Vec::new(),
                    );
                    at += sizes[i];
                    r
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(choose_order(3, &tails, lay, 300), vec![0, 1, 2]);
        assert_eq!(choose_order(3, &tails, lay, 250), vec![0, 2, 1]);
        assert!(check(&lay(&[0, 2, 1]), 250).is_ok());
        assert!(check(&lay(&choose_order(3, &tails, lay, 230)), 230).is_err());
    }

    #[test]
    fn a_patched_root_record_points_where_the_alias_says() {
        let mut builder = psx_iso::iso9660::IsoBuilder::new();
        builder.add_file("SYSTEM.CNF", psx_iso::iso9660::default_system_cnf());
        builder.add_file("PIANO1.XA", vec![0u8; 2048]);
        let mut bin = builder.build_bin();
        // A stand-in member image after the ISO: 10 sectors.
        bin.extend(std::iter::repeat_n(0u8, 10 * SECTOR_BYTES));
        let target = (bin.len() / SECTOR_BYTES) as u32 - 6;
        patch_root_record(&mut bin, "PIANO1.XA", target, 3 * 2048).unwrap();
        let disc = Disc::from_bin(bin.clone());
        let file = disc.find_file("PIANO1.XA").unwrap();
        assert_eq!((file.lba, file.size), (target, 3 * 2048));
        // The directory sector is still a valid Form 1 sector: re-protecting it changes nothing.
        let root = 20 * SECTOR_BYTES;
        let mut again = bin[root..root + SECTOR_BYTES].to_vec();
        psx_iso::iso9660::protect_mode2_form1(&mut again);
        assert_eq!(again, &bin[root..root + SECTOR_BYTES]);
        assert!(patch_root_record(&mut bin, "NOPE.XA", 0, 0).is_err());
    }

    #[test]
    fn aliases_parse_outer_member_file() {
        let a = Alias::parse("piano1.xa=VOXIDE:MUSIC1.XA").unwrap();
        assert_eq!(
            (a.outer.as_str(), a.member.as_str(), a.inner.as_str()),
            ("PIANO1.XA", "VOXIDE", "MUSIC1.XA")
        );
        assert!(Alias::parse("PIANO1.XA=VOXIDE").is_err());
        assert!(Alias::parse("=VOXIDE:MUSIC1.XA").is_err());
    }

    #[test]
    fn an_xa_file_matches_its_image_sectors_by_subheader_and_data() {
        let mut raw = vec![0u8; 2 * 2336];
        raw[2] = 0x64; // form 2 audio
        raw[2336 + 2] = 0x08; // form 1 data
        raw[100] = 7;
        raw[2336 + 100] = 9;
        let mut image = vec![0u8; 2 * SECTOR_BYTES];
        image[16..16 + 2336].copy_from_slice(&raw[..2336]);
        image[SECTOR_BYTES + 16..SECTOR_BYTES + 16 + 2336].copy_from_slice(&raw[2336..]);
        // The form 1 sector's EDC/ECC area is ignored.
        image[SECTOR_BYTES + 16 + 2056] = 0xAA;
        assert_eq!(xa_matches_image(&raw, &image), Ok(()));
        image[SECTOR_BYTES + 16 + 9] ^= 1;
        assert!(xa_matches_image(&raw, &image).is_err());
        assert!(xa_matches_image(&raw[..2336], &image).is_err());
    }
}
