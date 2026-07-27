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
use disc_toc::{Entry, Header, MAX_ENTRIES, TOC_BYTES, TOC_LBA};
use psx_font::{fonts::BASIC, FontAtlas};
use psx_gpu::{self as gpu, framebuf::FrameBuffer, Resolution, VideoMode};
use psx_io::cdda::CddaStarter;
use psx_io::cdrom;
use psx_spu::{self as spu, CdVolume, Volume};
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
const CREDIT: (u8, u8, u8) = (95, 125, 175);
const BLURB: (u8, u8, u8) = (215, 235, 255);
const LABEL: (u8, u8, u8) = (255, 255, 255);
const FAR_LABEL: (u8, u8, u8) = (110, 150, 200);
const ERROR: (u8, u8, u8) = (230, 90, 90);

const STARS: u32 = 90;

/// Turns per frame the ring eases toward its target, as a fraction: the gap
/// closes by an eighth each frame, which settles in about half a second.
const EASE_SHIFT: i32 = 3;
/// What the ball of balls drifts at when nobody is touching the pad.
const SPHERE_IDLE_SPIN: i32 = 5;
/// The shove browsing gives it. It spins up with the carousel and coasts back
/// down to the idle drift, so the whole screen reacts rather than just the ring.
const SPHERE_KICK: i32 = 110;
/// How fast that shove bleeds off: a sixteenth of the excess per frame.
const SPHERE_DECAY_SHIFT: i32 = 4;

/// Widest line the 8-pixel font fits on screen with a margin either side.
const WRAP_CHARS: usize = 36;

/// Ticks between drive-status polls while the menu track plays. Often enough
/// to restart the loop without a gap anyone notices, rare enough that the
/// polling does not fight the audio.
const CDDA_POLL_TICKS: u32 = 30;
/// Status bit the drive sets while it is playing CD-DA.
const CDDA_PLAYING: u8 = 0x80;
/// Spin budget per CD command. Silicon wants more than an emulator does.
const CDDA_SPINS: u32 = 0x10_0000;

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
    // Read the table before a note of music plays: a data read while the
    // drive is playing CD-DA is the one thing this hardware is worst at.
    let header = read_toc(&mut entries);
    let count = header.map_or(0, |h| h.count);
    if count == 0 {
        tty::println("launcher: no table of contents on this disc");
    }

    let menu_track = header.map_or(0, |h| h.menu_track) as u8;
    let menu_track_count = header.map_or(0, |h| h.menu_track_count).max(1) as u8;
    // Which of the run is playing. Cycling beats looping one track when the
    // disc might sit on the menu for a whole presentation.
    let mut menu_track_index: u8 = 0;
    let mut music = CddaStarter::new().with_spins(CDDA_SPINS);
    let mut tick: u32 = 0;
    let mut next_music_poll = CDDA_POLL_TICKS;
    if menu_track != 0 {
        // The CD controller playing is only half of it: the SPU's CD input
        // comes up silent, so without this the drive spins a track nobody
        // hears.
        spu::init();
        spu::set_main_volume(Volume::MAX, Volume::MAX);
        spu::set_cd_volume(CdVolume::MAX, CdVolume::MAX);
        spu::enable_cd_audio(true);
        music.begin(tick);
    }

    let mut selected: i32 = 0;
    let mut ring = 0i32;
    let mut spin = 0i32;
    let mut spin_rate = SPHERE_IDLE_SPIN;
    let mut italian = false;
    let mut prev_held = ButtonState::default();
    let mut order = [0usize; MAX_ENTRIES];
    let mut beads = [Bead::default(); SPHERE_POINTS];

    loop {
        tick = tick.wrapping_add(1);
        if menu_track != 0 {
            music.tick(tick, menu_track + menu_track_index);
            // The track is the last on the disc, so when it ends the drive
            // has nowhere to go. Notice and start it again.
            if music.started() && tick.wrapping_sub(next_music_poll) < u32::MAX / 2 {
                next_music_poll = tick.wrapping_add(CDDA_POLL_TICKS);
                let idle = match cdrom::try_get_stat(CDDA_SPINS) {
                    Some(status) => status
                        .bytes()
                        .first()
                        .is_some_and(|s| s & CDDA_PLAYING == 0),
                    None => false,
                };
                if idle {
                    menu_track_index = (menu_track_index + 1) % menu_track_count;
                    music.begin(tick);
                }
            }
        }

        let pad = poll_port1().buttons;
        let pressed = |b: u16| pad.is_held(b) && !prev_held.is_held(b);

        if pressed(button::UP) || pressed(button::DOWN) {
            italian = !italian;
        }
        if count > 0 {
            if pressed(button::LEFT) {
                selected -= 1;
                spin_rate -= SPHERE_KICK;
            }
            if pressed(button::RIGHT) {
                selected += 1;
                spin_rate += SPHERE_KICK;
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

        // Coast the ball back to its idle drift.
        spin_rate = carousel::ease_spin(spin_rate, SPHERE_IDLE_SPIN, SPHERE_DECAY_SHIFT);
        spin = (spin + spin_rate) & (TURN - 1);

        fb.clear(4, 6, 18);
        draw_starfield();
        draw_sphere(spin, &mut beads);

        centred(&font, 6, "PSOXIDE DEMO DISC", TITLE);

        if count == 0 {
            centred(&font, 126, "DISC TABLE OF CONTENTS UNREADABLE", ERROR);
        } else {
            let index = selected.rem_euclid(count as i32) as usize;
            draw_description(&font, &entries[index], italian);
            draw_ring(&font, &entries[..count], ring, step, &mut order);
        }
        // The music is used by permission, so the credit is not optional
        // decoration: it stays on screen the whole time the track plays.
        if let Some(header) = header {
            centred(&font, 230, header.credit_str(), CREDIT);
        }

        gpu::draw_sync();
        psx_rt::interrupts::wait_vblank();
        fb.swap();
    }
}

fn centred(font: &FontAtlas, y: i16, text: &str, tint: (u8, u8, u8)) {
    if text.is_empty() {
        return;
    }
    font.draw_text(160 - (font.text_width(text) as i16) / 2, y, text, tint);
}

/// Break `text` at the last space that fits, so a long blurb reads as two
/// tidy lines rather than one cut mid-word.
fn wrap(text: &str, max: usize) -> (&str, &str) {
    if text.len() <= max {
        return (text, "");
    }
    match text[..max].rfind(' ') {
        Some(at) => (&text[..at], text[at + 1..].trim_start()),
        None => (&text[..max], text[max..].trim_start()),
    }
}

/// Split a title at the space nearest its middle, so the two lines on a pill
/// come out roughly even. Titles with no space stay on one line.
fn split_title(name: &str) -> (&str, &str) {
    let middle = name.len() / 2;
    let mut best: Option<usize> = None;
    for (at, byte) in name.bytes().enumerate() {
        if byte == b' ' && best.is_none_or(|b| at.abs_diff(middle) < b.abs_diff(middle)) {
            best = Some(at);
        }
    }
    match best {
        Some(at) => (&name[..at], &name[at + 1..]),
        None => (name, ""),
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

/// The selected game's blurb in one language, under the flag of whichever
/// one it is. Up or down swaps.
fn draw_description(font: &FontAtlas, entry: &Entry, italian: bool) {
    if italian {
        paint::flag_it(150, 112);
    } else {
        paint::flag_uk(150, 112);
    }
    let text = if italian {
        entry.desc_it_str()
    } else {
        entry.desc_en_str()
    };
    let (first, second) = wrap(text, WRAP_CHARS);
    centred(font, 130, first, BLURB);
    centred(font, 140, second, BLURB);
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
            let tint = if item.front > 200 { LABEL } else { FAR_LABEL };
            let (top, bottom) = split_title(entries[slot].name_str());
            let line = |y: i16, text: &str| {
                if !text.is_empty() {
                    let width = font.text_width(text) as i16;
                    font.draw_text(item.x - width / 2, y, text, tint);
                }
            };
            if bottom.is_empty() {
                line(item.y - 4, top);
            } else {
                line(item.y - 9, top);
                line(item.y + 1, bottom);
            }
        }
    }
}

/// Read the table of contents. `None` if the disc has none.
fn read_toc(entries: &mut [Entry; MAX_ENTRIES]) -> Option<Header> {
    // SAFETY: single-threaded, polled; `main` runs once and nothing else
    // touches these statics.
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(READER) };
    let sector = unsafe { &mut *core::ptr::addr_of_mut!(TOC_SECTOR) };

    let ok = unsafe { reader.prepare() && reader.start_read(TOC_LBA) && reader.read_sector(sector) };
    unsafe { reader.stop() };
    if !ok {
        return None;
    }
    // SAFETY: a [u32; 512] is 2048 bytes; the target is little-endian, so the
    // word buffer and the on-disc byte order agree.
    let bytes: &[u8; TOC_BYTES] =
        unsafe { &*(sector.as_ptr() as *const u8 as *const [u8; TOC_BYTES]) };
    disc_toc::decode(bytes, entries)
}

/// Copy the chain-load blob high and jump to it. Never returns while the disc
/// is readable; the blob paints the screen red and stops if it is not.
fn boot(entry: &Entry) -> ! {
    assert!(LOADER_BLOB.len() <= LOADER_LIMIT, "loader blob too large");
    tty::println("launcher: chain-loading");

    // Let the GPU finish before the blob resets it out from under whatever is
    // still on screen, and get the drive off CD-DA before the blob starts
    // reading sectors with it.
    gpu::draw_sync();
    let _ = cdrom::try_stop(CDDA_SPINS);

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
