//! Attract mode for the Shelf: the shelf tour over the music floor.
//!
//! After the menu has sat idle the text fades and the scene starts moving by
//! itself, like a shop window:
//!
//! * the **shelf tour** glides the camera along the row and, at each case,
//!   lifts it a little, turns it to show its spine and then its back cover (a
//!   one-line blurb and two small screenshots), and turns it home again;
//! * the **music floor** lights the grid floor from the level meter (each cell
//!   shows one band of the spectrum, older rows further away, so the music
//!   seems to roll toward the horizon), swells the light pools under the cases
//!   on every beat, and catches the PSoXide logo's cyan X in a sweep of light.
//!
//! The two are independent ([`Variant`]); the default is both. Any button
//! ends it at once: the caller drops the [`Attract`] and puts the menu on
//! [`Attract::selection`].
//!
//! # Using it
//!
//! ```ignore
//! attract::upload_backs();                       // once, after the covers
//! let mut a = Attract::new(Variant::Both, entries, sel);
//! loop {                                          // every 60 Hz frame
//!     a.update(&bands);                           // the meter's 16 band bytes
//!     a.draw(&mut g, &ctx);
//! }
//! ```
//!
//! `bands` is the frame the level meter would draw right now (the launcher's
//! `spectrum_frame`); pass zeros while no music plays and the floor stays
//! calm. The prototype has no music, so it replays [`recorded_bands`], a
//! 24 second slice of the real analysis of `dnb-a` (`menu-art curve`).
//!
//! Art is baked by `tools/menu-art` (committed here, 85 KB):
//! `menu-art backs assets/shots menu-proto/assets` and
//! `menu-art curve audio/launcher/menu.spectrum 4 30 24 menu-proto/assets/attract_curve.bin`.
//!
//! The module needs, from the rest of the menu: `Gfx` (plus `ui_quad` and
//! `shot_ribbon`), the text faces and glyph atlas, the cover art in
//! `data::ARTS`, `common::CaseGeom` and `shelf::{layout, title_row}`.

use crate::common::{footer, header, CaseGeom, Ctx, HEAD_MID, MARGIN};
use crate::data::{Entry, ARTS};
use crate::ease;
use crate::gfx::*;
use crate::shelf::{self, FLOOR, HORIZON};
use crate::text::{self, Face, Glyph};
use crate::ui_atlas::*;
use psx_gte::math::{Mat3I16, Vec3I16, Vec3I32};
use psx_gte::scene;
use psx_math::{cos_q12, sin_q12};

/// Frames of no input before the menu hands over to the attract mode (20 s).
#[cfg(not(feature = "quick"))]
pub const IDLE_FRAMES: u32 = 20 * 60;
/// Prototype builds that capture the mode do not wait.
#[cfg(feature = "quick")]
pub const IDLE_FRAMES: u32 = 12;

/// Level-meter bands per frame, as `mkdisc analyse` writes them.
pub const BANDS: usize = 16;

/// Which of the two ideas runs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Camera glide and case turns on a calm floor.
    Tour,
    /// A still camera, a swaying case and the floor that follows the music.
    Floor,
    /// Both together.
    Both,
}

impl Variant {
    fn tour(self) -> bool {
        self != Variant::Floor
    }
    fn music(self) -> bool {
        self != Variant::Tour
    }
}

// ---- recorded music level (prototype stand-in) ---------------------------------

/// 24 s of `menu.spectrum` for dnb-a, 30 frames a second, 16 bands a frame.
static CURVE: &[u8] = include_bytes!("../assets/attract_curve.bin");

/// The band frame for display frame `t`: the analysis runs at 30 Hz.
pub fn recorded_bands(t: u32) -> &'static [u8] {
    let frames = (CURVE.len() / BANDS) as u32;
    let f = ((t / 2) % frames) as usize;
    &CURVE[f * BANDS..(f + 1) * BANDS]
}

// ---- back-cover content --------------------------------------------------------

/// Back-cover screenshots per program (slot order of [`ARTS`]): the baked
/// `back_<id>.bin` blobs, a 512 byte CLUT then one or two 64x48 shots.
macro_rules! back {
    ($id:literal) => {
        include_bytes!(concat!("../assets/back_", $id, ".bin")) as &[u8]
    };
}

static BACKS: [&[u8]; 11] = [
    back!("arcade"),
    back!("celeste"),
    back!("nitro"),
    back!("voxide"),
    back!("quake"),
    back!("cortex"),
    back!("psxcel"),
    back!("wipeout"),
    back!("hollow"),
    back!("cstrike"),
    back!("halflife"),
];

/// Bytes of one screenshot's pixels.
const SHOT_BYTES: usize = (SHOT_W * SHOT_H) as usize;

fn shots_of(prog: usize) -> usize {
    (BACKS[prog].len() - 512) / SHOT_BYTES
}

/// Slot of program `prog`'s first screenshot in the shot page.
fn first_shot(prog: usize) -> usize {
    (0..prog).map(shots_of).sum()
}

/// Put every back-cover screenshot and its palette into VRAM (above the
/// framebuffers' neighbours: page (320, 256), palettes at (320, 480+)).
pub fn upload_backs() {
    use psx_vram::{upload_bytes, VramRect};
    for (prog, blob) in BACKS.iter().enumerate() {
        upload_bytes(
            VramRect::new(SHOT_CLUT_X, SHOT_CLUT_Y + prog as u16, 256, 1),
            &blob[..512],
        );
        for k in 0..shots_of(prog) {
            let slot = first_shot(prog) + k;
            let (col, row) = ((slot % 4) as u16, (slot / 4) as u16);
            upload_bytes(
                VramRect::new(
                    SHOT_PAGE.0 + col * (SHOT_W as u16 / 2),
                    SHOT_PAGE.1 + row * SHOT_H as u16,
                    SHOT_W as u16 / 2,
                    SHOT_H as u16,
                ),
                &blob[512 + k * SHOT_BYTES..512 + (k + 1) * SHOT_BYTES],
            );
        }
    }
}

/// The back-cover blurb (English, Italian) of each program, condensed from the
/// disc table's descriptions.
static BLURBS: [(&str, &str); 11] = [
    (
        "Breakout, Space Invaders and Magikarp Pong, with a live CD-audio visualizer.",
        "Breakout, Space Invaders e Magikarp Pong, con visualizzatore CD audio.",
    ),
    (
        "Both Celeste Classic games, rebuilt from PICO-8 as native PlayStation games.",
        "I due Celeste Classic, ricostruiti da PICO-8 come giochi nativi PlayStation.",
    ),
    (
        "A Rocket League clone for the original PlayStation, with 2-player split screen.",
        "Un clone di Rocket League per PlayStation, con schermo diviso per 2 giocatori.",
    ),
    (
        "A Minecraft clone for the original PlayStation: mining, crafting and survival.",
        "Un clone di Minecraft per PlayStation: scavo, crafting e sopravvivenza.",
    ),
    (
        "Quake 1.06 shareware for PS1: all nine maps, the full arsenal and Chthon.",
        "Quake 1.06 shareware per PS1: nove mappe, arsenale completo e Chthon.",
    ),
    (
        "A Souls-like built from scratch for the original PlayStation. Early tech demo.",
        "Un souls-like sviluppato da zero per PlayStation. Primo tech demo.",
    ),
    (
        "A working Excel clone for PlayStation, played with a joypad: formulas, charts, saves.",
        "Un clone di Excel per PlayStation, col joypad: formule, grafici, salvataggi.",
    ),
    (
        "WipEout (1995) rewritten in Rust: tracks, ships, weapons and split screen.",
        "WipEout (1995) riscritto in Rust: piste, navi, armi e schermo diviso.",
    ),
    (
        "An early port of Hollow Knight: King's Pass, Dirtmouth and the Forgotten Crossroads.",
        "Un primo port di Hollow Knight: King's Pass, Dirtmouth e il Crocevia Dimenticato.",
    ),
    (
        "Counter-Strike 1.6 from scratch: bots, buy menu, bomb and hostage rounds, split screen.",
        "Counter-Strike 1.6 da zero: bot, acquisti, bomba, ostaggi e schermo diviso.",
    ),
    (
        "Half-Life from scratch: the campaign is converted, not yet playable end to end.",
        "Half-Life da zero: campagna convertita, non ancora giocabile da capo a fondo.",
    ),
];

/// The name printed down each spine.
static SPINES: [&str; 11] = [
    "ARCADE",
    "CELESTE",
    "NITROXIDE",
    "VOXIDE",
    "QUAKE",
    "CORTEX",
    "PSXCEL",
    "WIPEOUT",
    "HOLLOW KNIGHT",
    "COUNTER-STRIKE",
    "HALF-LIFE",
];

/// The most saturated, bright colour of a cover's palette: the program's accent
/// on its spine.
fn accent_of(prog: usize) -> Rgb {
    let clut = &ARTS[prog].l[..512];
    let mut best = ((0u8, 0u8, 0u8), 0i32);
    for c in clut.chunks_exact(2) {
        let v = u16::from_le_bytes([c[0], c[1]]);
        let (r, g, b) = (
            (v & 31) as i32,
            ((v >> 5) & 31) as i32,
            ((v >> 10) & 31) as i32,
        );
        let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
        let score = (hi - lo) * 4 + hi;
        if score > best.1 {
            best = (((r << 3) as u8, (g << 3) as u8, (b << 3) as u8), score);
        }
    }
    best.0
}

// ---- timeline ----------------------------------------------------------------

/// Frames of a stop (60 Hz). The glide to the case comes first; its length is
/// per stop (a long fly-back when the tour wraps).
const GLIDE: i32 = 64;
const GLIDE_WRAP: i32 = 150;
const FRONT: i32 = 30;
const TURN_SPINE: i32 = 56;
const HOLD_SPINE: i32 = 40;
const TURN_BACK: i32 = 56;
const HOLD_BACK: i32 = 180;
const TURN_HOME: i32 = 84;
/// A quarter turn in Q12.
const QUARTER: i32 = 1024;
/// Scale of the case while it is read, Q8.
const SC_READ: i32 = 330;
/// Thickness of a case at scale 256, in pixels.
const THICK: i32 = 18;

/// What the focused case is doing `st` frames into its stop (after the glide):
/// (turn angle in Q12 turns, zoom 0..=256).
fn turn_pose(st: i32) -> (i32, i32) {
    let b1 = FRONT;
    let b2 = b1 + TURN_SPINE;
    let b3 = b2 + HOLD_SPINE;
    let b4 = b3 + TURN_BACK;
    let b5 = b4 + HOLD_BACK;
    let b6 = b5 + TURN_HOME;
    let io = |a: i32, b: i32| ease::in_out(ease::span(st, a, b));
    let angle = if st < b1 {
        0
    } else if st < b2 {
        -QUARTER * io(b1, b2) / 256
    } else if st < b3 {
        -QUARTER
    } else if st < b4 {
        -QUARTER - QUARTER * io(b3, b4) / 256
    } else if st < b5 {
        -2 * QUARTER
    } else {
        -2 * QUARTER - 2 * QUARTER * io(b5, b6) / 256
    };
    let zoom = if st < b2 {
        io(b1, b2)
    } else if st < b5 {
        256
    } else {
        256 - io(b5, b6)
    };
    (angle, zoom)
}

/// Frames from the end of the glide to the end of the stop.
const STOP_BODY: i32 = FRONT + TURN_SPINE + HOLD_SPINE + TURN_BACK + HOLD_BACK + TURN_HOME;

// ---- music ---------------------------------------------------------------------

/// Rows of floor history.
const HIST: usize = 32;
/// Display frames between one floor row and the next.
const ROW_DELAY: usize = 5;

struct Music {
    prev: [u8; BANDS],
    hist: [[u8; BANDS]; HIST],
    head: usize,
    /// Smoothed loudness, 0..=256.
    level: i32,
    /// Beat envelope: 256 on the beat, decaying.
    beat: i32,
    /// Frames since the last beat (saturating).
    age: i32,
    /// Average onset strength, for the adaptive threshold.
    ema: i32,
    cool: i32,
}

impl Music {
    const fn new() -> Self {
        Music {
            prev: [0; BANDS],
            hist: [[0; BANDS]; HIST],
            head: 0,
            level: 0,
            beat: 0,
            age: 255,
            ema: 100,
            cool: 0,
        }
    }

    /// Fold in this frame's band bytes.
    fn update(&mut self, bands: &[u8]) {
        let mut now = [0u8; BANDS];
        let n = bands.len().min(BANDS);
        now[..n].copy_from_slice(&bands[..n]);
        // Onsets: the rise of the low and low-mid bands since the last analysis
        // frame. The kick and bass line live there; the hats above do not
        // make the floor jump.
        if now != self.prev {
            let flux: i32 = (0..5)
                .map(|i| (now[i] as i32 - self.prev[i] as i32).max(0))
                .sum();
            if self.cool > 0 {
                self.cool -= 1;
            }
            if flux > self.ema * 2 && flux > 60 && self.cool == 0 {
                self.beat = 256;
                self.age = 0;
                self.cool = 8;
            }
            self.ema += (flux - self.ema) >> 3;
            self.prev = now;
        }
        self.beat = (self.beat * 224) >> 8;
        self.age = (self.age + 1).min(255);
        let low: i32 = now[..8].iter().map(|&b| b as i32).sum::<i32>() / 8;
        let target = ((low - 100) * 256 / 110).clamp(0, 256);
        let shift = if target > self.level { 2 } else { 4 };
        self.level += (target - self.level) >> shift;
        self.head = (self.head + 1) % HIST;
        self.hist[self.head] = now;
    }

    /// The band frame `delay` display frames ago.
    fn past(&self, delay: usize) -> &[u8; BANDS] {
        &self.hist[(self.head + HIST - delay % HIST) % HIST]
    }
}

// ---- state ---------------------------------------------------------------------

pub struct Attract {
    variant: Variant,
    /// Frames since the attract mode began.
    t: u32,
    /// Entry indices of the stops (the programs with a cover), in row order.
    stops: [u8; 16],
    n_stops: usize,
    /// Current stop and frames since its glide began.
    stop: usize,
    st: i32,
    glide_len: i32,
    /// Row position at the start of the glide and now, Q8 case units.
    from: i32,
    pos: i32,
    music: Music,
    accent: [Rgb; 11],
}

impl Attract {
    /// Begin at the program nearest entry `first`.
    pub fn new(variant: Variant, entries: &[&Entry], first: usize) -> Self {
        let mut stops = [0u8; 16];
        let mut n = 0;
        let mut stop = 0;
        for (i, e) in entries.iter().enumerate() {
            if e.art.is_some() && n < stops.len() {
                if i <= first {
                    stop = n;
                }
                stops[n] = i as u8;
                n += 1;
            }
        }
        let mut accent = [(0, 0, 0); 11];
        for (prog, a) in accent.iter_mut().enumerate() {
            *a = accent_of(prog);
        }
        let pos = stops[stop] as i32 * 256;
        Attract {
            variant,
            t: 0,
            stops,
            n_stops: n.max(1),
            stop,
            // The first stop starts with the glide already done.
            st: GLIDE,
            glide_len: GLIDE,
            from: pos,
            pos,
            music: Music::new(),
            accent,
        }
    }

    /// The entry the menu should show when the attract mode ends.
    pub fn selection(&self) -> usize {
        self.stops[self.stop] as usize
    }

    /// Advance one display frame (60 Hz). `bands` is this frame's level meter.
    pub fn update(&mut self, bands: &[u8]) {
        self.t += 1;
        if self.variant.music() {
            self.music.update(bands);
        }
        if !self.variant.tour() {
            return;
        }
        self.st += 1;
        if self.st >= self.glide_len + STOP_BODY {
            self.from = self.stops[self.stop] as i32 * 256;
            let next = (self.stop + 1) % self.n_stops;
            self.glide_len = if next == 0 && self.n_stops > 1 {
                GLIDE_WRAP
            } else {
                GLIDE
            };
            self.stop = next;
            self.st = 0;
        }
        let to = self.stops[self.stop] as i32 * 256;
        let g = ease::in_out(ease::span(self.st, 0, self.glide_len));
        self.pos = ease::lerp(self.from, to, g);
    }

    /// The turn the focused case is in: (angle, zoom), or none while gliding.
    fn pose(&self) -> (i32, i32) {
        if !self.variant.tour() || self.st < self.glide_len {
            (0, 0)
        } else {
            turn_pose(self.st - self.glide_len)
        }
    }

    /// Draw the whole frame (the caller has begun it and cleared it).
    pub fn draw(&self, g: &mut Gfx, ctx: &Ctx) {
        let m = &self.music;
        let on = self.variant.music();
        let (turn, zoom) = self.pose();
        let sel = self.selection() as i32;

        // Backdrop and floor: pulsed by the music when it plays.
        let glow = if on {
            150 + (m.level >> 2) + (m.beat >> 3)
        } else {
            150
        };
        g.glow(
            160,
            78,
            glow,
            80,
            scale((5, 22, 28), 256 + (m.beat as u32 >> 2) * on as u32),
        );
        g.glow(
            160,
            HORIZON + 6,
            190,
            40,
            scale(
                (9, 38, 46),
                200 + (m.level as u32 >> 2) + (m.beat as u32 >> 2),
            ),
        );
        self.floor(g, on);

        // The menu's header and footer fade out over the first frames.
        if self.t < 40 {
            header(g, ctx, "DEMO DISC");
            footer(g, ctx);
            let v = (ease::span(self.t as i32, 0, 30) * 255 >> 8) as u8;
            g.quad_g_sub([(0, 0), (320, 0), (0, 24), (320, 24)], [(v, v, v); 4]);
            g.quad_g_sub([(0, 216), (320, 216), (0, 240), (320, 240)], [(v, v, v); 4]);
        }
        self.logo(g, on);

        // Cases, far to near.
        let n = ctx.entries.len() as i32;
        let mut order = [0i32; 16];
        let mut count = 0;
        for i in 0..n.min(16) {
            order[count] = i;
            count += 1;
        }
        let dist = |i: i32| (i * 256 - self.pos).abs();
        for a in 0..count {
            for b in a + 1..count {
                if dist(order[b]) > dist(order[a]) {
                    order.swap(a, b);
                }
            }
        }
        // The row makes room while the focused case is pulled forward.
        let spread = (zoom * 16) >> 8;
        for &i in &order[..count] {
            let d = i * 256 - self.pos;
            if d.abs() > 800 {
                continue;
            }
            let (dx, base_sc, dim) = shelf::layout(d);
            let push = if dx == 0 { 0 } else { spread };
            let dx = if d < 0 { dx - push } else { dx + push };
            let settle = (256 - d.abs() * 3 / 2).max(0);
            let subject = i == sel;
            let z = if subject { zoom } else { 0 };
            let sc = base_sc + ((SC_READ - 256) * z * settle >> 16);
            let lift = (3 * settle) >> 8;
            let lean = if d < 0 { -150 } else { 150 };
            let sway = shelf::sway_yaw(ctx.t) * (256 - z) / 256;
            let yaw = ((lean * (256 - settle)) >> 8) + ((sway * settle) >> 8);
            let mut angle = -yaw * 2;
            if subject {
                angle += turn;
            }
            // Pool of light under each case, swelling on the beat.
            let swell = (m.beat as u32 >> 3) * on as u32;
            let pool = scale(
                (10, 52, 62),
                (settle as u32 * 3 >> 2) + 20 + swell + (m.level as u32 >> 4),
            );
            let rx = ((66 * sc) >> 8) + ((m.beat * settle >> 12) * on as i32);
            g.glow(160 + dx, FLOOR + 3, rx, 6 + (m.beat >> 7) * on as i32, pool);
            if settle > 120 {
                let ring = scale((14, 60, 70), settle as u32);
                g.ellipse_ring(160 + dx, FLOOR + 3, rx + 10, 7, ring);
                g.ellipse_ring(160 + dx, FLOOR + 3, rx + 26, 11, scale(ring, 120));
                if on && subject && m.age < 40 {
                    // A ripple leaves the focused case on every beat.
                    let r = rx + 30 + m.age * 4;
                    g.ellipse_ring(
                        160 + dx,
                        FLOOR + 3,
                        r,
                        r / 5 + 4,
                        scale(ring, (256 - m.age * 6) as u32),
                    );
                }
            }
            let cd = CaseDraw {
                cx: 160 + dx,
                bottom: FLOOR - lift,
                sc,
                dim,
                focus: settle,
                angle,
            };
            draw_case(
                g,
                &cd,
                ctx.entries[i as usize],
                self.accent_of_entry(ctx.entries[i as usize]),
                ctx,
            );
        }

        self.chrome(g, ctx);
    }

    fn accent_of_entry(&self, e: &Entry) -> Rgb {
        e.art.map_or((20, 120, 140), |p| self.accent[p])
    }

    /// Title row, progress dots and the wake-up hint.
    fn chrome(&self, g: &mut Gfx, ctx: &Ctx) {
        let (_, zoom) = self.pose();
        let e = ctx.entries[self.selection()];
        // The title stands while the focused case is in place and blinks out
        // for the glide.
        let near = if self.variant.tour() && self.st < self.glide_len {
            let c = (self.glide_len / 2) as i32;
            let away = (self.st as i32 - c).abs();
            ease::span(away, 0, c / 2) as u32
        } else {
            256
        };
        let fade_in = ease::span(self.t as i32, 0, 40) as u32;
        let a = (near * (256 - ((256 - fade_in) >> 1)) >> 8).min(256);
        if a > 8 {
            shelf::title_row(g, e, 141, a);
        }
        // One dot per program; the focused one is lit.
        let n = if self.variant.tour() {
            self.n_stops as i32
        } else {
            0
        };
        let x0 = 160 - (n - 1) * 5;
        for k in 0..n {
            let on = k as usize == self.stop;
            let c = if on { scale(FOCUS, 256) } else { (30, 52, 62) };
            let wide = if on { 5 } else { 3 };
            g.rect(x0 + k * 10 - wide / 2, 160, wide, 3, c);
        }
        let _ = zoom;
        // The hint at the footer's centre, breathing slowly.
        if self.t >= 36 {
            let hint = if ctx.it {
                "PREMI UN TASTO"
            } else {
                "PRESS ANY BUTTON"
            };
            let breath = (sin_q12((self.t * 24) as u16 & 4095) + 4096) as u32 >> 6; // 0..=127
            let c = scale(DIM, 90 + breath);
            text::draw_centered_mid(g, &JOST9, 160, 228, hint, c, 1);
        }
    }

    /// The grid floor. With music each cell lights from one band of the
    /// spectrum, the rows further away showing older frames.
    fn floor(&self, g: &mut Gfx, music: bool) {
        let m = &self.music;
        let (y0, y1) = (HORIZON, 212);
        let offs = [0, 3, 8, 15, 26, 42, 66, 100];
        let row_y = |r: usize| y0 + (offs[r] * (y1 - y0)) / 100;
        // x of vertical line `k` at row `y`.
        let col_x = |k: i32, y: i32| 160 + k * (7 + (33 * (y - y0)) / (y1 - y0));
        if music {
            // 7 rows (horizon first), 16 columns.
            for r in 0..7usize {
                let (ya, yb) = (row_y(r), row_y(r + 1));
                // Newest music in the row nearest the viewer.
                let delay = (6 - r) * ROW_DELAY;
                let past = m.past(delay);
                let far = 70 + 186 * r as i32 / 6; // dimmer toward the horizon
                for c in 0..BANDS {
                    let k = c as i32 - 8;
                    let v = ((past[c] as i32 - 90).max(0) * 256 / 165).min(256);
                    let lit = (v * v >> 8) * far >> 8;
                    if lit < 10 {
                        continue;
                    }
                    let col = scale((10, 62, 78), lit as u32);
                    let q = [
                        (col_x(k, ya), ya),
                        (col_x(k + 1, ya), ya),
                        (col_x(k, yb), yb),
                        (col_x(k + 1, yb), yb),
                    ];
                    g.quad_g_add(q, [col; 4]);
                }
            }
        }
        if music && m.beat > 20 {
            // The whole floor takes a breath of light on the beat.
            let w = scale((6, 30, 36), m.beat as u32);
            let q = [(0, y0), (320, y0), (0, y1), (320, y1)];
            g.quad_g_add(q, [(0, 0, 0), (0, 0, 0), w, w]);
        }
        // Grid lines on top; the beat runs through them as a ripple.
        for (i, o) in offs.iter().enumerate() {
            let y = y0 + (*o * (y1 - y0)) / 100;
            let a = 6 + i as u32 * 3;
            let boost = if music {
                // The ripple front moves from the horizon toward the viewer.
                let front = m.age * 2 / 3;
                let gap = (i as i32 * 5 - front).abs();
                ((256 - gap * 40).max(0) as u32 * (m.beat as u32 + 64).min(256) >> 8) >> 3
            } else {
                0
            };
            let a = a + boost;
            g.rect_add(0, y, 320, 1, ((a / 2) as u8, a as u8, (a + 2) as u8));
        }
        let glow = if music { (m.level >> 5) as u8 } else { 0 };
        for k in -9..=9i32 {
            let top = (160 + k * 7, y0);
            let bot = (160 + k * 40, y1);
            g.line_add(
                top,
                bot,
                (2, 6 + glow, 8 + glow),
                (11, 26 + glow * 2, 30 + glow * 2),
            );
        }
    }

    /// The PSoXide wordmark with its cyan X, which catches a sweep of light on
    /// every beat.
    fn logo(&self, g: &mut Gfx, music: bool) {
        let m = &self.music;
        let cy = HEAD_MID;
        let y = cy - MICH12.cap_mid();
        let x = MARGIN;
        let w1 = MICH12.width("PSo", 0);
        let wx = MICH12.width("PSoX", 0) - w1;
        let xcx = x + w1 + wx / 2;
        if music && m.age < 30 {
            g.glow(
                xcx,
                cy,
                18,
                10,
                scale((16, 80, 98), (m.beat as u32 + 40).min(256)),
            );
        }
        text::draw(g, &MICH12, x, y, "PSo", TEXT, 0);
        text::draw(g, &MICH12, x + w1, y, "X", ACCENT, 0);
        text::draw(g, &MICH12, x + w1 + wx, y, "ide", TEXT, 0);
        if !music || m.age >= 26 {
            return;
        }
        // The sweep: a soft vertical band moving left to right over the word.
        let total = MICH12.width("PSoXide", 0);
        let pos = x - 10 + (total + 20) * m.age / 26;
        let mut pen = x;
        for (word, is_x) in [("PSo", false), ("X", true), ("ide", false)] {
            for c in word.bytes() {
                let gl = MICH12.glyph(c);
                lit_glyph(g, gl, pen, y, pos, if is_x { 256 } else { 70 });
                pen += gl.adv as i32;
            }
        }
        // A flare through the X as the band crosses it.
        let near = 256 - ((pos - xcx).abs() * 24).min(256);
        if near > 20 {
            let c = scale((90, 230, 255), near as u32);
            g.line_add((xcx - 22, cy), (xcx + 22, cy), (0, 0, 0), c);
            g.line_add((xcx - 22, cy), (xcx + 22, cy), c, (0, 0, 0));
        }
    }
}

/// Add light to one glyph where the sweep at `pos` passes: columns two pixels
/// wide, brightest at the band's centre, `gain` of 256 for full strength.
fn lit_glyph(g: &mut Gfx, gl: &Glyph, pen: i32, y: i32, pos: i32, gain: i32) {
    if gl.w == 0 {
        return;
    }
    g.ui_mode();
    let mut off = 0i32;
    while off < gl.w as i32 {
        let w = (gl.w as i32 - off).min(2);
        let x = pen + gl.xoff as i32 + off;
        let lum = (256 - (x + 1 - pos).abs() * 22).max(0) * gain >> 8;
        if lum > 12 {
            g.ui_sprite(
                x,
                y + gl.yoff as i32,
                w as u16,
                gl.h as u16,
                (gl.u + off as u8, gl.v),
                scale((160, 245, 255), lum as u32),
            );
        }
        off += 2;
    }
}

// ---- one case as a box ------------------------------------------------------------

/// Where and how to draw one case.
struct CaseDraw {
    cx: i32,
    bottom: i32,
    /// Scale, Q8.
    sc: i32,
    /// Brightness, 0..=256.
    dim: i32,
    /// How strongly the focus brackets and ring show, 0..=256.
    focus: i32,
    /// Turn about the vertical axis in Q12 turns; 0 faces the camera and
    /// negative angles bring the spine (the left edge) forward.
    angle: i32,
}

/// The GTE set up for one case: every point of the box goes through here.
struct Pose {
    mx: i32,
    my: i32,
    flat: bool,
}

impl Pose {
    /// Aim the GTE at a case of width `w` centred on `(mx, my)`, thick `ht` each way, turned by `angle`. The camera is one and a half case
    /// widths from the front face, which keeps the scale 1:1 there.
    fn aim(mx: i32, my: i32, w: i32, ht: i32, angle: i32) -> Self {
        let flat = angle & 4095 == 0;
        if !flat {
            // The shelf's close camera makes a leaning case look solid, but it
            // would blow the near edge of a case turned side-on out of
            // proportion, so the camera backs away as the turn grows.
            let a = angle as u16;
            let turn = (angle & 4095).min(4096 - (angle & 4095));
            let far = ((turn - 400) * 256 / 600).clamp(0, 256);
            let dist = w * 3 / 2 + w * 7 / 2 * far / 256;
            let (s, c) = (sin_q12(a), cos_q12(a));
            // +z points away from the camera; a positive angle brings the
            // right edge closer, a negative one the left edge.
            scene::load_rotation(&Mat3I16 {
                m: [
                    [c as i16, 0, s as i16],
                    [0, 4096, 0],
                    [-s as i16, 0, c as i16],
                ],
            });
            scene::load_translation(Vec3I32 {
                x: 0,
                y: 0,
                z: dist + ht,
            });
            scene::set_projection_plane(dist as u16);
            scene::set_screen_offset(mx << 16, my << 16);
        }
        Pose { mx, my, flat }
    }

    /// Screen position of the box point `(x, y, z)` measured from its centre.
    fn at(&self, x: i32, y: i32, z: i32) -> (i32, i32) {
        if self.flat {
            return (self.mx + x, self.my + y);
        }
        let p = scene::project_vertex(Vec3I16 {
            x: x as i16,
            y: y as i16,
            z: z as i16,
        });
        (p.sx as i32, p.sy as i32)
    }
}

/// Is the quad (top-left, top-right, bottom-left, ...) seen from its outside?
fn facing(q: &[(i32, i32); 4]) -> bool {
    (q[1].0 - q[0].0) * (q[2].1 - q[0].1) - (q[1].1 - q[0].1) * (q[2].0 - q[0].0) > 0
}

fn draw_case(g: &mut Gfx, cd: &CaseDraw, e: &Entry, accent: Rgb, ctx: &Ctx) {
    let geom = CaseGeom::new(cd.cx, cd.bottom, cd.sc);
    let (hw, hh) = (geom.w / 2, geom.h / 2);
    let ht = ((THICK * cd.sc) >> 9).max(2);
    let (mx, my) = (geom.x0 + hw, geom.y0 + hh);
    let pose = Pose::aim(mx, my, geom.w, ht, cd.angle);
    let dim = cd.dim as u32;

    // The faces, each as its screen quad from its own top-left.
    let front = [
        pose.at(-hw, -hh, -ht),
        pose.at(hw, -hh, -ht),
        pose.at(-hw, hh, -ht),
        pose.at(hw, hh, -ht),
    ];
    if !pose.flat {
        let spine = [
            pose.at(-hw, -hh, ht),
            pose.at(-hw, -hh, -ht),
            pose.at(-hw, hh, ht),
            pose.at(-hw, hh, -ht),
        ];
        if facing(&spine) {
            draw_spine(g, &pose, spine, (hw, hh, ht), e, accent, dim);
        }
        let side = [
            pose.at(hw, -hh, -ht),
            pose.at(hw, -hh, ht),
            pose.at(hw, hh, -ht),
            pose.at(hw, hh, ht),
        ];
        if facing(&side) {
            let c = scale(accent, 60 * dim >> 8);
            let d = scale(accent, 24 * dim >> 8);
            g.quad_g(side, [c, c, d, d]);
        }
        let back = [
            pose.at(hw, -hh, ht),
            pose.at(-hw, -hh, ht),
            pose.at(hw, hh, ht),
            pose.at(-hw, hh, ht),
        ];
        if facing(&back) {
            draw_back(g, &pose, back, &geom, ht, e, cd.angle & 4095 == 2048, ctx);
        }
    }
    if facing(&front) {
        draw_front(g, &pose, front, cd, &geom, ht, e);
    }
}

/// The closed case: plastic, cover art, gloss, frame and brackets.
fn draw_front(
    g: &mut Gfx,
    pose: &Pose,
    q: [(i32, i32); 4],
    cd: &CaseDraw,
    c: &CaseGeom,
    ht: i32,
    e: &Entry,
) {
    let (mx, my) = (c.x0 + c.w / 2, c.y0 + c.h / 2);
    let at = |x: i32, y: i32| pose.at(x - mx, y - my, -ht);
    let dim = cd.dim as u32;
    let bodyc = scale((9, 20, 27), dim);
    let foot = scale((4, 11, 15), dim);
    g.quad_g(q, [bodyc, bodyc, foot, foot]);
    g.line_g(
        at(c.x0 + 1, c.y0 + 1),
        at(c.x0 + 1, c.y0 + c.h - 2),
        scale((34, 64, 74), dim),
        scale((12, 24, 30), dim),
    );
    let tint = {
        let v = (128 * cd.dim) >> 8;
        (v as u8, v as u8, v as u8)
    };
    let size = if cd.sc > 220 {
        Size::L
    } else if cd.sc > 150 {
        Size::M
    } else {
        Size::S
    };
    if let Some(i) = e.art {
        if pose.flat && c.aw == L_W {
            g.art(Size::L, i, c.ax, c.ay, tint);
        } else {
            let n = if pose.flat {
                1
            } else {
                (c.aw / 16).clamp(2, 6) as usize
            };
            let mut tops = [(0, 0); 7];
            let mut bots = [(0, 0); 7];
            for k in 0..=n {
                let x = c.ax + c.aw * k as i32 / n as i32;
                tops[k] = at(x, c.ay);
                bots[k] = at(x, c.ay + c.ah);
            }
            g.art_ribbon(size, i, &tops[..=n], &bots[..=n], tint);
        }
    }
    let gl = scale((18, 30, 34), dim);
    g.quad_g_add(
        [
            q[0],
            at(c.x0 + c.w * 3 / 5, c.y0),
            at(c.x0, c.y0 + c.h / 2),
            at(c.x0 + c.w / 4, c.y0 + c.h / 2),
        ],
        [gl, (0, 0, 0), (0, 0, 0), (0, 0, 0)],
    );
    let ring = scale(mix(RING, FOCUS, cd.focus as u32), dim);
    let (x1, y1) = (c.x0 + c.w - 1, c.y0 + c.h - 1);
    let (a, b, d, f) = (at(c.x0, c.y0), at(x1, c.y0), at(c.x0, y1), at(x1, y1));
    g.line(a, b, ring);
    g.line(d, f, ring);
    g.line(a, d, ring);
    g.line(b, f, ring);
    if cd.focus > 100 {
        let col = scale(FOCUS, dim);
        let (bx0, by0, bx1, by1) = (c.x0 - 2, c.y0 - 2, c.x0 + c.w + 1, c.y0 + c.h + 1);
        for (cx, cy, sx, sy) in [
            (bx0, by0, 1, 1),
            (bx1, by0, -1, 1),
            (bx0, by1, 1, -1),
            (bx1, by1, -1, -1),
        ] {
            let corner = at(cx, cy);
            g.line(corner, at(cx + sx * 6, cy), col);
            g.line(corner, at(cx, cy + sy * 6), col);
        }
        let tint = scale((3, 14, 17), dim);
        g.quad_g_add(q, [tint; 4]);
    }
}

/// The spine: the program's accent as a dark gradient, its name stacked in
/// capitals down the middle.
fn draw_spine(
    g: &mut Gfx,
    pose: &Pose,
    q: [(i32, i32); 4],
    (hw, hh, ht): (i32, i32, i32),
    e: &Entry,
    accent: Rgb,
    dim: u32,
) {
    let top = scale(accent, 120 * dim >> 8);
    let bot = scale(accent, 44 * dim >> 8);
    g.quad_g(q, [top, top, bot, bot]);
    // Bright rails along both long edges, as a case's plastic catches light.
    let rail = scale(mix(accent, TEXT, 90), dim);
    g.line(q[0], q[2], rail);
    g.line(q[1], q[3], scale(rail, 140));
    let Some(prog) = e.art else { return };
    let name = SPINES[prog].as_bytes();
    // Letter pitch: two units per letter, one per space.
    let units: i32 = name.iter().map(|&c| if c == b' ' { 1 } else { 2 }).sum();
    let unit = ((2 * hh - 12) / units).min(5);
    let gh = MICH9.glyph(b'H').h as i32;
    // Squeeze the letters uniformly when the name is long.
    let sq = ((2 * unit - 1) * 256 / gh).min(256);
    let mut v = (2 * hh - units * unit) / 2;
    let col = scale(TEXT, dim);
    for &c in name {
        if c == b' ' {
            v += unit;
            continue;
        }
        let gl = MICH9.glyph(c);
        if gl.w > 0 {
            let (gw, gh) = ((gl.w as i32 * sq) >> 8, (gl.h as i32 * sq) >> 8);
            let (gx, gy) = (ht - gw / 2, v + ((gl.yoff as i32 * sq) >> 8));
            // Spine coordinates run from the back edge (left on screen) to the
            // front edge, and from the top down.
            let p = |u: i32, w: i32| pose.at(-hw, -hh + w, ht - u);
            g.ui_quad(
                [
                    p(gx, gy),
                    p(gx + gw, gy),
                    p(gx, gy + gh),
                    p(gx + gw, gy + gh),
                ],
                (
                    gl.u,
                    gl.v,
                    (gl.u as i32 + gl.w as i32).min(255) as u8,
                    (gl.v as i32 + gl.h as i32).min(255) as u8,
                ),
                col,
            );
        }
        v += 2 * unit;
    }
}

/// The back cover: a dark panel with the screenshots on top and the blurb
/// under them. `flat` is the moment it faces the camera squarely, when the text
/// is drawn as plain sprites so it is as crisp as the menu's own.
fn draw_back(
    g: &mut Gfx,
    pose: &Pose,
    q: [(i32, i32); 4],
    c: &CaseGeom,
    ht: i32,
    e: &Entry,
    flat: bool,
    ctx: &Ctx,
) {
    let (mx, my) = (c.x0 + c.w / 2, c.y0 + c.h / 2);
    let (c0, c1) = ((9, 20, 27), (4, 11, 15));
    g.quad_g(q, [c0, c0, c1, c1]);
    // Viewer coordinates `(bx, by)` are screen positions when the back faces
    // the camera; the case is turned about its centre, so they map to the
    // mirrored model x.
    let at = |bx: i32, by: i32| pose.at(mx - bx, by - my, ht);
    let (x1, y1) = (c.x0 + c.w - 1, c.y0 + c.h - 1);
    let ring = RING;
    let (a, b, d, f) = (at(c.x0, c.y0), at(x1, c.y0), at(c.x0, y1), at(x1, y1));
    g.line(a, b, ring);
    g.line(d, f, ring);
    g.line(a, d, ring);
    g.line(b, f, ring);
    let Some(prog) = e.art else { return };

    // Screenshots.
    let shots = shots_of(prog);
    let pad = 5;
    let inner = c.w - 2 * pad;
    let (dw, dh, gap) = if shots == 2 {
        ((inner - 4) / 2, (inner - 4) / 2 * SHOT_H / SHOT_W, 4)
    } else {
        (inner * 2 / 3, inner * 2 / 3 * SHOT_H / SHOT_W, 0)
    };
    let sy = c.y0 + pad;
    let total = shots as i32 * dw + (shots as i32 - 1) * gap;
    let first = first_shot(prog);
    for k in 0..shots {
        let sx = c.x0 + (c.w - total) / 2 + k as i32 * (dw + gap);
        let n = if flat { 1 } else { 5 };
        let mut tops = [(0, 0); 6];
        let mut bots = [(0, 0); 6];
        for j in 0..=n {
            let x = sx + dw * j as i32 / n as i32;
            tops[j] = at(x, sy);
            bots[j] = at(x, sy + dh);
        }
        g.shot_ribbon(prog, first + k, &tops[..=n], &bots[..=n], (128, 128, 128));
        let (e0, e1, e2, e3) = (
            at(sx, sy),
            at(sx + dw, sy),
            at(sx, sy + dh),
            at(sx + dw, sy + dh),
        );
        let edge = (40, 70, 82);
        g.line(e0, e1, edge);
        g.line(e2, e3, edge);
        g.line(e0, e2, edge);
        g.line(e1, e3, edge);
    }

    // Blurb.
    let blurb = if ctx.it {
        BLURBS[prog].1
    } else {
        BLURBS[prog].0
    };
    let face = &JOST9;
    let mut lines = [""; 5];
    let n = wrap(face, blurb, inner, &mut lines);
    let ty = sy + dh + 7;
    let col = scale(MUTED, 256);
    for (k, line) in lines[..n].iter().enumerate() {
        let ly = ty + k as i32 * 10;
        if flat {
            text::draw(g, face, c.x0 + pad, ly, line, col, 0);
        } else {
            let mut pen = c.x0 + pad;
            for ch in line.bytes() {
                let gl = face.glyph(ch);
                if gl.w > 0 {
                    let (gx, gy) = (pen + gl.xoff as i32, ly + gl.yoff as i32);
                    let (w, h) = (gl.w as i32, gl.h as i32);
                    g.ui_quad(
                        [
                            at(gx, gy),
                            at(gx + w, gy),
                            at(gx, gy + h),
                            at(gx + w, gy + h),
                        ],
                        (
                            gl.u,
                            gl.v,
                            (gl.u as i32 + w).min(255) as u8,
                            (gl.v as i32 + h).min(255) as u8,
                        ),
                        col,
                    );
                }
                pen += gl.adv as i32;
            }
        }
    }
}

/// Greedy word wrap of `s` into at most `out.len()` lines of `max_w` pixels.
fn wrap<'a>(face: &Face, s: &'a str, max_w: i32, out: &mut [&'a str]) -> usize {
    let space = face.glyph(b' ').adv as i32;
    let mut rest = s;
    let mut n = 0;
    while !rest.is_empty() && n < out.len() {
        let mut end = 0;
        let mut at = 0;
        let mut used = 0;
        loop {
            let next = rest[at..].find(' ').map_or(rest.len(), |i| at + i);
            let ww = face.width(&rest[at..next], 0);
            let cand = if end == 0 { ww } else { used + space + ww };
            if cand > max_w && end > 0 {
                break;
            }
            used = cand;
            end = next;
            if next >= rest.len() {
                break;
            }
            at = next + 1;
        }
        out[n] = &rest[..end];
        n += 1;
        rest = rest[end..].trim_start();
    }
    n
}
