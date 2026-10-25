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

use crate::cover::{self, Cover};
use crate::gfx::*;
use crate::text;
use crate::text::Glyph;
use crate::ui_atlas::*;
use disc_toc::Entry;
use psx_gte::math::{Mat3I16, Vec3I16, Vec3I32};
use psx_gte::scene;
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
/// Pixel row the song title is centred on, just under the header's rule.
const SONG_MID: i32 = 33;
/// Gap between neighbouring header groups.
const HEAD_GAP: i32 = 10;

/// What the music is doing, for the header.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Playback {
    /// The disc carries no menu songs: the header shows nothing about music.
    None,
    /// MUSIC OFF, the last slot of the L1/R1 walk.
    Off,
    /// The drive is picking the song up.
    Loading,
    Playing,
}

/// The header's music group.
#[derive(Clone, Copy)]
pub struct NowPlaying<'a> {
    pub state: Playback,
    /// The song's title from the disc table.
    pub title: &'a str,
    /// Band levels (0..=255, low frequencies first) when the disc carries an
    /// analysis for this song.
    pub levels: Option<&'a [u8]>,
}

/// Which screen the footer's hints belong to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Shelf,
    Credits,
}

/// What a frame knows about the menu.
pub struct Ctx<'a> {
    pub t: u32,
    pub it: bool,
    pub screen: Screen,
    /// The pressing's name and version, as the build wrote them.
    pub disc_title: &'a str,
    pub disc_version: &'a str,
    pub music: NowPlaying<'a>,
    /// The disc table's short music credit, shown while the menu idles.
    pub credit: &'a str,
    /// How far the idle menu has settled into its attract state: 0 is the
    /// menu as it is used, 256 is text and hints gone.
    pub attract: i32,
}

impl Ctx<'_> {
    pub fn desc<'e>(&self, e: &'e Entry) -> &'e str {
        if self.it {
            e.desc_it_str()
        } else {
            e.desc_en_str()
        }
    }

    /// Opacity of the header, footer and text (256 = fully shown).
    pub fn chrome(&self) -> u32 {
        (256 - self.attract).max(0) as u32
    }
}

pub fn wordmark(g: &mut Gfx, x: i32, cy: i32, a: u32) -> i32 {
    let y = cy - MICH12.cap_mid();
    text::draw(g, &MICH12, x, y, "PSo", scale(TEXT, a), 0);
    let w = MICH12.width("PSo", 0);
    text::draw(g, &MICH12, x + w, y, "X", scale(ACCENT, a), 0);
    let w2 = MICH12.width("PSoX", 0);
    text::draw(g, &MICH12, x + w2, y, "ide", scale(TEXT, a), 0);
    x + MICH12.width("PSoXide", 0)
}

fn icon(g: &mut Gfx, gl: &Glyph, x: i32, cy: i32, c: Rgb) {
    g.ui_mode();
    let y = cy - gl.h as i32 / 2;
    g.ui_sprite(x, y, gl.w as u16, gl.h as u16, (gl.u, gl.v), c);
}

/// Header: wordmark, disc name and version on the left; the song, its level
/// meter and the language switch on the right; one rule under it. Everything
/// is centred on [`HEAD_MID`].
pub fn header(g: &mut Gfx, ctx: &Ctx) {
    let a = ctx.chrome();
    if a == 0 {
        return;
    }
    let cy = HEAD_MID;
    let mut x = wordmark(g, MARGIN, cy, a);
    x += 6;
    g.rect(x, cy - 4, 1, 9, scale((34, 52, 62), a));
    x += 1 + 6;
    text::draw_mid(g, &JOST9, x, cy, ctx.disc_title, scale(DIM, a), 1);
    x += JOST9.width(ctx.disc_title, 1) + 6;
    text::draw_mid(g, &JOST9, x, cy, ctx.disc_version, scale(RING, a), 0);

    // Language switch: two cells in one frame, the active one lit, flush with
    // the right margin.
    let cell = JOST9.width("EN", 1).max(JOST9.width("IT", 1)) + 8;
    let lx = RIGHT - (2 * cell + 1);
    g.frame(lx, cy - CHIP_H / 2, 2 * cell + 1, CHIP_H, scale(RING, a));
    let lit = if ctx.it { lx + cell + 1 } else { lx + 1 };
    g.rect(lit, cy - CHIP_H / 2 + 1, cell - 1, CHIP_H - 2, scale((10, 50, 62), a));
    g.rect(lx + cell, cy - CHIP_H / 2 + 1, 1, CHIP_H - 2, scale(RING, a));
    let (en, it) = if ctx.it { (DIM, TEXT) } else { (TEXT, DIM) };
    text::draw_centered_mid(g, &JOST9, lx + cell / 2 + 1, cy, "EN", scale(en, a), 1);
    text::draw_centered_mid(g, &JOST9, lx + cell + 1 + cell / 2, cy, "IT", scale(it, a), 1);

    // The Language key sits beside its switch, and the level meter beside that.
    let kx = lx - ICON_GAP - ICON_TRIANGLE.w as i32;
    icon(g, &ICON_TRIANGLE, kx, cy, scale(RING, a));
    if ctx.music.state != Playback::None {
        meter(g, &ctx.music, ctx.t, kx - HEAD_GAP - METER_W, cy, a);
        // The song's name has the row under the header, at the right margin.
        let (label, tint) = match ctx.music.state {
            Playback::Off => ("MUSIC OFF", DIM),
            Playback::Loading => ("LOADING", DIM),
            _ => (ctx.music.title, MUTED),
        };
        text::draw_right_mid(g, &JOST9, RIGHT, SONG_MID, label, scale(tint, a), 0);
    }
    g.rect_add(0, HEAD_RULE, 320, 1, scale((16, 26, 30), a));
}

/// Bars in the header's level meter, their width and pitch, and its width.
const METER_BARS: i32 = 8;
const METER_W: i32 = METER_BARS * 3 - 1;

/// The level meter: the disc's analysis folded to eight bars (low frequencies
/// on the left), mirrored about the header's centre line. A song with no
/// analysis rides a slow wave instead, and a silent or loading menu is flat.
fn meter(g: &mut Gfx, now: &NowPlaying, t: u32, bx: i32, cy: i32, a: u32) {
    for b in 0..METER_BARS {
        let level = match (now.state, now.levels) {
            (Playback::Playing, Some(levels)) => {
                let per = (levels.len() as i32 / METER_BARS).max(1);
                let mut sum = 0;
                for k in 0..per {
                    sum += levels.get((b * per + k) as usize).copied().unwrap_or(0) as i32;
                }
                (sum / per).min(255)
            }
            (Playback::Playing, None) => {
                let ph = (t as i32 * 53 + b * 520) as u16 & 4095;
                (sin_q12(ph) + 4096) >> 5
            }
            _ => 0,
        };
        let half = (level * 5 + 127) / 255; // 0..=5
        let h = 1 + 2 * half;
        let c = mix(DIM, ACCENT, (level * 256 / 255) as u32);
        g.rect(bx + b * 3, cy - half, 2, h, scale(c, a));
    }
}

/// A shoulder-button tag: an outlined box with the key name inside.
fn tag(g: &mut Gfx, x: i32, cy: i32, name: &str, a: u32) -> i32 {
    let w = JOST9.width(name, 0) + 8;
    g.frame(x, cy - CHIP_H / 2, w, CHIP_H, scale(RING, a));
    text::draw_centered_mid(g, &JOST9, x + w / 2, cy, name, scale(DIM, a), 0);
    w
}

/// Gap between an icon and its label.
const ICON_GAP: i32 = 4;
/// Gap between footer hints.
const HINT_GAP: i32 = 10;

/// One footer hint: glyph then label. Returns the x after it.
fn hint(g: &mut Gfx, x: i32, gl: &Glyph, label: &str, a: u32) -> i32 {
    icon(g, gl, x, FOOT_MID, scale(RING, a));
    text::draw_mid(
        g,
        &JOST9,
        x + gl.w as i32 + ICON_GAP,
        FOOT_MID,
        label,
        scale(MUTED, a),
        0,
    );
    x + gl.w as i32 + ICON_GAP + JOST9.width(label, 0) + HINT_GAP
}

/// A hint whose key is a tag box (SELECT, L1, R1) rather than a glyph.
fn tag_hint(g: &mut Gfx, x: i32, key: &str, label: &str, a: u32) -> i32 {
    let w = tag(g, x, FOOT_MID, key, a);
    text::draw_mid(g, &JOST9, x + w + ICON_GAP, FOOT_MID, label, scale(MUTED, a), 0);
    x + w + ICON_GAP + JOST9.width(label, 0) + HINT_GAP
}

/// Footer: the hints for the screen from the left margin, the music keys at
/// the right one, all centred on [`FOOT_MID`].
pub fn footer(g: &mut Gfx, ctx: &Ctx) {
    let a = ctx.chrome();
    if a == 0 {
        return;
    }
    let it = ctx.it;
    g.rect_add(0, FOOT_RULE, 320, 1, scale((12, 20, 24), a));
    let mut x = MARGIN;
    match ctx.screen {
        Screen::Shelf => {
            icon(g, &ICON_CHEV_L, x, FOOT_MID, scale(RING, a));
            x += ICON_CHEV_L.w as i32 + 2;
            icon(g, &ICON_CHEV_R, x, FOOT_MID, scale(RING, a));
            x += ICON_CHEV_R.w as i32 + ICON_GAP;
            let browse = if it { "Sfoglia" } else { "Browse" };
            text::draw_mid(g, &JOST9, x, FOOT_MID, browse, scale(MUTED, a), 0);
            x += JOST9.width(browse, 0) + HINT_GAP;
            x = hint(g, x, &ICON_CROSS, if it { "Avvia" } else { "Start" }, a);
            let _ = tag_hint(g, x, "SELECT", if it { "Crediti" } else { "Credits" }, a);
        }
        Screen::Credits => {
            x = hint(g, x, &ICON_CIRCLE, if it { "Indietro" } else { "Back" }, a);
            let _ = hint(g, x, &ICON_TRIANGLE, if it { "Lingua" } else { "Language" }, a);
        }
    }
    // Music keys, right-aligned.
    if ctx.music.state != Playback::None {
        let label = if it { "Musica" } else { "Music" };
        let lw = JOST9.width(label, 0);
        text::draw_right_mid(g, &JOST9, RIGHT, FOOT_MID, label, scale(MUTED, a), 0);
        let tw = JOST9.width("R1", 0) + 8;
        let rx = RIGHT - lw - ICON_GAP - tw;
        tag(g, rx, FOOT_MID, "R1", a);
        tag(g, rx - 2 - JOST9.width("L1", 0) - 8, FOOT_MID, "L1", a);
    }
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

/// Where a case sits on screen at scale `sc` (Q8): the body box, which hugs
/// the art box with `pad` pixels on every side, and the art box inside it.
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
        let (aw, ah) = ((L_W * sc) >> 8, (L_H * sc) >> 8);
        let pad = ((3 * sc) >> 8).max(2);
        let (w, h) = (aw + 2 * pad, ah + 2 * pad);
        let (x0, y0) = (cx - w / 2, bottom - h);
        CaseGeom {
            x0,
            y0,
            w,
            h,
            ax: x0 + pad,
            ay: y0 + pad,
            aw,
            ah,
        }
    }
}

/// Turn of a case about its vertical axis, from the shelf's `yaw` units
/// (-256..=256, positive leans the far edge up on the left) to Q12 turns.
fn yaw_angle(yaw: i32) -> i32 {
    -yaw * 2
}

/// The one transform a case is drawn through: the GTE rotates the case about
/// its vertical axis and projects it, and the picture, the plastic body, the
/// frame, the corner brackets and the glare all take their screen points
/// from [`CaseXform::at`]. They can no longer disagree about the tilt.
///
/// Points are given where the case would be drawn if it faced the camera;
/// `at` returns where the tilted case puts them. A case that faces the camera
/// (`flat`) maps every point to itself.
pub struct CaseXform {
    mx: i32,
    my: i32,
    flat: bool,
}

impl CaseXform {
    /// Aim the GTE at `geom`'s case turned by `yaw`. The camera sits one and a
    /// half case widths away, so the near edge of a strongly turned case is
    /// about a third taller than the far one.
    ///
    /// The GTE state is global: use the transform before the next `aim`.
    pub fn aim(geom: &CaseGeom, yaw: i32) -> Self {
        let (mx, my) = (geom.x0 + geom.w / 2, geom.y0 + geom.h / 2);
        let angle = yaw_angle(yaw);
        let flat = angle == 0;
        if !flat {
            let dist = geom.w * 3 / 2;
            let (s, c) = (sin_q12(angle as u16), cos_q12(angle as u16));
            // Rotation about the vertical axis; +z points away from the
            // camera, so a positive angle brings the right edge closer.
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
            scene::set_screen_offset(mx << 16, my << 16);
        }
        CaseXform { mx, my, flat }
    }

    /// Where the flat-case point `(x, y)` lands.
    pub fn at(&self, x: i32, y: i32) -> (i32, i32) {
        if self.flat {
            return (x, y);
        }
        let p = scene::project_vertex(Vec3I16 {
            x: (x - self.mx) as i16,
            y: (y - self.my) as i16,
            z: 0,
        });
        (p.sx as i32, p.sy as i32)
    }

    /// The four corners of a flat rectangle, top-left first, in the order a
    /// quad takes them (top-left, top-right, bottom-left, bottom-right).
    pub fn quad(&self, x: i32, y: i32, w: i32, h: i32) -> [(i32, i32); 4] {
        [
            self.at(x, y),
            self.at(x + w, y),
            self.at(x, y + h),
            self.at(x + w, y + h),
        ]
    }

    /// A rectangle's outline (`w` by `h` pixels, last pixel inclusive).
    pub fn frame(&self, g: &mut Gfx, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        let (x1, y1) = (x + w - 1, y + h - 1);
        let (a, b, d, e) = (
            self.at(x, y),
            self.at(x1, y),
            self.at(x, y1),
            self.at(x1, y1),
        );
        g.line(a, b, c);
        g.line(d, e, c);
        g.line(a, d, c);
        g.line(b, e, c);
    }

    /// Four corner brackets around a rectangle, `len` long.
    pub fn brackets(&self, g: &mut Gfx, x: i32, y: i32, w: i32, h: i32, len: i32, c: Rgb) {
        let (x1, y1) = (x + w - 1, y + h - 1);
        for (cx, cy, sx, sy) in [
            (x, y, 1, 1),
            (x1, y, -1, 1),
            (x, y1, 1, -1),
            (x1, y1, -1, -1),
        ] {
            let corner = self.at(cx, cy);
            g.line(corner, self.at(cx + sx * (len - 1), cy), c);
            g.line(corner, self.at(cx, cy + sy * (len - 1)), c);
        }
    }
}

/// A case: framed cover with gloss, scaled `sc` (Q8) about its bottom centre.
/// `yaw` (-256..=256) turns it about its vertical axis; `dim` darkens;
/// `focus` brightens the outline and brackets.
pub struct CaseView {
    pub cx: i32,
    pub bottom: i32,
    pub sc: i32,
    pub dim: i32,
    pub focus: i32,
    pub yaw: i32,
}

/// The case's plastic body: a base with a lit spine down the hinge side.
pub fn case_body(g: &mut Gfx, cv: &CaseView, xf: &CaseXform) {
    let c = CaseGeom::new(cv.cx, cv.bottom, cv.sc);
    let dim = cv.dim as u32;
    let bodyc = scale((9, 20, 27), dim);
    let foot = scale((4, 11, 15), dim);
    g.quad_g(xf.quad(c.x0, c.y0, c.w, c.h), [bodyc, bodyc, foot, foot]);
    g.line_g(
        xf.at(c.x0 + 1, c.y0 + 1),
        xf.at(c.x0 + 1, c.y0 + c.h - 2),
        scale((34, 64, 74), dim),
        scale((12, 24, 30), dim),
    );
}

/// The case's front: cover art, gloss and outline (the closed case).
pub fn case_front(g: &mut Gfx, cv: &CaseView, xf: &CaseXform, e: &Entry) {
    let c = CaseGeom::new(cv.cx, cv.bottom, cv.sc);
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
    let aq = xf.quad(c.ax, c.ay, c.aw, c.ah);
    match cover::of(e.name_str()) {
        Cover::Art(i) => {
            if xf.flat && c.aw == L_W {
                g.art(Size::L, i, c.ax, c.ay, tint);
            } else {
                // One ribbon of quads whose vertices are projected points, so
                // the picture takes the same perspective as everything else.
                let n = if xf.flat {
                    1
                } else {
                    (c.aw / 16).clamp(2, 6) as usize
                };
                let mut tops = [(0, 0); 7];
                let mut bots = [(0, 0); 7];
                for k in 0..=n {
                    let x = c.ax + c.aw * k as i32 / n as i32;
                    tops[k] = xf.at(x, c.ay);
                    bots[k] = xf.at(x, c.ay + c.ah);
                }
                g.art_ribbon(size, i, &tops[..=n], &bots[..=n], tint);
            }
        }
        Cover::Plain => plain_cover(g, aq, dim),
    }
    // Gloss: a faint additive wedge from the top-left, as the shell's glass.
    let gl = scale((18, 30, 34), dim);
    g.quad_g_add(
        [
            xf.at(c.x0, c.y0),
            xf.at(c.x0 + c.w * 3 / 5, c.y0),
            xf.at(c.x0, c.y0 + c.h / 2),
            xf.at(c.x0 + c.w / 4, c.y0 + c.h / 2),
        ],
        [gl, (0, 0, 0), (0, 0, 0), (0, 0, 0)],
    );
    case_outline(g, cv, xf);
}

/// The frame and, on a focused case, the corner brackets and a faint tint.
pub fn case_outline(g: &mut Gfx, cv: &CaseView, xf: &CaseXform) {
    let c = CaseGeom::new(cv.cx, cv.bottom, cv.sc);
    let dim = cv.dim as u32;
    let ring = scale(mix(RING, FOCUS, cv.focus as u32), dim);
    xf.frame(g, c.x0, c.y0, c.w, c.h, ring);
    if cv.focus > 100 {
        xf.brackets(
            g,
            c.x0 - 2,
            c.y0 - 2,
            c.w + 4,
            c.h + 4,
            7,
            scale(FOCUS, dim),
        );
        let tint = scale((3, 14, 17), dim);
        g.quad_g_add(xf.quad(c.x0, c.y0, c.w, c.h), [tint; 4]);
    }
}

pub fn draw_case(g: &mut Gfx, cv: &CaseView, e: &Entry) {
    let geom = CaseGeom::new(cv.cx, cv.bottom, cv.sc);
    let xf = CaseXform::aim(&geom, cv.yaw);
    case_body(g, cv, &xf);
    case_front(g, cv, &xf, e);
}

/// A cover for a program the menu has no art for: a teal panel and the disc.
fn plain_cover(g: &mut Gfx, q: [(i32, i32); 4], dim: u32) {
    let top = scale((14, 54, 66), dim);
    let bot = scale((4, 14, 20), dim);
    g.quad_g(q, [top, top, bot, bot]);
    let (cx, cy) = ((q[0].0 + q[1].0) / 2, (q[0].1 + q[2].1) / 2);
    let r = (q[1].0 - q[0].0) / 4;
    g.ellipse_ring(cx, cy, r, r, scale((20, 120, 140), dim));
    g.ellipse_ring(cx, cy, r / 3, r / 3, scale((20, 120, 140), dim));
}
