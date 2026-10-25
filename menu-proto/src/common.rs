//! Pieces every direction shares: header, footer hints, backgrounds, the
//! primary pill and the cover-art case.

use crate::data::Entry;
use crate::gfx::*;
use crate::text;
use crate::ui_atlas::*;
use psx_math::{cos_q12, sin_q12};

/// What a frame knows about the menu.
pub struct Ctx {
    pub t: u32,
    pub it: bool,
    pub entries: &'static [&'static crate::data::Entry],
    pub private: bool,
}

impl Ctx {
    pub fn desc(&self, e: &Entry) -> &'static str {
        if self.it {
            e.it
        } else {
            e.en
        }
    }
}

pub fn wordmark(g: &mut Gfx, x: i32, y: i32) {
    text::draw(g, &MICH12, x, y, "PSo", TEXT, 0);
    let w = MICH12.width("PSo", 0);
    text::draw(g, &MICH12, x + w, y, "X", ACCENT, 0);
    let w2 = MICH12.width("PSoX", 0);
    text::draw(g, &MICH12, x + w2, y, "ide", TEXT, 0);
}

fn icon(g: &mut Gfx, gl: &Glyph, x: i32, y: i32, c: Rgb) {
    g.ui_mode();
    g.ui_sprite(x, y, gl.w as u16, gl.h as u16, (gl.u, gl.v), c);
}

use crate::text::Glyph;

/// Header: wordmark left, music chip and language chip right, hairline under.
pub fn header(g: &mut Gfx, ctx: &Ctx, label: &str) {
    wordmark(g, 10, 5);
    text::draw(
        g,
        &MICH7,
        10 + MICH12.width("PSoXide", 0) + 8,
        9,
        label,
        DIM,
        1,
    );
    text::draw(
        g,
        &JOST9,
        10 + MICH12.width("PSoXide", 0) + 8 + MICH7.width(label, 1) + 6,
        8,
        "v0.5",
        RING,
        0,
    );
    // Music chip: six bars riding a slow wave, as the old menu's meter does.
    let bx = 232;
    for b in 0..6i32 {
        let ph = (ctx.t as i32 * 53 + b * 610) as u16 & 4095;
        let h = 2 + ((sin_q12(ph) + 4096) * 7 >> 13);
        g.rect(bx + b * 3, 15 - h, 2, h, mix(DIM, ACCENT, 140));
    }
    // Language chip.
    let (en, it) = if ctx.it { (DIM, TEXT) } else { (TEXT, DIM) };
    text::draw(g, &MICH7, 262, 9, "EN", en, 0);
    g.rect(280, 8, 1, 9, RING);
    text::draw(g, &MICH7, 285, 9, "IT", it, 0);
    g.rect(262, 18, 13, 1, if ctx.it { (0, 0, 0) } else { ACCENT });
    g.rect(285, 18, 13, 1, if ctx.it { ACCENT } else { (0, 0, 0) });
    g.rect_add(0, 22, 320, 1, (16, 26, 30));
}

/// A shoulder-button tag: 18x11 outline with the key name inside.
fn tag(g: &mut Gfx, x: i32, y: i32, name: &str) {
    g.frame(x, y, 18, 12, RING);
    text::draw_centered(g, &JOST9, x + 9, y + 1, name, DIM, 0);
}

/// One footer hint: glyph then label. Returns the x after it.
pub fn hint(g: &mut Gfx, x: i32, gl: &Glyph, label: &str) -> i32 {
    icon(g, gl, x, 224, RING);
    text::draw(g, &JOST9, x + gl.w as i32 + 4, 226, label, MUTED, 0);
    x + gl.w as i32 + 4 + JOST9.width(label, 0) + 12
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Row,
    Grid,
    Column,
}

pub fn footer(g: &mut Gfx, ctx: &Ctx, nav: Nav, browse: &str) {
    g.rect_add(0, 221, 320, 1, (12, 20, 24));
    let mut x = 10;
    let (first, second) = match nav {
        Nav::Row => (&ICON_CHEV_L, &ICON_CHEV_R),
        Nav::Grid => (&ICON_CHEV_L, &ICON_CHEV_R),
        Nav::Column => (&ICON_CHEV_U, &ICON_CHEV_D),
    };
    let vertical = nav == Nav::Column;
    let (w1, w2) = (first.w as i32, second.w as i32);
    icon(g, first, x, if vertical { 227 } else { 225 }, RING);
    x += w1 + 3;
    icon(g, second, x, if vertical { 227 } else { 225 }, RING);
    x += w2 + 4;
    text::draw(g, &JOST9, x, 226, browse, MUTED, 0);
    x += JOST9.width(browse, 0) + 14;
    x = hint(g, x, &ICON_CROSS, if ctx.it { "Avvia" } else { "Start" });
    x = hint(
        g,
        x,
        &ICON_TRIANGLE,
        if ctx.it { "Lingua" } else { "Language" },
    );
    let _ = hint(
        g,
        x,
        &ICON_SQUARE,
        if ctx.private { "Private" } else { "Public" },
    );
    // Music keys, right-aligned: the menu keeps the old L1/R1 skip.
    let label = if ctx.it { "Musica" } else { "Music" };
    let lw = JOST9.width(label, 0);
    text::draw_right(g, &JOST9, 310, 226, label, MUTED, 0);
    tag(g, 310 - lw - 4 - 18, 225, "R1");
    tag(g, 310 - lw - 4 - 18 - 21, 225, "L1");
}

/// The primary pill: accent fill, dark label, cross glyph.
pub fn pill(g: &mut Gfx, cx: i32, y: i32, label: &str, glow: u32) {
    let w = 30 + MICH7.width(label, 1) + 14;
    let x = cx - w / 2;
    if glow > 0 {
        g.glow(cx, y + 8, w / 2 + 14, 12, scale((10, 60, 74), glow));
    }
    g.quad_g(
        [
            (x + 4, y),
            (x + w - 4, y),
            (x + 4, y + 15),
            (x + w - 4, y + 15),
        ],
        [
            (60, 220, 242),
            (60, 220, 242),
            (30, 178, 206),
            (30, 178, 206),
        ],
    );
    g.quad_g(
        [(x, y + 3), (x + 4, y + 3), (x, y + 12), (x + 4, y + 12)],
        [
            (52, 206, 230),
            (60, 220, 242),
            (30, 178, 206),
            (30, 178, 206),
        ],
    );
    g.quad_g(
        [
            (x + w - 4, y + 3),
            (x + w, y + 3),
            (x + w - 4, y + 12),
            (x + w, y + 12),
        ],
        [
            (60, 220, 242),
            (52, 206, 230),
            (30, 178, 206),
            (30, 178, 206),
        ],
    );
    // Dark glyph and label: subtract the difference from the fill.
    g.ui_dark(true);
    let ink = (60, 200, 220);
    g.ui_mode();
    g.ui_sprite(x + 8, y + 1, 14, 14, (ICON_CROSS.u, ICON_CROSS.v), ink);
    text::draw(g, &MICH7, x + 27, y + 4, label, ink, 1);
    g.ui_dark(false);
}

/// Ground gradient plus the soft teal wash the shell sits in.
pub fn ground(_g: &mut Gfx, _top: Rgb, _bottom: Rgb) {
    // A flat fill (the frame clear) rather than a gradient quad: the ground
    // only moves 1 step per channel top to bottom, and the full-screen
    // Gouraud quad cost about 100k GPU cycles a frame for it.
}

/// Floor lines receding to a horizon at `y0`: the shell's grid, at 320x240.
pub fn floor_grid(g: &mut Gfx, y0: i32, y1: i32, t: u32) {
    // Horizontals bunch toward the horizon.
    let offs = [0, 3, 8, 15, 26, 42, 66, 100];
    for (i, o) in offs.iter().enumerate() {
        let y = y0 + (*o * (y1 - y0)) / 100;
        let a = 6 + i as u32 * 3;
        g.rect_add(0, y, 320, 1, (a as u8 / 2, a as u8, (a + 2) as u8));
    }
    // Verticals fan out from the horizon centre; drift sideways slowly.
    let drift = ((t / 3) % 24) as i32;
    for k in -9..=9i32 {
        let top = (160 + k * 7, y0);
        let bot = (160 + k * 40 - drift * 0 + 0, y1);
        g.line_add(top, bot, (2, 6, 8), (11, 26, 30));
    }
}

/// A case: framed cover with gloss, scaled `sc` (Q8) about its bottom centre.
/// `yaw` (-256..=256) tilts the far edge; `dim` darkens; `focus` brightens
/// the outline and brackets.
pub struct CaseView {
    pub cx: i32,
    pub bottom: i32,
    pub sc: i32,
    pub dim: i32,
    pub focus: i32,
    pub yaw: i32,
}

pub fn draw_case(g: &mut Gfx, cv: &CaseView, e: &Entry, label_t: u32) {
    let (w, h) = ((112 * cv.sc) >> 8, (102 * cv.sc) >> 8);
    let (aw, ah) = ((L_W * cv.sc) >> 8, (L_H * cv.sc) >> 8);
    let pad = (4 * cv.sc) >> 8;
    let x0 = cv.cx - w / 2;
    let y0 = cv.bottom - h;
    let tilt = (cv.yaw * h) >> 11; // edge height change, up to ~h/8
    let (tl, tr) = (tilt, -tilt);
    let dim = cv.dim as u32;
    // Body.
    let bodyc = scale((9, 20, 27), dim);
    g.quad_g(
        [
            (x0, y0 - tl),
            (x0 + w, y0 - tr),
            (x0, y0 + h + tl),
            (x0 + w, y0 + h + tr),
        ],
        [
            bodyc,
            bodyc,
            scale((4, 11, 15), dim),
            scale((4, 11, 15), dim),
        ],
    );
    // Spine: a lit strip down the hinge side.
    let sp = ((3 * cv.sc) >> 8).max(1);
    g.quad_g(
        [
            (x0 + 1, y0 + 1 - tl),
            (x0 + 1 + sp, y0 + 1 - tl),
            (x0 + 1, y0 + h - 1 + tl),
            (x0 + 1 + sp, y0 + h - 1 + tl),
        ],
        [
            scale((34, 64, 74), dim),
            scale((18, 36, 44), dim),
            scale((14, 28, 34), dim),
            scale((8, 16, 20), dim),
        ],
    );
    // Cover.
    let (ax, ay) = (x0 + pad, y0 + pad);
    let tint = {
        let v = (128 * cv.dim) >> 8;
        (v as u8, v as u8, v as u8)
    };
    let size = if cv.sc > 220 {
        Size::L
    } else if cv.sc > 150 {
        Size::M
    } else {
        Size::S
    };
    let aq = [
        (ax, ay - tl),
        (ax + aw, ay - tr),
        (ax, ay + ah + tl),
        (ax + aw, ay + ah + tr),
    ];
    match e.art {
        Some(i) => {
            if cv.sc >= 250 && cv.yaw == 0 {
                g.art(Size::L, i, ax, ay, tint);
            } else {
                g.art_strips(
                    size,
                    i,
                    ax,
                    ax + aw,
                    (ay - tl, ay + ah + tl),
                    (ay - tr, ay + ah + tr),
                    tint,
                );
            }
        }
        None => credits_cover(g, aq, dim, label_t),
    }
    // Plate: version at the foot of the case.
    let py = ay + ah + (2 * cv.sc >> 8);
    if cv.sc > 200 {
        let c = scale(MUTED, dim);
        icon(g, &ICON_DISC, ax + 1, py + 1, scale(RING, dim));
        if !e.version.is_empty() {
            text::draw(g, &JOST9, ax + 19, py + 3, "v", c, 0);
            text::draw(g, &JOST9, ax + 24, py + 3, e.version, c, 0);
        } else {
            text::draw(g, &JOST9, ax + 19, py + 3, "Info", c, 0);
        }
    }
    // Gloss: a faint additive wedge from the top-left, as the shell's glass.
    let gl = scale((18, 30, 34), dim);
    g.quad_g_add(
        [
            (x0, y0 - tl),
            (x0 + w * 3 / 5, y0 - tl),
            (x0, y0 + h / 2),
            (x0 + w / 4, y0 + h / 2),
        ],
        [gl, (0, 0, 0), (0, 0, 0), (0, 0, 0)],
    );
    // Outline.
    let ring = mix(scale(RING, dim), FOCUS, cv.focus as u32);
    g.frame(x0, y0, w, h, ring);
    if cv.focus > 100 {
        g.brackets(x0 - 2, y0 - 2, w + 4, h + 4, 7, FOCUS);
        g.rect_add(x0, y0, w, h, (3, 14, 17));
    }
}

fn credits_cover(g: &mut Gfx, q: [(i32, i32); 4], dim: u32, t: u32) {
    let top = scale((14, 54, 66), dim);
    let bot = scale((4, 14, 20), dim);
    g.quad_g(q, [top, top, bot, bot]);
    let (cx, cy) = ((q[0].0 + q[1].0) / 2, (q[0].1 + q[2].1) / 2);
    let r = (q[1].0 - q[0].0) / 4;
    g.ellipse_ring(cx, cy, r, r, scale((20, 120, 140), dim));
    g.ellipse_ring(cx, cy, r / 3, r / 3, scale((20, 120, 140), dim));
    let _ = (t, cos_q12(0));
}
