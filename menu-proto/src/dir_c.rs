//! Direction C, "Index": the shell's flat list with a media panel beside it.
//!
//! Rows are hairline-separated labels with a disc mark, like the shell's
//! Settings and Controls lists. The highlight bar glides between rows; the
//! panel on the right holds the cover, the description and the version. No
//! 3D at all, which makes it the cheapest direction and the easiest to read
//! on a composite TV.

use crate::common::*;
use crate::ease;
use crate::gfx::*;
use crate::text;
use crate::ui_atlas::*;

const LIST_X: i32 = 10;
const LIST_W: i32 = 142;
const ROW_H: i32 = 19;
const LIST_TOP: i32 = 44;
const VISIBLE: i32 = 9;

pub struct State {
    /// Highlight bar position and list scroll, both in Q8 rows.
    pub bar: i32,
    pub scroll: i32,
    /// Panel content fade, 0..=256, dips on every move.
    pub swap: i32,
    pub last: i32,
}

pub fn update(s: &mut State, sel: i32, n: i32) {
    let top = (sel - 4).clamp(0, (n - VISIBLE).max(0));
    s.scroll = ease::follow(s.scroll, top * 256, 2);
    s.bar = ease::follow(s.bar, sel * 256, 2);
    if s.last != sel {
        s.last = sel;
        s.swap = 0;
    }
    s.swap = (s.swap + 40).min(256);
}

pub fn draw(g: &mut Gfx, s: &State, ctx: &Ctx, sel: i32, hide_sel: bool) {
    ground(g, (4, 9, 13), (3, 6, 9));
    g.glow(240, 90, 120, 90, (6, 30, 38));
    g.glow(30, 230, 120, 40, (4, 20, 26));
    header(g, ctx, "DEMO DISC");

    // Section label and list.
    text::draw(
        g,
        &MICH7,
        LIST_X + 4,
        31,
        if ctx.it { "PROGRAMMI" } else { "PROGRAMS" },
        DIM,
        2,
    );
    g.rect_add(LIST_X, 41, LIST_W, 1, (16, 26, 30));
    let n = ctx.entries.len() as i32;
    let row_y = |i: i32| LIST_TOP + ((i * 256 - s.scroll) * ROW_H >> 8);

    // The highlight bar under the labels.
    let by = row_y(0) + ((s.bar * ROW_H) >> 8);
    if !hide_sel {
        g.quad_g_add(
            [
                (LIST_X, by),
                (LIST_X + LIST_W, by),
                (LIST_X, by + ROW_H - 1),
                (LIST_X + LIST_W, by + ROW_H - 1),
            ],
            [(4, 30, 38), (1, 10, 14), (4, 30, 38), (1, 10, 14)],
        );
        g.rect(LIST_X, by, 2, ROW_H - 1, ACCENT);
        g.brackets(LIST_X - 1, by - 1, LIST_W + 2, ROW_H + 1, 5, FOCUS);
    }

    for i in 0..n {
        let y = row_y(i);
        if y < LIST_TOP - ROW_H / 2 || y > LIST_TOP + VISIBLE * ROW_H {
            continue;
        }
        let e = ctx.entries[i as usize];
        // How close the bar is to this row decides its brightness.
        let near = (256 - ((i * 256 - s.bar).abs() * 2)).max(0) as u32;
        let label = mix(TEXT2, TEXT, near);
        let mark = mix(RING, ACCENT, near);
        g.ui_mode();
        g.ui_sprite(LIST_X + 7, y + 2, 14, 14, (ICON_DISC.u, ICON_DISC.v), mark);
        text::draw(g, &JOST11, LIST_X + 26, y + 3, e.short, label, 0);
        if !e.version.is_empty() {
            text::draw_right(
                g,
                &JOST9,
                LIST_X + LIST_W - 6,
                y + 5,
                e.version,
                mix(DIM, MUTED, near),
                0,
            );
        }
        g.rect_add(LIST_X, y + ROW_H - 1, LIST_W, 1, (9, 16, 19));
    }
    // Scroll chevrons when the list overflows.
    if n > VISIBLE {
        g.ui_mode();
        if s.scroll > 8 {
            g.ui_sprite(
                LIST_X + LIST_W - 16,
                32,
                10,
                6,
                (ICON_CHEV_U.u, ICON_CHEV_U.v),
                RING,
            );
        }
        if s.scroll < (n - VISIBLE) * 256 - 8 {
            g.ui_sprite(
                LIST_X + LIST_W - 16,
                215,
                10,
                6,
                (ICON_CHEV_D.u, ICON_CHEV_D.v),
                RING,
            );
        }
    }

    // Media panel with the top-right chamfer.
    let (x0, y0, x1, y1, ch) = (162, 30, 312, 216, 12);
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

    let e = ctx.entries[sel as usize];
    let a = if hide_sel { 0 } else { s.swap.max(0) as u32 };
    if a > 4 {
        let ax = x0 + (x1 - x0 - L_W) / 2 - 3;
        let ay = y0 + 12;
        match e.art {
            Some(i) => {
                let v = ((128 * a) >> 8) as u8;
                g.art(Size::L, i, ax, ay, (v, v, v));
            }
            None => {
                let (t, b) = (scale((14, 54, 66), a), scale((4, 14, 20), a));
                g.quad_g(
                    [
                        (ax, ay),
                        (ax + L_W, ay),
                        (ax, ay + L_H),
                        (ax + L_W, ay + L_H),
                    ],
                    [t, t, b, b],
                );
                g.ellipse_ring(ax + 52, ay + 39, 24, 24, scale((20, 120, 140), a));
                g.ellipse_ring(ax + 52, ay + 39, 8, 8, scale((20, 120, 140), a));
            }
        }
        g.frame(ax - 1, ay - 1, L_W + 2, L_H + 2, scale(RING, a));
        g.brackets(ax - 4, ay - 4, L_W + 8, L_H + 8, 6, scale(FOCUS, a));
        let big = MICH12.width(e.name, 0) <= 136;
        if big {
            text::draw(g, &MICH12, ax - 4, ay + L_H + 8, e.name, scale(TEXT, a), 0);
        } else {
            text::draw(g, &MICH7, ax - 4, ay + L_H + 11, e.name, scale(TEXT, a), 0);
        }
        let off = ((256 - a as i32) * 10) >> 8;
        text::draw_wrapped(
            g,
            &JOST9,
            ax - 4 + off,
            ay + L_H + 28,
            138,
            10,
            6,
            ctx.desc(e),
            scale(MUTED, a),
            false,
        );
        pill(
            g,
            (x0 + x1) / 2 - 6,
            y1 - 24,
            if e.art.is_some() { "START" } else { "OPEN" },
            0,
        );
    }
    footer(
        g,
        ctx,
        Nav::Column,
        if ctx.it { "Scorri" } else { "Scroll" },
    );
}
