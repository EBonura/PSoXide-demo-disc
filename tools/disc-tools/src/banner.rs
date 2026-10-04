//! Convert the PSoXide logo into a 4bpp CLUT texture the launcher can upload.
//!
//! The menu draws no textures anywhere else, so this is the one asset on the
//! screen. It has to fit a single PlayStation texture page: UVs are bytes, which
//! caps a page at 256 texels, and 4bpp keeps sixteen colours in 80 bytes a row.
//!
//! The logo is composited onto black rather than kept transparent: its edges are
//! antialiased against nothing, and thresholding that alpha left them ragged.
//!
//! Palette index 0 is therefore opaque black, written 0x8000 and not 0x0000. On
//! this hardware a CLUT entry of 0x0000 means see-through, so a literal black
//! would punch holes in the mark instead of backing it.
//!
//!     disc-tools banner <logo.png> <out-dir> [width] [height] [crop-rows]
//!
//! `crop-rows` keeps only the top N rows of the source. The logo's tagline is
//! unreadable once the mark fits a menu corner, so it is cropped off rather than
//! shipped as a grey smear.
//!
//! Writes `banner.tex` (packed 4bpp indices) and `banner.clut` (16 little-endian
//! 15-bit colours) for `include_bytes!`.
//!
//! The float arithmetic is kept in the order the Python original used, so the
//! cooked bytes match it bit for bit (`powf` calls the same system `pow`).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::util::{Error, Result};

const BLACK: u8 = 0;
const COLOURS: usize = 16;
/// How hard the unsharp mask pulls: enough to define the letterforms, short of
/// the bright halo that over-sharpening leaves.
const SHARPEN: f64 = 0.9;
/// Black with the mask bit set. 0x0000 would be transparent.
const OPAQUE_BLACK: u16 = 0x8000;

type Rgba = [u8; 4];
type Rows = Vec<Vec<Rgba>>;
type Linear = [f64; 3];

const USAGE: &str = "usage: disc-tools banner <logo.png> <out-dir> [width] [height] [crop-rows]";

/// Decode a PNG to RGBA rows. Only 8-bit RGB and RGBA are accepted: that is
/// every file this tool exists to convert.
fn read_png(path: &str) -> Result<(usize, usize, Rows)> {
    let data = fs::read(path).map_err(|e| Error(format!("{path}: {e}")))?;
    if data.get(..8) != Some(b"\x89PNG\r\n\x1a\n".as_slice()) {
        bail!("{path}: not a PNG");
    }
    let mut reader = png::Decoder::new(data.as_slice())
        .read_info()
        .map_err(|e| Error(format!("{path}: {e}")))?;
    let (depth, colour) = {
        let info = reader.info();
        let code = match info.color_type {
            png::ColorType::Grayscale => 0,
            png::ColorType::Rgb => 2,
            png::ColorType::Indexed => 3,
            png::ColorType::GrayscaleAlpha => 4,
            png::ColorType::Rgba => 6,
        };
        (info.bit_depth as u8, code)
    };
    if depth != 8 || (colour != 2 && colour != 6) {
        bail!("{path}: need 8-bit RGB or RGBA, got depth {depth} type {colour}");
    }
    let channels = if colour == 6 { 4 } else { 3 };
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|e| Error(format!("{path}: {e}")))?;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut rows = Vec::with_capacity(h);
    for y in 0..h {
        let line = &buffer[y * frame.line_size..(y + 1) * frame.line_size];
        rows.push(
            (0..w)
                .map(|x| {
                    let p = &line[x * channels..(x + 1) * channels];
                    [p[0], p[1], p[2], if channels == 4 { p[3] } else { 255 }]
                })
                .collect(),
        );
    }
    Ok((w, h, rows))
}

/// Crop to the topmost run of non-empty rows, and to its ink horizontally.
///
/// Doing this on the bitmap rather than by hand-picking a viewBox is the
/// difference between guessing and measuring: a rasteriser letterboxes to
/// preserve aspect, so a viewBox chosen to frame the mark still comes back with
/// blank rows and, worse, a sliver of whatever sits below it. Keeping only the
/// first band drops the tagline whatever the crop did.
fn trim_to_first_band(rows: Rows, w: usize, h: usize) -> (Rows, usize, usize) {
    let inked: Vec<usize> = (0..h)
        .filter(|&y| (0..w).any(|x| rows[y][x][3] > 2))
        .collect();
    let Some(&top) = inked.first() else {
        return (rows, w, h);
    };
    let mut bottom = top;
    for &y in &inked[1..] {
        if y != bottom + 1 {
            break;
        }
        bottom = y;
    }
    let cols: Vec<usize> = (0..w)
        .filter(|&x| (top..=bottom).any(|y| rows[y][x][3] > 2))
        .collect();
    let (left, right) = (cols[0], cols[cols.len() - 1]);
    let cropped = rows[top..=bottom]
        .iter()
        .map(|line| line[left..=right].to_vec())
        .collect();
    (cropped, right - left + 1, bottom - top + 1)
}

/// sRGB byte to linear light.
fn to_linear(v: f64) -> f64 {
    let c = v / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light back to an sRGB byte.
fn to_srgb(v: f64) -> i64 {
    let c = v.clamp(0.0, 1.0);
    let c = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0 + 0.5) as i64
}

/// Python 3.12+ `sum()` of floats (Neumaier compensation); the unsharp mask's
/// neighbour mean went through it.
fn float_sum(values: [f64; 4]) -> f64 {
    let mut total = 0.0f64;
    let mut compensation = 0.0f64;
    for x in values {
        let t = total + x;
        if total.abs() >= x.abs() {
            compensation += (total - t) + x;
        } else {
            compensation += (x - t) + total;
        }
        total = t;
    }
    if compensation != 0.0 && compensation.is_finite() {
        total += compensation;
    }
    total
}

/// Unsharp mask. An eight-to-one downscale is soft however carefully it is
/// averaged, and the logo is a hard-edged mark; putting some of the edge back is
/// the difference between a wordmark and a smudge.
fn sharpen(rows: &[Vec<Linear>], w: usize, h: usize, amount: f64) -> Vec<Vec<Linear>> {
    let mut out = Vec::with_capacity(h);
    for y in 0..h {
        let mut line = Vec::with_capacity(w);
        for x in 0..w {
            let here = rows[y][x];
            // Mean of the four neighbours, clamped at the edges.
            let near = [
                rows[y.saturating_sub(1)][x],
                rows[(y + 1).min(h - 1)][x],
                rows[y][x.saturating_sub(1)],
                rows[y][(x + 1).min(w - 1)],
            ];
            let mut pixel = [0.0; 3];
            for c in 0..3 {
                let blurred = float_sum([near[0][c], near[1][c], near[2][c], near[3][c]]) / 4.0;
                pixel[c] = (here[c] + (here[c] - blurred) * amount).max(0.0);
            }
            line.push(pixel);
        }
        out.push(line);
    }
    out
}

/// Box filter down to the target size, averaging in linear light.
///
/// Averaging sRGB bytes directly is the usual way to get a muddy downscale: the
/// values are perceptual, not physical, so the mean of black and white lands
/// well below the midpoint and every edge loses contrast.
fn resample(
    rows: &Rows,
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<Vec<Linear>> {
    let mut out = Vec::with_capacity(dst_h);
    for y in 0..dst_h {
        let y0 = y * src_h / dst_h;
        let y1 = (y0 + 1).max((y + 1) * src_h / dst_h);
        let mut line = Vec::with_capacity(dst_w);
        for x in 0..dst_w {
            let x0 = x * src_w / dst_w;
            let x1 = (x0 + 1).max((x + 1) * src_w / dst_w);
            let (mut r, mut g, mut b) = (0.0f64, 0.0f64, 0.0f64);
            let (mut a, mut n) = (0i64, 0i64);
            for row in &rows[y0..y1] {
                for px in &row[x0..x1] {
                    let pa = px[3] as f64;
                    // Weight colour by coverage so transparent pixels do not
                    // drag the edges toward black.
                    r += to_linear(px[0] as f64) * pa;
                    g += to_linear(px[1] as f64) * pa;
                    b += to_linear(px[2] as f64) * pa;
                    a += px[3] as i64;
                    n += 1;
                }
            }
            // Composited onto black: coverage scales the colour toward it, in
            // linear light where that scaling is physically meaningful.
            let coverage = (a as f64 / n as f64) / 255.0;
            if a != 0 {
                let a = a as f64;
                line.push([r / a * coverage, g / a * coverage, b / a * coverage]);
            } else {
                line.push([0.0, 0.0, 0.0]);
            }
        }
        out.push(line);
    }
    out
}

fn to_555(r: i64, g: i64, b: i64) -> u16 {
    (((b >> 3) << 10) | ((g >> 3) << 5) | (r >> 3)) as u16
}

/// Fifteen colours by popularity in 15-bit space, plus opaque black at index 0.
/// The logo is a flat mark with a glow, so a handful of levels covers it; there
/// is no need for anything cleverer than counting.
fn build_palette(pixels: &[[i64; 3]]) -> Vec<u16> {
    // Counted in first-seen order: the ranking below is a stable sort, so ties
    // keep that order exactly as the dict-based original did.
    let mut seen: HashMap<u16, usize> = HashMap::new();
    let mut counts: Vec<(u16, usize)> = Vec::new();
    for p in pixels {
        let key = to_555(p[0], p[1], p[2]);
        match seen.get(&key) {
            Some(&at) => counts[at].1 += 1,
            None => {
                seen.insert(key, counts.len());
                counts.push((key, 1));
            }
        }
    }
    counts.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let mut palette = vec![OPAQUE_BLACK];
    palette.extend(counts.iter().take(COLOURS - 1).map(|c| c.0));
    palette
}

fn channels(colour: u16) -> [i64; 3] {
    [
        (colour & 31) as i64,
        ((colour >> 5) & 31) as i64,
        ((colour >> 10) & 31) as i64,
    ]
}

/// Closest palette entry, skipping index 0 which means transparent.
fn nearest(palette: &[u16], colour: u16) -> u8 {
    let [r, g, b] = channels(colour);
    let (mut best, mut best_d) = (1usize, None::<i64>);
    for (i, &entry) in palette.iter().enumerate().skip(1) {
        let [pr, pg, pb] = channels(entry);
        let d = (r - pr).pow(2) + (g - pg).pow(2) + (b - pb).pow(2);
        if best_d.is_none_or(|current| d < current) {
            best = i;
            best_d = Some(d);
        }
    }
    best as u8
}

/// What the cooking produced, before anything is written.
struct Cooked {
    packed: Vec<u8>,
    clut: Vec<u8>,
    src_w: usize,
    src_h: usize,
    slots_used: usize,
    inked_percent: usize,
}

fn cook(
    mut rows: Rows,
    mut src_w: usize,
    mut src_h: usize,
    crop_rows: usize,
    width: usize,
    height: usize,
) -> Result<Cooked> {
    if crop_rows != 0 {
        ensure!(
            crop_rows <= src_h,
            "crop-rows {crop_rows} is taller than the {src_h}-row source"
        );
        rows.truncate(crop_rows);
        src_h = crop_rows;
    }
    (rows, src_w, src_h) = trim_to_first_band(rows, src_w, src_h);
    let small: Vec<Vec<[i64; 3]>> = if (src_w, src_h) == (width, height) {
        // Already at the target size, so it was rendered there rather than
        // reduced. A vector rasterised at final size computes its coverage from
        // the geometry, which is as good as this gets; resampling and sharpening
        // it again would only add ringing. Composite onto black at the alpha the
        // rasteriser produced.
        rows.iter()
            .map(|line| {
                line.iter()
                    .map(|px| {
                        let on_black = |c: u8| (c as f64 * px[3] as f64 / 255.0 + 0.5) as i64;
                        [on_black(px[0]), on_black(px[1]), on_black(px[2])]
                    })
                    .collect()
            })
            .collect()
    } else {
        let sharp = sharpen(
            &resample(&rows, src_w, src_h, width, height),
            width,
            height,
            SHARPEN,
        );
        sharp
            .iter()
            .map(|line| {
                line.iter()
                    .map(|px| [to_srgb(px[0]), to_srgb(px[1]), to_srgb(px[2])])
                    .collect()
            })
            .collect()
    };
    let flat: Vec<[i64; 3]> = small.into_iter().flatten().collect();
    let mut palette = build_palette(&flat);
    palette.resize(COLOURS, 0x0000);

    let mut lookup: HashMap<u16, u8> = HashMap::new();
    let mut indices = Vec::with_capacity(flat.len());
    for p in &flat {
        let key = to_555(p[0], p[1], p[2]);
        if key == 0 {
            indices.push(BLACK);
            continue;
        }
        let index = *lookup.entry(key).or_insert_with(|| nearest(&palette, key));
        indices.push(index);
    }
    ensure!(
        indices.len() == width * height,
        "cooked {} texels for a {width}x{height} banner",
        indices.len()
    );

    let mut packed = Vec::with_capacity(indices.len() / 2);
    for y in 0..height {
        let row = &indices[y * width..(y + 1) * width];
        for x in (0..width).step_by(2) {
            packed.push(row[x] | (row[x + 1] << 4));
        }
    }
    let mut used = indices.clone();
    used.sort_unstable();
    used.dedup();
    let inked = indices.iter().filter(|&&i| i != BLACK).count();
    Ok(Cooked {
        packed,
        clut: palette.iter().flat_map(|c| c.to_le_bytes()).collect(),
        src_w,
        src_h,
        slots_used: used.len(),
        inked_percent: inked * 100 / indices.len(),
    })
}

fn number(text: Option<&String>, default: usize, what: &str) -> Result<usize> {
    match text {
        None => Ok(default),
        Some(t) => t
            .trim()
            .parse::<usize>()
            .map_err(|_| Error(format!("{what} must be a non-negative integer, got {t:?}"))),
    }
}

pub fn run(args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        println!("{USAGE}");
        return Ok(1);
    }
    let (src, out_dir) = (&args[0], Path::new(&args[1]));
    let width = number(args.get(2), 160, "width")?;
    let height = number(args.get(3), 42, "height")?;
    let crop_rows = number(args.get(4), 0, "crop-rows")?;
    ensure!(
        width % 4 == 0,
        "width must be a multiple of 4: 4bpp packs four texels a halfword"
    );
    ensure!(
        width <= 256,
        "width must be at most 256: UVs are bytes, so a page is 256 texels"
    );

    let (src_w, src_h, rows) = read_png(src)?;
    let cooked = cook(rows, src_w, src_h, crop_rows, width, height)?;
    fs::create_dir_all(out_dir)?;
    fs::write(out_dir.join("banner.tex"), &cooked.packed)?;
    fs::write(out_dir.join("banner.clut"), &cooked.clut)?;
    println!(
        "{src} {}x{} -> {width}x{height}, {} bytes of 4bpp, {} of {COLOURS} palette slots used, {}% inked over black",
        cooked.src_w,
        cooked.src_h,
        cooked.packed.len(),
        cooked.slots_used,
        cooked.inked_percent
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(
        path: &Path,
        w: u32,
        h: u32,
        colour: png::ColorType,
        depth: png::BitDepth,
        data: &[u8],
    ) {
        let file = fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, w, h);
        encoder.set_color(colour);
        encoder.set_depth(depth);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(data)
            .unwrap();
    }

    #[test]
    fn packing_and_555_follow_the_original() {
        assert_eq!(to_555(255, 255, 255), 0x7FFF);
        assert_eq!(to_555(8, 0, 0), 1);
        assert_eq!(to_555(0, 8, 0), 1 << 5);
        assert_eq!(to_555(0, 0, 8), 1 << 10);
        assert_eq!(to_srgb(1.0), 255);
        assert_eq!(to_srgb(-3.0), 0);
        assert_eq!(to_srgb(0.0), 0);
    }

    #[test]
    fn palette_ranks_by_count_and_keeps_first_seen_on_ties() {
        let pixels = [[8, 0, 0], [16, 0, 0], [16, 0, 0], [24, 0, 0]];
        // 16 is the most popular; 8 and 24 tie and 8 was seen first.
        assert_eq!(build_palette(&pixels), vec![OPAQUE_BLACK, 2, 1, 3]);
    }

    #[test]
    fn nearest_skips_index_zero_and_prefers_the_first_tie() {
        let palette = [OPAQUE_BLACK, 1, 3, 0, 0];
        assert_eq!(nearest(&palette, 2), 1);
        assert_eq!(nearest(&palette, 0), 3);
    }

    #[test]
    fn cooks_a_synthetic_logo_at_target_size() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("logo.png");
        // 4x2: a white pixel, a half-transparent white, and two empty ones per row.
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend([
                255, 255, 255, 255, 255, 255, 255, 128, 0, 0, 0, 0, 0, 0, 0, 0,
            ]);
        }
        write_png(
            &src,
            4,
            2,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &data,
        );
        let out = dir.path().join("out");
        let argv: Vec<String> = [src.to_str().unwrap(), out.to_str().unwrap(), "4", "2"]
            .map(String::from)
            .to_vec();
        // The ink is two columns wide, so the trimmed source is 2x2, not 4x2:
        // this takes the resample path, and must still produce 2 bytes.
        assert_eq!(run(&argv).unwrap(), 0);
        assert_eq!(fs::read(out.join("banner.tex")).unwrap().len(), 4);
        let clut = fs::read(out.join("banner.clut")).unwrap();
        assert_eq!(clut.len(), 32);
        assert_eq!(&clut[..2], &[0x00, 0x80]);
    }

    #[test]
    fn rejects_other_png_formats_and_non_png() {
        let dir = tempfile::tempdir().unwrap();
        let grey = dir.path().join("grey.png");
        write_png(
            &grey,
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &[7],
        );
        let error = read_png(grey.to_str().unwrap()).unwrap_err();
        assert!(
            error
                .0
                .contains("need 8-bit RGB or RGBA, got depth 8 type 0"),
            "{error}"
        );
        let deep = dir.path().join("deep.png");
        write_png(
            &deep,
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Sixteen,
            &[0; 6],
        );
        let error = read_png(deep.to_str().unwrap()).unwrap_err();
        assert!(error.0.contains("depth 16 type 2"), "{error}");
        let text = dir.path().join("x.png");
        fs::write(&text, b"hello world").unwrap();
        assert!(read_png(text.to_str().unwrap())
            .unwrap_err()
            .0
            .contains("not a PNG"));
    }

    #[test]
    fn width_rules_are_enforced() {
        let args: Vec<String> = ["x.png", "out", "6"].map(String::from).to_vec();
        assert!(run(&args).unwrap_err().0.contains("multiple of 4"));
        let args: Vec<String> = ["x.png", "out", "260"].map(String::from).to_vec();
        assert!(run(&args).unwrap_err().0.contains("at most 256"));
    }
}
