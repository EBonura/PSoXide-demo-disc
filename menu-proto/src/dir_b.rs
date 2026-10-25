//! Direction B, "Grid": every program as a small case, details below.
//!
//! The shell's compact grid at 320x240. Four covers across; the focused one
//! lifts, takes the bright outline and corner brackets, and the chamfered card
//! below slides its text in. The grid scrolls by whole rows, so the private
//! pressing (twelve programs) uses the same screen.

use crate::common::*;
use crate::ease;
use crate::gfx::*;
use crate::text;
use crate::ui_atlas::*;

pub const COLS: i32 = 4;
const PITCH_X: i32 = 76;
const PITCH_Y: i32 = 58;
const TOP: i32 = 29;
/// Tile frame around the 68x51 cover.
pub const TILE_W: i32 = 70;
pub const TILE_H: i32 = 53;
const PANEL_Y: i32 = 146;

pub struct State {
    /// Per-tile focus, 0..=256, each following its target.
    pub focus: [i32; 16],
    /// Scroll in Q8 rows.
    pub scroll: i32,
}

pub fn update(s: &mut State, sel: i32, n: i32) {
    for i in 0..n.min(16) {
        let target = if i == sel { 256 } else { 0 };
        s.focus[i as usize] = ease::follow(s.focus[i as usize], target, 2);
    }
    let rows = (n + COLS - 1) / COLS;
    let target_row = ((sel / COLS) - 1).clamp(0, (rows - 2).max(0));
    s.scroll = ease::follow(s.scroll, target_row * 256, 2);
}

pub fn tile_origin(i: i32, scroll: i32) -> (i32, i32) {
    let (col, row) = (i % COLS, i / COLS);
    (
        160 + (col * 2 - (COLS - 1)) * PITCH_X / 2 - TILE_W / 2,
        TOP + row * PITCH_Y - ((scroll * PITCH_Y) >> 8),
    )
}

fn credits_tile(g: &mut Gfx, x: i32, y: i32, dim: u32) {
    let (t, b) = (scale((14, 54, 66), dim), scale((4, 14, 20), dim));
    g.quad_g(
        [(x, y), (x + M_W, y), (x, y + M_H), (x + M_W, y + M_H)],
        [t, t, b, b],
    );
    g.ellipse_ring(x + M_W / 2, y + M_H / 2, 14, 14, scale((20, 120, 140), dim));
    g.ellipse_ring(x + M_W / 2, y + M_H / 2, 5, 5, scale((20, 120, 140), dim));
}

fn card(g: &mut Gfx, x0: i32, y0: i32, x1: i32, y1: i32, ch: i32) {
    g.quad_g(
        [(x0, y0), (x1 - ch, y0), (x0, y1), (x1 - ch, y1)],
        [(14, 34, 42), (14, 34, 42), (7, 15, 20), (7, 15, 20)],
    );
    g.quad_g(
        [(x1 - ch, y0 + ch), (x1, y0 + ch), (x1 - ch, y1), (x1, y1)],
        [(12, 30, 38), (12, 30, 38), (7, 15, 20), (7, 15, 20)],
    );
    g.tri_g(
        [(x1 - ch, y0), (x1 - ch, y0 + ch), (x1, y0 + ch)],
        [(14, 34, 42), (12, 30, 38), (12, 30, 38)],
    );
    let edge = (30, 56, 66);
    g.rect(x0, y0, x1 - x0 - ch, 1, edge);
    g.line_add((x1 - ch, y0), (x1, y0 + ch), edge, edge);
    g.rect(x1 - 1, y0 + ch, 1, y1 - y0 - ch, edge);
    g.rect(x0, y1 - 1, x1 - x0, 1, edge);
    g.rect(x0, y0, 1, y1 - y0, edge);
}

pub fn draw(g: &mut Gfx, s: &State, ctx: &Ctx, sel: i32, hide_sel: bool) {
    ground(g, (4, 9, 13), (3, 7, 10));
    g.glow(60, 30, 130, 60, (6, 30, 38));
    g.glow(280, 200, 140, 60, (5, 26, 32));
    let n = ctx.entries.len() as i32;
    for i in 0..n {
        if hide_sel && i == sel {
            continue;
        }
        let (x, y0) = tile_origin(i, s.scroll);
        if y0 > PANEL_Y || y0 < -TILE_H {
            continue;
        }
        let f = s.focus[i as usize & 15];
        let y = y0 - ((3 * f) >> 8);
        let e = ctx.entries[i as usize];
        let dim = 150 + ((f * 106) >> 8) as u32;
        let body = scale((9, 20, 27), dim);
        g.quad_g(
            [
                (x, y),
                (x + TILE_W, y),
                (x, y + TILE_H),
                (x + TILE_W, y + TILE_H),
            ],
            [body, body, scale((4, 11, 15), dim), scale((4, 11, 15), dim)],
        );
        match e.art {
            Some(a) => {
                let v = ((128 * dim) >> 8) as u8;
                g.art(Size::M, a, x + 1, y + 1, (v, v, v));
            }
            None => credits_tile(g, x + 1, y + 1, dim),
        }
        g.quad_g_add(
            [(x, y), (x + 40, y), (x, y + 26), (x + 14, y + 26)],
            [scale((16, 28, 32), dim), (0, 0, 0), (0, 0, 0), (0, 0, 0)],
        );
        g.frame(x, y, TILE_W, TILE_H, mix(scale(RING, dim), FOCUS, f as u32));
        if f > 90 {
            g.frame(x - 1, y - 1, TILE_W + 2, TILE_H + 2, scale(FOCUS, f as u32));
            g.brackets(x - 3, y - 3, TILE_W + 6, TILE_H + 6, 6, FOCUS);
            g.rect_add(x, y, TILE_W, TILE_H, (2, 14, 18));
        }
    }

    // Tiles that scrolled under the header or the card are hidden by the
    // ground drawn over them again.
    g.quad_g([(0, 0), (320, 0), (0, 24), (320, 24)], [(4, 9, 13); 4]);
    header(g, ctx, "DEMO DISC");
    g.quad_g(
        [(0, PANEL_Y - 5), (320, PANEL_Y - 5), (0, 240), (320, 240)],
        [(3, 7, 10); 4],
    );

    let e = ctx.entries[sel as usize];
    let (x0, y0, x1, y1) = (10, PANEL_Y, 310, 217);
    card(g, x0, y0, x1, y1, 10);
    let a = if hide_sel {
        0
    } else {
        (256 - (256 - s.focus[sel as usize & 15]) * 2).max(0) as u32
    };
    if a > 8 {
        let off = ((256 - a as i32) * 10) >> 8;
        text::draw(g, &MICH12, 22 - off, y0 + 9, e.name, scale(TEXT, a), 0);
        text::draw_wrapped(
            g,
            &JOST9,
            22 - off * 2,
            y0 + 28,
            204,
            10,
            4,
            ctx.desc(e),
            scale(MUTED, a),
            false,
        );
        if let Some(i) = e.art {
            let v = ((128 * a) >> 8) as u8;
            g.art(Size::M, i, 235, y0 + 4, (v, v, v));
            g.frame(234, y0 + 3, M_W + 2, M_H + 2, scale(RING, a));
            g.brackets(232, y0 + 1, M_W + 6, M_H + 6, 5, scale(FOCUS, a));
        } else {
            credits_tile(g, 235, y0 + 4, a);
        }
        if !e.version.is_empty() {
            let w = JOST9.width(e.version, 0) + 14;
            g.frame(234, y0 + 57, w, 11, scale(RING, a));
            text::draw(g, &JOST9, 240, y0 + 58, "v", scale(DIM, a), 0);
            text::draw(g, &JOST9, 245, y0 + 58, e.version, scale(ACCENT, a), 0);
        }
    }
    footer(g, ctx, Nav::Grid, if ctx.it { "Muovi" } else { "Move" });
}
