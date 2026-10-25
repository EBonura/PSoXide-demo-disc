//! The entries the prototype menu shows: the same names, versions and
//! descriptions the real disc table carries (Makefile `--describe` strings),
//! for the Public and the Private pressing.

use crate::gfx::CoverArt;

macro_rules! art {
    ($id:literal) => {
        CoverArt {
            l: include_bytes!(concat!("../assets/art_l_", $id, ".bin")),
            m: include_bytes!(concat!("../assets/art_m_", $id, ".bin")),
            s: include_bytes!(concat!("../assets/art_s_", $id, ".bin")),
        }
    };
}

/// Cover art in slot order.
pub static ARTS: [CoverArt; 11] = [
    art!("arcade"),
    art!("celeste"),
    art!("nitro"),
    art!("voxide"),
    art!("quake"),
    art!("cortex"),
    art!("psxcel"),
    art!("wipeout"),
    art!("hollow"),
    art!("cstrike"),
    art!("halflife"),
];

pub struct Entry {
    pub name: &'static str,
    #[allow(dead_code)]
    pub short: &'static str,
    pub version: &'static str,
    pub en: &'static str,
    pub it: &'static str,
    /// Slot in [`ARTS`], or none for the credits card.
    pub art: Option<usize>,
}

pub const ARCADE: Entry = Entry {
    name: "PSOXIDE ARCADE",
    short: "Arcade",
    version: "0.1.1",
    en: "Three complete native PlayStation arcade games in one collection: Breakout, Space Invaders and Magikarp Pong, with its own live CD-audio visualizer.",
    it: "Tre giochi arcade completi e nativi per PlayStation in una raccolta: Breakout, Space Invaders e Magikarp Pong, con visualizzatore CD audio.",
    art: Some(0),
};
pub const CELESTE: Entry = Entry {
    name: "CELESTE COLLECTION",
    short: "Celeste",
    version: "0.2.7",
    en: "Both Celeste Classic games, rebuilt from PICO-8 as native PlayStation games.",
    it: "I due Celeste Classic, ricostruiti da PICO-8 come giochi nativi PlayStation.",
    art: Some(1),
};
pub const NITRO: Entry = Entry {
    name: "NITROXIDE",
    short: "NitroXide",
    version: "0.4.1",
    en: "A Rocket League clone built for the original PlayStation. Supports 2 players in split screen.",
    it: "Un clone di Rocket League per la prima PlayStation. Supporta 2 giocatori a schermo diviso.",
    art: Some(2),
};
pub const VOXIDE: Entry = Entry {
    name: "VOXIDE",
    short: "VoXide",
    version: "0.4.1",
    en: "A Minecraft clone built for the original PlayStation. This is an early playable build: world generation, mining, crafting and survival work.",
    it: "Un clone di Minecraft per la prima PlayStation. Prima versione giocabile: generazione del mondo, scavo, crafting e sopravvivenza funzionano.",
    art: Some(3),
};
pub const QUAKE: Entry = Entry {
    name: "QUAKE SHAREWARE",
    short: "Quake",
    version: "0.1.1",
    en: "Quake 1.06 shareware for PS1, built on PSoXide with id's GPL source and the QuakePSX C port as references. All nine maps, the full arsenal and Chthon. Still in development.",
    it: "Quake 1.06 shareware per PS1, costruito su PSoXide usando il sorgente GPL di id e il port C QuakePSX come riferimenti. Nove mappe, arsenale completo e Chthon. In sviluppo.",
    art: Some(4),
};
pub const CORTEX: Entry = Entry {
    name: "CORTEX IGNITION",
    short: "Cortex",
    version: "Graybox Reach",
    en: "A Souls-like built from the ground up for the original PlayStation. This is an early tech demo.",
    it: "Un souls-like sviluppato da zero per la prima PlayStation. Questo e un primo tech demo.",
    art: Some(5),
};
pub const PSXCEL: Entry = Entry {
    name: "PSXCEL",
    short: "PSXcel",
    version: "0.2.0",
    en: "A working Microsoft Excel clone for the original PlayStation, controlled with a joypad. This build is fully functional, with formulas, charts, themes and memory-card saves.",
    it: "Un clone funzionante di Microsoft Excel per PlayStation, controllato col joypad. Questa versione e completa e include formule, grafici, temi e salvataggi su memory card.",
    art: Some(6),
};
pub const CREDITS: Entry = Entry {
    name: "CREDITS AND LINKS",
    short: "Credits",
    version: "",
    en: "Who made what is on the disc, the menu music credit (Mulula VGM Loops, CC BY 4.0) and every link, with a QR code.",
    it: "Chi ha fatto cosa sul disco, il credito della musica del menu (Mulula VGM Loops, CC BY 4.0) e tutti i link, con un codice QR.",
    art: None,
};
pub const WIPEOUT: Entry = Entry {
    name: "WIPEOUT PSOXIDE",
    short: "WipEout",
    version: "0.1.0",
    en: "WipEout (1995) rewritten in Rust for the original PlayStation, cooked from your own copy of the game. Tracks, ships, weapons and two-player split screen.",
    it: "WipEout (1995) riscritto in Rust per la prima PlayStation, partendo dalla propria copia del gioco. Piste, navi, armi e schermo diviso per due.",
    art: Some(7),
};
pub const HOLLOW: Entry = Entry {
    name: "HOLLOW KNIGHT",
    short: "Hollow Knight",
    version: "0.1.0",
    en: "An early from-scratch PlayStation port of Hollow Knight, cooked from your own copy. King's Pass, Dirtmouth and the Forgotten Crossroads can be explored, with enemies, benches and saves.",
    it: "Un primo port di Hollow Knight per PlayStation, fatto da zero partendo dalla propria copia. Si esplorano King's Pass, Dirtmouth e il Crocevia Dimenticato, con nemici, panchine e salvataggi.",
    art: Some(8),
};
pub const CSTRIKE: Entry = Entry {
    name: "COUNTER-STRIKE",
    short: "Counter-Strike",
    version: "0.2.0",
    en: "A from-scratch PlayStation port of Counter-Strike 1.6, cooked from your own copy. Bots, the buy menu, bomb and hostage rounds, and two-player split screen. Still in development.",
    it: "Counter-Strike 1.6 portato su PlayStation da zero, partendo dalla propria copia del gioco. Bot, menu acquisti, bomba, ostaggi e schermo diviso per due giocatori. In sviluppo.",
    art: Some(9),
};
pub const HALFLIFE: Entry = Entry {
    name: "HALF-LIFE",
    short: "Half-Life",
    version: "0.2.0",
    en: "A from-scratch PlayStation port of Half-Life. The full campaign has been converted and much of the game works, but it is not yet playable from start to finish.",
    it: "Half-Life portato su PlayStation da zero. L'intera campagna e stata convertita e gran parte del gioco funziona, ma non e ancora giocabile dall'inizio alla fine.",
    art: Some(10),
};

/// The v0.44 public pressing, in carousel order.
pub static PUBLIC: [&Entry; 8] = [
    &ARCADE, &CELESTE, &NITRO, &VOXIDE, &QUAKE, &CORTEX, &PSXCEL, &CREDITS,
];

/// The private pressing: the same list with the four bring-your-own-assets
/// ports (the hardware tests have their own disc).
pub static PRIVATE: [&Entry; 12] = [
    &ARCADE, &CELESTE, &NITRO, &VOXIDE, &WIPEOUT, &HOLLOW, &CSTRIKE, &HALFLIFE, &QUAKE, &CORTEX,
    &PSXCEL, &CREDITS,
];
