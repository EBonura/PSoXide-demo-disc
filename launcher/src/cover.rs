//! Cover art: which pictures go on which program's case.
//!
//! The pictures are baked into the launcher by `tools/menu-art` (three sizes
//! of each, cut from `assets/shots`, and the attract mode's back-cover shots)
//! and found by the program's name in the disc table. A program with no
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

/// What a case shows.
#[derive(Clone, Copy)]
pub enum Cover {
    /// Slot `i` of [`PROGRAMS`].
    Art(usize),
    Plain,
}

/// Every program in this build. The index is the cover's slot in VRAM (twelve
/// fit).
pub static PROGRAMS: &[Program] = &[
    program!(
        "PSOXIDE ARCADE",
        "arcade",
        "ARCADE",
        "Breakout, Space Invaders and Magikarp Pong, with a live CD-audio visualizer.",
        "Breakout, Space Invaders e Magikarp Pong, con visualizzatore CD audio."
    ),
    program!(
        "CELESTE COLLECTION",
        "celeste",
        "CELESTE",
        "Both Celeste Classic games, rebuilt from PICO-8 as native PlayStation games.",
        "I due Celeste Classic, ricostruiti da PICO-8 come giochi nativi PlayStation."
    ),
    program!(
        "NITROXIDE",
        "nitro",
        "NITROXIDE",
        "A Rocket League clone for the original PlayStation, with 2-player split screen.",
        "Un clone di Rocket League per PlayStation, con schermo diviso per 2 giocatori."
    ),
    program!(
        "VOXIDE",
        "voxide",
        "VOXIDE",
        "A Minecraft clone for the original PlayStation: mining, crafting and survival.",
        "Un clone di Minecraft per PlayStation: scavo, crafting e sopravvivenza."
    ),
    program!(
        "QUAKE SHAREWARE",
        "quake",
        "QUAKE",
        "Quake 1.06 shareware for PS1: all nine maps, the full arsenal and Chthon.",
        "Quake 1.06 shareware per PS1: nove mappe, arsenale completo e Chthon."
    ),
    program!(
        "CORTEX IGNITION",
        "cortex",
        "CORTEX",
        "A Souls-like built from scratch for the original PlayStation. Early tech demo.",
        "Un souls-like sviluppato da zero per PlayStation. Primo tech demo."
    ),
    program!(
        "PSXCEL",
        "psxcel",
        "PSXCEL",
        "A working Excel clone for PlayStation, played with a joypad: formulas, charts, saves.",
        "Un clone di Excel per PlayStation, col joypad: formule, grafici, salvataggi."
    ),
    #[cfg(feature = "private")]
    program!(
        "WIPEOUT PSOXIDE",
        "wipeout",
        "WIPEOUT",
        "WipEout (1995) rewritten in Rust: tracks, ships, weapons and split screen.",
        "WipEout (1995) riscritto in Rust: piste, navi, armi e schermo diviso."
    ),
    #[cfg(feature = "private")]
    program!(
        "HOLLOW KNIGHT",
        "hollow",
        "HOLLOW KNIGHT",
        "An early port of Hollow Knight: King's Pass, Dirtmouth and the Forgotten Crossroads.",
        "Un primo port di Hollow Knight: King's Pass, Dirtmouth e il Crocevia Dimenticato."
    ),
    #[cfg(feature = "private")]
    program!(
        "COUNTER-STRIKE",
        "cstrike",
        "COUNTER-STRIKE",
        "Counter-Strike 1.6 from scratch: bots, buy menu, bomb and hostage rounds, split screen.",
        "Counter-Strike 1.6 da zero: bot, acquisti, bomba, ostaggi e schermo diviso."
    ),
    #[cfg(feature = "private")]
    program!(
        "HALF-LIFE",
        "halflife",
        "HALF-LIFE",
        "Half-Life from scratch: the campaign is converted, not yet playable end to end.",
        "Half-Life da zero: campagna convertita, non ancora giocabile da capo a fondo."
    ),
    #[cfg(feature = "private")]
    program!(
        "OCARINA OF TIME",
        "oot",
        "OCARINA OF TIME",
        "An early port of Ocarina of Time, cooked from your own ROM: the opening and Kokiri Forest.",
        "Un primo port di Ocarina of Time, dalla propria ROM: l'apertura e la Foresta Kokiri."
    ),
];

/// The cover for the program the table calls `name`.
pub fn of(name: &str) -> Cover {
    PROGRAMS
        .iter()
        .position(|p| p.name == name)
        .map_or(Cover::Plain, Cover::Art)
}
