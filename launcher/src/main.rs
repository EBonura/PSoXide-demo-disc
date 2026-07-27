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
use psx_font::{
    fonts::{BASIC, SPLEEN_5X8},
    FontAtlas,
};
use psx_gpu::{self as gpu, framebuf::FrameBuffer, Resolution, VideoMode};
use psx_io::cdda::{CddaClock, CddaStarter};
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

/// The header's own font: same eight-pixel height as BASIC but five wide, so
/// the music column fits beside a centred mark instead of running under it.
const SMALL_TPAGE: Tpage = Tpage::new(448, 0, TexDepth::Bit4);
const SMALL_CLUT: Clut = Clut::new(416, 256);

/// The banner sits in its own 4bpp page, clear of both framebuffers and of
/// the font. Tpage X must be a multiple of 64.
const BANNER_TPAGE: Tpage = Tpage::new(384, 0, TexDepth::Bit4);
const BANNER_CLUT: Clut = Clut::new(400, 256);
const BANNER_W: i16 = 120;
const BANNER_H: i16 = 22;
static BANNER_TEX: &[u8] = include_bytes!("../assets/banner.tex");
static BANNER_CLUT_DATA: &[u8] = include_bytes!("../assets/banner.clut");

const TITLE: (u8, u8, u8) = (255, 84, 62);
const HINT: (u8, u8, u8) = (168, 44, 40);
const NOW_PLAYING: (u8, u8, u8) = (172, 40, 34);
const TRACK_NAME: (u8, u8, u8) = (255, 88, 64);
const BLURB: (u8, u8, u8) = (255, 206, 196);
const LABEL: (u8, u8, u8) = (255, 255, 255);
const FAR_LABEL: (u8, u8, u8) = (215, 78, 62);
const ERROR: (u8, u8, u8) = (255, 214, 90);

/// The carousel entry that shows the credits instead of running something.
const CREDITS_NAME: &str = "CREDITS";

const STARS: u32 = 90;

/// Turns per frame the ring eases toward its target, as a fraction: the gap
/// closes by an eighth each frame, which settles in about half a second.
const EASE_SHIFT: i32 = 3;
/// What the ball of balls drifts at when nobody is touching the pad, before
/// the bar swings it.
const SPHERE_IDLE_SPIN: i32 = 5;
/// How much faster it turns at the top of a bar than at the end of one.
const SPHERE_BAR_SWING: i32 = 6;
/// How far the ball blows outward on a beat, in 256ths of its radius, on the
/// downbeat and on the other three.
const SWELL_DOWNBEAT: i32 = 44;
const SWELL_BEAT: i32 = 22;
/// How far a hard browse throws the beads apart, and the ceiling on it.
const SCATTER_PER_KICK: i32 = 4;
const SCATTER_MAX: i32 = 130;
/// The shove browsing gives it. It spins up with the carousel and coasts back
/// down to the idle drift, so the whole screen reacts rather than just the ring.
const SPHERE_KICK: i32 = 110;
/// How fast that shove bleeds off: a sixteenth of the excess per frame.
const SPHERE_DECAY_SHIFT: i32 = 4;

/// Widest line the 8-pixel font fits on screen with a margin either side.
const WRAP_CHARS: usize = disc_toc::DESC_COLUMNS;
/// Top of the description block, and the gap between its lines.
/// A black header strip across the whole screen, holding the mark and the
/// words under it. Full width rather than a box round the logo: centred is
/// where the mark belongs, and anything narrower collides with the music
/// panel, which is what pushed it off-centre before.
const HEADER_H: i16 = 44;
/// The music panel stacks down the header's left edge: label, title, meter,
/// hint. The mark sits below the title row rather than beside it, because a
/// title runs to 112 pixels and a centred mark starts at 90.
const MUSIC_TOP: i16 = 4;
const TRACK_TOP: i16 = 13;
const METER_BASE: i16 = 38;
/// The mark sits beside the column now rather than under it: at five pixels a
/// character the widest track title stops well short of a centred mark.
const BANNER_Y: i16 = 2;
const DESC_TOP: i16 = 88;
const DESC_LEADING: i16 = 9;

/// Ticks between drive-status polls while the menu track plays. Often enough
/// to restart the loop without a gap anyone notices, rare enough that the
/// polling does not fight the audio.
const CDDA_POLL_TICKS: u32 = 30;
/// Status bit the drive sets while it is playing CD-DA.
const CDDA_PLAYING: u8 = 0x80;
/// Spin budget per CD command. Silicon wants more than an emulator does.
const CDDA_SPINS: u32 = 0x10_0000;
/// Display frames a second, which is what the CD clock counts in.
const TICKS_HZ: u32 = 60;

// The reader owns a one-sector bounce buffer; keep it off the 32 KiB stack.
static mut READER: SectorReader = SectorReader::new();
static mut TOC_SECTOR: [u32; SECTOR_WORDS * disc_toc::TOC_SECTORS as usize] =
    [0; SECTOR_WORDS * disc_toc::TOC_SECTORS as usize];

/// Room for every menu track's level-meter data. Read once at boot rather
/// than per track, so skipping stays as quick as the drive allows.
const SPECTRUM_MAX_SECTORS: usize = 192;
static mut SPECTRUM: [u32; SECTOR_WORDS * SPECTRUM_MAX_SECTORS] =
    [0; SECTOR_WORDS * SPECTRUM_MAX_SECTORS];

#[no_mangle]
fn main() {
    tty::println("launcher: booted");

    gpu::init(VideoMode::Ntsc, Resolution::R320X240);
    let mut fb = FrameBuffer::new(320, 240);
    gpu::set_draw_area(0, 0, 319, 239);
    gpu::set_draw_offset(0, 0);
    let font = FontAtlas::upload(&BASIC, FONT_TPAGE, FONT_CLUT);
    let small = FontAtlas::upload(&SPLEEN_5X8, SMALL_TPAGE, SMALL_CLUT);
    let banner = paint::Banner::upload(
        BANNER_TEX,
        BANNER_CLUT_DATA,
        BANNER_W,
        BANNER_H,
        BANNER_TPAGE,
        BANNER_CLUT,
    );

    let mut entries = [Entry::new("", 0, 0, 0); MAX_ENTRIES];
    // Read the table before a note of music plays: a data read while the
    // drive is playing CD-DA is the one thing this hardware is worst at.
    let header = read_toc(&mut entries);
    // Credits ride the carousel like everything else, with no program behind
    // them. A zero LBA is what marks an entry as nothing to boot.
    let mut count = header.map_or(0, |h| h.count);
    if count > 0 && count < MAX_ENTRIES {
        entries[count] = Entry::new(CREDITS_NAME, 0, 0, 0);
        count += 1;
    }
    let count = count;
    if count == 0 {
        tty::println("launcher: no table of contents on this disc");
    }

    // Before the music starts, for the same reason the table is: reading the
    // disc while it plays CD-DA is what this hardware is worst at.
    let spectrum_frames = read_spectrum(header.as_ref());

    let menu_track = header.map_or(0, |h| h.menu_track) as u8;
    let menu_track_count = header.map_or(0, |h| h.menu_track_count).max(1) as u8;
    // Which of the run is playing. Cycling beats looping one track when the
    // disc might sit on the menu for a whole presentation.
    let mut menu_track_index: u8 = 0;
    let mut music = CddaStarter::new().with_spins(CDDA_SPINS);
    let mut clock = CddaClock::new(TICKS_HZ);
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
    // How far the camera has flown into the starfield.
    let mut travel: i32 = 0;
    let mut italian = false;
    let mut prev_held = ButtonState::default();
    let mut order = [0usize; MAX_ENTRIES];
    let mut beads = [Bead::default(); SPHERE_POINTS];

    loop {
        tick = tick.wrapping_add(1);
        if menu_track != 0 {
            if music.tick(tick, menu_track + menu_track_index) {
                clock.start(tick);
            }
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
        // Skipping tracks by hand. The drive is already playing, so this is
        // the same handshake the end of a track takes, just triggered early.
        // The drive takes the better part of a second to pick up a new track.
        // Ignore further presses until it has, or the handshake gets re-armed
        // from the start each time and never finishes.
        let loading = menu_track != 0 && !music.started();
        if menu_track != 0 && menu_track_count > 1 && !loading {
            let skip = if pressed(button::R1) {
                1
            } else if pressed(button::L1) {
                menu_track_count - 1 // one back, without going negative
            } else {
                0
            };
            if skip != 0 {
                menu_track_index = (menu_track_index + skip) % menu_track_count;
                // Silence first: the handshake re-issues Play, and leaving the
                // old track running under it is how the drive got wedged.
                let _ = cdrom::try_stop(CDDA_SPINS);
                music.begin(tick);
            }
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
                // Nothing behind the credits entry to chain-load.
                if entries[index].exe_lba != 0 {
                    // Never returns when the disc is readable.
                    boot(&entries[index]);
                }
            }
        }
        prev_held = pad;

        // Ease toward the selection instead of snapping. `target` is allowed
        // to run past a full turn so the ring keeps spinning the short way
        // rather than unwinding.
        let step = TURN / count.max(1) as i32;
        let target = -selected * step;
        ring += (target - ring) >> EASE_SHIFT;

        // Everything visual answers the beat. The grid was measured off the
        // audio and shipped in the table, so this stays in step for the whole
        // length of a track rather than drifting out of it.
        let song_ms = if clock.playing() { clock.tick(tick) } else { 0 };
        let beat = match header.and_then(|h| h.beat(menu_track_index as usize)) {
            Some((beat_ms, phase_ms)) if clock.playing() => {
                carousel::beat_at(song_ms, beat_ms, phase_ms)
            }
            _ => carousel::Beat::default(),
        };
        // The downbeat gets the bigger shove. A hard browse adds to the same
        // term, so the ball blows apart and re-forms as the kick decays.
        let swell_beat = (beat.pulse as i32
            * if beat.is_downbeat() {
                SWELL_DOWNBEAT
            } else {
                SWELL_BEAT
            })
            / 255;
        let scatter = ((spin_rate.abs() - SPHERE_IDLE_SPIN).max(0) * SCATTER_PER_KICK / 5)
            .min(SCATTER_MAX);
        let swell = swell_beat + scatter;

        // Coast the ball back to its idle drift, which is itself riding the
        // bar: quickest just after the downbeat, slowest going into the next.
        let idle = SPHERE_IDLE_SPIN + (beat.bar as i32 * SPHERE_BAR_SWING) / 255;
        spin_rate = carousel::ease_spin(spin_rate, idle, SPHERE_DECAY_SHIFT);
        spin = (spin + spin_rate) & (TURN - 1);
        // Flying forward the whole time, and a browse shoves the camera along
        // with the ball.
        travel = travel.wrapping_add(spin_rate.max(1));

        fb.clear(26, 0, 4);
        draw_starfield(travel, beat.offbeat);
        draw_sphere(spin, swell, &mut beads);

        paint::header_strip(HEADER_H);
        banner.draw(160 - BANNER_W / 2, BANNER_Y);
        centred(&font, BANNER_Y + BANNER_H + 2, "DEMO DISC", TITLE);
        if let Some(header) = header {
            draw_music_panel(
                &small,
                &header,
                menu_track_index,
                &beat,
                menu_track_count > 1,
                loading,
                spectrum_frame(&header, menu_track_index, song_ms, spectrum_frames),
            );
        }

        if count == 0 {
            centred(&font, DESC_TOP, "DISC TABLE OF CONTENTS UNREADABLE", ERROR);
        } else {
            let index = selected.rem_euclid(count as i32) as usize;
            if entries[index].exe_lba == 0 {
                draw_credits(&font, &header.expect("count came from it"));
            } else {
                draw_description(&font, &entries[index], italian);
            }
            draw_ring(&font, &entries[..count], ring, step, &beat, &mut order);
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

/// `offbeat` is 255 between two beats and 0 on them, so the sky twinkles in
/// the gaps the ball and the pills leave.
/// Stars flying past the camera. `offbeat` is 255 between two beats and 0 on
/// them, so the sky twinkles in the gaps the ball and the pills leave.
fn draw_starfield(travel: i32, offbeat: u8) {
    for i in 0..STARS {
        let star = carousel::star(i, travel);
        if !star.visible {
            continue;
        }
        let twinkler = i % 7 == 0;
        let lift = if twinkler { offbeat / 3 } else { offbeat / 8 };
        let b = star.bright.saturating_add(lift);
        let size = star.size + u16::from(twinkler && offbeat > 190);
        // Warm white going to ember red as they fade into the distance.
        gpu::draw_rect_flat(star.x, star.y, size, size, b, (b * 3) / 8, (b * 4) / 16);
    }
}

/// `swell` pushes the beads outward from the centre, so the ball itself grows
/// and snaps back rather than each bead getting fatter in place.
fn draw_sphere(spin: i32, swell: i32, beads: &mut [Bead; SPHERE_POINTS]) {
    let n = carousel::sphere(spin, swell, beads);
    carousel::sort_by_depth(&mut beads[..n], |b| b.z);
    let lift = (swell.clamp(0, 255) / 3) as u8;
    for bead in &beads[..n] {
        paint::bead(&Bead {
            lit: bead.lit.saturating_add(lift),
            ..*bead
        });
    }
}

/// Who made what is on the disc. The music is here by permission, and an
/// attribution that only exists in a README is not an attribution, so this is
/// the page that discharges it: artist first, then every track by name.
fn draw_credits(font: &FontAtlas, header: &Header) {
    let bottom = DESC_TOP + (disc_toc::DESC_LINES as i16 - 1) * DESC_LEADING + 8;
    paint::text_panel(8, DESC_TOP - 6, 304, bottom - DESC_TOP + 12);

    // The same font and the same wrap as a description, because this is one:
    // an entry on the carousel that happens to have no program behind it.
    // The artist takes two lines and the four tracks take the other four,
    // which is exactly the room a description has.
    let mut line = 0i16;
    let mut rest = header.credit_str();
    while !rest.is_empty() && line < 2 {
        let (head, tail) = wrap(rest, WRAP_CHARS);
        centred(font, DESC_TOP + line * DESC_LEADING, head, BLURB);
        line += 1;
        rest = tail;
    }
    for track in 0..header.menu_track_count as usize {
        if line >= disc_toc::DESC_LINES as i16 {
            break;
        }
        let title = header.title(track);
        if !title.is_empty() {
            centred(font, DESC_TOP + line * DESC_LEADING, title, TRACK_NAME);
            line += 1;
        }
    }
}

/// Top-left: what is playing, a level meter that dances on the beat, and the
/// one control worth labelling. Nothing about the screen suggests the
/// shoulder buttons do anything, so that one is spelled out.
fn draw_music_panel(
    font: &FontAtlas,
    header: &Header,
    track: u8,
    beat: &carousel::Beat,
    skippable: bool,
    loading: bool,
    levels: Option<&[u8]>,
) {
    let title = header.title(track as usize);
    if title.is_empty() {
        return;
    }
    // All of it down the header's left edge, in reading order.
    if loading {
        // The drive takes a moment to pick a track up, and a menu that just
        // goes quiet reads as broken. Say what it is doing, where the label
        // that it is playing would be.
        font.draw_text(4, MUSIC_TOP, "LOADING", NOW_PLAYING);
        font.draw_text(6, TRACK_TOP, title, TRACK_NAME);
        return;
    }
    // Label and control share a row. Spelling out the one control worth
    // labelling costs nothing here and saves a row of header.
    font.draw_text(
        4,
        MUSIC_TOP,
        if skippable {
            "NOW PLAYING | L1/R1"
        } else {
            "NOW PLAYING"
        },
        NOW_PLAYING,
    );
    // The title brightens on the beat, so the words themselves keep time.
    let lift = beat.pulse / 4;
    let tint = (
        TRACK_NAME.0.saturating_add(lift),
        TRACK_NAME.1.saturating_add(lift),
        TRACK_NAME.2.saturating_add(lift),
    );
    font.draw_text(6, TRACK_TOP, title, tint);
    match levels {
        Some(levels) => paint::level_meter(6, METER_BASE, levels),
        // No analysis for this track: keep time off the beat instead of
        // leaving a dead space where the meter should be.
        None => paint::level_meter_beat(6, METER_BASE, beat.pulse),
    }
}

/// The selected game's blurb in one language, under the flag of whichever
/// one it is. Up or down swaps.
fn draw_description(font: &FontAtlas, entry: &Entry, italian: bool) {
    // Top-right, clear of the description band and the ball.
    if italian {
        paint::flag_it(320 - paint::FLAG_W - 5, 4);
    } else {
        paint::flag_uk(320 - paint::FLAG_W - 5, 4);
    }
    let text = if italian {
        entry.desc_it_str()
    } else {
        entry.desc_en_str()
    };
    // A panel first, so the text does not have to compete with the ball and
    // the starfield behind it.
    let bottom = DESC_TOP + (disc_toc::DESC_LINES as i16 - 1) * DESC_LEADING + 8;
    paint::text_panel(8, DESC_TOP - 6, 304, bottom - DESC_TOP + 12);

    // Greedy wrap, a line at a time. mkdisc has already checked the text fits
    // in DESC_LINES of them, so nothing is dropped here.
    let mut rest = text;
    for line in 0..disc_toc::DESC_LINES as i16 {
        let (head, tail) = wrap(rest, WRAP_CHARS);
        centred(font, DESC_TOP + line * DESC_LEADING, head, BLURB);
        rest = tail;
        if rest.is_empty() {
            break;
        }
    }
}

/// The carousel: place every entry on the ring, draw back to front, and label
/// each pill.
fn draw_ring(
    font: &FontAtlas,
    entries: &[Entry],
    ring: i32,
    step: i32,
    beat: &carousel::Beat,
    order: &mut [usize; MAX_ENTRIES],
) {
    let count = entries.len();
    for (slot, item) in order.iter_mut().enumerate().take(count) {
        *item = slot;
    }
    let placed = |slot: usize| -> Placed { carousel::place(ring + slot as i32 * step) };
    carousel::sort_by_depth(&mut order[..count], |slot| placed(*slot).z);

    // Reflections first, all of them, so a nearer pill's reflection cannot
    // draw over a nearer pill.
    for &slot in &order[..count] {
        paint::pill_reflection(&placed(slot), carousel::FLOOR_Y);
    }

    // Only the downbeat flashes the pills. Lifting them every beat left
    // nothing for the downbeat to be.
    let flash = if beat.is_downbeat() { beat.pulse } else { 0 };
    for &slot in &order[..count] {
        let item = placed(slot);
        paint::pill(&item, flash);

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

/// Read the level-meter data for every menu track. Returns how many frames
/// landed in the buffer, which is 0 when the disc carries none or when it
/// carries more than there is room for.
fn read_spectrum(header: Option<&Header>) -> u32 {
    let Some(header) = header else { return 0 };
    if header.spectrum_lba == 0 {
        return 0;
    }
    let total: u32 = header.spectrum_frames.iter().sum();
    let bytes = total as usize * disc_toc::SPECTRUM_BANDS;
    let sectors = bytes.div_ceil(SECTOR_WORDS * 4);
    if sectors > SPECTRUM_MAX_SECTORS {
        tty::println("launcher: spectrum too large for the buffer, meter off");
        return 0;
    }
    // SAFETY: single-threaded, polled; only `main` reaches these statics.
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(READER) };
    let buffer = unsafe { &mut *core::ptr::addr_of_mut!(SPECTRUM) };

    let mut ok = unsafe { reader.prepare() && reader.start_read(header.spectrum_lba) };
    for chunk in buffer.chunks_exact_mut(SECTOR_WORDS).take(sectors) {
        let slot: &mut [u32; SECTOR_WORDS] = chunk.try_into().expect("exact chunks");
        ok = ok && unsafe { reader.read_sector(slot) };
    }
    unsafe { reader.stop() };
    if ok {
        total
    } else {
        0
    }
}

/// The band levels to draw right now, or `None` when this track has no
/// analysis on the disc.
fn spectrum_frame(header: &Header, track: u8, song_ms: u32, loaded_frames: u32) -> Option<&[u8]> {
    if loaded_frames == 0 {
        return None;
    }
    let (offset, frames) = header.spectrum_span(track as usize)?;
    let frame = song_ms * disc_toc::SPECTRUM_FRAME_RATE / 1000;
    // Hold the last frame rather than wrapping: the clock can run a little
    // past the end of a track while the drive notices it has finished.
    let at = (offset + frame.min(frames.saturating_sub(1))) as usize;
    let start = at * disc_toc::SPECTRUM_BANDS;
    // SAFETY: as `read_spectrum`; read-only here.
    let buffer = unsafe { &*core::ptr::addr_of!(SPECTRUM) };
    let bytes = unsafe {
        core::slice::from_raw_parts(buffer.as_ptr() as *const u8, buffer.len() * 4)
    };
    bytes.get(start..start + disc_toc::SPECTRUM_BANDS)
}

/// Read the table of contents. `None` if the disc has none.
fn read_toc(entries: &mut [Entry; MAX_ENTRIES]) -> Option<Header> {
    // SAFETY: single-threaded, polled; `main` runs once and nothing else
    // touches these statics.
    let reader = unsafe { &mut *core::ptr::addr_of_mut!(READER) };
    let sector = unsafe { &mut *core::ptr::addr_of_mut!(TOC_SECTOR) };

    // The table spans more than one sector now; `read_sector` walks the same
    // ReadN stream, so they arrive back to back.
    let mut ok = unsafe { reader.prepare() && reader.start_read(TOC_LBA) };
    for chunk in sector.chunks_exact_mut(SECTOR_WORDS) {
        let slot: &mut [u32; SECTOR_WORDS] = chunk.try_into().expect("exact chunks");
        ok = ok && unsafe { reader.read_sector(slot) };
    }
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
