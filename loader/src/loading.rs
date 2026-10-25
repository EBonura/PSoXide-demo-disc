//! The loading presentation: the launch animation's disc, still turning.
//!
//! The launcher's hand-off ends on a disc spinning at the middle of the stage;
//! this module draws that same disc (the code is shared, see `disc_art`) and
//! keeps it turning at the same rate while the program streams in, with one
//! label fading in under it in the Shelf menu's own face. Nothing of the
//! launcher is resident: the label's glyphs are sprites cut from the menu's
//! atlas, which the launcher left in VRAM where it put it.
//!
//! The animation runs from `tick`, which the transport calls between CD steps
//! (after every sector, once per spin of a seek wait, every 4 KiB of the
//! checksum pass). A tick does nothing until a VBlank edge has arrived; then
//! it flips to the frame drawn since the last edge and draws the next one into
//! the other buffer, so there are always two buffers and the display only
//! ever shows a finished one. Only the part of the screen the disc and the
//! label can change is redrawn (clipped by the draw area), which keeps a
//! tick's work well under one sector's 13 ms at double speed.
//!
//! Interrupts stay off throughout, as in the rest of the blob.

use crate::disc_art::{self, Canvas, Rgb};
use crate::paint;
use crate::text::Face;

/// The two 320x240 buffers: the launcher's own pair, so nothing beyond what
/// the launcher already used is touched (the menu's palettes sit below row 480).
const BUFFERS: [i16; 2] = [0, 240];

/// The parts of a buffer a frame can change, in screen pixels (x, y, width,
/// height): the disc's rings reach 52 pixels from its centre, and the label
/// sits under it. Repainting only these keeps a frame's GPU work, and so the
/// CPU time a tick spends waiting on it, down to a few hundred microseconds
/// to a couple of milliseconds.
const DISC_BOX: (i16, i16, i16, i16) = (104, 58, 112, 108);
const LABEL_BOX: (i16, i16, i16, i16) = (112, 176, 96, 28);
const WHOLE: (i16, i16, i16, i16) = (0, 0, 320, 240);

/// GP0(E1h): the menu atlas page (x = 832) with additive blending and
/// dithering, drawing allowed. Opaque primitives ignore the blend bits, so one
/// state serves every packet the loader sends.
const DRAW_MODE: u32 = 0xE100_0000 | 13 | (1 << 5) | (1 << 9) | (1 << 10);
/// The menu atlas's palette row, as a sprite's CLUT word: (256, 480).
const ATLAS_CLUT: u32 = 16 | (480 << 6);

const LABEL: &str = "LOADING";
/// The label's capitals are centred on this row, clear of the disc's glow.
const LABEL_MID: i32 = 190;
/// Frames the label takes to fade in after the hand-off.
const LABEL_FADE_FRAMES: u32 = 20;
/// The menu's muted text colour.
const LABEL_COLOR: Rgb = (157, 176, 190);

/// The checksum pass ticks once per 4 KiB block; a sector is 2 KiB.
const SECTORS_PER_TICK: u32 = 2;
/// Frames the scene takes to dim at the end of the checksum pass. Dimming
/// redraws the whole screen, so it is kept short: that is CPU time the pass
/// would otherwise spend hashing.
const FADE_OUT_FRAMES: u32 = 12;
/// VBlanks between frames while the checksum pass runs. The pass is the one
/// stretch of the load that is bound by the CPU and not by the drive, so every
/// millisecond a frame takes is a millisecond added to the load; half the
/// frame rate halves that, and the spin keeps its speed (the time advances
/// by the VBlanks that passed).
const VERIFY_PERIOD: u32 = 2;

/// How far the checksum pass has got, in its own ticks and in VBlanks.
#[derive(Clone, Copy)]
struct Verify {
    ticks: u32,
    frames: u32,
}

/// Which part of the screen a frame repaints.
#[derive(Clone, Copy)]
enum Pass {
    /// All of it, with everything.
    Whole,
    /// The disc's square: the disc and what it sits in.
    Disc,
    /// The label's strip: what it sits in, and the label.
    Label,
}

pub struct LoadingScreen {
    /// The launch's frame number, carried on: the frame most recently drawn is
    /// where the spin-up and the glow have got to.
    t: i32,
    /// Frames drawn since the hand-off.
    frame: u32,
    /// Index into [`BUFFERS`] of the buffer the display shows.
    shown: usize,
    /// A finished frame waiting in the other buffer for the next VBlank.
    ready: bool,
    /// VBlanks since the last frame was drawn.
    edges: u32,
    /// Sectors in the payload, as the last chunk reported them.
    sectors: u32,
    /// Once the payload is in: the checksum pass so far.
    verifying: Option<Verify>,
}

impl LoadingScreen {
    pub fn new() -> Self {
        // SAFETY: the launcher wrote this word before it jumped here (see
        // `chainload_layout::SHOWN_BUFFER_MAILBOX`); it is plain RAM inside the
        // blob's reservation. Anything but the second buffer's row means the
        // first.
        let shown_row = unsafe {
            core::ptr::read_volatile(chainload_layout::SHOWN_BUFFER_MAILBOX as *const u32)
        };
        Self {
            t: disc_art::LAST_SHOWN,
            frame: 0,
            shown: usize::from(shown_row == BUFFERS[1] as u32),
            ready: false,
            edges: 0,
            sectors: 0,
            verifying: None,
        }
    }

    /// Program the display after the GPU reset, as `paint::setup` does, but
    /// with its start on the buffer the launcher was showing: the screen
    /// (and anything that looks at VRAM while the display is off) keeps the
    /// launcher's last picture, whole, until the first frame is ready.
    pub fn setup(&mut self) {
        let start = 0x0500_0000 | ((BUFFERS[self.shown] as u32 & 0x1ff) << 10);
        for word in [
            0x0300_0001,                                     // display off while painting
            0x0800_0001,                                     // 320x240, NTSC
            0x0600_0000 | 0x260 | ((0x260 + 320 * 8) << 12), // the centred picture
            0x0700_0000 | 0x10 | ((0x10 + 240) << 10),
            start,
        ] {
            psx_io::gpu::write_display_control(word);
        }
    }

    /// Paint the first frame into the buffer the launcher was not showing,
    /// then show it and turn the display on: nothing partial is ever on the
    /// screen. The frame is the one after the launch's last. The buffer the
    /// launcher was showing is painted next, with the frame after that, so the
    /// screen is not kept waiting for it. Both get the whole scene once; after
    /// that a frame only repaints what changes.
    pub fn begin(&mut self) {
        send(&[DRAW_MODE]);
        self.t += 1;
        self.draw(BUFFERS[self.shown ^ 1], Pass::Whole, 256);
        paint::draw_sync();
        self.shown ^= 1;
        display(BUFFERS[self.shown]);
        paint::show();
        // An edge latched before now is not a frame of this screen.
        paint::ack_vblank();
        self.t += 1;
        self.frame += 1;
        self.draw(BUFFERS[self.shown ^ 1], Pass::Whole, 256);
        self.ready = true;
    }

    /// A chunk is in; remember how big the payload is.
    pub fn progress(&mut self, sectors: u32) {
        self.sectors = sectors;
        self.tick();
    }

    /// Time passes: when a VBlank edge has arrived, show the finished frame
    /// and start the next.
    pub fn tick(&mut self) {
        if let Some(pass) = &mut self.verifying {
            pass.ticks += 1;
        }
        if !paint::take_vblank() {
            return;
        }
        self.edges += 1;
        if let Some(pass) = &mut self.verifying {
            pass.frames += 1;
        }
        // A frame still drawing is shown at the next edge instead.
        if self.ready && gpu_idle() {
            self.shown ^= 1;
            display(BUFFERS[self.shown]);
            self.ready = false;
        }
        let period = if self.verifying.is_some() && self.level() == 256 {
            VERIFY_PERIOD
        } else {
            1
        };
        if !self.ready && self.edges >= period {
            self.draw_next();
        }
    }

    /// The payload is in and the checksum pass is next. The scene dims over
    /// the last frames of that pass, so the program takes over from a dark
    /// screen and not from a cut; a payload so small that the whole pass is
    /// over in a few frames dims for as long as it lasts.
    pub fn finish(&mut self) {
        self.verifying = Some(Verify {
            ticks: 0,
            frames: 0,
        });
    }

    /// How bright the scene is, 0..=256: full until the checksum pass is
    /// within [`FADE_OUT_FRAMES`] of its end, then falling to nothing. The
    /// frames left are estimated from the pass's pace so far.
    fn level(&self) -> u32 {
        let Some(pass) = self.verifying else {
            return 256;
        };
        // No pace to go by in the pass's first frames: assume a long pass.
        if pass.frames < 2 {
            return 256;
        }
        let total = (self.sectors / SECTORS_PER_TICK).max(1);
        let left = total.saturating_sub(pass.ticks) * pass.frames / pass.ticks.max(1);
        if left >= FADE_OUT_FRAMES {
            256
        } else {
            256 * left / FADE_OUT_FRAMES
        }
    }

    fn draw_next(&mut self) {
        // The spin keeps its speed over any VBlanks that went by.
        self.t += self.edges as i32;
        self.edges = 0;
        self.frame += 1;
        let level = self.level();
        let back = BUFFERS[self.shown ^ 1];
        if level < 256 || disc_art::glow_level(self.t) < 256 {
            // A dimming scene changes everywhere, glows included.
            self.draw(back, Pass::Whole, level);
        } else {
            self.draw(back, Pass::Disc, level);
            // Both buffers see the label's last step before it is left alone.
            if self.frame <= LABEL_FADE_FRAMES + 1 {
                self.draw(back, Pass::Label, level);
            }
        }
        self.ready = true;
    }

    /// One frame of the scene, or the part of it a pass covers, into the
    /// buffer at `y`: `level` (0..=256) dims everything, which is how the
    /// scene fades out.
    fn draw(&self, y: i16, pass: Pass, level: u32) {
        let (x0, y0, w, h) = match pass {
            Pass::Whole => WHOLE,
            Pass::Disc => DISC_BOX,
            Pass::Label => LABEL_BOX,
        };
        target(y, x0, y0, w, h);
        paint::rect(x0, y0, w, h, 0);
        let mut canvas = Gp0;
        disc_art::disc_glow(&mut canvas, level * disc_art::glow_level(self.t) >> 8);
        if !matches!(pass, Pass::Label) {
            disc_art::disc(
                &mut canvas,
                disc_art::CX,
                disc_art::CY,
                disc_art::R,
                disc_art::spin(self.t),
                level,
            );
        }
        disc_art::stage_glow(&mut canvas, level);
        if !matches!(pass, Pass::Disc) {
            let fade_in = (self.frame * 256 / LABEL_FADE_FRAMES).min(256);
            label(fade_in * level >> 8);
        }
    }
}

/// Whether the GPU has finished everything sent so far (GPUSTAT bits 28 and
/// 26, as `paint::draw_sync` waits for them), without waiting.
fn gpu_idle() -> bool {
    psx_io::gpu::try_wait_dma_ready(0) && psx_io::gpu::try_wait_command_ready(0)
}

/// Show the buffer at VRAM row `y` now.
fn display(y: i16) {
    psx_io::gpu::write_display_control(0x0500_0000 | ((y as u32 & 0x1ff) << 10));
}

/// Aim drawing at buffer row `y`, clipped to the given screen rectangle.
/// Coordinates sent afterwards stay screen-relative through the draw offset.
fn target(y: i16, x0: i16, y0: i16, w: i16, h: i16) {
    let top = (y + y0) as u32 & 0x1ff;
    let bottom = (y + y0 + h - 1) as u32 & 0x1ff;
    send(&[
        0xE300_0000 | (top << 10) | x0 as u32,
        0xE400_0000 | (bottom << 10) | (x0 + w - 1) as u32,
        0xE500_0000 | ((y as u32 & 0x7ff) << 11),
    ]);
}

/// One GP0 packet: wait once for the GPU to take commands, then every word.
fn send(words: &[u32]) {
    // Best effort: a dead GPU must not hang the loader.
    let _ = psx_io::gpu::try_wait_command_ready(1_000_000);
    for &word in words {
        psx_io::gpu::write_command(word);
    }
}

fn rgb((r, g, b): Rgb) -> u32 {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}

fn vertex((x, y): (i32, i32)) -> u32 {
    ((y.clamp(-1000, 1000) as u32) << 16) | (x.clamp(-1000, 1000) as u32 & 0xffff)
}

/// The disc's primitives as raw GP0 packets.
struct Gp0;

impl Canvas for Gp0 {
    fn quad_g(&mut self, v: [(i32, i32); 4], c: [Rgb; 4]) {
        send(&[
            0x3800_0000 | rgb(c[0]),
            vertex(v[0]),
            rgb(c[1]),
            vertex(v[1]),
            rgb(c[2]),
            vertex(v[2]),
            rgb(c[3]),
            vertex(v[3]),
        ]);
    }

    fn tri_g(&mut self, v: [(i32, i32); 3], c: [Rgb; 3]) {
        send(&[
            0x3000_0000 | rgb(c[0]),
            vertex(v[0]),
            rgb(c[1]),
            vertex(v[1]),
            rgb(c[2]),
            vertex(v[2]),
        ]);
    }

    fn tri_g_add(&mut self, v: [(i32, i32); 3], c: [Rgb; 3]) {
        send(&[
            0x3200_0000 | rgb(c[0]),
            vertex(v[0]),
            rgb(c[1]),
            vertex(v[1]),
            rgb(c[2]),
            vertex(v[2]),
        ]);
    }

    fn line_add(&mut self, a: (i32, i32), b: (i32, i32), ca: Rgb, cb: Rgb) {
        send(&[0x5200_0000 | rgb(ca), vertex(a), rgb(cb), vertex(b)]);
    }
}

/// The label, centred under the disc, at `alpha` (0..=256) of its colour. Its
/// glyphs are additive sprites out of the menu atlas, as the menu draws text.
fn label(alpha: u32) {
    if alpha == 0 {
        return;
    }
    let face: &Face = &crate::ui_atlas::MICH12;
    let (r, g, b) = disc_art::scale(LABEL_COLOR, alpha);
    // The atlas is modulated by half the colour: texel 128 is unity.
    let tint = rgb((r >> 1, g >> 1, b >> 1));
    let top = LABEL_MID - face.cap_mid();
    let mut pen = disc_art::CX - face.width(LABEL, 1) / 2;
    for c in LABEL.bytes() {
        let glyph = face.glyph(c);
        if glyph.w > 0 {
            send(&[
                0x6600_0000 | tint,
                vertex((pen + glyph.xoff as i32, top + glyph.yoff as i32)),
                (ATLAS_CLUT << 16) | ((glyph.v as u32) << 8) | glyph.u as u32,
                ((glyph.h as u32) << 16) | glyph.w as u32,
            ]);
        }
        pen += glyph.adv as i32 + 1;
    }
}
