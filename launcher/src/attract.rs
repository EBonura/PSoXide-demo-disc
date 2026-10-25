//! The idle menu's attract state.
//!
//! A demo disc spends most of its life unattended. After [`AFTER`] frames with
//! no button the text and hints fade out and the shelf drifts: the focused
//! case sways wider, the row creeps a fifth of a case either way, the other
//! cases dim, and the music credit takes the footer's place. Any button ends it
//! on the spot.
//!
//! Everything the rest of the menu knows about attract mode is in this file
//! and the one number it exposes, [`Attract::level`] (0 = the menu as it is
//! used, 256 = fully idle, carried to the drawing code as `Ctx::attract`).
//! A richer attract sequence replaces this module and keeps those entry points.

use crate::common::*;
use crate::gfx::*;
use crate::text;
use crate::ui_atlas::*;
use psx_math::sin_q12;

/// Idle frames before the menu settles into its attract state (20 s).
pub const AFTER: u32 = 60 * 20;
/// How fast the state comes in, per frame out of 256 (about a second).
const STEP: i32 = 4;

/// Where the menu stands between used and idle.
pub struct Attract {
    level: i32,
}

impl Attract {
    pub const fn new() -> Self {
        Attract { level: 0 }
    }

    /// Call once a frame. `idle` counts frames since the last button; `live`
    /// is false whenever the menu is not the plain shelf (a launch, the credits
    /// screen), which cancels the state.
    pub fn update(&mut self, idle: u32, live: bool) {
        self.level = if live && idle > AFTER {
            (self.level + STEP).min(256)
        } else {
            0
        };
    }

    pub fn level(&self) -> i32 {
        self.level
    }
}

/// How far the row has crept from its resting place, in Q8 case units.
pub fn row_creep(t: u32, level: i32) -> i32 {
    ((sin_q12((t * 3) as u16 & 4095) * 52) >> 12) * level >> 8
}

/// The focused case's sway scaled for the idle state: the same hinge swing,
/// up to three times as wide.
pub fn sway(base: i32, level: i32) -> i32 {
    (base * (256 + level * 2)) >> 8
}

/// How much of its light a case keeps. `settle` is its focus (256 = the
/// focused case, which keeps all of it).
pub fn dim(dim: i32, level: i32, settle: i32) -> i32 {
    dim * (256 - (level * 3 / 4) * (256 - settle) / 256) >> 8
}

/// The music credit, fading in where the hints were, so the licence line is on
/// screen whenever the menu is left alone.
pub fn credit_line(g: &mut Gfx, ctx: &Ctx) {
    if ctx.attract > 8 && !ctx.credit.is_empty() {
        let tint = scale(MUTED, ctx.attract as u32);
        text::draw_centered_mid(g, &JOST9, 160, FOOT_MID, ctx.credit, tint, 0);
    }
}
