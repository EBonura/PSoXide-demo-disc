//! Immediate-mode drawing for the menu prototype: gradients, additive glow,
//! brackets, textured art, and the baked UI atlas.
//!
//! Every primitive here is at most 12 GP0 words (the textured Gouraud quad),
//! so the same calls could be linked into an ordering table without a node
//! over the 16-word limit.

use psx_gpu::display::{DisplayConfig, Resolution, VideoMode};
use psx_gpu::framebuf::FrameBuffer;
use psx_gpu::material::{BlendMode, TextureMaterial};
use psx_gpu::prim::{
    FillRect, LineGouraud, QuadFlat, QuadGouraud, QuadTexturedMaterial, Sprite, TriGouraud,
};
use psx_gpu::Gpu;
use psx_io::periph::GpuDma;
use psx_math::{cos_q12, sin_q12};
use psx_vram::{Clut, TexDepth, Tpage, VramRect};

pub type Rgb = (u8, u8, u8);

// ---- palette (the home shell's tokens, on a 15-bit display) ---------------

pub const TEXT: Rgb = (244, 248, 251);
pub const TEXT2: Rgb = (196, 211, 222);
pub const MUTED: Rgb = (157, 176, 190);
pub const DIM: Rgb = (129, 150, 166);
pub const RING: Rgb = (108, 127, 143);
pub const ACCENT: Rgb = (34, 200, 229);
pub const FOCUS: Rgb = (128, 239, 255);

pub fn scale(c: Rgb, num: u32) -> Rgb {
    // num: 0..=256
    (
        ((c.0 as u32 * num) >> 8) as u8,
        ((c.1 as u32 * num) >> 8) as u8,
        ((c.2 as u32 * num) >> 8) as u8,
    )
}

pub fn mix(a: Rgb, b: Rgb, t: u32) -> Rgb {
    // t: 0..=256
    let f = |x: u8, y: u8| (((x as u32) * (256 - t) + (y as u32) * t) >> 8) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

// ---- VRAM layout (halfword coordinates) -----------------------------------

/// Large cover pages: 104x78, 6 per page (2 across, 3 down).
pub const L_PAGES: [(u16, u16); 2] = [(320, 0), (448, 0)];
/// Medium covers: 68x51, 15 per page (3 across, 5 down).
pub const M_PAGE: (u16, u16) = (576, 0);
/// Small covers: 52x39, 24 per page (4 across, 6 down).
pub const S_PAGE: (u16, u16) = (704, 0);
/// The UI atlas, 4bpp, 256x256 texels.
pub const UI_PAGE: (u16, u16) = (832, 0);
/// One 256-entry CLUT row per game, below the framebuffers.
pub const ART_CLUT_X: u16 = 0;
pub const ART_CLUT_Y: u16 = 480;
/// The UI atlas ramp.
pub const UI_CLUT: Clut = Clut::new(256, 480);

pub const L_W: i32 = 104;
pub const L_H: i32 = 78;
pub const M_W: i32 = 68;
pub const M_H: i32 = 51;
pub const S_W: i32 = 52;
pub const S_H: i32 = 39;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    L,
    M,
    S,
}

impl Size {
    pub fn dims(self) -> (i32, i32) {
        match self {
            Size::L => (L_W, L_H),
            Size::M => (M_W, M_H),
            Size::S => (S_W, S_H),
        }
    }

    /// (tpage x, tpage y, u0, v0) of cover `i`.
    fn slot(self, i: usize) -> (u16, u16, i32, i32) {
        match self {
            Size::L => {
                let (px, py) = L_PAGES[i / 6];
                let (col, row) = ((i % 2) as i32, ((i / 2) % 3) as i32);
                (px, py, col * L_W, row * L_H)
            }
            Size::M => {
                let (col, row) = ((i % 3) as i32, (i / 3) as i32);
                (M_PAGE.0, M_PAGE.1, col * M_W, row * M_H)
            }
            Size::S => {
                let (col, row) = ((i % 4) as i32, (i / 4) as i32);
                (S_PAGE.0, S_PAGE.1, col * S_W, row * S_H)
            }
        }
    }
}

pub struct Gfx {
    gpu: Gpu,
    pub fb: FrameBuffer,
    key: u32,
    /// Primitives drawn this frame, for the cost table.
    pub prims: u32,
    ui_sub: bool,
}

#[derive(Clone, Copy)]
pub struct CoverArt {
    pub l: &'static [u8],
    pub m: &'static [u8],
    pub s: &'static [u8],
}

fn clip(v: i32) -> i16 {
    v.clamp(-1000, 1000) as i16
}

fn p(v: (i32, i32)) -> (i16, i16) {
    (clip(v.0), clip(v.1))
}

impl Gfx {
    pub fn new() -> Self {
        // SAFETY: the prototype is the only owner of the GPU DMA channel.
        let dma = unsafe { GpuDma::steal() };
        let gpu = Gpu::new(
            dma,
            DisplayConfig::new(VideoMode::Ntsc, Resolution::R320X240),
        );
        let mut g = Gfx {
            gpu,
            fb: FrameBuffer::new(320, 240),
            key: u32::MAX,
            prims: 0,
            ui_sub: false,
        };
        g.gpu.set_draw_area((0, 0), (319, 239));
        g.gpu.set_draw_offset((0, 0));
        g
    }

    /// Select blend equation and texture page (dither on, drawing allowed).
    fn mode(&mut self, tpage_word: u16, blend: BlendMode) {
        let key = (tpage_word as u32) << 8 | blend as u32;
        if key != self.key {
            self.key = key;
            self.gpu.set_draw_mode(
                TextureMaterial::blended(0, tpage_word, (128, 128, 128), blend).with_dither(true),
            );
        }
    }

    fn cur_page(&self) -> u16 {
        (self.key >> 8) as u16
    }

    pub fn blend(&mut self, blend: BlendMode) {
        let page = if self.key == u32::MAX {
            0
        } else {
            self.cur_page()
        };
        self.mode(page, blend);
    }

    // ---- uploads ---------------------------------------------------------

    pub fn upload_ui(&mut self, tex: &[u8]) {
        psx_vram::upload_bytes(VramRect::new(UI_PAGE.0, UI_PAGE.1, 64, 256), tex);
        // Texel k is grey k/15 with the semi-transparency bit, so the sprite
        // blends additively; texel 0 stays transparent.
        let mut clut = [0u8; 32];
        for k in 1..16u16 {
            let v = (k * 31 + 7) / 15;
            let c = 0x8000 | v | (v << 5) | (v << 10);
            clut[k as usize * 2..k as usize * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        psx_vram::upload_bytes(VramRect::new(UI_CLUT.x(), UI_CLUT.y(), 16, 1), &clut);
    }

    pub fn upload_art(&mut self, i: usize, art: &CoverArt) {
        let clut_y = ART_CLUT_Y + i as u16;
        psx_vram::upload_bytes(VramRect::new(ART_CLUT_X, clut_y, 256, 1), &art.l[..512]);
        for (size, blob) in [(Size::L, art.l), (Size::M, art.m), (Size::S, art.s)] {
            let (px, py, u0, v0) = size.slot(i);
            let (w, h) = size.dims();
            psx_vram::upload_bytes(
                VramRect::new(
                    px + (u0 / 2) as u16,
                    py + v0 as u16,
                    (w / 2) as u16,
                    h as u16,
                ),
                &blob[512..],
            );
        }
    }

    // ---- frame -----------------------------------------------------------

    pub fn begin(&mut self) {
        self.prims = 0;
        self.key = u32::MAX;
        self.blend(BlendMode::Opaque);
    }

    pub fn end(&mut self) {
        self.gpu.wait_idle();
        psx_rt::interrupts::wait_vblank();
        self.fb.swap();
    }

    pub fn clear(&mut self, c: Rgb) {
        self.gpu.draw(&FillRect::new(
            (0, self.fb.buffer_y(self.fb.drawing)),
            (320, 240),
            c,
        ));
    }

    // ---- flat and gouraud ------------------------------------------------

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.prims += 1;
        self.gpu
            .draw(&QuadFlat::rect((clip(x), clip(y)), (w as u16, h as u16), c));
    }

    pub fn rect_add(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.blend(BlendMode::Add);
        self.prims += 1;
        self.gpu
            .draw(&QuadFlat::rect((clip(x), clip(y)), (w as u16, h as u16), c).translucent());
    }

    pub fn quad_g(&mut self, v: [(i32, i32); 4], c: [Rgb; 4]) {
        self.prims += 1;
        self.gpu.draw(&QuadGouraud::new(v.map(p), c));
    }

    pub fn quad_g_add(&mut self, v: [(i32, i32); 4], c: [Rgb; 4]) {
        self.blend(BlendMode::Add);
        self.prims += 1;
        self.gpu.draw(&QuadGouraud::new(v.map(p), c).translucent());
    }

    pub fn quad_g_sub(&mut self, v: [(i32, i32); 4], c: [Rgb; 4]) {
        self.blend(BlendMode::Subtract);
        self.prims += 1;
        self.gpu.draw(&QuadGouraud::new(v.map(p), c).translucent());
    }

    pub fn tri_g(&mut self, v: [(i32, i32); 3], c: [Rgb; 3]) {
        self.prims += 1;
        self.gpu.draw(&TriGouraud::new(v.map(p), c));
    }

    pub fn tri_g_add(&mut self, v: [(i32, i32); 3], c: [Rgb; 3]) {
        self.blend(BlendMode::Add);
        self.prims += 1;
        self.gpu.draw(&TriGouraud::new(v.map(p), c).translucent());
    }

    pub fn line_add(&mut self, a: (i32, i32), b: (i32, i32), ca: Rgb, cb: Rgb) {
        self.blend(BlendMode::Add);
        self.prims += 1;
        self.gpu
            .draw(&LineGouraud::new(p(a), ca, p(b), cb).translucent());
    }

    /// A soft glow: an additive fan from `c` at the centre to black at the rim.
    pub fn glow(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, c: Rgb) {
        const N: u16 = 16;
        self.blend(BlendMode::Add);
        let ring = |k: u16| {
            let a = k * (4096 / N);
            (
                cx + ((rx * sin_q12(a)) >> 12),
                cy - ((ry * cos_q12(a)) >> 12),
            )
        };
        let mut prev = ring(0);
        for k in 1..=N {
            let cur = ring(k % N);
            self.tri_g_add([(cx, cy), prev, cur], [c, (0, 0, 0), (0, 0, 0)]);
            prev = cur;
        }
    }

    /// An ellipse ring (outline) as short additive quads, `t` thick.
    pub fn ellipse_ring(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, c: Rgb) {
        const N: u16 = 24;
        let pt = |k: u16| {
            let a = k * (4096 / N);
            (
                cx + ((rx * sin_q12(a)) >> 12),
                cy - ((ry * cos_q12(a)) >> 12),
            )
        };
        let mut prev = pt(0);
        for k in 1..=N {
            let cur = pt(k % N);
            self.line_add(prev, cur, c, c);
            prev = cur;
        }
    }

    /// Four corner brackets around a rectangle, `len` long and 1 thick,
    /// plus an optional faint tint inside.
    pub fn brackets(&mut self, x: i32, y: i32, w: i32, h: i32, len: i32, c: Rgb) {
        let (x1, y1) = (x + w - 1, y + h - 1);
        self.rect(x, y, len, 1, c);
        self.rect(x, y, 1, len, c);
        self.rect(x1 - len + 1, y, len, 1, c);
        self.rect(x1, y, 1, len, c);
        self.rect(x, y1, len, 1, c);
        self.rect(x, y1 - len + 1, 1, len, c);
        self.rect(x1 - len + 1, y1, len, 1, c);
        self.rect(x1, y1 - len + 1, 1, len, c);
    }

    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        self.rect(x, y, w, 1, c);
        self.rect(x, y + h - 1, w, 1, c);
        self.rect(x, y, 1, h, c);
        self.rect(x + w - 1, y, 1, h, c);
    }

    // ---- textured --------------------------------------------------------

    /// Switch to the UI atlas for a run of sprites.
    pub fn ui_mode(&mut self) {
        let tp = Tpage::new(UI_PAGE.0, UI_PAGE.1, TexDepth::Bit4).uv_tpage_word(1);
        let blend = if self.ui_sub {
            BlendMode::Subtract
        } else {
            BlendMode::Add
        };
        self.mode(tp, blend);
    }

    /// Draw atlas sprites subtractively (dark ink on a light fill) or additively.
    pub fn ui_dark(&mut self, on: bool) {
        self.ui_sub = on;
    }

    /// One UI-atlas sprite, additive, tinted so that full coverage is `color`.
    pub fn ui_sprite(&mut self, x: i32, y: i32, w: u16, h: u16, uv: (u8, u8), color: Rgb) {
        let tint = (color.0 >> 1, color.1 >> 1, color.2 >> 1);
        let blend = if self.ui_sub {
            BlendMode::Subtract
        } else {
            BlendMode::Add
        };
        let m = TextureMaterial::blended(UI_CLUT.uv_clut_word(), 0, tint, blend);
        self.prims += 1;
        self.gpu.draw(&Sprite::with_material(
            clip(x) as i16,
            clip(y) as i16,
            w,
            h,
            uv,
            m,
        ));
    }

    fn art_material(&self, size: Size, i: usize, tint: Rgb) -> (TextureMaterial, i32, i32) {
        let (px, py, u0, v0) = size.slot(i);
        let clut = Clut::new(ART_CLUT_X, ART_CLUT_Y + i as u16).uv_clut_word();
        let tp = Tpage::new(px, py, TexDepth::Bit8).uv_tpage_word(0);
        (
            TextureMaterial::opaque(clut, tp, tint).with_dither(true),
            u0,
            v0,
        )
    }

    /// Cover `i` at `size`, unscaled, with its top-left at `x, y`.
    pub fn art(&mut self, size: Size, i: usize, x: i32, y: i32, tint: Rgb) {
        let (m, u0, v0) = self.art_material(size, i, tint);
        let (w, h) = size.dims();
        self.gpu.set_draw_mode(m);
        self.key = u32::MAX;
        self.prims += 1;
        self.gpu.draw(&Sprite::with_material(
            clip(x) as i16,
            clip(y) as i16,
            w as u16,
            h as u16,
            (u0 as u8, v0 as u8),
            m,
        ));
    }
}

impl Gfx {
    /// Cover `i` on a trapezoid with vertical left and right edges, drawn as
    /// vertical strips so the GPU's affine texturing stays square on each.
    /// Left edge spans `lt..lb`, right edge `rt..rb` (screen y).
    pub fn art_strips(
        &mut self,
        size: Size,
        i: usize,
        x0: i32,
        x1: i32,
        (lt, lb): (i32, i32),
        (rt, rb): (i32, i32),
        tint: Rgb,
    ) {
        if x1 - x0 < 1 {
            return;
        }
        let (m, u0, v0) = self.art_material(size, i, tint);
        let (sw, sh) = size.dims();
        self.key = u32::MAX;
        let n: i32 = if lt == rt && lb == rb {
            1
        } else {
            ((x1 - x0) / 12).clamp(2, 6)
        };
        let (va, vb) = (v0 as u8, (v0 + sh - 1) as u8);
        for k in 0..n {
            let xa = x0 + (x1 - x0) * k / n;
            let xb = x0 + (x1 - x0) * (k + 1) / n;
            let ta = lt + (rt - lt) * k / n;
            let tb = lt + (rt - lt) * (k + 1) / n;
            let ba = lb + (rb - lb) * k / n;
            let bb = lb + (rb - lb) * (k + 1) / n;
            let ua = (u0 + (sw - 1) * k / n) as u8;
            let ub = (u0 + (sw - 1) * (k + 1) / n) as u8;
            self.prims += 1;
            self.gpu.draw(&QuadTexturedMaterial::with_material(
                [(xa, ta), (xb, tb), (xa, ba), (xb, bb)].map(p),
                [(ua, va), (ub, va), (ua, vb), (ub, vb)],
                m,
            ));
        }
    }
}

impl Gfx {
    /// Cover `i` on a ribbon of quads: `tops[k]` and `bots[k]` are where the
    /// vertical line at texture column `k / (len - 1)` lands on screen. The
    /// caller projects real 3D points, so each vertex is perspective-correct
    /// and only the span between two neighbours is affine. A ribbon whose
    /// columns run right to left shows the cover mirrored, as the back of a
    /// swung-open cover does.
    ///
    /// # Panics
    ///
    /// If `tops` and `bots` differ in length or hold fewer than two points.
    pub fn art_ribbon(
        &mut self,
        size: Size,
        i: usize,
        tops: &[(i32, i32)],
        bots: &[(i32, i32)],
        tint: Rgb,
    ) {
        assert!(tops.len() == bots.len() && tops.len() >= 2);
        let n = tops.len() as i32 - 1;
        let (m, u0, v0) = self.art_material(size, i, tint);
        let (sw, sh) = size.dims();
        self.key = u32::MAX;
        let (va, vb) = (v0 as u8, (v0 + sh - 1) as u8);
        for k in 0..n {
            let ua = (u0 + (sw - 1) * k / n) as u8;
            let ub = (u0 + (sw - 1) * (k + 1) / n) as u8;
            let (k, k1) = (k as usize, k as usize + 1);
            self.prims += 1;
            self.gpu.draw(&QuadTexturedMaterial::with_material(
                [tops[k], tops[k1], bots[k], bots[k1]].map(p),
                [(ua, va), (ub, va), (ua, vb), (ub, vb)],
                m,
            ));
        }
    }
}
