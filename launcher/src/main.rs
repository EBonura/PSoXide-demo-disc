//! Demo disc boot menu.
//!
//! Reads the disc's table of contents ([`disc_toc`]), spins its contents round
//! a carousel, and chain-loads the selection. The look is a homage to the
//! PlayStation demo discs: glossy blue pills on a tilted ring, a ball of balls
//! turning above them, a starfield behind.
//!
//! The chain-load itself cannot happen here: every PSoXide program links to
//! `0x80010000`, which is where this launcher is running, so streaming a game
//! in would overwrite the code doing the streaming. Instead the `loader` crate
//! is built separately at a high base address, embedded below as raw bytes,
//! copied up, and jumped to. See `loader/loader.ld`.

#![no_std]
#![no_main]

extern crate psx_rt;

mod paint;

use carousel::{Bead, Placed, SPHERE_POINTS, TURN};
use disc_toc::{Entry, MAX_ENTRIES, TOC_BYTES, TOC_LBA};
use psx_font::{fonts::BASIC, FontAtlas};
use psx_gpu::{self as gpu, framebuf::FrameBuffer, Resolution, VideoMode};
use psx_pack::cd::{SectorReader, SECTOR_WORDS};
use psx_pad::{button, poll_port1, ButtonState};
use psx_rt::tty;
use psx_vram::{Clut, TexDepth, Tpage};

/// The chain-load blob, linked at `LOADER_BASE` by `loader/loader.ld`.
const LOADER_BLOB: &[u8] = include_bytes!(env!("LOADER_BLOB"));

/// Where the blob expects to run. Must match `loader.ld`.
const LOADER_BASE: u32 = 0x801F_0000;

/// Blob budget, also from `loader.ld`. Overrunning it would walk into the
/// stack, so check rather than trust.
const LOADER_LIMIT: usize = 32 * 1024;

const FONT_TPAGE: Tpage = Tpage::new(320, 0, TexDepth::Bit4);
const FONT_CLUT: Clut = Clut::new(320, 256);

const TITLE: (u8, u8, u8) = (170, 220, 255);
const HINT: (u8, u8, u8) = (90, 120, 170);
const ENGLISH: (u8, u8, u8) = (225, 240, 255);
const ITALIAN: (u8, u8, u8) = (120, 175, 235);
const LABEL: (u8, u8, u8) = (255, 255, 255);
const FAR_LABEL: (u8, u8, u8) = (110, 150, 200);
const ERROR: (u8, u8, u8) = (230, 90, 90);

const STARS: u32 = 90;

/// Turns per frame the ring eases toward its target, as a fraction: the gap
/// closes by 1/6 each frame, which settles in about half a second.
const EASE_SHIFT: i32 = 3;
/// The ball of balls turns this much per frame, slowly.
const SPHERE_SPIN: i32 = 6;

// The reader owns a one-sector bounce buffer; keep it off the 32 KiB stack.
static mut READER: SectorReader = SectorReader::new();
static mut TOC_SECTOR: [u32; SECTOR_WORDS] = [0; SECTOR_WORDS];

#[no_mangle]
fn main() {
    tty::println("launcher: booted");

    gpu::init(VideoMode::Ntsc, Resolution::R320X240);
    let mut fb = FrameBuffer::new(320, 240);
    gpu::set_draw_area(0, 0, 319, 239);
    gpu::set_draw_offset(0, 0);
    let font = FontAtlas::upload(&BASIC, FONT_TPAGE, FONT_CLUT);

    let mut entries = [Entry::new("", 0, 0, 0); MAX_ENTRIES];
    let count = read_toc(&mut entries);
    if count == 0 {
        tty::println("launcher: no table of contents on this disc");
    }

    let mut selected: i32 = 0;
    let mut ring = 0i32;
    let mut spin = 0i32;
    let mut prev_held = ButtonState::default();
    let mut order = [0usize; MAX_ENTRIES];
    let mut beads = [Bead::default(); SPHERE_POINTS];

    loop {
        let pad = poll_port1().buttons;
        let pressed = |b: u16| pad.is_held(b) && !prev_held.is_held(b);

        if count > 0 {
            // The carousel turns; up/left and down/right both make sense on a
            // ring, so take either.
            if pressed(button::LEFT) || pressed(button::UP) {
                selected -= 1;
            }
            if pressed(button::RIGHT) || pressed(button::DOWN) {
                selected += 1;
            }
            if pressed(button::CROSS) || pressed(button::START) {
                let index = selected.rem_euclid(count as i32) as usize;
                // Never returns when the disc is readable.
                boot(&entries[index]);
            }
        }
        prev_held = pad;

        // Ease toward the selection instead of snapping. `target` is allowed
        // to run past a full turn so the ring keeps spinning the short way
        // rather than unwinding.
        let step = TURN / count.max(1) as i32;
        let target = -selected * step;
        ring += (target - ring) >> EASE_SHIFT;
        spin = (spin + SPHERE_SPIN) & (TURN - 1);

        fb.clear(4, 6, 18);
        draw_starfield();
        draw_sphere(spin, &mut beads);

        font.draw_text(10, 8, "PSOXIDE DEMO DISC", TITLE);
        font.draw_text(10, 20, "LEFT/RIGHT to browse", HINT);
        font.draw_text(10, 30, "X to run", HINT);

        if count == 0 {
            font.draw_text(10, 122, "DISC TABLE OF CONTENTS UNREADABLE", ERROR);
        } else {
            let index = selected.rem_euclid(count as i32) as usize;
            draw_description(&font, &entries[index]);
            draw_ring(&font, &entries[..count], ring, step, &mut order);
        }

        gpu::draw_sync();
        psx_rt::interrupts::wait_vblank();
        fb.swap();
    }
}

fn draw_starfield() {
    for i in 0..STARS {
        let (x, y, b) = carousel::star(i);
        let size = if i % 7 == 0 { 2 } else { 1 };
        gpu::draw_rect_flat(x, y, size, size, b / 2, (b * 3) / 4, b);
    }
}

fn draw_sphere(spin: i32, beads: &mut [Bead; SPHERE_POINTS]) {
    let n = carousel::sphere(spin, beads);
    carousel::sort_by_depth(&mut beads[..n], |b| b.z);
    for bead in &beads[..n] {
        paint::bead(bead);
    }
}

/// The selected game's blurb, English over Italian.
fn draw_description(font: &FontAtlas, entry: &Entry) {
    let centred = |y: i16, text: &str, tint: (u8, u8, u8)| {
        if text.is_empty() {
            return;
        }
        let x = 160 - (font.text_width(text) as i16) / 2;
        font.draw_text(x, y, text, tint);
    };
    centred(122, entry.desc_en_str(), ENGLISH);
    centred(134, entry.desc_it_str(), ITALIAN);
}

/// The carousel: place every entry on the ring, draw back to front, and label
/// each pill.
fn draw_ring(
    font: &FontAtlas,
    entries: &[Entry],
    ring: i32,
    step: i32,
    order: &mut [usize; MAX_ENTRIES],
) {
    let count = entries.len();
    for (slot, item) in order.iter_mut().enumerate().take(count) {
        *item = slot;
    }
    let placed = |slot: usize| -> Placed { carousel::place(ring + slot as i32 * step) };
    carousel::sort_by_depth(&mut order[..count], |slot| placed(*slot).z);

    for &slot in &order[..count] {
        let item = placed(slot);
        paint::pill(&item);

        // Titles wider than their pill are left to overhang, the way the demo
        // discs did it. The ones round the back are dropped instead: at that
        // size they are unreadable and only add clutter.
        if item.front > 96 {
            let name = entries[slot].name_str();
            let width = font.text_width(name) as i16;
            let tint = if item.front > 200 { LABEL } else { FAR_LABEL };
            font.draw_text(item.x - width / 2, item.y - 4, name, tint);
        }
    }
}

/// Read the table of contents. Returns 0 if the disc has none.
fn read_toc(entries: &mut [Entry; MAX_ENTRIES]) -> usize {
    // SAFETY: single-threaded, polled; `main` runs once and nothing else
    // touches these statics.
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(READER) };
    let sector = unsafe { &mut *core::ptr::addr_of_mut!(TOC_SECTOR) };

    let ok = unsafe { reader.prepare() && reader.start_read(TOC_LBA) && reader.read_sector(sector) };
    unsafe { reader.stop() };
    if !ok {
        return 0;
    }
    // SAFETY: a [u32; 512] is 2048 bytes; the target is little-endian, so the
    // word buffer and the on-disc byte order agree.
    let bytes: &[u8; TOC_BYTES] =
        unsafe { &*(sector.as_ptr() as *const u8 as *const [u8; TOC_BYTES]) };
    disc_toc::decode(bytes, entries).unwrap_or(0)
}

/// Copy the chain-load blob high and jump to it. Never returns while the disc
/// is readable; the blob paints the screen red and stops if it is not.
fn boot(entry: &Entry) -> ! {
    assert!(LOADER_BLOB.len() <= LOADER_LIMIT, "loader blob too large");
    tty::println("launcher: chain-loading");

    // Let the GPU finish before the blob resets it out from under whatever is
    // still on screen.
    gpu::draw_sync();

    // SAFETY: `LOADER_BASE` is above every game's payload (mkdisc enforces
    // that) and below the stack, so nothing live is being overwritten. The
    // blob is position-dependent and linked for exactly this address.
    unsafe {
        core::ptr::copy_nonoverlapping(
            LOADER_BLOB.as_ptr(),
            LOADER_BASE as *mut u8,
            LOADER_BLOB.len(),
        );
        psx_rt::bios::flush_cache();
        let blob: unsafe extern "C" fn(u32, u32, u32) -> ! =
            core::mem::transmute(LOADER_BASE as usize);
        blob(entry.exe_lba, entry.lba_offset, entry.cdda_track_base)
    }
}
