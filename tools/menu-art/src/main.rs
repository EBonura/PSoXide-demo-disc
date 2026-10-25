//! Bakes the demo disc launcher's art into PS1 texture blobs.
//!
//! * `ui.tex`: one 256x256 4bpp texture page holding three anti-aliased
//!   faces (Michroma display, Michroma small caps, Jost body) and the button
//!   glyphs. Texel 0 is transparent, texels 1..=15 are coverage levels, so the
//!   guest tints it with the palette ramp and blends it additively.
//! * `ui_atlas.rs`: the glyph tables for that page, as `const` data.
//! * `art_<size>_<name>.art`: a cover image resized with a Lanczos filter and
//!   cut to 256 colours with median cut, as 512 bytes of CLUT followed by the
//!   8bpp pixels.
//!
//! * `back_<name>.art`: the attract mode's back-cover screenshots.
//!
//! Usage: `menu-art bake <michroma.ttf> <jost.ttf> <shots-dir> <out-dir>`
//!        `menu-art backs <shots-dir> <out-dir>`

use fontdue::{Font, FontSettings};
use image::imageops::FilterType;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

mod attract;

const PAGE: usize = 256;

/// First and last character baked for each face.
const FIRST: u8 = 32;
const LAST: u8 = 126;

#[derive(Clone, Copy, Default)]
struct Glyph {
    u: u8,
    v: u8,
    w: u8,
    h: u8,
    xoff: i8,
    yoff: i8,
    adv: u8,
}

struct Atlas {
    px: Vec<u8>,
    x: usize,
    y: usize,
    row_h: usize,
}

impl Atlas {
    fn new() -> Self {
        Atlas {
            px: vec![0; PAGE * PAGE],
            x: 1,
            y: 1,
            row_h: 0,
        }
    }

    /// Place a `w` x `h` bitmap of 0..=15 levels, one texel of gutter around it.
    fn place(&mut self, w: usize, h: usize, levels: &[u8]) -> (u8, u8) {
        if self.x + w + 1 > PAGE {
            self.x = 1;
            self.y += self.row_h + 1;
            self.row_h = 0;
        }
        assert!(self.y + h < PAGE, "atlas page is full");
        for row in 0..h {
            for col in 0..w {
                self.px[(self.y + row) * PAGE + self.x + col] = levels[row * w + col];
            }
        }
        let at = (self.x as u8, self.y as u8);
        self.x += w + 1;
        self.row_h = self.row_h.max(h);
        at
    }

    /// Two texels a byte, the left one in the low nibble, as the GPU reads 4bpp.
    fn packed(&self) -> Vec<u8> {
        self.px
            .chunks(2)
            .map(|pair| pair[0] | (pair[1] << 4))
            .collect()
    }
}

fn level(coverage: f32) -> u8 {
    // A touch of gamma lifts thin strokes, which the 4bpp ramp would eat.
    let c = coverage.clamp(0.0, 1.0).powf(0.8);
    (c * 15.0 + 0.5) as u8
}

struct Face {
    name: &'static str,
    glyphs: Vec<Glyph>,
    ascent: i32,
    line: i32,
}

fn bake_face(atlas: &mut Atlas, name: &'static str, font: &Font, px: f32) -> Face {
    let lm = font
        .horizontal_line_metrics(px)
        .expect("font has line metrics");
    let ascent = lm.ascent.round() as i32;
    let line = lm.new_line_size.round() as i32;
    let mut glyphs = Vec::new();
    for c in FIRST..=LAST {
        let (m, cov) = font.rasterize(c as char, px);
        let levels: Vec<u8> = cov.iter().map(|&b| level(b as f32 / 255.0)).collect();
        let mut g = Glyph {
            adv: m.advance_width.round() as u8,
            ..Glyph::default()
        };
        if m.width > 0 && m.height > 0 {
            let (u, v) = atlas.place(m.width, m.height, &levels);
            g.u = u;
            g.v = v;
            g.w = m.width as u8;
            g.h = m.height as u8;
            g.xoff = m.xmin as i8;
            g.yoff = (ascent - (m.ymin + m.height as i32)) as i8;
        }
        glyphs.push(g);
    }
    Face {
        name,
        glyphs,
        ascent,
        line,
    }
}

// ---- button glyphs ------------------------------------------------------

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

fn raster(w: usize, h: usize, inside: &dyn Fn(f32, f32) -> bool) -> Vec<u8> {
    const SS: usize = 4;
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut hit = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    if inside(fx, fy) {
                        hit += 1;
                    }
                }
            }
            out[y * w + x] = level(hit as f32 / (SS * SS) as f32);
        }
    }
    out
}

fn ring(px: f32, py: f32, cx: f32, cy: f32, r: f32, t: f32) -> bool {
    (((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r).abs() <= t / 2.0
}

fn poly_outline(px: f32, py: f32, pts: &[(f32, f32)], t: f32) -> bool {
    (0..pts.len()).any(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        seg_dist(px, py, a.0, a.1, b.0, b.1) <= t / 2.0
    })
}

/// The four face-button marks in a ring, an arrow pair and the disc mark.
fn bake_icons(atlas: &mut Atlas) -> Vec<(&'static str, Glyph)> {
    let mut out = Vec::new();
    let mut add = |name: &'static str, w: usize, h: usize, f: &dyn Fn(f32, f32) -> bool| {
        let levels = raster(w, h, f);
        let (u, v) = atlas.place(w, h, &levels);
        out.push((
            name,
            Glyph {
                u,
                v,
                w: w as u8,
                h: h as u8,
                xoff: 0,
                yoff: 0,
                adv: w as u8,
            },
        ));
    };
    // Odd sizes on purpose: an odd box has a middle pixel row and column, so
    // the marks sit exactly on the menu's centre lines.
    let c = 7.5;
    add("CROSS", 15, 15, &|x, y| {
        ring(x, y, c, c, 6.7, 1.3)
            || seg_dist(x, y, 5.0, 5.0, 10.0, 10.0) <= 0.75
            || seg_dist(x, y, 10.0, 5.0, 5.0, 10.0) <= 0.75
    });
    add("CIRCLE", 15, 15, &|x, y| {
        ring(x, y, c, c, 6.7, 1.3) || ring(x, y, c, c, 2.9, 1.3)
    });
    let tri = [(7.5, 4.0), (11.4, 10.4), (3.6, 10.4)];
    add("TRIANGLE", 15, 15, &move |x, y| {
        ring(x, y, c, c, 6.7, 1.3) || poly_outline(x, y, &tri, 1.2)
    });
    let sq = [(4.7, 4.7), (10.3, 4.7), (10.3, 10.3), (4.7, 10.3)];
    add("SQUARE", 15, 15, &move |x, y| {
        ring(x, y, c, c, 6.7, 1.3) || poly_outline(x, y, &sq, 1.2)
    });
    // Disc: outer ring, hub ring and a hub dot, 15 px.
    add("DISC", 15, 15, &|x, y| {
        ring(x, y, c, c, 6.7, 1.4)
            || ring(x, y, c, c, 2.8, 1.2)
            || ((x - c).powi(2) + (y - c).powi(2)) <= 0.5
    });
    // Chevrons for the carousel hint.
    add("CHEV_L", 7, 11, &|x, y| {
        seg_dist(x, y, 5.2, 1.2, 1.6, 5.5) <= 0.7 || seg_dist(x, y, 1.6, 5.5, 5.2, 9.8) <= 0.7
    });
    add("CHEV_R", 7, 11, &|x, y| {
        seg_dist(x, y, 1.8, 1.2, 5.4, 5.5) <= 0.7 || seg_dist(x, y, 5.4, 5.5, 1.8, 9.8) <= 0.7
    });
    add("CHEV_U", 11, 7, &|x, y| {
        seg_dist(x, y, 1.2, 5.2, 5.5, 1.6) <= 0.7 || seg_dist(x, y, 5.5, 1.6, 9.8, 5.2) <= 0.7
    });
    add("CHEV_D", 11, 7, &|x, y| {
        seg_dist(x, y, 1.2, 1.8, 5.5, 5.4) <= 0.7 || seg_dist(x, y, 5.5, 5.4, 9.8, 1.8) <= 0.7
    });
    out
}

// ---- cover art ----------------------------------------------------------

type Rgb = [u8; 3];

/// Median cut to at most 256 colours.
fn median_cut(pixels: &[Rgb]) -> Vec<Rgb> {
    let mut boxes: Vec<Vec<Rgb>> = vec![pixels.to_vec()];
    while boxes.len() < 256 {
        // Split the box with the widest channel range.
        let (idx, chan, range) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| {
                let mut best = (0usize, 0i32);
                for ch in 0..3 {
                    let lo = b.iter().map(|p| p[ch]).min().unwrap() as i32;
                    let hi = b.iter().map(|p| p[ch]).max().unwrap() as i32;
                    if hi - lo > best.1 {
                        best = (ch, hi - lo);
                    }
                }
                (i, best.0, best.1 * (b.len() as f32).sqrt() as i32)
            })
            .max_by_key(|&(_, _, r)| r)
            .unwrap_or((0, 0, 0));
        if range == 0 {
            break;
        }
        let mut b = boxes.swap_remove(idx);
        b.sort_by_key(|p| p[chan]);
        let hi = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(hi);
    }
    boxes
        .iter()
        .map(|b| {
            let n = b.len() as u32;
            let mut sum = [0u32; 3];
            for p in b {
                for ch in 0..3 {
                    sum[ch] += p[ch] as u32;
                }
            }
            [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
        })
        .collect()
}

fn to_555(c: Rgb) -> u16 {
    let v = ((c[0] as u16 >> 3) & 31)
        | (((c[1] as u16 >> 3) & 31) << 5)
        | (((c[2] as u16 >> 3) & 31) << 10);
    // 0x0000 is transparent on this hardware; nudge true black off it.
    if v == 0 {
        0x0421
    } else {
        v
    }
}

fn palette_to_clut(palette: &[Rgb]) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    for i in 0..256 {
        let c = palette.get(i).copied().unwrap_or([0, 0, 0]);
        out.extend_from_slice(&to_555(c).to_le_bytes());
    }
    out
}

fn index_with(palette: &[Rgb], pixels: &[Rgb]) -> Vec<u8> {
    let nearest = |p: &Rgb| -> u8 {
        let mut best = (0usize, i32::MAX);
        for (i, q) in palette.iter().enumerate() {
            let d: i32 = (0..3).map(|ch| (p[ch] as i32 - q[ch] as i32).pow(2)).sum();
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0 as u8
    };
    pixels.iter().map(nearest).collect()
}

/// One source image at every size, all sharing the palette cut for the largest,
/// so the guest needs one CLUT row per game rather than one per size.
fn cook_art(src: &Path, sizes: &[(&str, u32, u32)]) -> Vec<(String, Vec<u8>)> {
    let img = image::open(src).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    let resize = |w: u32, h: u32| -> Vec<Rgb> {
        img.resize_exact(w, h, FilterType::Lanczos3)
            .to_rgb8()
            .pixels()
            .map(|p| p.0)
            .collect()
    };
    let (_, w0, h0) = sizes[0];
    let palette = median_cut(&resize(w0, h0));
    let mut out = Vec::new();
    for (tag, w, h) in sizes {
        let mut blob = palette_to_clut(&palette);
        blob.extend(index_with(&palette, &resize(*w, *h)));
        out.push((tag.to_string(), blob));
    }
    out
}

// ---- main ---------------------------------------------------------------

/// (file stem in the shots directory, id used by the guest).
const ARTS: &[(&str, &str)] = &[
    ("breakout", "arcade"),
    ("celeste", "celeste"),
    ("nitroxide-aerial", "nitro"),
    ("voxide-day", "voxide"),
    ("quake-gameplay", "quake"),
    ("cortex-current-gameplay", "cortex"),
    ("psxcel-chart", "psxcel"),
    ("wipeout1", "wipeout"),
    ("hollowknight", "hollow"),
    ("counterstrike", "cstrike"),
    ("halflife", "halflife"),
    ("oot1", "oot"),
];

/// Cover sizes: (tag, width, height). Large is the focused case or the media
/// panel, small is a grid tile.
const SIZES: &[(&str, u32, u32)] = &[("l", 104, 78), ("m", 68, 51), ("s", 52, 39)];

fn emit_face(s: &mut String, f: &Face) {
    let up = f.name.to_uppercase();
    writeln!(s, "pub static {up}: Face = Face {{").unwrap();
    writeln!(s, "    ascent: {}, line: {},", f.ascent, f.line).unwrap();
    writeln!(s, "    glyphs: [").unwrap();
    for g in &f.glyphs {
        writeln!(
            s,
            "        Glyph {{ u: {}, v: {}, w: {}, h: {}, xoff: {}, yoff: {}, adv: {} }},",
            g.u, g.v, g.w, g.h, g.xoff, g.yoff, g.adv
        )
        .unwrap();
    }
    writeln!(s, "    ],\n}};").unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "backs" {
        return attract::bake_backs(Path::new(&args[2]), Path::new(&args[3]));
    }
    if args.len() != 6 || args[1] != "bake" {
        eprintln!("usage: menu-art bake <michroma.ttf> <jost.ttf> <shots-dir> <out-dir>");
        eprintln!("       menu-art backs <shots-dir> <out-dir>");
        std::process::exit(2);
    }
    let load = |p: &str| {
        Font::from_bytes(
            fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}")),
            FontSettings::default(),
        )
        .expect("font parses")
    };
    let michroma = load(&args[2]);
    let jost = load(&args[3]);
    let shots = Path::new(&args[4]);
    let out = Path::new(&args[5]);
    fs::create_dir_all(out).unwrap();

    let mut atlas = Atlas::new();
    let faces = [
        bake_face(&mut atlas, "mich12", &michroma, 12.0),
        bake_face(&mut atlas, "mich9", &michroma, 9.0),
        bake_face(&mut atlas, "jost9", &jost, 9.0),
        bake_face(&mut atlas, "jost11", &jost, 11.0),
    ];
    let icons = bake_icons(&mut atlas);

    let mut rs = String::new();
    writeln!(rs, "// Generated by tools/menu-art. Do not edit.").unwrap();
    writeln!(rs, "use crate::text::{{Face, Glyph}};\n").unwrap();
    for f in &faces {
        emit_face(&mut rs, f);
    }
    for (name, g) in &icons {
        writeln!(
            rs,
            "pub const ICON_{name}: Glyph = Glyph {{ u: {}, v: {}, w: {}, h: {}, xoff: 0, yoff: 0, adv: {} }};",
            g.u, g.v, g.w, g.h, g.adv
        )
        .unwrap();
    }
    fs::write(out.join("ui_atlas.rs"), rs).unwrap();
    fs::write(out.join("ui.tex"), atlas.packed()).unwrap();

    // A shots directory that does not exist means "interface only": the cover
    // blobs already in the output directory are left as they are.
    if shots.is_dir() {
        for (stem, id) in ARTS {
            for (tag, blob) in cook_art(&shots.join(format!("{stem}.png")), SIZES) {
                fs::write(out.join(format!("art_{tag}_{id}.art")), blob).unwrap();
            }
        }
    }
    println!(
        "baked {} faces, {} icons, {} arts",
        faces.len(),
        icons.len(),
        ARTS.len() * SIZES.len()
    );
}
