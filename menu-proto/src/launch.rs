//! The hand-off: the chosen case turns to the front, opens, the disc lifts
//! out and spins up into a flash. One timeline for all three directions; only
//! the case it starts from differs. 96 frames at 60 Hz.
//!
//! In the real launcher the chain-load can start reading at frame 0 and this
//! plays while the drive seeks; the flash then ends on the program's own
//! first frame instead of black.

use crate::common::*;
use crate::ease;
use crate::gfx::*;
use psx_math::{cos_q12, sin_q12};

pub const FRAMES: i32 = 96;

/// Where the case starts: centre x, bottom y, scale (Q8).
#[derive(Clone, Copy)]
pub struct From {
    pub cx: i32,
    pub bottom: i32,
    pub sc: i32,
}

fn disc(g: &mut Gfx, cx: i32, cy: i32, r: i32, phase: u16, bright: u32) {
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
        let (k2, _) = ((k + 1) % N, 0);
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
    g.ellipse_ring(cx, cy, r, r, scale((60, 130, 148), bright));
    g.ellipse_ring(cx, cy, label, label, scale((20, 120, 140), bright));
    g.ellipse_ring(cx, cy, label - 3, label - 3, scale((10, 50, 60), bright));
    // The hole shows the tray.
    for k in 0..N {
        let k2 = (k + 1) % N;
        g.tri_g([(cx, cy), pt(k, hole), pt(k2, hole)], [(3, 8, 11); 3]);
    }
}

/// `t` counts frames from the press. Draws over whatever the direction drew
/// (which hid its own selected case).
pub fn draw(g: &mut Gfx, t: i32, from: From, e: &crate::data::Entry, ctx: &Ctx) {
    // Veil the menu: subtract toward the dark stage (the menu stops being
    // drawn once the veil has taken it to black).
    let veil = ease::out(ease::span(t, 0, 28)) * 230 / 256;
    let v = veil as u8;
    g.quad_g_sub([(0, 0), (320, 0), (0, 240), (320, 240)], [(v, v, v); 4]);
    let rise = ease::out(ease::span(t, 0, 30)) as u32;

    // Case travels to the stage and grows a little.
    let move_t = ease::out(ease::span(t, 0, 26));
    let sc = ease::lerp(from.sc, 285, move_t);
    let cx = ease::lerp(from.cx, 160, move_t);
    let mut bottom = ease::lerp(from.bottom, 150, move_t);
    // Recede: after the disc leaves, the case slides back and down.
    let rec = ease::in_out(ease::span(t, 38, 60));
    bottom += (rec * 18) >> 8;
    let sc = ease::lerp(sc, 200, rec);
    let dim = ease::lerp(256, 0, rec);
    let (w, h) = ((112 * sc) >> 8, (102 * sc) >> 8);
    let x0 = cx - w / 2;
    let y0 = bottom - h;
    let (aw, ah) = ((L_W * sc) >> 8, (L_H * sc) >> 8);
    let pad = (4 * sc) >> 8;
    let d = dim as u32;

    if dim > 6 {
        // Tray inside the case.
        g.quad_g(
            [(x0, y0), (x0 + w, y0), (x0, y0 + h), (x0 + w, y0 + h)],
            [
                scale((9, 20, 27), d),
                scale((9, 20, 27), d),
                scale((4, 11, 15), d),
                scale((4, 11, 15), d),
            ],
        );
    }

    // One disc for the whole sequence: seated in the tray, lifted out of the
    // case, then flown to the middle where it spins up.
    let open = ease::out(ease::span(t, 14, 42));
    let lift = ease::in_out(ease::span(t, 34, 56));
    let fly = ease::in_out(ease::span(t, 50, 72));
    let seat = (x0 + w / 2, y0 + pad + ah / 2);
    let top = (seat.0, seat.1 - 54);
    let dcx = ease::lerp(ease::lerp(seat.0, top.0, lift), 160, fly);
    let dcy = ease::lerp(ease::lerp(seat.1, top.1, lift), 112, fly);
    let dr = ease::lerp(ease::lerp(ah * 40 / 100, ah * 40 / 100 + 18, lift), 52, fly);
    // Angular speed: a slow turn, then a quadratic ramp, then constant.
    let ramp = ease::quad_in(ease::span(t, 56, 86));
    let spin = (t * 16 + ((ramp * t) >> 3)) as u16 & 4095;
    if open > 10 {
        let glow = ease::span(t, 58, 92) as u32;
        if glow > 0 {
            g.glow(dcx, dcy, dr + 30, dr + 30, scale((12, 70, 84), glow));
        }
        disc(
            g,
            dcx,
            dcy,
            dr,
            spin,
            120 + (136 * ease::span(t, 14, 60) as u32 >> 8),
        );
        // Two echo rings behind the spin-up, as the shell's tray echoes.
        if t > 62 {
            for k in 0..2i32 {
                let e = ease::span(t - 62 - k * 5, 0, 20);
                if e > 0 && e < 256 {
                    let fade = (256 - e) as u32;
                    g.ellipse_ring(
                        dcx,
                        dcy,
                        dr + 4 + (e * 26 >> 8),
                        dr + 4 + (e * 26 >> 8),
                        scale((24, 90, 106), fade),
                    );
                }
            }
        }
    }

    if dim > 6 {
        // Cover swinging open on its left hinge: width shrinks to zero.
        let ang = ((open * 1024) >> 8) as u16; // 0..=90 degrees in Q12
        let cw = (aw * cos_q12(ang)) >> 12;
        let far = (ah * (256 + ((sin_q12(ang) * 22) >> 12))) >> 8;
        let hx = x0 + pad;
        let hy = y0 + pad + ah / 2;
        if open < 250 && cw > 1 {
            let tint = (128 * d / 256) as u32;
            let fade = (256 - (open * 190 >> 8)) as u32;
            let tt = (tint * fade >> 8) as u8;
            let q = [
                (hx, hy - ah / 2),
                (hx + cw, hy - far / 2),
                (hx, hy + ah / 2),
                (hx + cw, hy + far / 2),
            ];
            match e.art {
                Some(i) => g.art_strips(
                    Size::L,
                    i,
                    hx,
                    hx + cw,
                    (q[0].1, q[2].1),
                    (q[1].1, q[3].1),
                    (tt, tt, tt),
                ),
                None => g.quad_g(q, [scale((14, 54, 66), d); 4]),
            }
            g.frame(hx, hy - ah / 2, cw.max(1), ah, scale(FOCUS, d * fade / 256));
        }
        g.frame(x0, y0, w, h, scale(FOCUS, d));
        g.brackets(x0 - 2, y0 - 2, w + 4, h + 4, 7, scale(FOCUS, d));
    }

    // Stage glow rises under it all, added over the case so a dimmed case
    // never leaves a dark box on the floor.
    g.glow(160, 150, 150, 26, scale((10, 58, 70), rise));

    // The name and a status line under the disc, as the program loads.
    let lab = ease::out(ease::span(t, 56, 72)) as u32;
    if lab > 8 {
        crate::text::draw_centered(
            g,
            &crate::ui_atlas::MICH12,
            160,
            184,
            e.name,
            scale(TEXT, lab),
            0,
        );
        let msg = if ctx.it { "Caricamento" } else { "Loading" };
        crate::text::draw_centered(
            g,
            &crate::ui_atlas::JOST9,
            160,
            202,
            msg,
            scale(MUTED, lab),
            0,
        );
    }

    // Flash to the program, then black.
    let fl = ease::span(t, 78, 92) as u32;
    if fl > 0 {
        let c = scale((200, 244, 255), fl);
        g.quad_g_add([(0, 0), (320, 0), (0, 240), (320, 240)], [c; 4]);
    }
    let out = ease::span(t, 90, 96);
    if out > 0 {
        let v = (out * 255 / 256) as u8;
        g.quad_g_sub([(0, 0), (320, 0), (0, 240), (320, 240)], [(v, v, v); 4]);
    }
}
