//! IRQ probe: a disc entry that diagnoses the chain-load vblank freeze.
//!
//! Every game on the second debug burn boots, draws its first frame, and
//! freezes in its vblank wait, on silicon only, while the menu's identical
//! vblank machinery works. This probe is the discriminating instrument: it
//! never blocks on the thing it is measuring (all pacing is GPUSTAT
//! polling), and it repaints live:
//!
//! - `LOOP`: iterations, proving the probe itself is running,
//! - `VBL`: the SDK's IRQ-driven vblank counter (the thing that freezes),
//! - `RAW`: GPUSTAT bit-31 transitions per loop window (video timing
//!   alive?),
//! - `STAT`/`MASK`: raw `I_STAT` / `I_MASK`,
//! - `SR`/`CAUSE`: COP0 state (IP2 pending? IEc on? BEV where?),
//! - `VEC`: the two exception-vector words now vs right after install.
//!
//! One photo of this screen says whether the vblank IRQ is not generated,
//! not unmasked, not taken, or taken into a wrong vector.

#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

extern crate psx_rt;

use psx_font::{fonts::BASIC, FontAtlas};
use psx_gpu::{self as gpu, Resolution, VideoMode};
use psx_vram::{Clut, TexDepth, Tpage};

const FONT_TPAGE: Tpage = Tpage::new(320, 0, TexDepth::Bit4);
const FONT_CLUT: Clut = Clut::new(320, 256);

const GPUSTAT: u32 = 0x1F80_1814;
const I_STAT: u32 = 0x1F80_1070;
const I_MASK: u32 = 0x1F80_1074;
const VECTOR: u32 = 0x8000_0080;

const LABEL: (u8, u8, u8) = (255, 255, 255);
const GOOD: (u8, u8, u8) = (120, 255, 120);
const BAD: (u8, u8, u8) = (255, 120, 90);

fn rv(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn sr() -> u32 {
    let v: u32;
    unsafe { core::arch::asm!("mfc0 $8, $12", "nop", lateout("$8") v) };
    v
}

fn cause() -> u32 {
    let v: u32;
    unsafe { core::arch::asm!("mfc0 $8, $13", "nop", lateout("$8") v) };
    v
}

/// Count GPUSTAT bit-31 transitions across a fixed volatile-read window.
/// This is the raw "is video timing alive" signal and also the loop's
/// frame pacing; roughly a frame or two of real time.
fn raw_vblank_window() -> u32 {
    let mut last = rv(GPUSTAT) >> 31;
    let mut transitions = 0;
    for _ in 0..300_000u32 {
        let now = rv(GPUSTAT) >> 31;
        if now != last {
            transitions += 1;
            last = now;
        }
    }
    transitions
}

struct Line {
    buf: [u8; 40],
    n: usize,
}

impl Line {
    fn new() -> Self {
        Line { buf: [0; 40], n: 0 }
    }
    fn s(&mut self, s: &str) -> &mut Self {
        self.buf[self.n..self.n + s.len()].copy_from_slice(s.as_bytes());
        self.n += s.len();
        self
    }
    fn hex(&mut self, v: u32, digits: usize) -> &mut Self {
        const H: &[u8; 16] = b"0123456789ABCDEF";
        for i in (0..digits).rev() {
            self.buf[self.n] = H[((v >> (i * 4)) & 0xF) as usize];
            self.n += 1;
        }
        self
    }
    fn dec(&mut self, v: u32) -> &mut Self {
        let mut d = [0u8; 10];
        let mut n = 0;
        let mut v = v;
        loop {
            d[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
            if v == 0 {
                break;
            }
        }
        for i in 0..n {
            self.buf[self.n + i] = d[n - 1 - i];
        }
        self.n += n;
        self
    }
    fn draw(&self, font: &FontAtlas, y: i16, tint: (u8, u8, u8)) {
        // SAFETY: only ASCII is written above.
        let text = unsafe { core::str::from_utf8_unchecked(&self.buf[..self.n]) };
        font.draw_text(12, y, text, tint);
    }
}

#[no_mangle]
fn main() {
    gpu::init(VideoMode::Ntsc, Resolution::R320X240);
    let font = FontAtlas::upload(&BASIC, FONT_TPAGE, FONT_CLUT);

    // Proof of life before the vblank machinery is touched: if install
    // itself dies, this stays on screen.
    gpu::draw_rect_flat(0, 0, 320, 240, 12, 12, 40);
    font.draw_text(12, 12, "IRQ PROBE: BEFORE INSTALL", LABEL);
    gpu::draw_sync();

    psx_rt::interrupts::install_vblank_counter();
    let vec0 = rv(VECTOR);
    let vec1 = rv(VECTOR + 4);

    let mut loops: u32 = 0;
    let mut last_vbl: u32 = 0;
    let mut frozen_windows: u32 = 0;
    loop {
        let raw = raw_vblank_window();
        let vbl = psx_rt::interrupts::vblank_count();
        // Consecutive windows with raw vblank pulses but no counter
        // movement: the freeze signature, made explicit.
        if raw > 0 && vbl == last_vbl {
            frozen_windows = frozen_windows.saturating_add(1);
        } else {
            frozen_windows = 0;
        }
        last_vbl = vbl;
        loops = loops.wrapping_add(1);

        gpu::draw_rect_flat(0, 0, 320, 240, 12, 12, 40);
        font.draw_text(12, 12, "IRQ PROBE", LABEL);

        Line::new().s("LOOP ").dec(loops).draw(&font, 32, LABEL);
        Line::new()
            .s("VBL ")
            .dec(vbl)
            .s("  RAW ")
            .dec(raw)
            .draw(&font, 44, if vbl > 0 { GOOD } else { BAD });
        Line::new()
            .s("STAT ")
            .hex(rv(I_STAT), 4)
            .s(" MASK ")
            .hex(rv(I_MASK), 4)
            .draw(&font, 56, LABEL);
        Line::new()
            .s("SR ")
            .hex(sr(), 8)
            .s(" CAUSE ")
            .hex(cause(), 8)
            .draw(&font, 68, LABEL);
        Line::new()
            .s("VEC ")
            .hex(rv(VECTOR), 8)
            .s(" ")
            .hex(rv(VECTOR + 4), 8)
            .draw(&font, 80, LABEL);
        Line::new()
            .s("AT INSTALL ")
            .hex(vec0, 8)
            .s(" ")
            .hex(vec1, 8)
            .draw(
                &font,
                92,
                if rv(VECTOR) == vec0 && rv(VECTOR + 4) == vec1 {
                    GOOD
                } else {
                    BAD
                },
            );
        if frozen_windows >= 3 {
            Line::new()
                .s("FROZEN: RAW PULSES, VBL STUCK")
                .draw(&font, 110, BAD);
        }
        gpu::draw_sync();
    }
}
