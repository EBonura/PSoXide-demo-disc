//! Drawing the carousel: everything that talks to the GPU.
//!
//! [`carousel`] works out where things go; this turns those positions into
//! gouraud triangles. Ellipses are triangle fans shaded top to bottom, which
//! at this size reads as a glossy lozenge without a single texture.

use carousel::{Bead, Placed, TURN};
use psx_gpu as gpu;
use psx_math::{cos_q12, sin_q12};

/// Segments per ellipse. Twelve is enough to read as round at this size and
/// keeps the whole screen inside a few hundred triangles.
const SEGMENTS: usize = 12;

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
    let centre = mix(top, bottom, 128);
    let vertex = |seg: usize| -> ((i16, i16), (u8, u8, u8)) {
        let a = ((seg as i32 * TURN) / SEGMENTS as i32) as u16;
        let sx = cx as i32 + ((rx as i32 * sin_q12(a)) >> 12);
        let sy = cy as i32 - ((ry as i32 * cos_q12(a)) >> 12);
        // cos is +1 at the top of the ellipse, -1 at the bottom.
        let t = ((4096 - cos_q12(a)) * 255 / 8192) as u8;
        ((sx as i16, sy as i16), mix(top, bottom, t))
    };

    let (mut prev_p, mut prev_c) = vertex(0);
    for seg in 1..=SEGMENTS {
        let (p, c) = vertex(seg);
        gpu::draw_tri_gouraud([(cx, cy), prev_p, p], [centre, prev_c, c]);
        prev_p = p;
        prev_c = c;
    }
}

/// One carousel pill: the lozenge, a rim, and a specular blob up and left.
pub fn pill(item: &Placed) {
    let dim = 90 + ((item.front as u32 * 165) >> 8) as u8;
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
    if bead.r >= 3 {
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
