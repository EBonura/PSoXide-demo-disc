//! Demo disc boot menu.
//!
//! Reads the disc's table of contents ([`disc_toc`]), lists what is on the
//! disc, and chain-loads the selection.
//!
//! The chain-load itself cannot happen here: every PSoXide program links to
//! `0x80010000`, which is where this launcher is running, so streaming a game
//! in would overwrite the code doing the streaming. Instead the `loader` crate
//! is built separately at a high base address, embedded below as raw bytes,
//! copied up, and jumped to. See `loader/loader.ld`.

#![no_std]
#![no_main]

extern crate psx_rt;

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

const WHITE: (u8, u8, u8) = (230, 230, 230);
const DIM: (u8, u8, u8) = (120, 120, 130);
const HILITE: (u8, u8, u8) = (255, 210, 90);
const RED: (u8, u8, u8) = (230, 90, 90);

const ROW_HEIGHT: i16 = 14;
const LIST_TOP: i16 = 62;

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

    let mut selected: usize = 0;
    let mut prev_held = ButtonState::default();

    loop {
        let pad = poll_port1().buttons;
        let pressed = |b: u16| pad.is_held(b) && !prev_held.is_held(b);

        if count > 0 {
            if pressed(button::UP) {
                selected = if selected == 0 { count - 1 } else { selected - 1 };
            }
            if pressed(button::DOWN) {
                selected = (selected + 1) % count;
            }
            if pressed(button::CROSS) || pressed(button::START) {
                // Never returns when the disc is readable.
                boot(&entries[selected]);
            }
        }
        prev_held = pad;

        fb.clear(8, 10, 24);
        font.draw_text(16, 20, "PSOXIDE DEMO DISC", WHITE);
        font.draw_text(16, 34, "UP/DOWN to choose, X to run", DIM);

        if count == 0 {
            font.draw_text(16, LIST_TOP, "DISC TABLE OF CONTENTS UNREADABLE", RED);
        }
        for (i, entry) in entries.iter().enumerate().take(count) {
            let y = LIST_TOP + (i as i16) * ROW_HEIGHT;
            let on = i == selected;
            font.draw_text(16, y, if on { ">" } else { " " }, HILITE);
            font.draw_text(32, y, entry.name_str(), if on { HILITE } else { WHITE });
        }

        gpu::draw_sync();
        psx_rt::interrupts::wait_vblank();
        fb.swap();
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

    // Blank the display first: the game's own boot decides what to show, and
    // the blob resets the GPU out from under whatever is on screen.
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
