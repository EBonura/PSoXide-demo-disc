//! Demo disc boot menu.
//!
//! The disc's first program, on the scene core: plays the SDK splash once, reads the disc's
//! scene table and its own table of contents ([`disc_toc`]), shows the programs as cases on a
//! shelf, plays the menu songs, and starts the selection with a core switch. The
//! look is the PSoXide home shell's at 320x240: near-black blue-grey, a soft
//! teal glow, white type and one cyan accent.
//!
//! The pieces: [`shelf`] is the row of cases and the title, description and
//! start button under it; [`launch`] is the jewel case opening into the disc
//! that carries the program in; [`credits`] is the card for the music credit,
//! the links and the QR; [`chain`] is the hand-off through the core;
//! [`music`] plays the XA songs. [`common`] and [`gfx`] hold the header,
//! footer, case drawing and the immediate-mode GPU wrapper, [`text`] and
//! `assets/ui_atlas.rs` the baked faces, and [`cover`] the baked pictures.
//!
//! Which programs there are, their names, versions, descriptions and songs all
//! come from the disc table, so one launcher serves every pressing. The
//! `private` feature only decides which covers (and names) are compiled in.

#![no_std]
#![no_main]
#![allow(clippy::too_many_arguments)]

extern crate psx_rt;

mod attract;
mod chain;
mod common;
mod cover;
mod credits;
// The disc the launch ends on, shared with the chain loader (see its header).
#[path = "../../loader/src/disc_art.rs"]
#[allow(dead_code)] // the loader uses the rest
mod disc_art;
mod face;
mod gfx;
mod launch;
mod music;
mod shelf;
mod state;
mod text;
#[path = "../assets/ui_atlas.rs"]
#[allow(dead_code)]
mod ui_atlas;

use common::{Ctx, NowPlaying, Playback, Screen};
use disc_toc::{Entry, Header, MAX_ENTRIES, TOC_BYTES};
use gfx::Gfx;
use music::Music;
use playlist::Slot;
use psx_pack::cd::{SectorReader, SECTOR_WORDS};
use psx_pad::{button, AnalogRequirement, ButtonState, Controllers, Port, RadialStick};
use psx_rt::Peripherals;
use psx_sfx::{Bank, OneShot, Player};
use psx_spu::{self as spu, Adsr, CdVolume, Pitch, Spu, SpuAddr, Voice, Volume};

/// Which pressing this is: `git describe` at build time, on screen in the
/// header so every photo and recording says which disc is in the console.
const DISC_VERSION: &str = match option_env!("DISC_VERSION") {
    Some(v) => v,
    None => "DEV",
};

/// The pressing's name beside the logo, `DEMO DISC` unless the build says
/// otherwise.
const DISC_TITLE: &str = match option_env!("DISC_TITLE") {
    Some(v) => v,
    None => "DEMO DISC",
};

static UI_TEX: &[u8] = include_bytes!("../assets/ui.tex");
static ICONS_TEX: &[u8] = include_bytes!("../assets/icons.tex");
static ICONS_CLUT: &[u8] = include_bytes!("../assets/icons.clut");

/// Display frames a second, which is what the song clock counts in.
const TICKS_HZ: u32 = 60;

/// How loud the two blips sit against the menu music, which plays at full
/// CD volume. Browse fires on every move so it stays the quieter of the two;
/// select is a one-off and can afford to land.
const BROWSE_GAIN: Volume = Volume::linear(1, 4);
const SELECT_GAIN: Volume = Volume::linear(1, 3);
/// The launch swoosh rides under the disc's spin-up, so it carries the moment.
const LAUNCH_GAIN: Volume = Volume::linear(2, 5);
/// The swoosh is 0.56 s; pitched to 11/16 of unity it lasts about 0.82 s, which
/// is the spin-up.
const LAUNCH_STRETCH_NUM: u32 = 11;
const LAUNCH_STRETCH_DEN: u32 = 16;
/// Launch frame the swoosh starts on, so it ends at the hand-off (the loader
/// silences the SPU as it takes over).
const SWOOSH_FRAME: i32 = launch::FRAMES - 49;

/// A blip when the shelf moves and a heavier one when a program is chosen.
/// Two voices, well clear of the CD input; the swoosh gets its own so the
/// confirm chirp under it is not cut off.
const VOICE_BROWSE: Voice = Voice::V0;
const VOICE_SELECT: Voice = Voice::V1;
const VOICE_LAUNCH: Voice = Voice::V2;
const SFX_BASE: SpuAddr = SpuAddr::new(0x1010);
/// The carousel tick of old, chosen by measuring every sound in the project:
/// `click` is 0.10 s and the brightest short sound there is, which is what a
/// tick heard a hundred times a minute over music on a TV speaker needs.
static SFX_BROWSE: &[u8] =
    psx_sfx::include_psau!("../../games/PSoXide-sdk/assets/audio/voxide-ui/psau/ui_click.psau");
static SFX_SELECT: &[u8] =
    psx_sfx::include_psau!("../../games/PSoXide-sdk/assets/audio/voxide-ui/psau/ui_confirm.psau");
static SFX_LAUNCH: &[u8] =
    psx_sfx::include_psau!("../../games/PSoXide-sdk/assets/audio/freesfx/psau/swoosh.psau");
/// Slots into the sound player, in the order its voices were handed over.
const SFX_BROWSE_SLOT: usize = 0;
const SFX_SELECT_SLOT: usize = 1;
const SFX_LAUNCH_SLOT: usize = 2;

/// Frames the menu fades in from black at boot.
const BOOT_FADE: u32 = 30;

/// `SCENES.TBL` then the table of contents, read together at boot (`scene_layout`).
const BOOT_TABLE_SECTORS: usize = 1 + disc_toc::TOC_SECTORS as usize;
static mut TOC_SECTOR: [u32; SECTOR_WORDS * BOOT_TABLE_SECTORS] =
    [0; SECTOR_WORDS * BOOT_TABLE_SECTORS];

/// The launcher's RAM copy of `SCENES.TBL`, the table every core switch takes: the header and
/// every row (`16 + 8 * (scenes + images)` words, at most one sector).
static mut SCENES: [u32; SECTOR_WORDS] = [0; SECTOR_WORDS];

/// Room for every menu song's level-meter data. Read once at boot rather
/// than per song, so skipping stays as quick as the drive allows. 37,484
/// frames of 16 bands (the six songs, 21 minutes) take 293 sectors; this is
/// 320, 640 KiB of .bss, and `read_spectrum` turns the meter off and says so
/// if a pressing carries more.
const SPECTRUM_MAX_SECTORS: usize = 320;
static mut SPECTRUM: [u32; SECTOR_WORDS * SPECTRUM_MAX_SECTORS] =
    [0; SECTOR_WORDS * SPECTRUM_MAX_SECTORS];

#[no_mangle]
fn main() {
    // psx-rt's VBlank handler flips every frame the presenter queues.
    psx_rt::interrupts::install_vblank_counter();
    // The SDK caps presentation at 30 fps by default; the launcher keeps flipping every VBlank.
    psx_rt::present::set_cap(psx_tick::PresentCap::UNCAPPED);

    let peripherals = Peripherals::take().expect("peripherals are taken once");
    let mut port = peripherals.controller_port;
    let mut g = Gfx::new(peripherals.gpu_dma);
    // The disc needs a DualShock in analog mode. `Controllers` puts a pad that
    // answers there and locks it on the first poll, so the Analog button cannot
    // drop it back to digital mid-menu, and returns the last clean state when a
    // packet fails validation, so a garbled one never turns into a phantom press.
    // A pad that is not in analog mode (a digital pad, one plugged in later, an
    // empty port) holds the menu on a notice; an empty port is probed once in
    // 60 polls. The splash's polls run the handshake.
    let mut pads = Controllers::new();
    // The SDK splash, once per disc: the launcher is the disc's first program, so it plays on
    // a BIOS boot and never after a core switch (no program switches back to the menu today).
    if !entered_by_switch() {
        splash(&mut g, &mut || pads.poll(&mut port, Port::One).buttons);
    }
    g.upload_ui(UI_TEX);
    for (i, program) in cover::PROGRAMS.iter().enumerate() {
        g.upload_art(i, &program.art);
    }
    attract::upload_backs(&mut g);
    g.upload_link_icons(ICONS_TEX, ICONS_CLUT, credits::ICON_COUNT);

    // The sound chip comes up before anything is uploaded to it.
    let mut spu = Spu::new(peripherals.spu_dma);

    let mut entries = [Entry::new("", 0, 0, 0); MAX_ENTRIES];
    // Read the tables before a note of music plays: the drive is busy for the
    // whole of a song, so every data read the menu needs comes first.
    let mut reader = SectorReader::with_cd(peripherals.cd);
    // SAFETY: single-threaded, polled; only `main` reaches this static.
    let scenes = unsafe { &mut *core::ptr::addr_of_mut!(SCENES) };
    let header = read_tables(&mut reader, &mut entries, scenes);
    let count = header.map_or(0, |h| h.count);
    let entries = &entries[..count];

    // Before the music starts, for the same reason the table is.
    let spectrum_frames = read_spectrum(&mut reader, header.as_ref());

    // The menu songs are XA files in this ISO, found by name. The table's
    // song count says how many the pressing was built with.
    let mut music = Music::locate(reader, header.map_or(0, |h| h.menu_track_count as usize));
    let has_music = music.available();
    let mut tick: u32 = 0;
    if has_music {
        // The CD controller playing is only half of it: the SPU's CD input
        // comes up silent, so without this the drive spins a song nobody
        // hears.
        spu::set_main_volume(Volume::MAX, Volume::MAX);
        spu::set_cd_volume(CdVolume::MAX, CdVolume::MAX);
        spu::enable_cd_audio(true);
        music.begin(tick);
    }
    // Independent of the music: the blips play whether or not the disc
    // carries a menu track. psx-sfx owns the sequential upload, the key-on
    // that writes a repeat address, and the cutoff that stops a one-shot on a
    // clock because END will not.
    let mut bank = Bank::new(SFX_BASE);
    // A sample whose upload failed is never played (the SDK latches the fault; no retry).
    let sfx_browse = bank
        .upload_on(&mut spu, SFX_BROWSE)
        .ok()
        .map(|sample| OneShot::new(sample, BROWSE_GAIN));
    let sfx_select = bank
        .upload_on(&mut spu, SFX_SELECT)
        .ok()
        .map(|sample| OneShot::new(sample, SELECT_GAIN).with_adsr(Adsr::percussive()));
    // 0x1000 is unity; scaling it down stretches the sample over the launch.
    let sfx_launch = bank.upload_on(&mut spu, SFX_LAUNCH).ok().map(|sample| {
        OneShot::new(sample, LAUNCH_GAIN).with_pitch(Pitch::raw(
            (0x1000 * LAUNCH_STRETCH_NUM / LAUNCH_STRETCH_DEN) as u16,
        ))
    });
    let mut sfx: Player<3> = Player::new([VOICE_BROWSE, VOICE_SELECT, VOICE_LAUNCH], TICKS_HZ);

    let mut sel: i32 = 0;
    let mut shelf = shelf::State { pos: 0 };
    let mut screen = Screen::Shelf;
    let mut italian = false;
    // Frames since the start press, or -1 while the menu is just a menu.
    let mut launching: i32 = -1;
    let mut seek: Option<chain::Seek> = None;
    let mut attract: Option<attract::Attract> = None;
    // Frames since the pad last did anything.
    let mut idle: u32 = 0;
    let mut prev_held = ButtonState::default();
    // A poll before the menu draws (the handshake, when the splash was skipped).
    pads.poll(&mut port, Port::One);
    // The left stick drives the menu like the d-pad: one press per push, no
    // repeat. A push registers past 64 counts of the 127 a healthy stick reaches
    // (about half travel) and the stick has to come back inside 32 before it can
    // press again, so one resting near the threshold cannot chatter.
    let mut stick = RadialStick::<64, 32>::new();
    let mut analog_ok = pads.analog(Port::One) == AnalogRequirement::Analog;

    loop {
        tick = tick.wrapping_add(1);
        // The playlist's last slot is MUSIC OFF: the L1/R1 walk goes through
        // every song and then silence, so muting needs no button of its own
        // and the header can say which state it is in.
        let slot = music.slot();
        let muted = has_music && slot == Slot::Off;
        // Which song the table's titles and meter data belong to.
        let track = match slot {
            Slot::Song(n) => n,
            Slot::Off => 0,
        };
        // The player reads the head a few times a second and notices when a
        // song ends by itself, which moves the playlist on.
        music.frame(tick);
        // One clock read a frame, for the meter.
        let song_ms = if !muted && music.playing() {
            music.song_ms()
        } else {
            0
        };

        // Stop any SFX voice that has outlived its own sample. Volume, not
        // key_off: immediate, and owing nothing to envelope behaviour.
        sfx.tick(tick);

        let state = *pads.poll(&mut port, Port::One);
        // Once a launch is under way the menu no longer reacts to the pad, so
        // an unplugged one does not put the notice over the animation.
        if launching < 0 {
            analog_ok = pads.analog(Port::One) == AnalogRequirement::Analog;
        }
        // The SDK's one stick rule: a pad that reports no sticks holds nothing and
        // lets go of any push.
        let stick_bits = stick.update_pad(&state);
        let pad = ButtonState::from_bits(state.buttons.bits() | stick_bits);
        // Buttons still track while the notice is up, so one held through
        // it does not fire when the pad is accepted.
        let pressed = |b: u16| analog_ok && pad.is_held(b) && !prev_held.is_held(b);
        // Any button held, or the stick pushed, is activity; any of them also
        // ends the attract mode at once, on the case it was showing, and is not
        // taken as a menu command as well.
        let touched = pad.bits() != 0;
        idle = if touched { 0 } else { idle.saturating_add(1) };
        let mut woke = false;
        if let Some(a) = attract.take_if(|_| touched) {
            sel = a.selection() as i32;
            shelf.pos = sel * 256;
            woke = true;
        }

        // The pad goes quiet once a launch is under way: the animation is
        // short, and half a browse queued behind it would land in the game.
        if launching < 0 && !woke {
            if pressed(button::TRIANGLE) {
                italian = !italian;
            }
            // Skipping songs by hand, through the shuffle and then the MUSIC
            // OFF slot. A skip while the last song is still starting is fine:
            // starting a song begins with a stop.
            if has_music {
                if pressed(button::R1) {
                    music.skip_forward(tick);
                } else if pressed(button::L1) {
                    music.skip_back(tick);
                }
            }
            match screen {
                Screen::Shelf if count > 0 => {
                    let step = pressed(button::RIGHT) as i32 - pressed(button::LEFT) as i32;
                    let moved = (sel + step).clamp(0, count as i32 - 1);
                    if moved != sel {
                        sel = moved;
                        if let Some(shot) = &sfx_browse {
                            sfx.play_on(SFX_BROWSE_SLOT, shot, tick);
                        }
                    }
                    if pressed(button::SELECT) {
                        screen = Screen::Credits;
                        if let Some(shot) = &sfx_select {
                            sfx.play_on(SFX_SELECT_SLOT, shot, tick);
                        }
                    }
                    if pressed(button::CROSS) || pressed(button::START) {
                        // Confirm chirp now; the swoosh joins on its frame.
                        if let Some(shot) = &sfx_select {
                            sfx.play_on(SFX_SELECT_SLOT, shot, tick);
                        }
                        launching = 0;
                    }
                }
                Screen::Shelf => {
                    if pressed(button::SELECT) {
                        screen = Screen::Credits;
                    }
                }
                Screen::Credits => {
                    if pressed(button::CIRCLE) || pressed(button::SELECT) {
                        screen = Screen::Shelf;
                        if let Some(shot) = &sfx_browse {
                            sfx.play_on(SFX_BROWSE_SLOT, shot, tick);
                        }
                    }
                }
            }
        }
        // Nothing touched for twenty seconds: the shelf tours itself.
        if launching < 0
            && screen == Screen::Shelf
            && analog_ok
            && count > 0
            && attract.is_none()
            && idle == attract::IDLE_FRAMES
        {
            attract = Some(attract::Attract::new(entries, sel as usize));
        }
        prev_held = pad;
        state::publish(&state::Frame {
            entries: count,
            analog_ok,
            pad: pads.analog(Port::One),
            screen,
            attract: attract.is_some(),
            selected: sel,
            launching: launching >= 0,
            frames: tick,
        });

        // The playing state for the header.
        let playback = if !has_music {
            Playback::None
        } else if muted {
            Playback::Off
        } else if music.loading() {
            Playback::Loading
        } else {
            Playback::Playing
        };
        let levels = header
            .as_ref()
            .and_then(|h| spectrum_frame(h, track, song_ms, spectrum_frames));
        let ctx = Ctx {
            t: tick,
            it: italian,
            screen,
            disc_title: DISC_TITLE,
            disc_version: DISC_VERSION,
            music: NowPlaying {
                state: playback,
                title: header.as_ref().map_or("", |h| h.title(track as usize)),
                levels,
            },
            credit: header.as_ref().map_or("", |h| h.credit_str()),
        };

        g.begin();
        g.clear((4, 8, 12));
        if count == 0 {
            shelf::backdrop(&mut g);
            common::header(&mut g, &ctx);
            text::draw_centered_mid(
                &mut g,
                &ui_atlas::SMALL8,
                160,
                118,
                "DISC TABLE OF CONTENTS UNREADABLE",
                gfx::ACCENT,
                0,
            );
        } else if !analog_ok {
            shelf::backdrop(&mut g);
            common::header(&mut g, &ctx);
            draw_controller_notice(&mut g);
        } else if screen == Screen::Credits {
            credits::draw(&mut g, &ctx);
        } else if let Some(a) = attract.as_mut() {
            a.update(levels.unwrap_or(&[0; attract::BANDS]));
            a.draw(&mut g, &ctx, entries);
        } else {
            shelf::update(&mut shelf, sel);
            // Past frame 30 the veil has taken the menu to black, so it is not
            // drawn at all: the hand-off is cheaper than the menu it replaces.
            if launching < 30 {
                shelf::draw(&mut g, &shelf, &ctx, entries, sel, launching);
            }
            if (0..=launch::FRAMES).contains(&launching) {
                let from = launch::From {
                    cx: 160,
                    bottom: shelf::FLOOR - 3,
                    sc: 256,
                };
                launch::draw(&mut g, launching, from, &entries[sel as usize], &ctx);
            }
        }
        if tick < BOOT_FADE {
            let v = (255 * (BOOT_FADE - tick) / BOOT_FADE) as u8;
            g.quad_g_sub([(0, 0), (320, 0), (0, 240), (320, 240)], [(v, v, v); 4]);
        }
        g.end();

        if launching >= 0 {
            let chosen = &entries[sel as usize];
            if launching == 0 {
                // The first frame of the launch is on screen: now stop the
                // song and point the drive at the program, while the case
                // opens.
                seek = Some(chain::Seek::start(chosen, &mut music));
            }
            if launching == SWOOSH_FRAME {
                if let Some(shot) = &sfx_launch {
                    sfx.play_on(SFX_LAUNCH_SLOT, shot, tick);
                }
            }
            if let Some(seek) = seek.as_mut() {
                seek.frame();
                if launching >= launch::FRAMES {
                    state::hand_off();
                    // Never returns: the core loads the program, or shows why it cannot.
                    chain::boot(sel as usize, scenes, &mut g, seek);
                }
            }
            launching += 1;
        }
    }
}

/// What the menu shows while port 1 holds no DualShock in analog mode.
fn draw_controller_notice(g: &mut Gfx) {
    const LINES: [&str; 5] = [
        "This disc needs an analog controller",
        "(DualShock) in port 1.",
        "",
        "Questo disco richiede un controller",
        "analogico (DualShock) nella porta 1.",
    ];
    let (x, w, h) = (36, 248, 78);
    let y = 120 - h / 2;
    let (top, bot) = ((9, 20, 27), (4, 11, 15));
    g.quad_g([(x, y), (x + w, y), (x, y + h), (x + w, y + h)], [top, top, bot, bot]);
    g.frame(x, y, w, h, gfx::FOCUS);
    for (row, line) in LINES.iter().enumerate() {
        let tint = if row < 2 { gfx::TEXT } else { gfx::MUTED };
        text::draw_centered_mid(
            g,
            &ui_atlas::SMALL8,
            160,
            y + 14 + row as i32 * 12,
            line,
            tint,
            0,
        );
    }
}

/// Read the level-meter data for every menu track. Returns how many frames
/// landed in the buffer, which is 0 when the disc carries none or when it
/// carries more than there is room for.
fn read_spectrum(reader: &mut SectorReader, header: Option<&Header>) -> u32 {
    let Some(header) = header else { return 0 };
    if header.spectrum_lba == 0 {
        return 0;
    }
    let total: u32 = header.spectrum_frames.iter().sum();
    let bytes = total as usize * disc_toc::SPECTRUM_BANDS;
    let sectors = bytes.div_ceil(SECTOR_WORDS * 4);
    if sectors > SPECTRUM_MAX_SECTORS {
        return 0;
    }
    // SAFETY: single-threaded, polled; only `main` reaches this static.
    let buffer = unsafe { &mut *core::ptr::addr_of_mut!(SPECTRUM) };

    let mut ok = reader.prepare() && reader.start_read(header.spectrum_lba);
    for chunk in buffer.chunks_exact_mut(SECTOR_WORDS).take(sectors) {
        let slot: &mut [u32; SECTOR_WORDS] = chunk.try_into().expect("exact chunks");
        ok = ok && reader.read_sector(slot);
    }
    reader.stop();
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
    let bytes =
        unsafe { core::slice::from_raw_parts(buffer.as_ptr() as *const u8, buffer.len() * 4) };
    bytes.get(start..start + disc_toc::SPECTRUM_BANDS)
}

/// Whether a core switch entered this program (the handoff block says so); false on a BIOS boot.
fn entered_by_switch() -> bool {
    use psx_core::abi::block;
    psx_core::link::block_valid()
        && psx_core::link::block_word(block::OFF_FLAGS) & block::FLAG_CORE_ENTRY != 0
}

/// The SDK splash, from boot-only memory: psoxide-boot.ld lays this function in `.boot` with
/// psx-splash's own code, and none of it stays resident. It returns before anything writes over
/// `.boot`'s run range (the heap is not used before the menu).
#[inline(never)]
#[cfg_attr(target_arch = "mips", link_section = ".text.boot_only.launcher_splash")]
fn splash(g: &mut Gfx, buttons: &mut dyn FnMut() -> ButtonState) {
    let (gpu, buffers) = g.splash_parts();
    psx_splash::show(
        gpu,
        buffers,
        "Built with PSoXide",
        psx_display::Brightness::DEFAULT,
        buttons,
    );
}

/// Read `SCENES.TBL` (LBA 22) into `scenes` and the table of contents after it (LBA 23) into
/// `entries`, in one read. `None` if the disc has either missing, or if their counts disagree
/// (entry `i` is started by scene `i + 1`, `scene_layout`): the menu then says the table is
/// unreadable rather than start the wrong program.
fn read_tables(
    reader: &mut SectorReader,
    entries: &mut [Entry; MAX_ENTRIES],
    scenes: &mut [u32; SECTOR_WORDS],
) -> Option<Header> {
    // SAFETY: single-threaded, polled; `main` runs once and nothing else
    // touches this static.
    let sector = unsafe { &mut *core::ptr::addr_of_mut!(TOC_SECTOR) };

    // The tables span more than one sector; `read_sector` walks the same
    // ReadN stream, so they arrive back to back.
    let mut ok = reader.prepare() && reader.start_read(scene_layout::SCENES_LBA);
    for chunk in sector.chunks_exact_mut(SECTOR_WORDS) {
        let slot: &mut [u32; SECTOR_WORDS] = chunk.try_into().expect("exact chunks");
        ok = ok && reader.read_sector(slot);
    }
    reader.stop();
    if !ok {
        return None;
    }
    let (table, toc) = sector.split_at(SECTOR_WORDS);
    // The core switches with the scene table as it is on the disc (the header's magic is what
    // tells `psx_core::link::switch` it needs no read of its own).
    use psx_core::abi::table;
    if table[0] != table::MAGIC[0] || table[1] != table::MAGIC[1] {
        return None;
    }
    let scene_count = (table[3] & 0xffff) as usize;
    scenes.copy_from_slice(table);
    // SAFETY: a [u32; 512] is 2048 bytes; the target is little-endian, so the
    // word buffer and the on-disc byte order agree.
    let bytes: &[u8; TOC_BYTES] = unsafe { &*(toc.as_ptr() as *const u8 as *const [u8; TOC_BYTES]) };
    let header = disc_toc::decode(bytes, entries)?;
    (scene_count == header.count + 1).then_some(header)
}
