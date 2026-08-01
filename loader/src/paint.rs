//! Diagnostic painting for the chain-load blob: exact-pixel rectangles
//! and a small text renderer built out of them.
//!
//! The first debug burn drew panels with GP0(02h) FillVram, whose X
//! coordinate and width snap to 16-pixel steps on hardware: shapes fused
//! and grids slid off their own columns. Everything here goes through
//! GP0(60h) monochrome rectangles instead, which honour exact
//! coordinates, at the cost of needing the drawing area configured after
//! the GPU reset `quiesce()` performs.

use psx_font::fonts::basic::BASIC_BITMAP;

/// GP0/GP1 ports.
const GP0: u32 = 0x1F80_1810;
const GP1: u32 = 0x1F80_1814;

fn gp0(word: u32) {
    // Wait for GPUSTAT bit 26 (ready for the next command word) before
    // every write. The panel once burst hundreds of words unpaced;
    // silicon dropped enough of them mid-fill to desync the command
    // stream into garbage rectangles, while an emulator FIFO never
    // overflows and rendered it perfectly. Bounded so a dead GPU cannot
    // hang the panel that is trying to report on it.
    for _ in 0..1_000_000u32 {
        if unsafe { psx_io::read32(GP1) } & (1 << 26) != 0 {
            break;
        }
    }
    unsafe { psx_io::write32(GP0, word) };
}

/// Configure drawing after a GP1(00h) reset: drawing area covering the
/// whole displayed framebuffer, zero offset, display area at 0,0.
pub fn setup() {
    unsafe { psx_io::write32(GP1, 0x0300_0001) }; // display off while painting
    gp0(0xE1_00_0000); // texpage/draw-mode defaults
    gp0(0xE3_00_0000); // drawing area top-left (0,0)
    gp0(0xE4_00_0000 | (255 << 10) | 320); // bottom-right (320,255)
    gp0(0xE5_00_0000); // drawing offset 0
}

pub fn show() {
    unsafe { psx_io::write32(GP1, 0x0300_0000) }; // display on
}

/// Solid rectangle at exact pixel coordinates. `rgb` is `0xBBGGRR`.
pub fn rect(x: i16, y: i16, w: i16, h: i16, rgb: u32) {
    gp0(0x60_00_0000 | rgb);
    gp0(((y as u32) << 16) | (x as u32 & 0xFFFF));
    gp0(((h as u32) << 16) | (w as u32 & 0xFFFF));
}

pub const WHITE: u32 = 0x00FF_FFFF;
pub const GREEN: u32 = 0x0000_D000;
pub const YELLOW: u32 = 0x0000_D8FF;
/// Pending / de-emphasised text. Light enough to survive a phone photo
/// of a CRT, dark enough to read as "not yet".
pub const DIM: u32 = 0x0080_8080;
pub const RED_BASE: u32 = 0x0000_0040;

/// The SDK's 8x8 public-domain font (dhepper font8x8), reused as plain
/// data: glyphs are drawn as runs of GP0 rectangles, so the blob needs
/// no VRAM upload and leaves no texture state behind for the game.
static FONT: [u8; 1024] = BASIC_BITMAP;

/// Draw `s` at (x, y), every glyph pixel a `scale`-square rectangle.
/// Returns the x just past the text, so calls chain along one line.
pub fn text(x: i16, y: i16, scale: i16, s: &str, rgb: u32) -> i16 {
    text_bytes(x, y, scale, s.as_bytes(), rgb)
}

pub fn text_bytes(mut x: i16, y: i16, scale: i16, s: &[u8], rgb: u32) -> i16 {
    for &b in s {
        glyph(x, y, scale, b, rgb);
        x += 8 * scale;
    }
    x
}

/// `v` as eight hex digits. Returns the x just past them.
pub fn hex32(mut x: i16, y: i16, scale: i16, v: u32, rgb: u32) -> i16 {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in 0..8 {
        glyph(x, y, scale, HEX[((v >> (28 - 4 * i)) & 0xF) as usize], rgb);
        x += 8 * scale;
    }
    x
}

/// One glyph as horizontal runs of set pixels, one rectangle per run:
/// a fifth of the GP0 traffic of a rect per pixel, on a port every word
/// of which is paced against GPUSTAT.
fn glyph(x: i16, y: i16, scale: i16, code: u8, rgb: u32) {
    let rows = &FONT[(code as usize & 0x7F) * 8..][..8];
    for (row, bits) in rows.iter().enumerate() {
        let mut col: u32 = 0;
        while col < 8 {
            if bits & (1 << col) == 0 {
                col += 1;
                continue;
            }
            let start = col;
            while col < 8 && bits & (1 << col) != 0 {
                col += 1;
            }
            // Bit 0 is the leftmost pixel (the font8x8 convention).
            rect(
                x + start as i16 * scale,
                y + row as i16 * scale,
                (col - start) as i16 * scale,
                scale,
                rgb,
            );
        }
    }
}
