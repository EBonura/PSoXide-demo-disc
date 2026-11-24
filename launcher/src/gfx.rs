//! Immediate-mode drawing for the menu: gradients, additive glow, brackets,
//! textured art, and the baked UI atlas.
//!
//! Every primitive here is at most 12 GP0 words (the textured Gouraud quad),
//! so the same calls could be linked into an ordering table without a node
//! over the 16-word limit.

use psx_gpu::display::{DisplayConfig, DoubleBuffer, Resolution, VideoMode};
use psx_gpu::material::{BlendMode, TextureMaterial};
use psx_gpu::prim::{
    FillRect, LineGouraud, QuadFlat, QuadGouraud, QuadTexturedMaterial, Sprite, TriGouraud,
};
use psx_gpu::Gpu;
use psx_rt::present::Presenter;
use psx_io::periph::GpuDma;
use psx_vram::{Clut, TextureDepth, TexturePage, VramRect};

pub type Rgb = (u8, u8, u8);

// ---- palette (the home shell's tokens, on a 15-bit display) ---------------

pub const TEXT: Rgb = (244, 248, 251);
pub const MUTED: Rgb = (157, 176, 190);
pub const DIM: Rgb = (129, 150, 166);
pub const RING: Rgb = (108, 127, 143);
pub const ACCENT: Rgb = (34, 200, 229);
pub const FOCUS: Rgb = (128, 239, 255);

// The loader shares this body (`num`: 0..=256), so the disc has one.
pub use crate::disc_art::scale;

pub fn mix(a: Rgb, b: Rgb, t: u32) -> Rgb {
    // t: 0..=256
    let f = |x: u8, y: u8| (((x as u32) * (256 - t) + (y as u32) * t) >> 8) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

// ---- VRAM layout (halfword coordinates) -----------------------------------
//
// VRAM is 1024 x 512 halfwords. The two 320x240 frame buffers take x 0..320, y 0..480.
// Below them, y 480..512 holds one CLUT row per program (the cover palettes at x 0, the
// back-cover palettes at x 320) and the two small palettes of the UI atlas and the credits'
// marks. Everything right of x 320 is texture pages: an 8bpp page is 128 halfwords (256
// texels) wide and 256 high, a 4bpp page 64 wide, and a page starts at a multiple of 64 in x
// and 0 or 256 in y (checked by [`VRAM_CHECK`]).
//
//   x:   320      448      576      704      832      896      960
//   y 0   L 0      L 1      M 0      S 0      UI      icons    (free)
//   y 256 shots 0  shots 1  L 2      M 1      shots 2  (free)  (free)
//   y 480 CLUTs: art 0..256, UI 256, icons 288, back covers 320..576
//
// A cover size is a [`PageSet`]: pages of equal cells, so adding a page is adding a
// coordinate, and the cover table's capacity ([`crate::cover::MAX_COVERS`]) follows.

/// One size of cover art, spread over 8bpp texture pages: `w` x `h` texel cells in rows,
/// as many to a page as fit in 256x256, and as many pages as `pages` lists.
#[derive(Clone, Copy)]
pub struct PageSet {
    /// Top-left of each page in VRAM halfwords.
    pub pages: &'static [(u16, u16)],
    /// Size of one cell in texels.
    pub w: i32,
    pub h: i32,
}

impl PageSet {
    /// Cells across one page.
    pub const fn across(&self) -> usize {
        256 / self.w as usize
    }

    /// Cells down one page.
    pub const fn down(&self) -> usize {
        256 / self.h as usize
    }

    /// Cells one page holds.
    pub const fn per_page(&self) -> usize {
        self.across() * self.down()
    }

    /// Cells all the pages hold.
    pub const fn capacity(&self) -> usize {
        self.pages.len() * self.per_page()
    }

    /// (tpage x, tpage y, u0, v0) of cell `i`.
    pub fn cell(&self, i: usize) -> (u16, u16, i32, i32) {
        let (px, py) = self.pages[i / self.per_page()];
        let at = i % self.per_page();
        (px, py, (at % self.across()) as i32 * self.w, (at / self.across()) as i32 * self.h)
    }

    /// VRAM halfwords the used cells of one page cover: (x, y, width, height).
    const fn used(&self, page: usize) -> (u16, u16, u16, u16) {
        let (x, y) = self.pages[page];
        (x, y, (self.across() as i32 * self.w / 2) as u16, (self.down() as i32 * self.h) as u16)
    }
}

/// Large cover pages: 104x78, 6 per page (2 across, 3 down). The third sits
/// below the UI atlas's columns, right of the back-cover shots and clear of the
/// CLUT rows at y 480 (x 576..704, y 256..512).
pub const L_PAGES: [(u16, u16); 3] = [(320, 0), (448, 0), (576, 256)];
/// Medium covers: 68x51, 15 per page (3 across, 5 down); the second page is the one
/// below the small covers.
pub const M_PAGES: [(u16, u16); 2] = [(576, 0), (704, 256)];
/// Small covers: 52x39, 24 per page (4 across, 6 down).
pub const S_PAGES: [(u16, u16); 1] = [(704, 0)];
/// The UI atlas, 4bpp, 256x256 texels.
pub const UI_PAGE: (u16, u16) = (832, 0);
/// One 256-entry CLUT row per game, below the framebuffers.
pub const ART_CLUT_X: u16 = 0;
pub const ART_CLUT_Y: u16 = 480;
/// CLUT rows from [`ART_CLUT_Y`] to the bottom of VRAM: the most programs that can have a
/// palette, for the covers and for the back covers alike.
pub const CLUT_ROWS: usize = 512 - ART_CLUT_Y as usize;
/// The UI atlas ramp.
const UI_CLUT_AT: (u16, u16) = (256, 480);
pub const UI_CLUT: Clut = Clut::new(UI_CLUT_AT.0, UI_CLUT_AT.1);
/// The credits screen's platform marks: a 4bpp strip of six 12x12 cells.
pub const ICON_PAGE: (u16, u16) = (896, 0);
/// Their palette, a grey ramp the tint colours.
const ICON_CLUT_AT: (u16, u16) = (288, 480);
pub const ICON_CLUT: Clut = Clut::new(ICON_CLUT_AT.0, ICON_CLUT_AT.1);
/// Width of one mark's cell in the strip, and the mark's size.
pub const ICON_CELL: i32 = 12;

pub const L_W: i32 = 104;
pub const L_H: i32 = 78;
pub const M_W: i32 = 68;
pub const M_H: i32 = 51;
pub const S_W: i32 = 52;
pub const S_H: i32 = 39;

/// The three sizes' pages.
pub const L_SET: PageSet = PageSet { pages: &L_PAGES, w: L_W, h: L_H };
pub const M_SET: PageSet = PageSet { pages: &M_PAGES, w: M_W, h: M_H };
pub const S_SET: PageSet = PageSet { pages: &S_PAGES, w: S_W, h: S_H };

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
            Size::L => L_SET.cell(i),
            Size::M => M_SET.cell(i),
            Size::S => S_SET.cell(i),
        }
    }
}

pub struct Gfx {
    gpu: Gpu,
    pub fb: DoubleBuffer,
    presenter: Presenter,
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
    /// Take over the GPU through its DMA token.
    pub fn new(dma: GpuDma) -> Self {
        let gpu = Gpu::new(
            dma,
            DisplayConfig::new(VideoMode::Ntsc, Resolution::R320X240),
        );
        let mut g = Gfx {
            gpu,
            fb: DoubleBuffer::new(Resolution::R320X240),
            presenter: Presenter::new(None),
            key: u32::MAX,
            prims: 0,
            ui_sub: false,
        };
        g.gpu.set_draw_area((0, 0), (319, 239));
        g.gpu.set_draw_offset((0, 0));
        g
    }

    /// The GPU and the buffer pair, for the boot splash (psx-splash draws with its own code).
    pub fn splash_parts(&mut self) -> (&mut Gpu, &mut DoubleBuffer) {
        (&mut self.gpu, &mut self.fb)
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

    /// Upload the platform-mark strip (`tex`, 4bpp, `cells` cells of
    /// [`ICON_CELL`] pixels, one row of marks) and its 16-colour palette.
    pub fn upload_link_icons(&mut self, tex: &[u8], clut: &[u8], cells: i32) {
        let halfwords = (cells * ICON_CELL) as u16 / 4;
        psx_vram::upload_bytes(
            VramRect::new(ICON_PAGE.0, ICON_PAGE.1, halfwords, ICON_CELL as u16),
            tex,
        );
        psx_vram::upload_bytes(VramRect::new(ICON_CLUT.x(), ICON_CLUT.y(), 16, 1), clut);
    }

    /// Platform mark `slot` with its top-left at `x, y`, tinted so that full
    /// coverage is `color`.
    pub fn link_icon(&mut self, slot: i32, x: i32, y: i32, color: Rgb) {
        let tp = TexturePage::new(ICON_PAGE.0, ICON_PAGE.1, TextureDepth::Bit4).uv_word(0);
        let tint = (color.0 >> 1, color.1 >> 1, color.2 >> 1);
        let m = TextureMaterial::opaque(ICON_CLUT.uv_word(), tp, tint);
        self.gpu.set_draw_mode(m);
        self.key = u32::MAX;
        self.prims += 1;
        self.gpu.draw(&Sprite::with_material(
            clip(x) as i16,
            clip(y) as i16,
            ICON_CELL as u16,
            ICON_CELL as u16,
            ((slot * ICON_CELL) as u8, 0),
            m,
        ));
    }

    /// Wait for the GPU to finish everything queued (before a hand-off).
    pub fn wait_idle(&mut self) {
        psx_rt::present::wait_drawn_idle();
        self.gpu.wait_idle();
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
        self.presenter.begin_frame(&self.fb);
        self.prims = 0;
        self.key = u32::MAX;
        self.blend(BlendMode::Opaque);
    }

    pub fn end(&mut self) {
        self.presenter.present_drawn(&mut self.fb);
    }

    /// [`end`](Self::end), and wait until the frame is on screen (the loading frame).
    pub fn end_blocking(&mut self) {
        self.presenter.present_blocking(&mut self.fb);
    }

    pub fn clear(&mut self, c: Rgb) {
        self.gpu
            .draw(&FillRect::new(self.fb.draw_origin(), (320, 240), c));
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

    /// An opaque one-pixel line.
    pub fn line(&mut self, a: (i32, i32), b: (i32, i32), c: Rgb) {
        self.line_g(a, b, c, c);
    }

    /// An opaque one-pixel line shaded from `ca` to `cb`.
    pub fn line_g(&mut self, a: (i32, i32), b: (i32, i32), ca: Rgb, cb: Rgb) {
        self.prims += 1;
        self.gpu.draw(&LineGouraud::new(p(a), ca, p(b), cb));
    }

    pub fn line_add(&mut self, a: (i32, i32), b: (i32, i32), ca: Rgb, cb: Rgb) {
        self.blend(BlendMode::Add);
        self.prims += 1;
        self.gpu
            .draw(&LineGouraud::new(p(a), ca, p(b), cb).translucent());
    }

    /// A soft glow: an additive fan from `c` at the centre to black at the rim.
    pub fn glow(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, c: Rgb) {
        crate::disc_art::glow(self, cx, cy, rx, ry, c);
    }

    /// An ellipse ring (outline) as short additive lines.
    pub fn ellipse_ring(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, c: Rgb) {
        crate::disc_art::ellipse_ring(self, cx, cy, rx, ry, c);
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
        let tp = TexturePage::new(UI_PAGE.0, UI_PAGE.1, TextureDepth::Bit4).uv_word(1);
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
        let m = TextureMaterial::blended(UI_CLUT.uv_word(), 0, tint, blend);
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
        let clut = Clut::new(ART_CLUT_X, ART_CLUT_Y + i as u16).uv_word();
        let tp = TexturePage::new(px, py, TextureDepth::Bit8).uv_word(0);
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

// ---- attract mode primitives ------------------------------------------------

/// Where the back-cover screenshots live: 8bpp pages below the covers' first
/// pages and the UI atlas, 64x48 texels per shot, four across and four down (sixteen a
/// page; the first two pages stop short of the CLUT rows at y 480).
pub const SHOT_PAGES: [(u16, u16); 3] = [(320, 256), (448, 256), (832, 256)];
/// Shots one page holds.
pub const SHOTS_PER_PAGE: usize = 16;
/// Shots across a page, and down.
const SHOTS_ACROSS: usize = 4;
const SHOTS_DOWN: usize = 4;
/// Shots all the pages hold. A program's back cover takes one or two.
pub const SHOT_SLOTS: usize = SHOT_PAGES.len() * SHOTS_PER_PAGE;
/// One 256-entry CLUT row per program, right of the cover CLUTs and below the
/// framebuffers.
pub const SHOT_CLUT_X: u16 = 320;
pub const SHOT_CLUT_Y: u16 = 480;
/// Size of one screenshot in texels.
pub const SHOT_W: i32 = 64;
pub const SHOT_H: i32 = 48;

impl Gfx {
    /// One UI-atlas glyph or icon as a quad, so the caller can tilt it with the
    /// rest of a face it belongs to. `uv` is `(u0, v0, u1, v1)` with the far
    /// edge half-open, so an unscaled quad lands texel for texel on the screen.
    pub fn ui_quad(&mut self, v: [(i32, i32); 4], uv: (u8, u8, u8, u8), color: Rgb) {
        let tint = (color.0 >> 1, color.1 >> 1, color.2 >> 1);
        let tpage = TexturePage::new(UI_PAGE.0, UI_PAGE.1, TextureDepth::Bit4).uv_word(0);
        let m = TextureMaterial::blended(UI_CLUT.uv_word(), tpage, tint, BlendMode::Add);
        let (u0, v0, u1, v1) = uv;
        self.key = u32::MAX;
        self.prims += 1;
        self.gpu.draw(&QuadTexturedMaterial::with_material(
            v.map(p),
            [(u0, v0), (u1, v0), (u0, v1), (u1, v1)],
            m,
        ));
    }

    /// Put program `prog`'s back-cover palette and `shots` screenshots (`pixels`
    /// is their 8bpp texels one after another) into VRAM, the first at `slot`.
    pub fn upload_back(&mut self, prog: usize, clut: &[u8], pixels: &[u8], slot: usize) {
        psx_vram::upload_bytes(
            VramRect::new(SHOT_CLUT_X, SHOT_CLUT_Y + prog as u16, 256, 1),
            clut,
        );
        let bytes = (SHOT_W * SHOT_H) as usize;
        for (k, shot) in pixels.chunks_exact(bytes).enumerate() {
            let (page, col, row) = shot_cell(slot + k);
            psx_vram::upload_bytes(
                VramRect::new(
                    page.0 + (col * SHOT_W / 2) as u16,
                    page.1 + (row * SHOT_H) as u16,
                    (SHOT_W / 2) as u16,
                    SHOT_H as u16,
                ),
                shot,
            );
        }
    }

    /// Screenshot `shot` (a slot in the back-cover pages) of program `prog`,
    /// laid on a ribbon of quads like [`Gfx::art_ribbon`]. `tops[k]` and
    /// `bots[k]` are where texture column `k / (len - 1)` lands on screen.
    ///
    /// # Panics
    ///
    /// If `tops` and `bots` differ in length or hold fewer than two points.
    pub fn shot_ribbon(
        &mut self,
        prog: usize,
        shot: usize,
        tops: &[(i32, i32)],
        bots: &[(i32, i32)],
        tint: Rgb,
    ) {
        assert!(tops.len() == bots.len() && tops.len() >= 2);
        let n = tops.len() as i32 - 1;
        let clut = Clut::new(SHOT_CLUT_X, SHOT_CLUT_Y + prog as u16).uv_word();
        let (page, col, row) = shot_cell(shot);
        let tp = TexturePage::new(page.0, page.1, TextureDepth::Bit8).uv_word(0);
        let m = TextureMaterial::opaque(clut, tp, tint).with_dither(true);
        let (u0, v0) = (col * SHOT_W, row * SHOT_H);
        self.key = u32::MAX;
        let (va, vb) = (v0 as u8, (v0 + SHOT_H - 1) as u8);
        for k in 0..n {
            let ua = (u0 + (SHOT_W - 1) * k / n) as u8;
            let ub = (u0 + (SHOT_W - 1) * (k + 1) / n) as u8;
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

/// The page, column and row of back-cover slot `slot`.
fn shot_cell(slot: usize) -> ((u16, u16), i32, i32) {
    let page = SHOT_PAGES[slot / SHOTS_PER_PAGE];
    let at = slot % SHOTS_PER_PAGE;
    (page, (at % SHOTS_ACROSS) as i32, (at / SHOTS_ACROSS) as i32)
}

/// The disc is drawn through the same four primitives the chain loader uses.
impl crate::disc_art::Canvas for Gfx {
    fn quad_g(&mut self, v: [(i32, i32); 4], c: [Rgb; 4]) {
        Gfx::quad_g(self, v, c);
    }
    fn tri_g(&mut self, v: [(i32, i32); 3], c: [Rgb; 3]) {
        Gfx::tri_g(self, v, c);
    }
    fn tri_g_add(&mut self, v: [(i32, i32); 3], c: [Rgb; 3]) {
        Gfx::tri_g_add(self, v, c);
    }
    fn line_add(&mut self, a: (i32, i32), b: (i32, i32), ca: Rgb, cb: Rgb) {
        Gfx::line_add(self, a, b, ca, cb);
    }
}

// ---- VRAM map check ---------------------------------------------------------------

/// A rectangle of VRAM halfwords: x, y, width, height.
type Rect = (u16, u16, u16, u16);

const fn overlap(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Every region the launcher uses, and how many of the slots hold one.
const fn regions() -> ([Rect; 32], usize) {
    let mut r = [(0, 0, 0, 0); 32];
    let mut n = 0;
    // The two frame buffers, one above the other.
    r[n] = (0, 0, 320, 480);
    n += 1;
    r[n] = (ART_CLUT_X, ART_CLUT_Y, 256, CLUT_ROWS as u16);
    n += 1;
    r[n] = (SHOT_CLUT_X, SHOT_CLUT_Y, 256, CLUT_ROWS as u16);
    n += 1;
    r[n] = (UI_CLUT_AT.0, UI_CLUT_AT.1, 16, 1);
    n += 1;
    r[n] = (ICON_CLUT_AT.0, ICON_CLUT_AT.1, 16, 1);
    n += 1;
    // The UI atlas and the marks' strip: 4bpp pages, 64 halfwords wide.
    r[n] = (UI_PAGE.0, UI_PAGE.1, 64, 256);
    n += 1;
    r[n] = (ICON_PAGE.0, ICON_PAGE.1, 64, ICON_CELL as u16);
    n += 1;
    let sets = [L_SET, M_SET, S_SET];
    let mut s = 0;
    while s < sets.len() {
        let mut k = 0;
        while k < sets[s].pages.len() {
            r[n] = sets[s].used(k);
            n += 1;
            k += 1;
        }
        s += 1;
    }
    let mut k = 0;
    while k < SHOT_PAGES.len() {
        let (x, y) = SHOT_PAGES[k];
        r[n] = (x, y, (SHOTS_ACROSS as i32 * SHOT_W / 2) as u16, (SHOTS_DOWN as i32 * SHOT_H) as u16);
        n += 1;
        k += 1;
    }
    (r, n)
}

/// No two regions overlap, every one is inside VRAM, and every 8bpp page starts where a
/// texture page can (x a multiple of 64, y 0 or 256) and keeps its 256 texels inside VRAM.
const _: () = {
    let (r, n) = regions();
    let mut i = 0;
    while i < n {
        assert!(r[i].0 + r[i].2 <= 1024 && r[i].1 + r[i].3 <= 512, "a VRAM region is off the end of VRAM");
        let mut j = i + 1;
        while j < n {
            assert!(!overlap(r[i], r[j]), "two VRAM regions overlap");
            j += 1;
        }
        i += 1;
    }
    let sets = [L_SET, M_SET, S_SET];
    let mut s = 0;
    while s < sets.len() {
        let mut k = 0;
        while k < sets[s].pages.len() {
            let (x, y) = sets[s].pages[k];
            assert!(x % 64 == 0 && (y == 0 || y == 256), "a texture page is not where a page can start");
            assert!(x + 128 <= 1024, "an 8bpp texture page runs off the end of VRAM");
            k += 1;
        }
        s += 1;
    }
    let mut k = 0;
    while k < SHOT_PAGES.len() {
        let (x, y) = SHOT_PAGES[k];
        assert!(x % 64 == 0 && (y == 0 || y == 256), "a shot page is not where a page can start");
        assert!(x + 128 <= 1024, "a shot page runs off the end of VRAM");
        k += 1;
    }
};
