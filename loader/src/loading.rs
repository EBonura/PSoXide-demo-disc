//! Self-contained loading presentation.
//!
//! The launcher cannot remain resident while a guest streams into its load
//! address, so this module redraws the same procedural globe and starfield
//! from the high-RAM loader. Two VRAM buffers keep the animation tear-free
//! without re-enabling interrupts or retaining launcher assets.

use carousel::{Bead, SPHERE_POINTS, TURN};
use psx_math::{cos_q12, sin_q12};

use crate::paint;

const BACK_BUFFER_Y: i16 = 256;
const STARS: u32 = 120;
const BACKGROUND: u32 = 0x0004_001A;
const BAR_W: i16 = 160;
const BAR_H: i16 = 12;
const BAR_X: i16 = (320 - BAR_W) / 2;
const BAR_Y: i16 = 202;
const BAR_INSET: i16 = 2;
const BAR_FILL_W: i16 = BAR_W - BAR_INSET * 2;

const MAX_SEGMENTS: usize = 9;
const MIN_SEGMENTS: usize = 6;

const GLOSS_TOP: (u8, u8, u8) = (255, 66, 44);
const GLOSS_BOTTOM: (u8, u8, u8) = (58, 0, 2);
const SPECULAR: (u8, u8, u8) = (255, 196, 170);

pub struct LoadingScreen {
    yaw: i32,
    pitch: i32,
    travel: i32,
    progress: i16,
    back_y: i16,
    beads: [Bead; SPHERE_POINTS],
}

impl LoadingScreen {
    pub fn new() -> Self {
        Self {
            yaw: 0,
            pitch: TURN / 16,
            travel: 0,
            progress: 0,
            back_y: BACK_BUFFER_Y,
            beads: [Bead::default(); SPHERE_POINTS],
        }
    }

    /// Paint the first complete frame while the display is still off.
    pub fn begin(&mut self) {
        paint::set_draw_buffer(0);
        self.draw_frame();
        paint::draw_sync();
        paint::present_at_next_vblank(0);
        paint::show();
    }

    /// Advance between CD chunks, while the drive is paused. Rendering or
    /// waiting for VBlank during an active stream can leave the sector FIFO
    /// unattended long enough to corrupt the payload on real hardware.
    pub fn update(&mut self, done: u32, total: u32) {
        self.progress = ((done * BAR_FILL_W as u32) / total.max(1)).min(BAR_FILL_W as u32) as i16;
        self.advance();
        paint::set_draw_buffer(self.back_y);
        self.draw_frame();
        paint::draw_sync();
        paint::present_at_next_vblank(self.back_y);
        self.back_y = other_buffer(self.back_y);
    }

    /// Present a full bar for the final frame before the payload checksum and
    /// jump. The bounded loader work following it leaves the completed visual
    /// visible instead of abandoning a nearly full frame.
    pub fn finish(&mut self) {
        self.progress = BAR_FILL_W;
        self.advance();
        paint::set_draw_buffer(self.back_y);
        self.draw_frame();
        paint::draw_sync();
        paint::present_at_next_vblank(self.back_y);
    }

    fn advance(&mut self) {
        // Chunk boundaries are farther apart than launcher frames, so each
        // safe presentation advances enough to read as deliberate rotation.
        self.yaw = (self.yaw + 72) & (TURN - 1);
        self.pitch = (self.pitch + 16) & (TURN - 1);
        self.travel = self.travel.wrapping_add(56);
    }

    fn draw_frame(&mut self) {
        paint::rect(0, 0, 320, 240, BACKGROUND);
        draw_starfield(self.travel);
        draw_sphere(self.yaw, self.pitch, &mut self.beads);

        paint::rect(BAR_X, BAR_Y, BAR_W, BAR_H, paint::TRACK);
        if self.progress > 0 {
            paint::rect(
                BAR_X + BAR_INSET,
                BAR_Y + BAR_INSET,
                self.progress,
                BAR_H - BAR_INSET * 2,
                paint::WHITE,
            );
        }
    }
}

fn other_buffer(y: i16) -> i16 {
    if y == 0 {
        BACK_BUFFER_Y
    } else {
        0
    }
}

fn draw_starfield(travel: i32) {
    for i in 0..STARS {
        let star = carousel::star(i, travel);
        if !star.visible {
            continue;
        }
        let b = star.bright;
        let g = ((b as u16 * 13) / 16) as u8;
        let blue = ((b as u16 * 11) / 16) as u8;
        let rgb = b as u32 | ((g as u32) << 8) | ((blue as u32) << 16);
        paint::rect(star.x, star.y, star.size as i16, star.size as i16, rgb);
    }
}

fn draw_sphere(yaw: i32, pitch: i32, beads: &mut [Bead; SPHERE_POINTS]) {
    let n = carousel::sphere(yaw, pitch, 0, beads);
    carousel::sort_by_depth(&mut beads[..n], |bead| bead.z);
    for bead in &beads[..n] {
        draw_bead(bead);
    }
}

fn draw_bead(bead: &Bead) {
    if bead.r <= 0 {
        return;
    }
    ellipse(
        bead.x,
        bead.y,
        bead.r,
        bead.r,
        scale_rgb(GLOSS_TOP, bead.lit),
        scale_rgb(GLOSS_BOTTOM, bead.lit),
    );
    if bead.r >= 7 {
        ellipse(
            bead.x - bead.r / 3,
            bead.y - bead.r / 3,
            bead.r / 3,
            bead.r / 3,
            scale_rgb(SPECULAR, bead.lit),
            scale_rgb(GLOSS_TOP, bead.lit),
        );
    }
}

fn segments_for(rx: i16, ry: i16) -> usize {
    (rx.max(ry) as usize / 2).clamp(MIN_SEGMENTS, MAX_SEGMENTS)
}

fn ellipse(cx: i16, cy: i16, rx: i16, ry: i16, top: (u8, u8, u8), bottom: (u8, u8, u8)) {
    if rx <= 0 || ry <= 0 {
        return;
    }
    let segments = segments_for(rx, ry);
    let centre = mix(top, bottom, 128);
    let vertex = |segment: usize| -> ((i16, i16), (u8, u8, u8)) {
        let angle = ((segment as i32 * TURN) / segments as i32) as u16;
        let x = cx as i32 + ((rx as i32 * sin_q12(angle)) >> 12);
        let y = cy as i32 - ((ry as i32 * cos_q12(angle)) >> 12);
        let t = ((4096 - cos_q12(angle)) * 255 / 8192) as u8;
        ((x as i16, y as i16), mix(top, bottom, t))
    };

    let (mut previous, mut previous_color) = vertex(0);
    for segment in 1..=segments {
        let (point, color) = vertex(segment);
        paint::tri_gouraud([(cx, cy), previous, point], [centre, previous_color, color]);
        previous = point;
        previous_color = color;
    }
}

fn lerp(a: u8, b: u8, t: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    (a + (((b - a) * t as i32) >> 8)) as u8
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: u8) -> (u8, u8, u8) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

fn scale_rgb((r, g, b): (u8, u8, u8), t: u8) -> (u8, u8, u8) {
    (
        ((r as u32 * t as u32) >> 8) as u8,
        ((g as u32 * t as u32) >> 8) as u8,
        ((b as u32 * t as u32) >> 8) as u8,
    )
}
