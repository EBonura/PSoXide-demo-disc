//! Fail-panel painting for the chain-load blob, pixel-exact.
//!
//! The first debug burn drew the panel with GP0(02h) FillVram, whose X
//! coordinate and width snap to 16-pixel steps on hardware: the stage
//! blocks fused into bars and the bit grid slid off its own columns.
//! Everything here goes through GP0(60h) monochrome rectangles instead,
//! which honour exact coordinates, at the cost of needing the drawing
//! area configured after the GPU reset `quiesce()` performs.

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
pub const BLUE: u32 = 0x00A0_3000; // BBGGRR: readable against the red base
pub const RED_BASE: u32 = 0x0000_0040;
pub const GREEN: u32 = 0x0000_A000;

/// One 32-bit word as two rows of 16 cells, MSB first. Set bits white,
/// clear bits blue (blue survives video compression where dark grey did
/// not), a wider gap between bytes. Cell pitch 18px, cells 14px.
pub fn bits_rows(y: i16, word: u32) {
    for bit in 0..32u32 {
        let row = (bit / 16) as i16;
        let col = (bit % 16) as i16;
        let x = 8 + col * 18 + (col / 8) * 8;
        let set = word & (1 << (31 - bit)) != 0;
        rect(x, y + row * 16, 14, 14, if set { WHITE } else { BLUE });
    }
}

/// Alignment ruler: alternating white/blue cells on the same grid as
/// [`bits_rows`], so a photo can always recover the column positions.
pub fn ruler(y: i16) {
    for col in 0..16i16 {
        let x = 8 + col * 18 + (col / 8) * 8;
        let rgb = if col % 2 == 0 { WHITE } else { BLUE };
        rect(x, y, 14, 6, rgb);
    }
}
