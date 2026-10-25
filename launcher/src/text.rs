//! Anti-aliased bitmap faces baked by `tools/menu-art`.
//!
//! One 4bpp page holds every face. Texel 0 is transparent and 1..=15 are
//! coverage, so a glyph is a textured sprite blended additively: on the menu's
//! near-black ground that reads as the glyph's colour at its coverage.

pub use crate::face::{Face, Glyph};
use crate::gfx::{Gfx, Rgb};

/// Draw `s` with its top-left at `x, y`.
pub fn draw(g: &mut Gfx, face: &Face, x: i32, y: i32, s: &str, color: Rgb, track: i32) {
    g.ui_mode();
    let mut pen = x;
    for c in s.bytes() {
        let gl = face.glyph(c);
        if gl.w > 0 {
            g.ui_sprite(
                pen + gl.xoff as i32,
                y + gl.yoff as i32,
                gl.w as u16,
                gl.h as u16,
                (gl.u, gl.v),
                color,
            );
        }
        pen += gl.adv as i32 + track;
    }
}

/// Centred on `cx`.
pub fn draw_centered(g: &mut Gfx, face: &Face, cx: i32, y: i32, s: &str, color: Rgb, track: i32) {
    let w = face.width(s, track);
    draw(g, face, cx - w / 2, y, s, color, track);
}

/// Right-aligned to `right`.
pub fn draw_right(g: &mut Gfx, face: &Face, right: i32, y: i32, s: &str, color: Rgb, track: i32) {
    let w = face.width(s, track);
    draw(g, face, right - w, y, s, color, track);
}

/// How many lines [`draw_wrapped`] needs for `s` at `max_w` pixels.
pub fn wrapped_lines(face: &Face, s: &str, max_w: i32) -> usize {
    let space = face.glyph(b' ').adv as i32;
    let (mut lines, mut used) = (0, 0);
    for word in s.split(' ').filter(|w| !w.is_empty()) {
        let w = face.width(word, 0);
        if lines == 0 {
            lines = 1;
            used = w;
        } else if used + space + w > max_w {
            lines += 1;
            used = w;
        } else {
            used += space + w;
        }
    }
    lines
}

/// Greedy word wrap of `s` into lines of at most `max_w` pixels; each line is
/// drawn `line_h` below the last, at most `max_lines` of them. Returns the
/// number of lines drawn.
pub fn draw_wrapped(
    g: &mut Gfx,
    face: &Face,
    x: i32,
    y: i32,
    max_w: i32,
    line_h: i32,
    max_lines: usize,
    s: &str,
    color: Rgb,
    center: bool,
) -> usize {
    let mut lines = 0;
    let mut rest = s;
    while !rest.is_empty() && lines < max_lines {
        // Longest prefix of whole words that fits, measured one word at a time.
        let space = face.glyph(b' ').adv as i32;
        let mut end = 0;
        let mut at = 0;
        let mut used = 0;
        loop {
            let next = rest[at..].find(' ').map_or(rest.len(), |i| at + i);
            let ww = face.width(&rest[at..next], 0);
            let cand = if end == 0 { ww } else { used + space + ww };
            if cand > max_w && end > 0 {
                break;
            }
            used = cand;
            end = next;
            if next >= rest.len() {
                break;
            }
            at = next + 1;
        }
        let line = &rest[..end];
        let ly = y + lines as i32 * line_h;
        if center {
            draw_centered(g, face, x + max_w / 2, ly, line, color, 0);
        } else {
            draw(g, face, x, ly, line, color, 0);
        }
        lines += 1;
        rest = rest[end..].trim_start();
    }
    lines
}

/// Draw `s` with its capitals centred vertically on pixel row `cy`.
pub fn draw_mid(g: &mut Gfx, face: &Face, x: i32, cy: i32, s: &str, color: Rgb, track: i32) {
    draw(g, face, x, cy - face.cap_mid(), s, color, track);
}

/// As [`draw_mid`], centred horizontally on `cx`.
pub fn draw_centered_mid(
    g: &mut Gfx,
    face: &Face,
    cx: i32,
    cy: i32,
    s: &str,
    color: Rgb,
    track: i32,
) {
    draw_centered(g, face, cx, cy - face.cap_mid(), s, color, track);
}

/// As [`draw_mid`], right-aligned to the pixel edge `right`.
pub fn draw_right_mid(
    g: &mut Gfx,
    face: &Face,
    right: i32,
    cy: i32,
    s: &str,
    color: Rgb,
    track: i32,
) {
    draw_right(g, face, right, cy - face.cap_mid(), s, color, track);
}
