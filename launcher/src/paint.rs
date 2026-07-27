//! Drawing the carousel: everything that talks to the GPU.
//!
//! [`carousel`] works out where things go; this turns those positions into
//! gouraud triangles. Ellipses are triangle fans shaded top to bottom, which
//! at this size reads as a glossy lozenge without a single texture.

use carousel::{Bead, Placed, TURN};
use psx_gpu::material::BlendMode;
use psx_vram::{Clut, Color555, Tpage};
use psx_gpu::{self as gpu};
use psx_math::{cos_q12, sin_q12};

/// Most segments an ellipse is drawn with. These are triangle fans, so this is
/// the polygon count: at twelve the pills read as coarse dodecagons, and at
/// 320x240 with no antialiasing every facet shows.
const MAX_SEGMENTS: usize = 18;
/// Fewest. Below this a bead stops looking like a circle at all.
const MIN_SEGMENTS: usize = 9;

/// Segments worth spending on an ellipse of this size. The ball is seventy-odd
/// beads and most of them are a handful of pixels across, where twelve
/// segments buys nothing but triangles; at that count the menu was missing
/// 60 Hz, which stretched every time-driven effect with it.
fn segments_for(rx: i16, ry: i16) -> usize {
    let size = rx.max(ry) as usize;
    size.clamp(MIN_SEGMENTS, MAX_SEGMENTS)
}

const GLOSS_TOP: (u8, u8, u8) = (255, 66, 44);
const GLOSS_BOTTOM: (u8, u8, u8) = (58, 0, 2);
const GLOSS_EDGE: (u8, u8, u8) = (178, 10, 12);
const SPECULAR: (u8, u8, u8) = (255, 196, 170);

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

const METER_WIDTH: u16 = 3;
const METER_PITCH: i16 = 4;
const METER_TALLEST: i16 = 20;

fn meter_bar(x: i16, base_y: i16, level: u8) {
    let h = 1 + (level as i16 * (METER_TALLEST - 1)) / 255;
    // Tall bars run hot, short ones stay in the dark blue.
    // Tall bars run hot toward orange, short ones stay in the deep red.
    let heat = level / 2;
    gpu::draw_rect_flat(
        x,
        base_y - h,
        METER_WIDTH,
        h as u16,
        180u8.saturating_add(level / 4),
        20u8.saturating_add(heat),
        16u8.saturating_add(heat / 3),
    );
}

/// The level meter, one bar per band of the track's pre-analysed spectrum,
/// low frequencies on the left.
pub fn level_meter(x: i16, base_y: i16, levels: &[u8]) {
    for (band, level) in levels.iter().enumerate() {
        meter_bar(x + band as i16 * METER_PITCH, base_y, *level);
    }
}

/// Fallback for a track the disc carries no analysis for: bars lagging each
/// other off the beat, which keeps time without listening.
pub fn level_meter_beat(x: i16, base_y: i16, pulse: u8) {
    for bar in 0..6i16 {
        let level = pulse.saturating_sub((bar as u16 * 34) as u8);
        meter_bar(x + bar * METER_PITCH, base_y, level);
    }
}


/// Flags are drawn at this size in the top-right corner. Two to one, which is
/// the Union flag's own ratio; at three to two it read as squat.
pub const FLAG_W: i16 = 24;
pub const FLAG_H: i16 = 12;

/// The PSoXide mark, as the one texture on the screen.
///
/// 4bpp with a sixteen-colour palette, and palette entry zero left at
/// `0x0000`, which the PlayStation treats as transparent: the starfield shows
/// through the letterforms rather than the logo sitting in a black box.
pub struct Banner {
    tpage: Tpage,
    clut: Clut,
    w: i16,
    h: i16,
}

impl Banner {
    /// Push the texture and its palette into VRAM. `tpage` must be 4bpp and
    /// clear of the framebuffer; the whole thing has to fit one page, since
    /// UVs are bytes.
    pub fn upload(pixels: &[u8], palette: &[u8], w: i16, h: i16, tpage: Tpage, clut: Clut) -> Self {
        // Four texels a halfword, so a row is a quarter as wide in VRAM.
        let halfwords_per_row = (w as u16).div_ceil(4);
        psx_vram::upload_bytes(
            psx_vram::VramRect::new(tpage.x(), tpage.y(), halfwords_per_row, h as u16),
            pixels,
        );
        let entries: [Color555; 16] = core::array::from_fn(|i| {
            Color555::raw(u16::from_le_bytes([palette[i * 2], palette[i * 2 + 1]]))
        });
        psx_vram::upload_clut(clut, &entries);
        Banner { tpage, clut, w, h }
    }

    /// Draw it with its top-left at `x, y`, one texel to one pixel.
    pub fn draw(&self, x: i16, y: i16) {
        let (w, h) = (self.w, self.h);
        let (u1, v1) = ((w - 1) as u8, (h - 1) as u8);
        gpu::draw_quad_textured(
            [(x, y), (x + w, y), (x, y + h), (x + w, y + h)],
            [(0, 0), (u1, 0), (0, v1), (u1, v1)],
            self.clut.uv_clut_word(),
            self.tpage.uv_tpage_word(0),
            // Neutral tint: 128 is "as the texture is" on this hardware.
            (128, 128, 128),
        );
    }
}

/// The solid black band the mark and the title sit in, with the same border
/// the text panel uses so the two read as one system.
pub fn header_strip(h: i16) {
    const BORDER: (u8, u8, u8) = (150, 30, 34);
    gpu::draw_rect_flat(0, 0, 320, h as u16, 0, 0, 0);
    gpu::draw_rect_flat(0, h - 1, 320, 1, BORDER.0, BORDER.1, BORDER.2);
}

/// A dark panel to lay text over, with a thin border.
///
/// Black at [`BlendMode::Average`] is `(background + 0) / 2`, so it halves
/// whatever it covers rather than hiding it: the ball and the starfield stay
/// visible underneath, just far enough back for white text to sit on them.
pub fn text_panel(x: i16, y: i16, w: i16, h: i16) {
    const BORDER: (u8, u8, u8) = (150, 30, 34);
    // Two triangles, since the SDK blends triangles and not rectangles.
    for tri in [
        [(x, y), (x + w, y), (x, y + h)],
        [(x + w, y), (x, y + h), (x + w, y + h)],
    ] {
        gpu::draw_tri_flat_blended(tri, 0, 0, 0, BlendMode::Average);
    }
    // Border solid rather than blended, so the edge stays crisp against
    // whatever is behind it.
    gpu::draw_rect_flat(x, y, w as u16, 1, BORDER.0, BORDER.1, BORDER.2);
    gpu::draw_rect_flat(x, y + h - 1, w as u16, 1, BORDER.0, BORDER.1, BORDER.2);
    gpu::draw_rect_flat(x, y, 1, h as u16, BORDER.0, BORDER.1, BORDER.2);
    gpu::draw_rect_flat(x + w - 1, y, 1, h as u16, BORDER.0, BORDER.1, BORDER.2);
}

/// The Union flag.
///
/// Drawn a row at a time rather than as two big parallelograms. At 24x12 the
/// saltire steps exactly two across for one down, corner to corner, and
/// stepping it by hand gets that; handing the GPU a parallelogram lets its
/// own edge rules pick the steps, which came out ragged and uneven.
///
/// Proportions follow the real flag where they fit: the cross of St George is
/// a fifth of the height in red with a pixel of white either side. The arms of
/// the saltire are a pixel wider than geometry wants, because at twelve pixels
/// tall a true-width diagonal disappears.
///
/// The counterchange is real: St Patrick's red sits below the white diagonal
/// on the hoist and above it on the fly, which is the asymmetry the flag is
/// recognised by and the first thing an eyeballed version loses.
pub fn flag_uk(x: i16, y: i16) {
    const BLUE: (u8, u8, u8) = (8, 24, 92);
    const WHITE: (u8, u8, u8) = (238, 238, 242);
    const RED: (u8, u8, u8) = (184, 16, 36);
    let (w, h) = (FLAG_W, FLAG_H);

    gpu::draw_rect_flat(x, y, w as u16, h as u16, BLUE.0, BLUE.1, BLUE.2);

    // A horizontal run of one row, clipped to the flag.
    let run = |from: i16, len: i16, row: i16, c: (u8, u8, u8)| {
        let start = from.max(0);
        let end = (from + len).min(w);
        if end > start {
            gpu::draw_rect_flat(x + start, y + row, (end - start) as u16, 1, c.0, c.1, c.2);
        }
    };

    for row in 0..h {
        // Two across for one down, both ways.
        let down = row * 2;
        let up = w - 2 - down;
        run(down - 1, 4, row, WHITE);
        run(up - 1, 4, row, WHITE);

        // St Patrick, offset within the white. Hoist half low, fly half high,
        // which is what counterchanged means here.
        let hoist = row < h / 2;
        let down_red = if hoist { down + 1 } else { down - 1 };
        let up_red = if hoist { up - 1 } else { up + 1 };
        run(down_red, 2, row, RED);
        run(up_red, 2, row, RED);
    }

    // Cross of St George over the top, white-bordered.
    gpu::draw_rect_flat(x, y + 4, w as u16, 4, WHITE.0, WHITE.1, WHITE.2);
    gpu::draw_rect_flat(x + 10, y, 4, h as u16, WHITE.0, WHITE.1, WHITE.2);
    gpu::draw_rect_flat(x, y + 5, w as u16, 2, RED.0, RED.1, RED.2);
    gpu::draw_rect_flat(x + 11, y, 2, h as u16, RED.0, RED.1, RED.2);
}

/// The Italian tricolour.
pub fn flag_it(x: i16, y: i16) {
    let third = (FLAG_W / 3) as u16;
    let h = FLAG_H as u16;
    gpu::draw_rect_flat(x, y, third, h, 0, 140, 69);
    gpu::draw_rect_flat(x + third as i16, y, third, h, 240, 240, 240);
    gpu::draw_rect_flat(x + 2 * third as i16, y, third, h, 205, 33, 42);
}
