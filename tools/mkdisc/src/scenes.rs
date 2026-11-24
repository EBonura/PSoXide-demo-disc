//! The demo disc on the scene core: the outer `SCENES.TBL` and the checks behind it.
//!
//! Every program on the disc runs on one resident core (`psx-core`, CORE-ABI). The launcher is
//! the outer disc's boot program (image 0, scene 0). Each member image is a whole scene disc of
//! its own, nested verbatim: its `SCENES.TBL` at member LBA 22 and its boot program at 23. The
//! outer table carries one Program row per member, for the member's boot program, with the
//! member's start as its member base, so a core switch enters the member at its own disc base
//! and the member goes on with its own table from there (Half-Life's intro switching to its game,
//! 360X loading its front end). A member that is several programs is one row here: the launcher
//! only ever starts the member's boot program. Bare executables (`--game`) are Programs of the
//! outer disc itself, member base 0.
//!
//! Scene `i + 1` starts carousel entry `i` (`scene_layout`): the launcher's table of contents
//! keeps the presentation, the scene table what the core loads.
//!
//! Refusals: a member that is not a scene disc, a member table that does not match the boot
//! program it sits next to, and above all a program of another core: the core refuses a switch
//! to a program whose row names a different core hash, so the build refuses the disc first and
//! names the game and the program.
//!
//! The member's row itself is the SDK's: [`outer_table`] leaves a placeholder Program row at the
//! member's position and mkdisc places each member with `psx_iso::nest::nest`, which writes the row
//! (member base, LBA, the member's entry mask in word 5) and checks it with
//! `psx_iso::nest::check_nested_row` (CORE-ABI, "the outer row rule"). Members start on a 16-sector
//! boundary ([`MEMBER_ALIGN`]), as `nest` places them.

use psx_iso::scenes::{abi, program_image_row};
use psx_iso::SECTOR_BYTES;

use abi::{
    decode_header_checked, encode_table, table_image, table_scene, word_fnv, Header, ImageRow,
    Kind, SceneRow, BOOT_EXE_LBA, MAX_IMAGES, NAME_BYTES, SCENE_FLAG_START, TABLE_LBA,
};

/// Members start on a multiple of this many sectors (`psx_iso::nest::nest` places them so).
pub const MEMBER_ALIGN: u32 = 16;

/// Bytes of user data in a raw Mode 2 Form 1 sector, and where they start.
const USER_BYTES: usize = 2048;
const USER_AT: usize = 24;

/// What the outer table needs of one member image, read from the image itself.
#[derive(Clone, Debug)]
pub struct MemberBoot {
    /// The member's boot program row, as its own table has it (member-relative LBA).
    pub row: ImageRow,
    /// The member's START scene for its boot program, for its partner credit and block version.
    pub scene: SceneRow,
    /// Images in the member's own table: the outer row index must be at least this (see
    /// [`assign_indices`]).
    pub image_count: u8,
}

fn user_data(data: &[u8], lba: u32, sectors: u32) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(sectors as usize * USER_BYTES);
    for s in lba..lba + sectors {
        let at = s as usize * SECTOR_BYTES + USER_AT;
        out.extend_from_slice(data.get(at..at + USER_BYTES)?);
    }
    Some(out)
}

/// Read member `name`'s own scene table and boot program out of its raw data track and check
/// that every Program in it runs on `core` (the disc's core image).
pub fn read_member(name: &str, data: &[u8], core: &[u8]) -> Result<MemberBoot, String> {
    let core_hash = word_fnv(core);
    let table = user_data(data, TABLE_LBA, 1)
        .ok_or_else(|| format!("{name}: the image ends before LBA {TABLE_LBA}"))?;
    let header: Header = decode_header_checked(&table).map_err(|e| {
        format!(
            "{name}: no scene table at LBA {TABLE_LBA} ({e:?}). Every program on this disc runs \
             on the scene core: build {name} as a scene disc (mkisopsx --scenes) on the disc's \
             SDK pin"
        )
    })?;
    for index in 0..header.image_count as u8 {
        let image = table_image(&table, &header, index)
            .ok_or_else(|| format!("{name}: its scene table's image {index} does not decode"))?;
        if image.kind == Kind::Program && image.core_hash != core_hash {
            let program = program_name(&table, &header, index);
            return Err(format!(
                "{name}: program {program} (image {index}) is built on core {:08x}, this disc's \
                 core is {core_hash:08x}. Every program on one disc must embed the same core \
                 (CORE-ABI 4.4): rebuild {name} on the disc's SDK pin.",
                image.core_hash
            ));
        }
    }
    let row = table_image(&table, &header, 0).ok_or_else(|| format!("{name}: no image 0"))?;
    if row.kind != Kind::Program || row.lba != BOOT_EXE_LBA {
        return Err(format!(
            "{name}: image 0 of its scene table is not the boot program at LBA {BOOT_EXE_LBA}"
        ));
    }
    let scene = (0..header.scene_count)
        .filter_map(|id| table_scene(&table, &header, id))
        .find(|s| s.image == 0 && s.flags & SCENE_FLAG_START != 0)
        .ok_or_else(|| format!("{name}: no START scene for its boot program"))?;
    // The row must describe the exe it sits next to: a table left over from another build
    // would load the wrong bytes and fail its checksum on the console.
    let head = user_data(data, BOOT_EXE_LBA, 1)
        .ok_or_else(|| format!("{name}: the image ends before its boot program"))?;
    let payload = u32::from_le_bytes(head[0x1c..0x20].try_into().expect("4 bytes"));
    let exe = user_data(data, BOOT_EXE_LBA, 1 + payload.div_ceil(USER_BYTES as u32))
        .ok_or_else(|| format!("{name}: the image ends inside its boot program"))?;
    let (pressed, entry) = program_image_row("PSX.EXE", &exe, BOOT_EXE_LBA, None, Some(core))
        .map_err(|e| format!("{name}: its boot program: {e:?}"))?;
    if (
        pressed.size,
        pressed.load_addr,
        pressed.checksum,
        pressed.core_hash,
    ) != (row.size, row.load_addr, row.checksum, row.core_hash)
        || entry != scene.entry
    {
        return Err(format!(
            "{name}: its scene table's row for PSX.EXE does not match the exe at LBA \
             {BOOT_EXE_LBA} (a stale table?)"
        ));
    }
    Ok(MemberBoot {
        row,
        scene,
        image_count: header.image_count as u8,
    })
}

fn program_name(table: &[u8], header: &Header, image: u8) -> String {
    (0..header.scene_count)
        .filter_map(|id| table_scene(table, header, id))
        .find(|s| s.image == image && s.flags & SCENE_FLAG_START != 0)
        .map_or_else(|| format!("#{image}"), |s| s.name_str().to_string())
}

/// One program the launcher starts, in carousel order.
pub enum Start {
    /// A bare executable in the outer disc's own root: its exe bytes and header LBA.
    Exe { bytes: Vec<u8>, lba: u32 },
    /// A member image placed at `base`, its CD-DA tracks after `cdda_base` others.
    Member {
        boot: MemberBoot,
        base: u32,
        cdda_base: u32,
    },
}

/// Outer image indices for `needs` (each program's lowest allowed index), in order, from 1.
///
/// When the core enters a program it sets the loaded mask to the program's bit in the table it
/// switched with plus the row's carried overlays, and a member then goes on with its own table,
/// where its boot program is image 0. So a member's outer row carries bit 0 (its own image 0)
/// among its carried overlays, and its outer index must not be the index of a real image in the
/// member's own table, or the core would count that image as loaded. Indices that a member needs
/// to skip are filled with empty rows no scene names.
pub fn assign_indices(needs: &[u8]) -> Result<Vec<u8>, String> {
    let mut next = 1u8;
    let mut out = Vec::with_capacity(needs.len());
    for &need in needs {
        let index = next.max(need);
        if index as usize >= MAX_IMAGES {
            return Err(format!(
                "the scene table needs image index {index}, past the {MAX_IMAGES} the core's mask \
                 holds"
            ));
        }
        out.push(index);
        next = index + 1;
    }
    Ok(out)
}

/// An empty Program row: a member's position before `nest` writes it, and the filler rows no
/// scene names.
fn placeholder_row() -> ImageRow {
    ImageRow {
        lba: 0,
        size: 0,
        load_addr: 0,
        checksum: 0,
        core_hash: 0,
        evicts: 0,
        member_base: 0,
        kind: Kind::Program,
        class: None,
        owner: 0,
        cdda_delta: 0,
    }
}

fn scene_name(display: &str) -> [u8; NAME_BYTES] {
    let clean: String = display
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .take(NAME_BYTES)
        .collect();
    SceneRow::name_bytes(&clean)
}

/// The outer `SCENES.TBL` sector: the launcher (exe bytes, header at `launcher_lba`) as image 0
/// and scene 0, then one START scene per program in `starts` (carousel order, display names in
/// `names`). Returns the sector and, per program, its outer image index.
pub fn outer_table(
    launcher: &[u8],
    launcher_lba: u32,
    core: &[u8],
    starts: &[Start],
    names: &[&str],
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (boot_row, boot_entry) =
        program_image_row("PSX.EXE", launcher, launcher_lba, None, Some(core)).map_err(|e| {
            format!(
                "the launcher: {e:?} (it must link psx-core with psoxide-core.ld and embed the \
                 disc's core image)"
            )
        })?;
    let mut rows = Vec::with_capacity(starts.len());
    let mut needs = Vec::with_capacity(starts.len());
    for (start, name) in starts.iter().zip(names) {
        match start {
            Start::Exe { bytes, lba } => {
                let (row, entry) = program_image_row(name, bytes, *lba, None, Some(core))
                    .map_err(|e| format!("{name}: {e:?}"))?;
                let scene = SceneRow {
                    id: 0,
                    image: 0,
                    flags: SCENE_FLAG_START,
                    entry,
                    partner_credit: 0,
                    min_block_version: 0,
            needs: 0,
                    name: scene_name(name),
                };
                rows.push((row, scene));
                needs.push(1);
            }
            Start::Member { boot, cdda_base, .. } => {
                if *cdda_base != 0 {
                    return Err(format!(
                        "{name}: CD-DA ahead of a member on a scene disc (the nested row carries none)"
                    ));
                }
                // A placeholder: `psx_iso::nest::nest` writes the member's row when it places it.
                let row = placeholder_row();
                let mut scene = boot.scene;
                scene.name = scene_name(name);
                rows.push((row, scene));
                needs.push(boot.image_count);
            }
        }
    }
    let indices = assign_indices(&needs)?;
    let image_count = indices.last().map_or(1, |&i| i as usize + 1);
    let mut images = vec![placeholder_row(); image_count];
    images[0] = boot_row;
    let mut scenes = vec![SceneRow {
        id: 0,
        image: 0,
        flags: SCENE_FLAG_START,
        entry: boot_entry,
        partner_credit: 0,
        min_block_version: 0,
        needs: 0,
        name: scene_name("LAUNCHER"),
    }];
    for (i, ((row, mut scene), &index)) in rows.into_iter().zip(&indices).enumerate() {
        images[index as usize] = row;
        scene.id = (i + 1) as u16;
        scene.image = index;
        scene.flags |= SCENE_FLAG_START;
        scenes.push(scene);
    }
    let mut sector = vec![0u8; USER_BYTES];
    encode_table(&scenes, &images, 0, &mut sector)
        .map_err(|e| format!("the outer scene table: {e:?}"))?;
    Ok((sector, indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_skip_a_members_own_images() {
        // One-image members take the next index; a three-image member needs index 3 or more.
        assert_eq!(assign_indices(&[1, 1, 3, 1]).unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(assign_indices(&[3, 1]).unwrap(), vec![3, 4]);
        assert_eq!(assign_indices(&[1, 5]).unwrap(), vec![1, 5]);
    }

    /// A raw data track whose sector 22 holds a scene table with one Program on core `hash`.
    fn member_on_core(hash: u32) -> Vec<u8> {
        let image = ImageRow {
            lba: BOOT_EXE_LBA,
            size: 2048,
            load_addr: 0x8001_0000,
            checksum: 0,
            core_hash: hash,
            evicts: 0,
            member_base: 0,
            kind: Kind::Program,
            class: None,
            owner: 0,
            cdda_delta: 0,
        };
        let scene = SceneRow {
            id: 0,
            image: 0,
            flags: SCENE_FLAG_START,
            entry: 0x8001_0000,
            partner_credit: 0,
            min_block_version: 0,
            needs: 0,
            name: SceneRow::name_bytes("BOOT"),
        };
        let mut table = vec![0u8; USER_BYTES];
        encode_table(&[scene], &[image], 0, &mut table).unwrap();
        let mut data = vec![0u8; 30 * SECTOR_BYTES];
        let at = TABLE_LBA as usize * SECTOR_BYTES + USER_AT;
        data[at..at + USER_BYTES].copy_from_slice(&table);
        data
    }

    #[test]
    fn a_member_on_another_core_is_refused_by_name() {
        let core = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let other = word_fnv(&core) ^ 1;
        let err = read_member("HALF-LIFE", &member_on_core(other), &core).unwrap_err();
        assert!(
            err.starts_with("HALF-LIFE: program BOOT (image 0) is built on core"),
            "{err}"
        );
    }

    #[test]
    fn a_member_without_a_scene_table_is_refused_by_name() {
        let err = read_member("QD UP", &vec![0u8; 30 * SECTOR_BYTES], &[0; 8]).unwrap_err();
        assert!(err.starts_with("QD UP: no scene table at LBA 22"), "{err}");
    }

    #[test]
    fn indices_stop_at_the_mask_width() {
        assert!(assign_indices(&[32]).is_err());
    }
}
