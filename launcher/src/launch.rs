//! The hand-off: the chosen case comes to the middle of the stage, its cover
//! swings open on the left hinge like a real jewel case, the disc lifts out
//! of the tray and spins up into a flash. 96 frames at 60 Hz.
//!
//! The cover is a real 3D panel. Its corners and a few points between them
//! are rotated about the hinge and perspective-projected by the GTE, then
//! drawn as a ribbon of textured quads, so its far edge swings toward the
//! camera, passes in front of the case, and carries on to lie open on the
//! left, showing its back.
//!
//! The launcher points the drive at the program on frame 0 (see `chain`), so
//! the seek happens while this plays.

use crate::common::*;
use crate::cover::{self, Cover};
use crate::ease;
use crate::gfx::*;
use crate::shelf;
use disc_toc::Entry;
use psx_gte::math::{Mat3I16, Vec3I16, Vec3I32};
use psx_gte::scene;
use psx_math::{cos_q12, sin_q12};

pub const FRAMES: i32 = 96;

/// First and last frame of the cover swinging open.
const OPEN_FROM: i32 = 10;
const OPEN_TO: i32 = 50;
/// How far the cover swings, in Q12 turns (4096 is a full turn): 167 degrees.
const COVER_MAX: i32 = 1900;
/// A quarter turn: the cover edge-on to the camera.
const QUARTER: i32 = 1024;
/// How far the whole case slides right while it opens, so the open case (cover
/// and tray) stays centred on the stage.
const PAN: i32 = 56;
/// Ribbon quads across the cover.
const STRIPS: usize = 8;

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
    g.ellipse_ring(cx, cy, r, r, scale((60, 130, 148), bright));
    g.ellipse_ring(cx, cy, label, label, scale((20, 120, 140), bright));
    g.ellipse_ring(cx, cy, label - 3, label - 3, scale((10, 50, 60), bright));
    // The hole shows the tray.
    for k in 0..N {
        let k2 = (k + 1) % N;
        g.tri_g([(cx, cy), pt(k, hole), pt(k2, hole)], [(3, 8, 11); 3]);
    }
}

/// Project the cover-plane point (`x` from the hinge, `y` from the cover's
/// middle) with the camera set up by [`aim_camera`].
fn project(x: i32, y: i32) -> (i32, i32) {
    let p = scene::project_vertex(Vec3I16 {
        x: x as i16,
        y: y as i16,
        z: 0,
    });
    (p.sx as i32, p.sy as i32)
}

/// Point the GTE at a cover that is `width` wide, hinged at screen point
/// `(hx, hy)` and swung open by `angle` (Q12 turns). The camera is three cover
/// widths away, so the free edge at a quarter turn is half again as big as the
/// closed cover.
fn aim_camera(hx: i32, hy: i32, width: i32, angle: i32) {
    let dist = width * 3;
    let (s, c) = (sin_q12(angle as u16), cos_q12(angle as u16));
    // Rotation about the vertical (hinge) axis; +z points away from the
    // camera, so the free edge (x > 0) comes toward it as the angle grows.
    scene::load_rotation(&Mat3I16 {
        m: [
            [c as i16, 0, s as i16],
            [0, 4096, 0],
            [-s as i16, 0, c as i16],
        ],
    });
    scene::load_translation(Vec3I32 {
        x: 0,
        y: 0,
        z: dist,
    });
    scene::set_projection_plane(dist as u16);
    scene::set_screen_offset(hx << 16, hy << 16);
}

/// The cover, swung open by `angle` about the left edge of `geom`'s art box.
fn cover(g: &mut Gfx, geom: &CaseGeom, art: usize, angle: i32, dim: u32) {
    let (aw, ah) = (geom.aw, geom.ah);
    aim_camera(geom.ax, geom.ay + ah / 2, aw, angle);
    let c = cos_q12(angle as u16);
    // Edge-on: nothing to see.
    if (aw * c.abs()) >> 12 < 1 {
        return;
    }
    let mut tops = [(0, 0); STRIPS + 1];
    let mut bots = [(0, 0); STRIPS + 1];
    for k in 0..=STRIPS {
        let x = aw * k as i32 / STRIPS as i32;
        tops[k] = project(x, -ah / 2);
        bots[k] = project(x, ah / 2);
    }
    let front = angle <= QUARTER;
    // Lit from the front: the cover darkens as it turns edge-on, and its
    // back is dim glass.
    let lum = if front {
        70 + ((58 * c) >> 12)
    } else {
        34 + ((22 * -c) >> 12)
    };
    let v = (lum as u32 * dim >> 8) as u8;
    if angle == 0 && aw == L_W {
        g.art(Size::L, art, geom.ax, geom.ay, (v, v, v));
    } else {
        g.art_ribbon(Size::L, art, &tops, &bots, (v, v, v));
    }
    let edge = scale(if front { FOCUS } else { RING }, dim);
    g.line_add(tops[0], tops[STRIPS], edge, edge);
    g.line_add(bots[0], bots[STRIPS], edge, edge);
    g.line_add(tops[0], bots[0], edge, edge);
    g.line_add(tops[STRIPS], bots[STRIPS], edge, edge);
    if front {
        // Gloss wedge from the top-left, as on the closed case.
        let mid = project(0, 0);
        let mid2 = project(aw / 4, 0);
        let gl = scale((18, 30, 34), dim);
        g.quad_g_add(
            [tops[0], tops[STRIPS * 5 / 8], mid, mid2],
            [gl, (0, 0, 0), (0, 0, 0), (0, 0, 0)],
        );
        // A diagonal glare that slides across the glass as it catches the
        // light: a band leaning from top-left to bottom-right.
        let peak = (ease::span(angle, 150, 480) * (256 - ease::span(angle, 520, 960)) >> 8) as u32;
        if peak > 4 {
            let k = 2 + (angle * 3 / QUARTER).clamp(0, 3) as usize;
            let hi = scale((40, 66, 74), peak * dim >> 8);
            let off = (0, 0, 0);
            g.quad_g_add(
                [tops[k - 1], tops[k], bots[k], bots[k + 1]],
                [off, hi, off, hi],
            );
            g.quad_g_add(
                [tops[k], tops[k + 1], bots[k + 1], bots[k + 2]],
                [hi, off, hi, off],
            );
        }
    }
}

/// The tray the cover uncovers: a darker recess with the hub ring the disc
/// sits on and two guide rails.
fn tray(g: &mut Gfx, geom: &CaseGeom, dim: u32) {
    let (x, y, w, h) = (geom.ax, geom.ay, geom.aw, geom.ah);
    g.quad_g(
        [(x, y), (x + w, y), (x, y + h), (x + w, y + h)],
        [
            scale((6, 15, 20), dim),
            scale((6, 15, 20), dim),
            scale((3, 8, 11), dim),
            scale((3, 8, 11), dim),
        ],
    );
    let (cx, cy) = (x + w / 2, y + h / 2);
    let r = h * 14 / 100;
    g.ellipse_ring(cx, cy, r, r, scale((20, 70, 82), dim));
    g.ellipse_ring(cx, cy, r * 2 / 5, r * 2 / 5, scale((14, 50, 60), dim));
    g.rect(x + 5, y + 4, 1, h - 8, scale((16, 34, 40), dim));
    g.rect(x + w - 6, y + 4, 1, h - 8, scale((16, 34, 40), dim));
}

/// `t` counts frames from the press. Draws over whatever the menu drew (which
/// left the selected case to this).
pub fn draw(g: &mut Gfx, t: i32, from: From, e: &Entry, ctx: &Ctx) {
    // Veil the menu: subtract toward the dark stage (the menu stops being
    // drawn once the veil has taken it to black).
    let veil = ease::out(ease::span(t, 0, 28)) * 230 / 256;
    let v = veil as u8;
    g.quad_g_sub([(0, 0), (320, 0), (0, 240), (320, 240)], [(v, v, v); 4]);
    let rise = ease::out(ease::span(t, 0, 30)) as u32;

    // Case travels to the stage and grows a little, sliding right as it
    // opens.
    let move_t = ease::out(ease::span(t, 0, 26));
    let sc = ease::lerp(from.sc, 285, move_t);
    let cx = ease::lerp(from.cx, 160, move_t);
    let mut bottom = ease::lerp(from.bottom, 150, move_t);
    let pan = (ease::out(ease::span(t, OPEN_FROM - 4, OPEN_TO)) * PAN) >> 8;
    // Recede: after the disc leaves, the case slides back and down.
    let rec = ease::in_out(ease::span(t, 38, 60));
    bottom += (rec * 18) >> 8;
    let sc = ease::lerp(sc, 200, rec);
    let dim = ease::lerp(256, 0, rec);
    let d = dim as u32;
    let cv = CaseView {
        cx: cx + pan,
        bottom,
        sc,
        dim,
        focus: 256,
        // The menu's sway carries on and settles flat before the cover opens.
        yaw: (shelf::sway_yaw(ctx.t) * (256 - ease::span(t, 0, OPEN_FROM))) >> 8,
    };
    let geom = CaseGeom::new(cv.cx, cv.bottom, sc);
    let xf = CaseXform::aim(&geom, cv.yaw);
    // Until the cover moves the case is closed, drawn as on the shelf.
    let closed = t < OPEN_FROM;

    if dim > 6 {
        case_body(g, &cv, &xf);
        if !closed {
            tray(g, &geom, d);
        }
    }

    // One disc for the whole sequence: seated in the tray, lifted out of the
    // case, then flown to the middle where it spins up.
    let open = ease::in_out(ease::span(t, OPEN_FROM, OPEN_TO));
    let lift = ease::in_out(ease::span(t, 34, 56));
    let fly = ease::in_out(ease::span(t, 50, 72));
    let seat = (geom.ax + geom.aw / 2, geom.ay + geom.ah / 2);
    let top = (seat.0, seat.1 - 54);
    let dcx = ease::lerp(ease::lerp(seat.0, top.0, lift), 160, fly);
    let dcy = ease::lerp(ease::lerp(seat.1, top.1, lift), 112, fly);
    let dr = ease::lerp(
        ease::lerp(geom.ah * 40 / 100, geom.ah * 40 / 100 + 18, lift),
        52,
        fly,
    );
    // Angular speed: a slow turn, then a quadratic ramp, then constant.
    let ramp = ease::quad_in(ease::span(t, 56, 86));
    let spin = (t * 16 + ((ramp * t) >> 3)) as u16 & 4095;
    if open > 8 {
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
        if closed {
            case_front(g, &cv, &xf, e);
        } else {
            case_outline(g, &cv, &xf);
            if let Cover::Art(art) = cover::of(e.name_str()) {
                cover(g, &geom, art, (open * COVER_MAX) >> 8, d);
            }
        }
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
            e.name_str(),
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
