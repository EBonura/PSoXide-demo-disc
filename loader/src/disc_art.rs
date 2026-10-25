//! The spinning disc the launch animation ends on and the chain loader keeps
//! turning.
//!
//! One source, two builds: the launcher draws it through its `Gfx` and the
//! loader blob through raw GP0 packets, both via [`Canvas`], so the frame the
//! launcher hands over and the frame the loader draws first are the same
//! pixels by construction, not by two copies agreeing. The launcher includes
//! this file with `#[path]`.
//!
//! Integer maths only, no floats and no 64-bit values.

use psx_math::{cos_q12, sin_q12};

pub type Rgb = (u8, u8, u8);

/// The few primitives the disc is made of. Opaque ones overwrite; the `_add`
/// ones add to what is under them.
pub trait Canvas {
    fn quad_g(&mut self, v: [(i32, i32); 4], c: [Rgb; 4]);
    fn tri_g(&mut self, v: [(i32, i32); 3], c: [Rgb; 3]);
    fn tri_g_add(&mut self, v: [(i32, i32); 3], c: [Rgb; 3]);
    fn line_add(&mut self, a: (i32, i32), b: (i32, i32), ca: Rgb, cb: Rgb);
}

/// Centre and radius of the settled disc, in screen pixels.
pub const CX: i32 = 160;
pub const CY: i32 = 112;
pub const R: i32 = 52;

/// Glow around the settled disc, and the stage glow under the case.
pub const DISC_GLOW: Rgb = (12, 70, 84);
pub const DISC_GLOW_REACH: i32 = 30;
pub const STAGE_GLOW: Rgb = (10, 58, 70);

/// Launch frame the hand-off to the loader happens on. The disc has flown to
/// the middle by then; its spin-up and glow are functions of the frame number
/// ([`spin`], [`glow_level`]), so the loader finishes them where the launch
/// left off.
pub const HAND_OFF: i32 = 72;
/// The launch frame the display last showed. The launcher's swap shows a frame
/// one VBlank after it is drawn, and it boots the loader right after drawing
/// frame `HAND_OFF`, so that frame never reaches the screen: the last picture
/// is the one before it, and the loader's first frame must match that.
pub const LAST_SHOWN: i32 = HAND_OFF - 1;
/// The spin-up ramps over this window of launch frames, then runs at
/// 48 Q12 turns a frame for good.
const RAMP_FROM: i32 = 54;
const RAMP_TO: i32 = 84;
const GLOW_FROM: i32 = 56;
const GLOW_TO: i32 = 84;

/// The sheen phase on launch frame `t`: a slow turn, then a quadratic ramp,
/// then constant. From [`RAMP_TO`] on it is `48 * t`.
pub fn spin(t: i32) -> u16 {
    let x = ((t - RAMP_FROM) * 256 / (RAMP_TO - RAMP_FROM)).clamp(0, 256);
    let ramp = (x * x) >> 8;
    (t * 16 + ((ramp * t) >> 3)) as u16 & 4095
}

/// The disc glow's brightness on launch frame `t`, 0..=256: it comes up over
/// the spin-up.
pub fn glow_level(t: i32) -> u32 {
    ((t - GLOW_FROM) * 256 / (GLOW_TO - GLOW_FROM)).clamp(0, 256) as u32
}

pub fn scale(c: Rgb, num: u32) -> Rgb {
    // num: 0..=256
    (
        ((c.0 as u32 * num) >> 8) as u8,
        ((c.1 as u32 * num) >> 8) as u8,
        ((c.2 as u32 * num) >> 8) as u8,
    )
}

/// A soft glow: an additive fan from `c` at the centre to black at the rim.
pub fn glow<C: Canvas>(g: &mut C, cx: i32, cy: i32, rx: i32, ry: i32, c: Rgb) {
    const N: u16 = 16;
    let ring = |k: u16| {
        let a = k * (4096 / N);
        (
            cx + ((rx * sin_q12(a)) >> 12),
            cy - ((ry * cos_q12(a)) >> 12),
        )
    };
    let mut prev = ring(0);
    for k in 1..=N {
        let cur = ring(k % N);
        g.tri_g_add([(cx, cy), prev, cur], [c, (0, 0, 0), (0, 0, 0)]);
        prev = cur;
    }
}

/// An ellipse outline as short additive lines.
pub fn ellipse_ring<C: Canvas>(g: &mut C, cx: i32, cy: i32, rx: i32, ry: i32, c: Rgb) {
    const N: u16 = 24;
    let pt = |k: u16| {
        let a = k * (4096 / N);
        (
            cx + ((rx * sin_q12(a)) >> 12),
            cy - ((ry * cos_q12(a)) >> 12),
        )
    };
    let mut prev = pt(0);
    for k in 1..=N {
        let cur = pt(k % N);
        g.line_add(prev, cur, c, c);
        prev = cur;
    }
}

/// The disc: a Gouraud-shaded ring with two sheen lobes riding `phase`, the
/// label ring, and the hub hole. `bright` is 0..=256.
pub fn disc<C: Canvas>(g: &mut C, cx: i32, cy: i32, r: i32, phase: u16, bright: u32) {
    const N: u16 = 24;
    let hole = r * 17 / 100;
    let label = r * 56 / 100;
    let pt = |k: u16, rad: i32| {
        let a = k * (4096 / N);
        (
            cx + ((rad * sin_q12(a)) >> 12),
            cy - ((rad * cos_q12(a)) >> 12),
        )
    };
    let shade = |k: u16| -> Rgb {
        // Two opposite sheen lobes riding the phase, as light on a CD.
        let a = (k * (4096 / N)).wrapping_sub(phase) & 4095;
        let l = ((cos_q12((a * 2) & 4095) + 4096) >> 5) as u32; // 0..=255
        let l2 = (l * l) >> 8;
        let base = (20u32, 34u32, 42u32);
        let add = (l2 * 150) >> 8;
        scale(
            (
                (base.0 + add * 7 / 10).min(255) as u8,
                (base.1 + add * 9 / 10).min(255) as u8,
                (base.2 + add).min(255) as u8,
            ),
            bright,
        )
    };
    for k in 0..N {
        let k2 = (k + 1) % N;
        g.quad_g(
            [pt(k, r), pt(k2, r), pt(k, hole), pt(k2, hole)],
            [
                shade(k),
                shade(k2),
                scale(shade(k), 150),
                scale(shade(k2), 150),
            ],
        );
    }
    ellipse_ring(g, cx, cy, r, r, scale((60, 130, 148), bright));
    ellipse_ring(g, cx, cy, label, label, scale((20, 120, 140), bright));
    ellipse_ring(g, cx, cy, label - 3, label - 3, scale((10, 50, 60), bright));
    // The hole shows the tray.
    for k in 0..N {
        let k2 = (k + 1) % N;
        g.tri_g(
            [(cx, cy), pt(k, hole), pt(k2, hole)],
            [scale((3, 8, 11), bright); 3],
        );
    }
}

/// The glow around the disc at its settled place, `level` 0..=256.
pub fn disc_glow<C: Canvas>(g: &mut C, level: u32) {
    let reach = R + DISC_GLOW_REACH;
    glow(g, CX, CY, reach, reach, scale(DISC_GLOW, level));
}

/// The light on the stage floor under the whole scene, `level` 0..=256. Drawn
/// last, over the disc.
pub fn stage_glow<C: Canvas>(g: &mut C, level: u32) {
    glow(g, 160, 150, 150, 26, scale(STAGE_GLOW, level));
}
