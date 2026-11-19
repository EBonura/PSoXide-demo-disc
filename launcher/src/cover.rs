//! Cover art: which pictures go on which program's case.
//!
//! The pictures are baked into the launcher by `tools/menu-art` (three sizes
//! of each, and the attract mode's back-cover shots). Citadel's are cut from
//! `assets/shots`; every other program's come from its own repository's
//! `launcher-art/` (`external_program!`). They are found by the program's name
//! in the disc table. A program with no
//! picture here gets a plain cover, so a new lineup entry still shows and
//! starts.
//!
//! The Private programs' art and names exist only in a build with the
//! `private` feature: a Public disc must not carry them anywhere.

use crate::gfx::CoverArt;

/// One program's art and the words printed on its case.
pub struct Program {
    /// The name the disc table carries.
    pub name: &'static str,
    /// The front cover at three sizes.
    pub art: CoverArt,
    /// The back cover's screenshots: a 512 byte CLUT, then one or two 64x48
    /// pictures at 8bpp.
    pub back: &'static [u8],
    /// The name stacked down the spine.
    pub spine: &'static str,
    /// The one-line blurb on the back, in English and Italian.
    pub blurb: (&'static str, &'static str),
}

macro_rules! program {
    ($name:literal, $id:literal, $spine:literal, $en:literal, $it:literal) => {
        Program {
            name: $name,
            art: CoverArt {
                l: include_bytes!(concat!("../assets/art_l_", $id, ".art")),
                m: include_bytes!(concat!("../assets/art_m_", $id, ".art")),
                s: include_bytes!(concat!("../assets/art_s_", $id, ".art")),
            },
            back: include_bytes!(concat!("../assets/back_", $id, ".art")),
            spine: $spine,
            blurb: ($en, $it),
        }
    };
}

/// A program whose art lives in its own repository: the same entry, with the
/// blobs read from the directory `build.rs` was given (`LAUNCHER_ART_DIR`). The
/// entry using it carries `#[cfg(art_<id>)]`, so a build without that program's
/// blobs has no entry and its case is plain.
#[allow(unused_macros)]
macro_rules! external_program {
    ($name:literal, $id:literal, $spine:literal, $en:literal, $it:literal) => {
        Program {
            name: $name,
            art: CoverArt {
                l: include_bytes!(concat!(env!("LAUNCHER_ART_DIR"), "/art_l_", $id, ".art")),
                m: include_bytes!(concat!(env!("LAUNCHER_ART_DIR"), "/art_m_", $id, ".art")),
                s: include_bytes!(concat!(env!("LAUNCHER_ART_DIR"), "/art_s_", $id, ".art")),
            },
            back: include_bytes!(concat!(env!("LAUNCHER_ART_DIR"), "/back_", $id, ".art")),
            spine: $spine,
            blurb: ($en, $it),
        }
    };
}

/// What a case shows.
#[derive(Clone, Copy)]
pub enum Cover {
    /// Slot `i` of [`PROGRAMS`].
    Art(usize),
    Plain,
}

/// Every program in this build. The index is the cover's slot in VRAM (fifteen
/// fit: the medium cover page holds 15, the large 18, the small 24).
pub static PROGRAMS: &[Program] = &[
    #[cfg(art_arcade)]
    external_program!(
        "PSOXIDE ARCADE",
        "arcade",
        "ARCADE",
        "Breakout, Space Invaders and Magikarp Pong, with a live CD-audio visualizer.",
        "Breakout, Space Invaders e Magikarp Pong, con visualizzatore CD audio."
    ),
    #[cfg(art_celeste)]
    external_program!(
        "CELESTE COLLECTION",
        "celeste",
        "CELESTE",
        "Both Celeste Classic games, rebuilt from PICO-8 as native PlayStation games.",
        "I due Celeste Classic, ricostruiti da PICO-8 come giochi nativi PlayStation."
    ),
    #[cfg(art_nitro)]
    external_program!(
        "NITROXIDE",
        "nitro",
        "NITROXIDE",
        "A Rocket League clone for the original PlayStation, with 2-player split screen.",
        "Un clone di Rocket League per PlayStation, con schermo diviso per 2 giocatori."
    ),
    #[cfg(art_voxide)]
    external_program!(
        "VOXIDE",
        "voxide",
        "VOXIDE",
        "A Minecraft clone for the original PlayStation: mining, crafting and survival.",
        "Un clone di Minecraft per PlayStation: scavo, crafting e sopravvivenza."
    ),
    #[cfg(art_quake)]
    external_program!(
        "QUAKE SHAREWARE",
        "quake",
        "QUAKE",
        "Quake 1.06 shareware for PS1: all nine maps, the full arsenal and Chthon.",
        "Quake 1.06 shareware per PS1: nove mappe, arsenale completo e Chthon."
    ),
    // The Public edition has one Cortex, so its case says what it is. The Private
    // edition carries both engines and says which one this is.
    #[cfg(all(not(feature = "private"), art_cortex))]
    external_program!(
        "CORTEX IGNITION",
        "cortex",
        "CORTEX",
        "A Souls-like built from scratch for the original PlayStation. Early tech demo.",
        "Un souls-like sviluppato da zero per PlayStation. Primo tech demo."
    ),
    #[cfg(all(feature = "private", art_cortex))]
    external_program!(
        "CORTEX IGNITION",
        "cortex",
        "CORTEX",
        "The old Cortex engine demo, 0.4b: a Souls-like built from scratch for PlayStation.",
        "La demo del vecchio motore di Cortex, 0.4b: un souls-like sviluppato da zero."
    ),
    #[cfg(art_psxcel)]
    external_program!(
        "PSXCEL",
        "psxcel",
        "PSXCEL",
        "A working Excel clone for PlayStation, played with a joypad: formulas, charts, saves.",
        "Un clone di Excel per PlayStation, col joypad: formule, grafici, salvataggi."
    ),
    #[cfg(all(feature = "private", art_wipeout))]
    external_program!(
        "WIPEOUT PSOXIDE",
        "wipeout",
        "WIPEOUT",
        "WipEout (1995) rewritten in Rust: tracks, ships, weapons and split screen.",
        "WipEout (1995) riscritto in Rust: piste, navi, armi e schermo diviso."
    ),
    #[cfg(all(feature = "private", art_hollow))]
    external_program!(
        "HOLLOW KNIGHT",
        "hollow",
        "HOLLOW KNIGHT",
        "An early port of Hollow Knight: King's Pass, Dirtmouth and the Forgotten Crossroads.",
        "Un primo port di Hollow Knight: King's Pass, Dirtmouth e il Crocevia Dimenticato."
    ),
    #[cfg(all(feature = "private", art_cstrike))]
    external_program!(
        "COUNTER-STRIKE",
        "cstrike",
        "COUNTER-STRIKE",
        "Counter-Strike 1.6 from scratch: bots, buy menu, bomb and hostage rounds, split screen.",
        "Counter-Strike 1.6 da zero: bot, acquisti, bomba, ostaggi e schermo diviso."
    ),
    #[cfg(all(feature = "private", art_halflife))]
    external_program!(
        "HALF-LIFE",
        "halflife",
        "HALF-LIFE",
        "Half-Life from scratch: the campaign is converted, not yet playable end to end.",
        "Half-Life da zero: campagna convertita, non ancora giocabile da capo a fondo."
    ),
    #[cfg(all(feature = "private", art_oot))]
    external_program!(
        "OCARINA OF TIME",
        "oot",
        "OCARINA OF TIME",
        "An early port of Ocarina of Time, cooked from your own ROM: the opening and Kokiri Forest.",
        "Un primo port di Ocarina of Time, dalla propria ROM: l'apertura e la Foresta Kokiri."
    ),
    #[cfg(feature = "private")]
    program!(
        "CORTEX CITADEL",
        "citadel",
        "CITADEL",
        "The new Cortex engine, tested on a small Citadel level. An engine test, not a game demo.",
        "Il nuovo motore di Cortex, provato su un piccolo livello della Cittadella. Solo un test."
    ),
    // The restored games' art is in their own repository (launcher-art/).
    #[cfg(all(feature = "private", art_x360))]
    external_program!(
        "360X",
        "x360",
        "360X",
        "Turn, thrust, fire and grab pods across alien planets. Restored from its original source.",
        "Ruota, spingi, spara e raccogli le capsule su pianeti alieni. Restaurato dai sorgenti originali."
    ),
    #[cfg(all(feature = "private", art_qdup))]
    external_program!(
        "QD UP",
        "qdup",
        "QD UP",
        "A pool game restored from its original source, on a 3D table or a flat 2D one.",
        "Un gioco di biliardo restaurato dai sorgenti originali, su tavolo 3D o in 2D."
    ),
];

/// The cover for the program the table calls `name`.
pub fn of(name: &str) -> Cover {
    PROGRAMS
        .iter()
        .position(|p| p.name == name)
        .map_or(Cover::Plain, Cover::Art)
}
