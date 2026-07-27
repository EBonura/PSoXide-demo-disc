//! Drawing the carousel: everything that talks to the GPU.
//!
//! [`carousel`] works out where things go; this turns those positions into
//! gouraud triangles. Ellipses are triangle fans shaded top to bottom, which
//! at this size reads as a glossy lozenge without a single texture.

use carousel::{Bead, Placed, TURN};
use psx_gpu::material::BlendMode;
use psx_gpu::{self as gpu};
use psx_math::{cos_q12, sin_q12};

/// Most segments an ellipse is drawn with. Twelve reads as round at pill size.
const MAX_SEGMENTS: usize = 12;
/// Fewest. Below this a bead stops looking like a circle at all.
const MIN_SEGMENTS: usize = 6;

/// Segments worth spending on an ellipse of this size. The ball is seventy-odd
/// beads and most of them are a handful of pixels across, where twelve
/// segments buys nothing but triangles; at that count the menu was missing
/// 60 Hz, which stretched every time-driven effect with it.
fn segments_for(rx: i16, ry: i16) -> usize {
    let size = rx.max(ry) as usize;
    (size / 2).clamp(MIN_SEGMENTS, MAX_SEGMENTS)
}

const GLOSS_TOP: (u8, u8, u8) = (140, 205, 255);
const GLOSS_BOTTOM: (u8, u8, u8) = (6, 24, 82);
const GLOSS_EDGE: (u8, u8, u8) = (18, 66, 160);
const SPECULAR: (u8, u8, u8) = (235, 250, 255);

fn lerp(a: u8, b: u8, t: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    (a + (((b - a) * t as i32) >> 8)) as u8
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: u8) -> (u8, u8, u8) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

fn scale_rgb(c: (u8, u8, u8), t: u8) -> (u8, u8, u8) {
    (
        ((c.0 as u32 * t as u32) >> 8) as u8,
        ((c.1 as u32 * t as u32) >> 8) as u8,
        ((c.2 as u32 * t as u32) >> 8) as u8,
    )
}

fn ellipse(cx: i16, cy: i16, rx: i16, ry: i16, top: (u8, u8, u8), bottom: (u8, u8, u8)) {
    if rx <= 0 || ry <= 0 {
        return;
    }
    let segments = segments_for(rx, ry);
    let centre = mix(top, bottom, 128);
    let vertex = |seg: usize| -> ((i16, i16), (u8, u8, u8)) {
        let a = ((seg as i32 * TURN) / segments as i32) as u16;
        let sx = cx as i32 + ((rx as i32 * sin_q12(a)) >> 12);
        let sy = cy as i32 - ((ry as i32 * cos_q12(a)) >> 12);
        // cos is +1 at the top of the ellipse, -1 at the bottom.
        let t = ((4096 - cos_q12(a)) * 255 / 8192) as u8;
        ((sx as i16, sy as i16), mix(top, bottom, t))
    };

    let (mut prev_p, mut prev_c) = vertex(0);
    for seg in 1..=segments {
        let (p, c) = vertex(seg);
        gpu::draw_tri_gouraud([(cx, cy), prev_p, p], [centre, prev_c, c]);
        prev_p = p;
        prev_c = c;
    }
}

/// The shaft of light the demo-disc intros threw across the frame. Two
/// additive wedges, so it brightens whatever is behind it instead of hiding
/// it, and no texture is involved.
///
/// `pulse` swells it on the beat.
pub fn light_streak(pulse: u8) {
    let lift = 18 + (pulse as u16 * 26 / 255) as u8;
    let core = (lift, lift + lift / 2, lift * 2);
    let edge = (lift / 4, lift / 3, lift / 2);
    // A long wedge from off the top-left corner to off the bottom-right,
    // passing behind the ball.
    let wide = 26 + (pulse as i16 * 10 / 255);
    gpu::draw_tri_flat_blended(
        [(-40, -20), (-40 + wide, -20), (300, 260)],
        core.0,
        core.1,
        core.2,
        BlendMode::Add,
    );
    gpu::draw_tri_flat_blended(
        [(-40 + wide, -20), (300, 260), (300 + wide, 260)],
        edge.0,
        edge.1,
        edge.2,
        BlendMode::Add,
    );
}

/// One carousel pill: the lozenge, a rim, and a specular blob up and left.
///
/// `pulse` lifts the whole thing on the beat.
pub fn pill(item: &Placed, pulse: u8) {
    let lit = 90 + ((item.front as u32 * 165) >> 8) as u32;
    let dim = (lit + (pulse as u32 * 40 / 255)).min(255) as u8;
    ellipse(
        item.x,
        item.y,
        item.rx,
        item.ry,
        scale_rgb(GLOSS_TOP, dim),
        scale_rgb(GLOSS_BOTTOM, dim),
    );
    // A slightly smaller inner ellipse lifts the middle away from the rim.
    ellipse(
        item.x,
        item.y - item.ry / 6,
        (item.rx * 7) / 8,
        (item.ry * 5) / 8,
        scale_rgb(mix(GLOSS_TOP, SPECULAR, 90), dim),
        scale_rgb(GLOSS_EDGE, dim),
    );
    ellipse(
        item.x - (item.rx * 5) / 12,
        item.y - (item.ry * 5) / 12,
        item.rx / 5,
        item.ry / 4,
        scale_rgb(SPECULAR, dim),
        scale_rgb(mix(SPECULAR, GLOSS_TOP, 200), dim),
    );
}

/// The carousel mirrored in the floor it sits on: same pill, flipped about
/// `floor_y`, squashed and dimmed. Reflections of the far pills fall off the
/// bottom of the screen and clip away, which is the cheap half of the trick.
pub fn pill_reflection(item: &Placed, floor_y: i16) {
    let y = 2 * floor_y - item.y;
    let ry = (item.ry * 2) / 5;
    if y - ry > 239 || ry <= 0 {
        return;
    }
    // Dim, and darker at the top where it meets the real pill, so the two do
    // not read as one object.
    let lit = 40 + ((item.front as u32 * 55) >> 8) as u8;
    ellipse(
        item.x,
        y,
        item.rx,
        ry,
        scale_rgb(GLOSS_BOTTOM, lit),
        scale_rgb(GLOSS_TOP, lit),
    );
}

/// One of the small spheres in the cluster.
pub fn bead(bead: &Bead) {
    if bead.r <= 0 {
        return;
    }
    ellipse(
        bead.x,
        bead.y,
        bead.r,
        bead.r,
        scale_rgb(GLOSS_TOP, bead.lit),
        scale_rgb(GLOSS_BOTTOM, bead.lit),
    );
    // Only the near beads are big enough for a highlight to land on.
    if bead.r >= 5 {
        ellipse(
            bead.x - bead.r / 3,
            bead.y - bead.r / 3,
            bead.r / 3,
            bead.r / 3,
            scale_rgb(SPECULAR, bead.lit),
            scale_rgb(GLOSS_TOP, bead.lit),
        );
    }
}

/// The Union flag, 20x12. Not a texture: a blue field, then the white and red
/// saltires as fans of parallel lines, then the cross on top.
pub fn flag_uk(x: i16, y: i16) {
    const W: i16 = 20;
    const H: i16 = 12;
    const BLUE: (u8, u8, u8) = (10, 32, 110);
    const WHITE: (u8, u8, u8) = (240, 240, 245);
    const RED: (u8, u8, u8) = (200, 24, 44);

    gpu::draw_rect_flat(x, y, W as u16, H as u16, BLUE.0, BLUE.1, BLUE.2);
    for offset in -2..=2i16 {
        gpu::draw_line_mono(x + offset, y, x + W - 1 + offset, y + H - 1, WHITE.0, WHITE.1, WHITE.2);
        gpu::draw_line_mono(x + W - 1 + offset, y, x + offset, y + H - 1, WHITE.0, WHITE.1, WHITE.2);
    }
    gpu::draw_line_mono(x, y, x + W - 1, y + H - 1, RED.0, RED.1, RED.2);
    gpu::draw_line_mono(x + W - 1, y, x, y + H - 1, RED.0, RED.1, RED.2);
    gpu::draw_rect_flat(x, y + 3, W as u16, 6, WHITE.0, WHITE.1, WHITE.2);
    gpu::draw_rect_flat(x + 7, y, 6, H as u16, WHITE.0, WHITE.1, WHITE.2);
    gpu::draw_rect_flat(x, y + 5, W as u16, 2, RED.0, RED.1, RED.2);
    gpu::draw_rect_flat(x + 9, y, 2, H as u16, RED.0, RED.1, RED.2);
}

/// The Italian tricolour, 20x12.
pub fn flag_it(x: i16, y: i16) {
    gpu::draw_rect_flat(x, y, 7, 12, 0, 140, 69);
    gpu::draw_rect_flat(x + 7, y, 6, 12, 240, 240, 240);
    gpu::draw_rect_flat(x + 13, y, 7, 12, 205, 33, 42);
}
