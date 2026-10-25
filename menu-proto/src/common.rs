//! Pieces of the Shelf menu: header, footer hints, the start button, the
//! floor and the cover-art case.
//!
//! Layout rules, so nothing sits at "a different height":
//!
//! * Header and footer are mirror images. Each is a band whose content sits on
//!   one centre line (a pixel row), closed by a one-pixel rule.
//! * Every element on that line is centred on it by its capital height (text)
//!   or its own box (marks, chips, bars). All of those are an odd number of
//!   pixels tall, so each has a true middle row.
//! * Left and right margins are the same ([`MARGIN`]).

use crate::data::Entry;
use crate::gfx::*;
use crate::text;
use crate::text::Glyph;
use crate::ui_atlas::*;
use psx_math::{cos_q12, sin_q12};

/// Pixels between the screen edge and the first or last element.
pub const MARGIN: i32 = 12;
/// Pixel edge the right-aligned elements end on.
pub const RIGHT: i32 = 320 - MARGIN;
/// Pixel row the header's elements are centred on.
pub const HEAD_MID: i32 = 11;
/// Pixel row of the rule under the header.
pub const HEAD_RULE: i32 = 23;
/// Pixel row of the rule over the footer (the header's mirror image).
pub const FOOT_RULE: i32 = 239 - HEAD_RULE;
/// Pixel row the footer's elements are centred on.
pub const FOOT_MID: i32 = 239 - HEAD_MID;

/// Height of the header chips and footer tags, odd so they have a middle row.
const CHIP_H: i32 = 13;
/// Gap between neighbouring header groups.
const HEAD_GAP: i32 = 10;

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

pub fn wordmark(g: &mut Gfx, x: i32, cy: i32) -> i32 {
    let y = cy - MICH12.cap_mid();
    text::draw(g, &MICH12, x, y, "PSo", TEXT, 0);
    let w = MICH12.width("PSo", 0);
    text::draw(g, &MICH12, x + w, y, "X", ACCENT, 0);
    let w2 = MICH12.width("PSoX", 0);
    text::draw(g, &MICH12, x + w2, y, "ide", TEXT, 0);
    x + MICH12.width("PSoXide", 0)
}

fn icon(g: &mut Gfx, gl: &Glyph, x: i32, cy: i32, c: Rgb) {
    g.ui_mode();
    let y = cy - gl.h as i32 / 2;
    g.ui_sprite(x, y, gl.w as u16, gl.h as u16, (gl.u, gl.v), c);
}

/// A vertical divider 9 rows tall, centred on `cy`.
fn divider(g: &mut Gfx, x: i32, cy: i32) {
    g.rect(x, cy - 4, 1, 9, (34, 52, 62));
}

/// Header: wordmark and disc name on the left; music meter, language switch
/// and edition on the right; one rule under it. Everything is centred on
/// [`HEAD_MID`].
pub fn header(g: &mut Gfx, ctx: &Ctx, label: &str) {
    let cy = HEAD_MID;
    let mut x = wordmark(g, MARGIN, cy);
    x += 6;
    divider(g, x, cy);
    x += 1 + 6;
    text::draw_mid(g, &JOST9, x, cy, label, DIM, 1);
    x += JOST9.width(label, 1) + 6;
    text::draw_mid(g, &JOST9, x, cy, "v0.5", RING, 0);

    // Edition chip, flush with the right margin. Both names get the width of
    // the longer one so the chips to its left never shift when it toggles.
    let (name, ink, edge) = if ctx.private {
        ("PRIVATE", FOCUS, mix(RING, ACCENT, 150))
    } else {
        ("PUBLIC", TEXT2, RING)
    };
    let cw = JOST9.width("PRIVATE", 1) + 10;
    let ex = RIGHT - cw;
    g.frame(ex, cy - CHIP_H / 2, cw, CHIP_H, edge);
    text::draw_centered_mid(g, &JOST9, ex + cw / 2, cy, name, ink, 1);

    // Language switch: two cells in one frame, the active one lit.
    let cell = JOST9.width("EN", 1).max(JOST9.width("IT", 1)) + 8;
    let lx = ex - HEAD_GAP - (2 * cell + 1);
    g.frame(lx, cy - CHIP_H / 2, 2 * cell + 1, CHIP_H, RING);
    let lit = if ctx.it { lx + cell + 1 } else { lx + 1 };
    g.rect(lit, cy - CHIP_H / 2 + 1, cell - 1, CHIP_H - 2, (10, 50, 62));
    g.rect(lx + cell, cy - CHIP_H / 2 + 1, 1, CHIP_H - 2, RING);
    let (en, it) = if ctx.it { (DIM, TEXT) } else { (TEXT, DIM) };
    text::draw_centered_mid(g, &JOST9, lx + cell / 2 + 1, cy, "EN", en, 1);
    text::draw_centered_mid(g, &JOST9, lx + cell + 1 + cell / 2, cy, "IT", it, 1);

    // Music meter: five bars riding a slow wave, mirrored about the line.
    let bx = lx - HEAD_GAP - 14;
    for b in 0..5i32 {
        let ph = (ctx.t as i32 * 53 + b * 730) as u16 & 4095;
        let half = (sin_q12(ph) + 4096) * 5 >> 13; // 0..=5
        let h = 1 + 2 * half;
        g.rect(bx + b * 3, cy - half, 2, h, mix(DIM, ACCENT, 140));
    }
    g.rect_add(0, HEAD_RULE, 320, 1, (16, 26, 30));
}

/// A shoulder-button tag: an outlined box with the key name inside.
fn tag(g: &mut Gfx, x: i32, cy: i32, name: &str) -> i32 {
    let w = JOST9.width(name, 0) + 8;
    g.frame(x, cy - CHIP_H / 2, w, CHIP_H, RING);
    text::draw_centered_mid(g, &JOST9, x + w / 2, cy, name, DIM, 0);
    w
}

/// Gap between an icon and its label.
const ICON_GAP: i32 = 4;
/// Gap between footer hints.
const HINT_GAP: i32 = 10;

/// One footer hint: glyph then label. Returns the x after it.
fn hint(g: &mut Gfx, x: i32, gl: &Glyph, label: &str) -> i32 {
    icon(g, gl, x, FOOT_MID, RING);
    text::draw_mid(
        g,
        &JOST9,
        x + gl.w as i32 + ICON_GAP,
        FOOT_MID,
        label,
        MUTED,
        0,
    );
    x + gl.w as i32 + ICON_GAP + JOST9.width(label, 0) + HINT_GAP
}

/// Footer: the glyph hints from the left margin, the music keys at the right
/// one, all centred on [`FOOT_MID`].
pub fn footer(g: &mut Gfx, ctx: &Ctx) {
    g.rect_add(0, FOOT_RULE, 320, 1, (12, 20, 24));
    let mut x = MARGIN;
    icon(g, &ICON_CHEV_L, x, FOOT_MID, RING);
    x += ICON_CHEV_L.w as i32 + 2;
    icon(g, &ICON_CHEV_R, x, FOOT_MID, RING);
    x += ICON_CHEV_R.w as i32 + ICON_GAP;
    let browse = if ctx.it { "Sfoglia" } else { "Browse" };
    text::draw_mid(g, &JOST9, x, FOOT_MID, browse, MUTED, 0);
    x += JOST9.width(browse, 0) + HINT_GAP;
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
        if ctx.it { "Edizione" } else { "Edition" },
    );
    // Music keys, right-aligned: the menu keeps the old L1/R1 skip.
    let label = if ctx.it { "Musica" } else { "Music" };
    let lw = JOST9.width(label, 0);
    text::draw_right_mid(g, &JOST9, RIGHT, FOOT_MID, label, MUTED, 0);
    let tw = JOST9.width("R1", 0) + 8;
    let rx = RIGHT - lw - ICON_GAP - tw;
    tag(g, rx, FOOT_MID, "R1");
    tag(g, rx - 2 - JOST9.width("L1", 0) - 8, FOOT_MID, "L1");
}

/// The state of the start button.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// Focused and waiting: dark body, lit edge that breathes.
    Idle,
    /// Cross went down: filled, dark ink. `fade` (0..=256) is its opacity.
    Down,
}

/// Slant of the start button's sides, in pixels over its height.
const SLANT: i32 = 6;
/// Height of the start button, odd so the label has a middle row.
const BTN_H: i32 = 19;

/// The primary action: a rectangle whose left and right edges lean, centred
/// on `cx` with its middle row on `cy`. A cross glyph and the label.
pub fn start_button(g: &mut Gfx, cx: i32, cy: i32, label: &str, press: Press, t: u32, a: u32) {
    let content = ICON_CROSS.w as i32 + 7 + MICH9.width(label, 1);
    let w = content + 2 * 14 + SLANT;
    let x = cx - w / 2;
    let y = cy - BTN_H / 2;
    // Horizontal position of the left edge on row `r` of the button: the top
    // is pushed right by SLANT, the bottom is flush. The right edge is the
    // same shifted by `w - SLANT`.
    let left = |r: i32| x + SLANT - SLANT * r / BTN_H;
    let q = |inset_x: i32, r0: i32, r1: i32| {
        [
            (left(r0) + inset_x, y + r0),
            (left(r0) + w - SLANT - inset_x, y + r0),
            (left(r1) + inset_x, y + r1),
            (left(r1) + w - SLANT - inset_x, y + r1),
        ]
    };
    let pulse = (sin_q12((t * 40) as u16 & 4095) + 4096) as u32 >> 5; // 0..=255
    match press {
        Press::Idle => {
            let edge = mix(ACCENT, FOCUS, pulse);
            g.glow(cx, cy + 2, w / 2 + 10, 11, scale((6, 36, 44), a));
            g.quad_g(q(0, 0, BTN_H), [scale(edge, a); 4]);
            let (top, bot) = (scale((14, 54, 66), a), scale((5, 24, 31), a));
            g.quad_g(q(1, 1, BTN_H - 1), [top, top, bot, bot]);
            text::draw_mid(
                g,
                &MICH9,
                x + SLANT / 2 + 14 + ICON_CROSS.w as i32 + 7,
                cy,
                label,
                scale(TEXT, a),
                1,
            );
            icon(g, &ICON_CROSS, x + SLANT / 2 + 14, cy, scale(ACCENT, a));
        }
        Press::Down => {
            g.glow(cx, cy + 2, w / 2 + 14, 13, scale((12, 70, 86), a));
            let (top, bot) = (scale((90, 236, 255), a), scale((34, 190, 220), a));
            g.quad_g(q(0, 0, BTN_H), [top, top, bot, bot]);
            // Dark glyph and label: subtract the ink from the fill.
            let ink = scale((70, 205, 225), a);
            g.ui_dark(true);
            text::draw_mid(
                g,
                &MICH9,
                x + SLANT / 2 + 14 + ICON_CROSS.w as i32 + 7,
                cy,
                label,
                ink,
                1,
            );
            icon(g, &ICON_CROSS, x + SLANT / 2 + 14, cy, ink);
            g.ui_dark(false);
        }
    }
}

/// Floor lines receding to a horizon at `y0`: the shell's grid, at 320x240.
pub fn floor_grid(g: &mut Gfx, y0: i32, y1: i32) {
    // Horizontals bunch toward the horizon.
    let offs = [0, 3, 8, 15, 26, 42, 66, 100];
    for (i, o) in offs.iter().enumerate() {
        let y = y0 + (*o * (y1 - y0)) / 100;
        let a = 6 + i as u32 * 3;
        g.rect_add(0, y, 320, 1, (a as u8 / 2, a as u8, (a + 2) as u8));
    }
    // Verticals fan out from the horizon centre.
    for k in -9..=9i32 {
        let top = (160 + k * 7, y0);
        let bot = (160 + k * 40, y1);
        g.line_add(top, bot, (2, 6, 8), (11, 26, 30));
    }
}

/// Where a case sits on screen at scale `sc` (Q8): the body box and the art
/// box inside it. The art is inset by `pad`.
#[derive(Clone, Copy)]
pub struct CaseGeom {
    pub x0: i32,
    pub y0: i32,
    pub w: i32,
    pub h: i32,
    pub ax: i32,
    pub ay: i32,
    pub aw: i32,
    pub ah: i32,
}

impl CaseGeom {
    /// A case centred on `cx` whose lower edge is the row `bottom`.
    pub fn new(cx: i32, bottom: i32, sc: i32) -> Self {
        let (w, h) = ((112 * sc) >> 8, (102 * sc) >> 8);
        let pad = (4 * sc) >> 8;
        let (x0, y0) = (cx - w / 2, bottom - h);
        CaseGeom {
            x0,
            y0,
            w,
            h,
            ax: x0 + pad,
            ay: y0 + pad,
            aw: (L_W * sc) >> 8,
            ah: (L_H * sc) >> 8,
        }
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

/// The case's plastic body: base, lit spine, and the version plate at its foot.
pub fn case_body(g: &mut Gfx, cv: &CaseView, e: &Entry) {
    let c = CaseGeom::new(cv.cx, cv.bottom, cv.sc);
    let tilt = (cv.yaw * c.h) >> 11; // edge height change, up to ~h/8
    let (tl, tr) = (tilt, -tilt);
    let dim = cv.dim as u32;
    let bodyc = scale((9, 20, 27), dim);
    g.quad_g(
        [
            (c.x0, c.y0 - tl),
            (c.x0 + c.w, c.y0 - tr),
            (c.x0, c.y0 + c.h + tl),
            (c.x0 + c.w, c.y0 + c.h + tr),
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
            (c.x0 + 1, c.y0 + 1 - tl),
            (c.x0 + 1 + sp, c.y0 + 1 - tl),
            (c.x0 + 1, c.y0 + c.h - 1 + tl),
            (c.x0 + 1 + sp, c.y0 + c.h - 1 + tl),
        ],
        [
            scale((34, 64, 74), dim),
            scale((18, 36, 44), dim),
            scale((14, 28, 34), dim),
            scale((8, 16, 20), dim),
        ],
    );
    // Plate: version at the foot of the case.
    let py = c.ay + c.ah + (2 * cv.sc >> 8);
    if cv.sc > 200 {
        let col = scale(MUTED, dim);
        icon_at(g, &ICON_DISC, c.ax + 1, py + 1, scale(RING, dim));
        if !e.version.is_empty() {
            text::draw(g, &JOST9, c.ax + 19, py + 2, "v", col, 0);
            text::draw(g, &JOST9, c.ax + 24, py + 2, e.version, col, 0);
        } else {
            text::draw(g, &JOST9, c.ax + 19, py + 2, "Info", col, 0);
        }
    }
}

fn icon_at(g: &mut Gfx, gl: &Glyph, x: i32, y: i32, c: Rgb) {
    g.ui_mode();
    g.ui_sprite(x, y, gl.w as u16, gl.h as u16, (gl.u, gl.v), c);
}

/// The case's front: cover art, gloss and outline (the closed case).
pub fn case_front(g: &mut Gfx, cv: &CaseView, e: &Entry) {
    let c = CaseGeom::new(cv.cx, cv.bottom, cv.sc);
    let tilt = (cv.yaw * c.h) >> 11;
    let (tl, tr) = (tilt, -tilt);
    let dim = cv.dim as u32;
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
        (c.ax, c.ay - tl),
        (c.ax + c.aw, c.ay - tr),
        (c.ax, c.ay + c.ah + tl),
        (c.ax + c.aw, c.ay + c.ah + tr),
    ];
    match e.art {
        Some(i) => {
            if cv.sc >= 250 && cv.yaw == 0 {
                g.art(Size::L, i, c.ax, c.ay, tint);
            } else {
                g.art_strips(
                    size,
                    i,
                    c.ax,
                    c.ax + c.aw,
                    (c.ay - tl, c.ay + c.ah + tl),
                    (c.ay - tr, c.ay + c.ah + tr),
                    tint,
                );
            }
        }
        None => credits_cover(g, aq, dim),
    }
    // Gloss: a faint additive wedge from the top-left, as the shell's glass.
    let gl = scale((18, 30, 34), dim);
    g.quad_g_add(
        [
            (c.x0, c.y0 - tl),
            (c.x0 + c.w * 3 / 5, c.y0 - tl),
            (c.x0, c.y0 + c.h / 2),
            (c.x0 + c.w / 4, c.y0 + c.h / 2),
        ],
        [gl, (0, 0, 0), (0, 0, 0), (0, 0, 0)],
    );
    // Outline.
    let ring = mix(scale(RING, dim), FOCUS, cv.focus as u32);
    g.frame(c.x0, c.y0, c.w, c.h, ring);
    if cv.focus > 100 {
        g.brackets(c.x0 - 2, c.y0 - 2, c.w + 4, c.h + 4, 7, FOCUS);
        g.rect_add(c.x0, c.y0, c.w, c.h, (3, 14, 17));
    }
}

pub fn draw_case(g: &mut Gfx, cv: &CaseView, e: &Entry) {
    case_body(g, cv, e);
    case_front(g, cv, e);
}

fn credits_cover(g: &mut Gfx, q: [(i32, i32); 4], dim: u32) {
    let top = scale((14, 54, 66), dim);
    let bot = scale((4, 14, 20), dim);
    g.quad_g(q, [top, top, bot, bot]);
    let (cx, cy) = ((q[0].0 + q[1].0) / 2, (q[0].1 + q[2].1) / 2);
    let r = (q[1].0 - q[0].0) / 4;
    g.ellipse_ring(cx, cy, r, r, scale((20, 120, 140), dim));
    g.ellipse_ring(cx, cy, r / 3, r / 3, scale((20, 120, 140), dim));
    let _ = cos_q12(0);
}
