//! The Shelf: big cases on the shell's floor, one in focus.
//!
//! The home shell's Home screen at 320x240: the focused case lifts and
//! brightens inside corner brackets and sways on its hinge, its neighbours
//! recede in scale and light, a teal pool glows on the floor beneath it, and
//! the title, description and start button sit under the row. Vertical
//! positions are fixed rows (see the constants) so nothing drifts.

use crate::common::*;
use crate::ease;
use crate::gfx::*;
use crate::text;
use crate::ui_atlas::*;
use psx_math::sin_q12;

pub struct State {
    /// Position of the row in Q8 case units.
    pub pos: i32,
}

/// Row the focused case's feet stand on (before its lift).
pub const FLOOR: i32 = 133;
/// Where the floor grid meets the horizon.
pub const HORIZON: i32 = FLOOR + 7;
/// Pixel row the title's capitals are centred on.
const TITLE_MID: i32 = 141;
/// Pixel row of the description's first line (capitals centred), and the pitch.
const DESC_MID: i32 = 155;
const DESC_PITCH: i32 = 10;
/// Pixel row the start button is centred on.
const BUTTON_MID: i32 = 196;

pub fn update(s: &mut State, sel: i32) {
    s.pos = ease::follow(s.pos, sel * 256, 2);
}

/// Where a case `d` (Q8 slots from the centre) sits: (x offset, scale, dim).
fn layout(d: i32) -> (i32, i32, i32) {
    let a = d.abs();
    let x = ease::knots(a, &[(0, 0), (256, 98), (512, 158), (768, 215)]);
    let sc = ease::knots(a, &[(0, 256), (256, 168), (512, 128), (768, 100)]);
    let dim = ease::knots(a, &[(0, 256), (256, 128), (512, 74), (768, 40)]);
    (if d < 0 { -x } else { x }, sc, dim)
}

/// Draw the menu. `launch` is the frame count since the start press, or -1;
/// from frame 0 the selected case belongs to the launch sequence, and the
/// text and the button fade out under the pressed button's flash.
pub fn draw(g: &mut Gfx, s: &State, ctx: &Ctx, sel: i32, launch: i32) {
    g.glow(160, 78, 150, 80, (5, 22, 28));
    g.glow(160, HORIZON + 6, 190, 40, (9, 38, 46));
    floor_grid(g, HORIZON, 212);
    header(g, ctx, "DEMO DISC");
    footer(g, ctx);

    let hide_sel = launch >= 0;
    let n = ctx.entries.len() as i32;
    // Far to near so the focused case overlaps its neighbours.
    let mut order = [0i32; 16];
    let mut count = 0;
    for i in 0..n {
        order[count] = i;
        count += 1;
    }
    let dist = |i: i32| (i * 256 - s.pos).abs();
    for a in 0..count {
        for b in a + 1..count {
            if dist(order[b]) > dist(order[a]) {
                order.swap(a, b);
            }
        }
    }
    let sway = sin_q12((ctx.t * 6) as u16 & 4095) >> 4; // -256..=256
    for &i in &order[..count] {
        let d = i * 256 - s.pos;
        if d.abs() > 800 || (hide_sel && i == sel) {
            continue;
        }
        let (dx, sc, dim) = layout(d);
        let settle = (256 - d.abs() * 3 / 2).max(0);
        let lift = (3 * settle) >> 8;
        let yaw = if settle > 200 {
            (sway * settle) >> 11
        } else if d < 0 {
            -150
        } else {
            150
        };
        // Pool of light under each case, brightest for the focused one.
        let pool = scale((10, 52, 62), (settle as u32 * 3 >> 2) + 20);
        let rx = (66 * sc) >> 8;
        g.glow(160 + dx, FLOOR + 3, rx, 6, pool);
        if settle > 120 {
            let ring = scale((14, 60, 70), settle as u32);
            g.ellipse_ring(160 + dx, FLOOR + 3, rx + 10, 7, ring);
            g.ellipse_ring(160 + dx, FLOOR + 3, rx + 26, 11, scale(ring, 120));
        }
        let cv = CaseView {
            cx: 160 + dx,
            bottom: FLOOR - lift,
            sc,
            dim,
            focus: settle,
            yaw,
        };
        draw_case(g, &cv, ctx.entries[i as usize]);
    }

    // Title, description and the primary action for the selected entry.
    let e = ctx.entries[sel as usize];
    let near = ((256 - (sel * 256 - s.pos).abs() * 2).max(0)) as u32;
    let gone = if hide_sel {
        256 - ease::span(launch, 0, 16) as u32
    } else {
        256
    };
    let a = near * gone >> 8;
    if a > 8 {
        let off = ((256 - near as i32) * 6) >> 8;
        text::draw_centered_mid(g, &MICH12, 160, TITLE_MID + off, e.name, scale(TEXT, a), 0);
        text::draw_wrapped(
            g,
            &JOST9,
            56,
            DESC_MID - JOST9.cap_mid() + off * 2,
            208,
            DESC_PITCH,
            3,
            ctx.desc(e),
            scale(MUTED, a),
            true,
        );
        let label = match (e.art.is_some(), ctx.it) {
            (true, false) => "START",
            (true, true) => "AVVIA",
            (false, false) => "OPEN",
            (false, true) => "APRI",
        };
        let press = if hide_sel { Press::Down } else { Press::Idle };
        start_button(g, 160, BUTTON_MID + off * 2, label, press, ctx.t, a);
    }
}
