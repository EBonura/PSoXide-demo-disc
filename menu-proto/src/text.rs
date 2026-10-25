//! Anti-aliased bitmap faces baked by `tools/menu-art`.
//!
//! One 4bpp page holds every face. Texel 0 is transparent and 1..=15 are
//! coverage, so a glyph is a textured sprite blended additively: on the menu's
//! near-black ground that reads as the glyph's colour at its coverage.

use crate::gfx::{Gfx, Rgb};

/// One glyph's place in the atlas page and how to lay it out.
#[derive(Clone, Copy)]
pub struct Glyph {
    pub u: u8,
    pub v: u8,
    pub w: u8,
    pub h: u8,
    pub xoff: i8,
    pub yoff: i8,
    pub adv: u8,
}

/// Printable ASCII, space to tilde.
#[allow(dead_code)]
pub struct Face {
    pub ascent: i32,
    pub line: i32,
    pub glyphs: [Glyph; 95],
}

impl Face {
    /// Rows from a glyph's draw position down to the baseline row (the
    /// bottom row of `H`), for lining faces of different sizes up on one baseline.
    pub fn baseline(&self) -> i32 {
        let h = self.glyph(b'H');
        h.yoff as i32 + h.h as i32 - 1
    }

    fn glyph(&self, c: u8) -> &Glyph {
        let i = if (32..=126).contains(&c) {
            c - 32
        } else {
            b'?' - 32
        };
        &self.glyphs[i as usize]
    }

    /// Width in pixels of `s` with `track` extra pixels after every glyph.
    pub fn width(&self, s: &str, track: i32) -> i32 {
        if s.is_empty() {
            return 0;
        }
        s.bytes()
            .map(|c| self.glyph(c).adv as i32 + track)
            .sum::<i32>()
            - track
    }
}

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

impl Face {
    /// Rows from a glyph's draw position down to the middle row of the capital
    /// letters (taken from `H`). Aligning on this row, rather than on the
    /// baseline or the line box, puts faces of different sizes on one centre
    /// line.
    pub fn cap_mid(&self) -> i32 {
        let h = self.glyph(b'H');
        h.yoff as i32 + (h.h as i32 - 1) / 2
    }
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
