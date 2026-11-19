//! Cover art: which pictures go on which program's case.
//!
//! The pictures are baked into the launcher by `tools/menu-art` (three sizes
//! of each, and the attract mode's back-cover shots) and found by the
//! program's name in the disc table. Each program's own repository carries
//! them in `launcher-art/`; `build.rs` includes the ones it was given and
//! writes them into the generated table (`art.rs`) that this file filters.
//! Cortex Citadel's are still in `launcher/assets`. A program with no picture
//! gets a plain cover, so a new lineup entry still shows and starts.
//!
//! The Private programs' art and names exist only in a build with the
//! `private` feature: a Public disc must not carry them anywhere.

use crate::gfx::{CoverArt, L_PAGES, M_H, M_W, SHOTS_PER_PAGE, SHOT_PAGES};

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

/// The blobs of one program's art: the front at three sizes and the back cover.
#[derive(Clone, Copy)]
struct Blobs {
    l: &'static [u8],
    m: &'static [u8],
    s: &'static [u8],
    back: &'static [u8],
}

// `IDS` (the programs in the table's fixed order) and `ART`, the blobs of each
// that were there when `build.rs` ran, or `None`. No cfg flag says which: the
// launcher's code and symbols are the same whatever art a build was given.
include!(concat!(env!("OUT_DIR"), "/art.rs"));

/// Which editions show an entry.
#[derive(Clone, Copy)]
enum Vis {
    Both,
    Public,
    Private,
}

/// One entry of the table: the words printed on the case. The art comes from
/// `ART` at the same index.
struct Entry {
    id: &'static str,
    vis: Vis,
    name: &'static str,
    spine: &'static str,
    blurb: (&'static str, &'static str),
}

/// What a case shows.
#[derive(Clone, Copy)]
pub enum Cover {
    /// Slot `i` of [`PROGRAMS`].
    Art(usize),
    Plain,
}

/// Every program the launcher knows, in the order the shelf shows them. The
/// same ids in the same order as `build.rs`'s list (checked below). An entry
/// is in [`PROGRAMS`] when this edition shows it and its art was supplied.
const ALL: [Entry; 16] = [
    Entry {
        id: "arcade",
        vis: Vis::Both,
        name: "PSOXIDE ARCADE",
        spine: "ARCADE",
        blurb: ("Breakout, Space Invaders and Magikarp Pong, with a live CD-audio visualizer.", "Breakout, Space Invaders e Magikarp Pong, con visualizzatore CD audio."),
    },
    Entry {
        id: "celeste",
        vis: Vis::Both,
        name: "CELESTE COLLECTION",
        spine: "CELESTE",
        blurb: ("Both Celeste Classic games, rebuilt from PICO-8 as native PlayStation games.", "I due Celeste Classic, ricostruiti da PICO-8 come giochi nativi PlayStation."),
    },
    Entry {
        id: "nitro",
        vis: Vis::Both,
        name: "NITROXIDE",
        spine: "NITROXIDE",
        blurb: ("A Rocket League clone for the original PlayStation, with 2-player split screen.", "Un clone di Rocket League per PlayStation, con schermo diviso per 2 giocatori."),
    },
    Entry {
        id: "voxide",
        vis: Vis::Both,
        name: "VOXIDE",
        spine: "VOXIDE",
        blurb: ("A Minecraft clone for the original PlayStation: mining, crafting and survival.", "Un clone di Minecraft per PlayStation: scavo, crafting e sopravvivenza."),
    },
    Entry {
        id: "quake",
        vis: Vis::Both,
        name: "QUAKE SHAREWARE",
        spine: "QUAKE",
        blurb: ("Quake 1.06 shareware for PS1: all nine maps, the full arsenal and Chthon.", "Quake 1.06 shareware per PS1: nove mappe, arsenale completo e Chthon."),
    },
    // The Public edition has one Cortex, so its case says what it is. The Private
    // edition carries both engines and says which one this is.
    Entry {
        id: "cortex",
        vis: Vis::Public,
        name: "CORTEX IGNITION",
        spine: "CORTEX",
        blurb: ("A Souls-like built from scratch for the original PlayStation. Early tech demo.", "Un souls-like sviluppato da zero per PlayStation. Primo tech demo."),
    },
    Entry {
        id: "cortex",
        vis: Vis::Private,
        name: "CORTEX IGNITION",
        spine: "CORTEX",
        blurb: ("The old Cortex engine demo, 0.4b: a Souls-like built from scratch for PlayStation.", "La demo del vecchio motore di Cortex, 0.4b: un souls-like sviluppato da zero."),
    },
    Entry {
        id: "psxcel",
        vis: Vis::Both,
        name: "PSXCEL",
        spine: "PSXCEL",
        blurb: ("A working Excel clone for PlayStation, played with a joypad: formulas, charts, saves.", "Un clone di Excel per PlayStation, col joypad: formule, grafici, salvataggi."),
    },
    Entry {
        id: "wipeout",
        vis: Vis::Private,
        name: "WIPEOUT PSOXIDE",
        spine: "WIPEOUT",
        blurb: ("WipEout (1995) rewritten in Rust: tracks, ships, weapons and split screen.", "WipEout (1995) riscritto in Rust: piste, navi, armi e schermo diviso."),
    },
    Entry {
        id: "hollow",
        vis: Vis::Private,
        name: "HOLLOW KNIGHT",
        spine: "HOLLOW KNIGHT",
        blurb: ("An early port of Hollow Knight: King's Pass, Dirtmouth and the Forgotten Crossroads.", "Un primo port di Hollow Knight: King's Pass, Dirtmouth e il Crocevia Dimenticato."),
    },
    Entry {
        id: "cstrike",
        vis: Vis::Private,
        name: "COUNTER-STRIKE",
        spine: "COUNTER-STRIKE",
        blurb: ("Counter-Strike 1.6 from scratch: bots, buy menu, bomb and hostage rounds, split screen.", "Counter-Strike 1.6 da zero: bot, acquisti, bomba, ostaggi e schermo diviso."),
    },
    Entry {
        id: "halflife",
        vis: Vis::Private,
        name: "HALF-LIFE",
        spine: "HALF-LIFE",
        blurb: ("Half-Life from scratch: the campaign is converted, not yet playable end to end.", "Half-Life da zero: campagna convertita, non ancora giocabile da capo a fondo."),
    },
    Entry {
        id: "oot",
        vis: Vis::Private,
        name: "OCARINA OF TIME",
        spine: "OCARINA OF TIME",
        blurb: ("An early port of Ocarina of Time, cooked from your own ROM: the opening and Kokiri Forest.", "Un primo port di Ocarina of Time, dalla propria ROM: l'apertura e la Foresta Kokiri."),
    },
    Entry {
        id: "citadel",
        vis: Vis::Private,
        name: "CORTEX CITADEL",
        spine: "CITADEL",
        blurb: ("The new Cortex engine, tested on a small Citadel level. An engine test, not a game demo.", "Il nuovo motore di Cortex, provato su un piccolo livello della Cittadella. Solo un test."),
    },
    Entry {
        id: "x360",
        vis: Vis::Private,
        name: "360X",
        spine: "360X",
        blurb: ("Turn, thrust, fire and grab pods across alien planets. Restored from its original source.", "Ruota, spingi, spara e raccogli le capsule su pianeti alieni. Restaurato dai sorgenti originali."),
    },
    Entry {
        id: "qdup",
        vis: Vis::Private,
        name: "QD UP",
        spine: "QD UP",
        blurb: ("A pool game restored from its original source, on a 3D table or a flat 2D one.", "Un gioco di biliardo restaurato dai sorgenti originali, su tavolo 3D o in 2D."),
    },
];

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const _: () = {
    assert!(IDS.len() == ALL.len(), "build.rs and cover.rs list different programs");
    let mut i = 0;
    while i < ALL.len() {
        assert!(same(ALL[i].id, IDS[i]), "build.rs and cover.rs list the programs in a different order");
        i += 1;
    }
};

const fn shown(i: usize) -> bool {
    let edition = match ALL[i].vis {
        Vis::Both => true,
        Vis::Public => !cfg!(feature = "private"),
        Vis::Private => cfg!(feature = "private"),
    };
    edition && ART[i].is_some()
}

const COUNT: usize = {
    let (mut i, mut n) = (0, 0);
    while i < ALL.len() {
        if shown(i) {
            n += 1;
        }
        i += 1;
    }
    n
};

const EMPTY: Program = Program {
    name: "",
    art: CoverArt { l: &[], m: &[], s: &[] },
    back: &[],
    spine: "",
    blurb: ("", ""),
};

const fn table() -> [Program; COUNT] {
    let mut out = [EMPTY; COUNT];
    let (mut i, mut n) = (0, 0);
    while i < ALL.len() {
        if shown(i) {
            if let Some(b) = ART[i] {
                out[n] = Program {
                    name: ALL[i].name,
                    art: CoverArt { l: b.l, m: b.m, s: b.s },
                    back: b.back,
                    spine: ALL[i].spine,
                    blurb: ALL[i].blurb,
                };
            }
            n += 1;
        }
        i += 1;
    }
    out
}

/// How many covers the launcher's VRAM pages hold: the large pages take six
/// each, the medium page as many as fit in 256x256, the small page 24 and the
/// back-cover pages two shots per program. A table past it does not build.
pub const MAX_COVERS: usize = {
    let l = L_PAGES.len() * 6;
    let m = (256 / M_W as usize) * (256 / M_H as usize);
    let back = SHOT_PAGES.len() * SHOTS_PER_PAGE / 2;
    let mut max = if l < m { l } else { m };
    if back < max {
        max = back;
    }
    max
};
const _: () = assert!(COUNT <= MAX_COVERS, "more covers than the launcher's VRAM pages hold");

const TABLE: [Program; COUNT] = table();

/// Every program in this build. The index is the cover's slot in VRAM.
pub static PROGRAMS: &[Program] = &TABLE;

/// The cover for the program the table calls `name`.
pub fn of(name: &str) -> Cover {
    PROGRAMS
        .iter()
        .position(|p| p.name == name)
        .map_or(Cover::Plain, Cover::Art)
}
