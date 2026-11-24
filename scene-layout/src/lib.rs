//! Where the demo disc keeps what the launcher reads at boot, on the scene core.
//!
//! The scene core fixes LBA 22 for `SCENES.TBL` (one sector, `psx-scene-abi`): the table of
//! every program on the disc, the one the core switches with. The launcher's own presentation
//! table (`psx-disc-toc`: names, blurbs, versions, songs, shots) follows it at LBA 23, so the
//! launcher reads both in one read at boot. Its program entries are in carousel order, and entry
//! `i` is started by scene `i + 1` of `SCENES.TBL` (scene 0 is the launcher's own boot scene);
//! mkdisc writes both tables so, and the launcher refuses a disc where the counts disagree.
#![no_std]

/// `SCENES.TBL`, the scene core's table (`psx_scene_abi::TABLE_LBA`).
pub const SCENES_LBA: u32 = 22;

/// The launcher's presentation table (`psx-disc-toc`), right after `SCENES.TBL`.
pub const TOC_LBA: u32 = SCENES_LBA + 1;

/// The scene that starts carousel entry `index`.
pub const fn scene_of_entry(index: usize) -> u32 {
    index as u32 + 1
}
