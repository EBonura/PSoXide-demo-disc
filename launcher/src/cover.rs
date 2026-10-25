//! Cover art: which picture goes on which program's case.
//!
//! The pictures are baked into the launcher by `tools/menu-art` (three sizes
//! of each, cut from `assets/shots`) and found by the program's name in the
//! disc table. A program with no picture here gets a plain cover, so a new
//! lineup entry still shows and starts.
//!
//! The Private programs' art and names exist only in a build with the
//! `private` feature: a Public disc must not carry them anywhere.

use crate::gfx::CoverArt;

macro_rules! art {
    ($id:literal) => {
        CoverArt {
            l: include_bytes!(concat!("../assets/art_l_", $id, ".art")),
            m: include_bytes!(concat!("../assets/art_m_", $id, ".art")),
            s: include_bytes!(concat!("../assets/art_s_", $id, ".art")),
        }
    };
}

/// What a case shows.
#[derive(Clone, Copy)]
pub enum Cover {
    /// Slot `i` of [`ARTS`].
    Art(usize),
    Plain,
}

/// Every cover in this build, by the name the disc table carries. The index is
/// the cover's slot in VRAM (twelve fit).
pub static ARTS: &[(&str, CoverArt)] = &[
    ("PSOXIDE ARCADE", art!("arcade")),
    ("CELESTE COLLECTION", art!("celeste")),
    ("NITROXIDE", art!("nitro")),
    ("VOXIDE", art!("voxide")),
    ("QUAKE SHAREWARE", art!("quake")),
    ("CORTEX IGNITION", art!("cortex")),
    ("PSXCEL", art!("psxcel")),
    #[cfg(feature = "private")]
    ("WIPEOUT PSOXIDE", art!("wipeout")),
    #[cfg(feature = "private")]
    ("HOLLOW KNIGHT", art!("hollow")),
    #[cfg(feature = "private")]
    ("COUNTER-STRIKE", art!("cstrike")),
    #[cfg(feature = "private")]
    ("HALF-LIFE", art!("halflife")),
    #[cfg(feature = "private")]
    ("OCARINA OF TIME", art!("oot")),
];

/// The cover for the program the table calls `name`.
pub fn of(name: &str) -> Cover {
    ARTS.iter()
        .position(|(known, _)| *known == name)
        .map_or(Cover::Plain, Cover::Art)
}
