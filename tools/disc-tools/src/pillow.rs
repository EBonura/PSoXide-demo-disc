//! The parts of Pillow the disc's asset cookers lean on, ported so the Python
//! dependency can go: PNG decode to RGB, the LANCZOS and NEAREST resizes,
//! `getcolors`, and the median-cut quantiser behind
//! `convert("P", palette=ADAPTIVE)`.
//!
//! The cooked menu shots and icons are pressed into discs and compared by
//! hash, so "close enough" is a failure. Everything here follows Pillow
//! 12.0.0's libImaging (Resample.c, Quant.c, QuantHash.c, QuantHeap.c,
//! GetBBox.c, Geometry.c) step for step, including its fixed-point maths, its
//! tie-breaks and the order its hash table iterates in, because that order
//! decides which colour lands in which palette slot.

use crate::util::{Error, Result};

/// Decode any PNG to 8-bit RGBA, the way Pillow's `convert("RGBA")` would
/// see it: palettes and tRNS expanded, 16-bit samples cut to their high
/// byte, grey replicated. Callers drop the alpha for `convert("RGB")`, which
/// is also what Pillow does (no premultiplication).
pub fn decode_png_rgba(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| Error(format!("png: {e}")))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| Error(format!("png: {e}")))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let raw = &buf[..info.buffer_size()];
    let mut out = Vec::with_capacity(w * h * 4);
    match info.color_type {
        png::ColorType::Rgba => out.extend_from_slice(raw),
        png::ColorType::Rgb => raw
            .chunks_exact(3)
            .for_each(|p| out.extend_from_slice(&[p[0], p[1], p[2], 255])),
        png::ColorType::Grayscale => raw
            .iter()
            .for_each(|&g| out.extend_from_slice(&[g, g, g, 255])),
        png::ColorType::GrayscaleAlpha => raw
            .chunks_exact(2)
            .for_each(|p| out.extend_from_slice(&[p[0], p[0], p[0], p[1]])),
        png::ColorType::Indexed => return Err(Error("png: palette was not expanded".into())),
    }
    Ok((w, h, out))
}

/// RGBA to the packed RGB Pillow's `convert("RGB")` produces.
pub fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

// ---------------------------------------------------------------- resizing

/// Fixed-point fraction bits of a resampling coefficient: 8 bits of result,
/// two spare for filters whose taps sum past one or below zero.
const PRECISION_BITS: u32 = 32 - 8 - 2;

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    let x = x * std::f64::consts::PI;
    x.sin() / x
}

/// Pillow's LANCZOS: a sinc windowed by a wider sinc, support three.
fn lanczos(x: f64) -> f64 {
    if (-3.0..3.0).contains(&x) {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

const LANCZOS_SUPPORT: f64 = 3.0;

/// One output sample's source window and its filter taps, as Pillow's
/// `precompute_coeffs` lays them out, already rounded to fixed point.
struct Taps {
    /// First source index and tap count, per output sample.
    bounds: Vec<(usize, usize)>,
    /// `ksize` taps per output sample, back to back.
    coeffs: Vec<i32>,
    ksize: usize,
}

fn precompute_taps(in_size: usize, out_size: usize) -> Taps {
    // The box is the whole axis, so in0 = 0 and in1 = in_size.
    let scale = in_size as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = LANCZOS_SUPPORT * filterscale;
    let ksize = support.ceil() as usize * 2 + 1;
    let mut bounds = Vec::with_capacity(out_size);
    let mut coeffs = vec![0i32; out_size * ksize];
    let mut weights = vec![0f64; ksize];
    for xx in 0..out_size {
        let center = 0.0 + (xx as f64 + 0.5) * scale;
        let ss = 1.0 / filterscale;
        let xmin = ((center - support + 0.5) as i32).max(0) as usize;
        let xmax = (((center + support + 0.5) as i32).max(0) as usize).min(in_size) - xmin;
        let mut total = 0.0;
        for (x, w) in weights.iter_mut().enumerate().take(xmax) {
            *w = lanczos((x as f64 + xmin as f64 - center + 0.5) * ss);
            total += *w;
        }
        for (x, w) in weights.iter().enumerate() {
            let w = if x < xmax && total != 0.0 {
                w / total
            } else if x < xmax {
                *w
            } else {
                0.0
            };
            let scaled = w * f64::from(1u32 << PRECISION_BITS);
            coeffs[xx * ksize + x] = if w < 0.0 {
                (-0.5 + scaled) as i32
            } else {
                (0.5 + scaled) as i32
            };
        }
        bounds.push((xmin, xmax));
    }
    Taps {
        bounds,
        coeffs,
        ksize,
    }
}

/// Fixed-point sum back to a byte, clamped; Pillow does the clamp with a
/// lookup table over the shifted value.
fn clip8(sum: i32) -> u8 {
    (sum >> PRECISION_BITS).clamp(0, 255) as u8
}

/// Separable LANCZOS resize of interleaved 8-bit samples (`channels` per
/// pixel), horizontal pass first and only over the rows the vertical pass
/// reads, like Pillow's two-pass resampler. An axis whose size does not change
/// is passed through untouched, as Pillow skips it.
pub fn lanczos_resize(
    src: &[u8],
    channels: usize,
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
) -> Vec<u8> {
    assert_eq!(
        src.len(),
        sw * sh * channels,
        "source size does not match its dimensions"
    );
    let htaps = precompute_taps(sw, dw);
    let vtaps = precompute_taps(sh, dh);
    let first_row = vtaps.bounds[0].0;
    let last_row = vtaps.bounds[dh - 1].0 + vtaps.bounds[dh - 1].1;
    let half = 1i32 << (PRECISION_BITS - 1);

    let (mid, mid_w, mid_first) = if dw != sw {
        let rows = last_row - first_row;
        let mut mid = vec![0u8; dw * rows * channels];
        for y in 0..rows {
            let row = &src[(y + first_row) * sw * channels..][..sw * channels];
            for x in 0..dw {
                let (xmin, xmax) = htaps.bounds[x];
                let k = &htaps.coeffs[x * htaps.ksize..][..htaps.ksize];
                for c in 0..channels {
                    let mut sum = half;
                    for i in 0..xmax {
                        sum = sum.wrapping_add(
                            i32::from(row[(i + xmin) * channels + c]).wrapping_mul(k[i]),
                        );
                    }
                    mid[(y * dw + x) * channels + c] = clip8(sum);
                }
            }
        }
        (mid, dw, first_row)
    } else {
        (src.to_vec(), sw, 0)
    };

    if dh == sh {
        return mid;
    }
    let mut out = vec![0u8; dw * dh * channels];
    for y in 0..dh {
        let (ymin, ymax) = vtaps.bounds[y];
        let k = &vtaps.coeffs[y * vtaps.ksize..][..vtaps.ksize];
        for x in 0..mid_w {
            for c in 0..channels {
                let mut sum = half;
                for i in 0..ymax {
                    let v = mid[((i + ymin - mid_first) * mid_w + x) * channels + c];
                    sum = sum.wrapping_add(i32::from(v).wrapping_mul(k[i]));
                }
                out[(y * dw + x) * channels + c] = clip8(sum);
            }
        }
    }
    out
}

/// LANCZOS on packed RGB.
pub fn lanczos_resize_rgb(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    lanczos_resize(src, 3, sw, sh, dw, dh)
}

/// LANCZOS on a single 8-bit channel (the icons' coverage maps).
pub fn lanczos_resize_l(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    lanczos_resize(src, 1, sw, sh, dw, dh)
}

/// Pillow's NEAREST: each output pixel reads the source at the centre of its
/// footprint, `floor(scale * (x + 0.5))`, with the position accumulated the
/// way `ImagingScaleAffine` does.
pub fn nearest_resize_rgb(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    assert_eq!(
        src.len(),
        sw * sh * 3,
        "source size does not match its dimensions"
    );
    let coord = |v: f64| if v < 0.0 { -1 } else { v as i64 };
    let (ax, ay) = (sw as f64 / dw as f64, sh as f64 / dh as f64);
    let mut xs = Vec::with_capacity(dw);
    let mut xo = ax * 0.5;
    for _ in 0..dw {
        xs.push(coord(xo).clamp(0, sw as i64 - 1) as usize);
        xo += ax;
    }
    let mut out = vec![0u8; dw * dh * 3];
    let mut yo = ay * 0.5;
    for y in 0..dh {
        let yi = coord(yo).clamp(0, sh as i64 - 1) as usize;
        for (x, &xi) in xs.iter().enumerate() {
            out[(y * dw + x) * 3..][..3].copy_from_slice(&src[(yi * sw + xi) * 3..][..3]);
        }
        yo += ay;
    }
    out
}

// -------------------------------------------------------------- getcolors

/// `Image.getcolors(maxcolors)` on an RGB image, returning just the colours,
/// in the order Pillow reports them. That order is the order of an
/// open-addressed table (hash is the pixel value itself), not first-seen, and
/// the cooked palette inherits it. `None` means more than `max_colors`.
pub fn get_colors_rgb(rgb: &[u8], max_colors: usize) -> Option<Vec<[u8; 3]>> {
    // (size, polynomial) pairs, from GetBBox.c: first size above max_colors.
    const SIZES: [(u32, u32); 30] = [
        (4, 3),
        (8, 3),
        (16, 3),
        (32, 5),
        (64, 3),
        (128, 3),
        (256, 29),
        (512, 17),
        (1024, 9),
        (2048, 5),
        (4096, 83),
        (8192, 27),
        (16384, 43),
        (32768, 3),
        (65536, 45),
        (131072, 9),
        (262144, 39),
        (524288, 39),
        (1048576, 9),
        (2097152, 5),
        (4194304, 3),
        (8388608, 33),
        (16777216, 27),
        (33554432, 9),
        (67108864, 71),
        (134217728, 39),
        (268435456, 9),
        (536870912, 5),
        (1073741824, 83),
        (0, 0),
    ];
    let &(code_size, code_poly) = SIZES
        .iter()
        .find(|(size, _)| u64::from(*size) > max_colors as u64)?;
    if code_size == 0 {
        return None;
    }
    let mask = code_size - 1;
    // Slot = pixel value (0 means empty, so keep a separate used flag).
    let mut table: Vec<Option<(u32, u32)>> = vec![None; code_size as usize];
    let mut colors = 0usize;
    for p in rgb.chunks_exact(3) {
        let pixel = u32::from(p[0]) | u32::from(p[1]) << 8 | u32::from(p[2]) << 16;
        let h = pixel;
        let mut i = !h & mask;
        let mut incr = 0;
        loop {
            match &mut table[i as usize] {
                None => {
                    colors += 1;
                    if colors > max_colors {
                        return None;
                    }
                    table[i as usize] = Some((pixel, 1));
                    break;
                }
                Some((seen, count)) if *seen == pixel => {
                    *count += 1;
                    break;
                }
                Some(_) => {}
            }
            if incr == 0 {
                incr = (h ^ (h >> 3)) & mask;
                if incr == 0 {
                    incr = mask;
                }
            } else {
                incr <<= 1;
                if incr > mask {
                    incr ^= code_poly;
                }
            }
            i = (i + incr) & mask;
        }
    }
    Some(
        table
            .into_iter()
            .flatten()
            .map(|(pixel, _)| [pixel as u8, (pixel >> 8) as u8, (pixel >> 16) as u8])
            .collect(),
    )
}

// -------------------------------------------------------------- quantiser
//
// `convert("P", palette=ADAPTIVE)` is Pillow's median cut (method 0, no
// k-means). The pieces below mirror Quant.c, QuantHash.c and QuantHeap.c,
// kept deliberately close to the C (linked lists, a chained hash table whose
// buckets are kept sorted) because the iteration orders they produce decide
// ties, and ties decide the palette.

type Px = [u8; 3];

const MAX_HASH_ENTRIES: u32 = 65536;
const HASH_MIN_LENGTH: u32 = 11;
const HASH_RESIZE_FACTOR: u32 = 3;

/// Squared RGB distance, `_DISTSQR`.
fn dist_sqr(a: Px, b: Px) -> u32 {
    let d = |i: usize| {
        let x = i32::from(a[i]) - i32::from(b[i]);
        x * x
    };
    (d(0) + d(1) + d(2)) as u32
}

/// `PIXEL_HASH`: not injective on 24-bit colours, and the table compares
/// these hashes rather than the colours, so two colours that collide count as
/// one. Pillow does the same, so we must.
fn pixel_hash(p: Px, scale: u32) -> u32 {
    let (r, g, b) = (
        u32::from(p[0] >> scale),
        u32::from(p[1] >> scale),
        u32::from(p[2] >> scale),
    );
    r.wrapping_mul(463) ^ (g << 8).wrapping_mul(10069) ^ (b << 16).wrapping_mul(64997)
}

/// `_findPrime`. Despite the name it only filters on the low four bits (the
/// primality test never fires), so the "primes" are not prime; reproduce the
/// sequence, not the intent.
fn find_prime(mut start: u32, dir: i32) -> u32 {
    const UNIT: [bool; 16] = [
        false, true, false, true, false, false, false, true, false, true, false, true, false, true,
        false, false,
    ];
    while start > 1 {
        if UNIT[(start & 0x0f) as usize] {
            break;
        }
        start = start.wrapping_add_signed(dir);
    }
    start
}

/// The pixel-count hash table. Buckets are chains kept in ascending order of
/// the compare function, as the C inserts them.
struct HashTable {
    buckets: Vec<Vec<(Px, u32)>>,
    count: u32,
    scale: u32,
}

impl HashTable {
    fn new() -> Self {
        HashTable {
            buckets: vec![Vec::new(); HASH_MIN_LENGTH as usize],
            count: 0,
            scale: 0,
        }
    }

    fn slot(&self, key: Px) -> usize {
        (pixel_hash(key, self.scale) % self.buckets.len() as u32) as usize
    }

    /// Where `key` sits in its chain: `Ok(i)` if an equal entry is at `i`,
    /// `Err(i)` for the position a new entry would take.
    fn find(&self, key: Px) -> (usize, std::result::Result<usize, usize>) {
        let slot = self.slot(key);
        let want = pixel_hash(key, self.scale);
        let chain = &self.buckets[slot];
        for (i, (existing, _)) in chain.iter().enumerate() {
            let have = pixel_hash(*existing, self.scale);
            if have == want {
                return (slot, Ok(i));
            }
            if have > want {
                return (slot, Err(i));
            }
        }
        (slot, Err(chain.len()))
    }

    fn lookup(&self, key: Px) -> Option<u32> {
        let (slot, found) = self.find(key);
        found.ok().map(|i| self.buckets[slot][i].1)
    }

    /// `hashtable_insert_or_update_computed` with the "count pixels" callbacks:
    /// a new key starts at one, a known key adds one.
    fn count_pixel(&mut self, key: Px) {
        let (slot, found) = self.find(key);
        match found {
            Ok(i) => self.buckets[slot][i].1 += 1,
            Err(i) => {
                self.buckets[slot].insert(i, (key, 1));
                self.count += 1;
                self.resize();
            }
        }
    }

    /// `hashtable_insert`: replace the value of a known key, else add it.
    fn insert(&mut self, key: Px, value: u32) {
        let (slot, found) = self.find(key);
        match found {
            Ok(i) => self.buckets[slot][i].1 = value,
            Err(i) => {
                self.buckets[slot].insert(i, (key, value));
                self.count += 1;
                self.resize();
            }
        }
    }

    fn resize(&mut self) {
        let length = self.buckets.len() as u32;
        let mut new_size = length;
        if self.count * HASH_RESIZE_FACTOR < length {
            new_size = find_prime(length / 2 - 1, -1);
        } else if length * HASH_RESIZE_FACTOR < self.count {
            new_size = find_prime(length * 2 + 1, 1);
        }
        if new_size < HASH_MIN_LENGTH {
            new_size = length;
        }
        if new_size != length {
            self.rehash(new_size, false);
        }
    }

    /// Re-insert every entry into a table of `new_size` buckets. With
    /// `merge`, entries that now compare equal (the scale just went up) add
    /// their counts together.
    fn rehash(&mut self, new_size: u32, merge: bool) {
        let old = std::mem::replace(&mut self.buckets, vec![Vec::new(); new_size as usize]);
        self.count = 0;
        for (key, value) in old.into_iter().flatten() {
            let (slot, found) = self.find(key);
            match found {
                Ok(i) if merge => {
                    self.buckets[slot][i].0 = key;
                    self.buckets[slot][i].1 += value;
                }
                Ok(i) => self.buckets[slot][i] = (key, value),
                Err(i) => {
                    self.buckets[slot].insert(i, (key, value));
                    self.count += 1;
                }
            }
        }
    }

    /// Entries in table order: bucket by bucket, chain order.
    fn entries(&self) -> impl Iterator<Item = (Px, u32)> + '_ {
        self.buckets.iter().flatten().copied()
    }
}

/// `create_pixel_hash`: count every pixel, coarsening the key (shift the
/// colours right one more bit) whenever the table outgrows its limit.
fn create_pixel_hash(pixels: &[Px]) -> HashTable {
    let mut table = HashTable::new();
    for &pixel in pixels {
        table.count_pixel(pixel);
        while table.count > MAX_HASH_ENTRIES {
            table.scale += 1;
            let length = table.buckets.len() as u32;
            table.rehash(length, true);
        }
    }
    table
}

const NIL: usize = usize::MAX;

/// One distinct (coarsened) colour with its pixel count, threaded onto three
/// doubly linked lists, one sorted by each of R, G and B (largest first).
#[derive(Clone, Copy)]
struct PixelList {
    next: [usize; 3],
    prev: [usize; 3],
    px: Px,
    flag: bool,
    count: u32,
}

/// A median-cut box: the three sorted lists of the colours inside it.
struct BoxNode {
    l: usize,
    r: usize,
    head: [usize; 3],
    tail: [usize; 3],
    volume: i32,
    pixel_count: u32,
}

struct Cutter {
    lists: Vec<PixelList>,
    boxes: Vec<BoxNode>,
}

impl Cutter {
    /// `mergesort_pixels`: sort list `axis` descending. Equal values keep the
    /// right-hand run first, which is what the C does, and it matters.
    fn mergesort(&mut self, head: usize, axis: usize) -> usize {
        let i = axis;
        if head == NIL || self.lists[head].next[i] == NIL {
            if head != NIL {
                self.lists[head].next[i] = NIL;
                self.lists[head].prev[i] = NIL;
            }
            return head;
        }
        let (mut c, mut t) = (head, head);
        while c != NIL && t != NIL {
            c = self.lists[c].next[i];
            let tn = self.lists[t].next[i];
            t = if tn != NIL {
                self.lists[tn].next[i]
            } else {
                NIL
            };
        }
        if c != NIL {
            let before = self.lists[c].prev[i];
            if before != NIL {
                self.lists[before].next[i] = NIL;
            }
            self.lists[c].prev[i] = NIL;
        }
        let mut a = self.mergesort(head, i);
        let mut b = self.mergesort(c, i);
        let (mut merged, mut p, mut last) = (NIL, NIL, NIL);
        while a != NIL && b != NIL {
            if self.lists[a].px[i] > self.lists[b].px[i] {
                last = a;
                a = self.lists[a].next[i];
            } else {
                last = b;
                b = self.lists[b].next[i];
            }
            self.lists[last].prev[i] = p;
            self.lists[last].next[i] = NIL;
            if p != NIL {
                self.lists[p].next[i] = last;
            }
            p = last;
            if merged == NIL {
                merged = last;
            }
        }
        let rest = if a != NIL { a } else { b };
        if rest != NIL {
            self.lists[last].next[i] = rest;
            self.lists[rest].prev[i] = last;
        }
        merged
    }

    /// `compute_box_volume`, cached in the node.
    fn volume(&mut self, node: usize) -> i32 {
        if self.boxes[node].volume >= 0 {
            return self.boxes[node].volume;
        }
        let b = &self.boxes[node];
        let volume = if b.head[0] == NIL {
            0
        } else {
            let hi = |i: usize| i32::from(self.lists[b.head[i]].px[i]);
            let lo = |i: usize| i32::from(self.lists[b.tail[i]].px[i]);
            (hi(0) - lo(0) + 1) * (hi(1) - lo(1) + 1) * (hi(2) - lo(2) + 1)
        };
        self.boxes[node].volume = volume;
        volume
    }
}

impl Cutter {
    /// `splitlists` + the bookkeeping of `split`: cut `node` along its widest
    /// luminance-weighted axis at the count median, never splitting equal
    /// values apart, and hand each half its three lists in sorted order.
    fn split(&mut self, node: usize) {
        let (head, tail, pixel_count) = {
            let b = &self.boxes[node];
            (b.head, b.tail, b.pixel_count)
        };
        let span =
            |i: usize| i32::from(self.lists[head[i]].px[i]) - i32::from(self.lists[tail[i]].px[i]);
        let f = [span(0) * 77, span(1) * 150, span(2) * 29];
        let mut axis = 0;
        let mut best = f[0];
        for (i, &v) in f.iter().enumerate().skip(1) {
            if best < v {
                best = v;
                axis = i;
            }
        }

        let value = |s: &Self, c: usize| s.lists[c].px[axis];
        let mut counts = [0u32; 2];
        let mut left = 0u32;
        let mut c = head[axis];
        while c != NIL {
            left = left.wrapping_add(self.lists[c].count);
            counts[0] = counts[0].wrapping_add(self.lists[c].count);
            self.lists[c].flag = false;
            c = self.lists[c].next[axis];
            if left.wrapping_mul(2) > pixel_count {
                break;
            }
        }
        if c != NIL {
            let split_value = value(self, self.lists[c].prev[axis]);
            while c != NIL {
                if split_value != value(self, c) {
                    break;
                }
                self.lists[c].flag = false;
                counts[0] = counts[0].wrapping_add(self.lists[c].count);
                c = self.lists[c].next[axis];
            }
        }
        let mut n_right = 0;
        while c != NIL {
            self.lists[c].flag = true;
            n_right += 1;
            counts[1] = counts[1].wrapping_add(self.lists[c].count);
            c = self.lists[c].next[axis];
        }
        if n_right == 0 {
            // Everything fell on the left: peel the run of equal values at the
            // tail off to the right so the box really splits.
            let split_value = value(self, tail[axis]);
            let mut c = tail[axis];
            while c != NIL {
                if split_value != value(self, c) {
                    break;
                }
                self.lists[c].flag = true;
                counts[0] = counts[0].wrapping_sub(self.lists[c].count);
                counts[1] = counts[1].wrapping_add(self.lists[c].count);
                c = self.lists[c].prev[axis];
            }
        }

        // Stable partition of each axis's list by the flag.
        let mut heads = [[NIL; 3]; 2];
        let mut tails = [[NIL; 3]; 2];
        for i in 0..3 {
            let mut last = [NIL; 2];
            let mut c = head[i];
            while c != NIL {
                let n = self.lists[c].next[i];
                let side = usize::from(self.lists[c].flag);
                if last[side] != NIL {
                    self.lists[last[side]].next[i] = c;
                } else {
                    heads[side][i] = c;
                }
                self.lists[c].prev[i] = last[side];
                last[side] = c;
                c = n;
            }
            for side in 0..2 {
                if last[side] != NIL {
                    self.lists[last[side]].next[i] = NIL;
                }
                tails[side][i] = last[side];
            }
        }

        let first = self.boxes.len();
        for side in 0..2 {
            self.boxes.push(BoxNode {
                l: NIL,
                r: NIL,
                head: heads[side],
                tail: tails[side],
                volume: -1,
                pixel_count: counts[side],
            });
        }
        let b = &mut self.boxes[node];
        b.head = [NIL; 3];
        b.tail = [NIL; 3];
        b.l = first;
        b.r = first + 1;
    }
}

/// `box_heap_cmp`: boxes with more pixels come out of the heap first.
fn box_cmp(boxes: &[BoxNode], a: usize, b: usize) -> i32 {
    boxes[a].pixel_count as i32 - boxes[b].pixel_count as i32
}

/// `QuantHeap.c`: a binary max-heap, one-based, with the C's exact sift rules
/// (equal boxes are not swapped), since which box is cut next depends on them.
struct Heap {
    items: Vec<usize>,
    count: usize,
}

impl Heap {
    fn new() -> Self {
        Heap {
            items: vec![0; 2],
            count: 0,
        }
    }

    fn set(&mut self, k: usize, v: usize) {
        if k >= self.items.len() {
            self.items.resize(k + 1, 0);
        }
        self.items[k] = v;
    }

    fn add(&mut self, boxes: &[BoxNode], val: usize) {
        self.count += 1;
        let mut k = self.count;
        while k != 1 {
            if box_cmp(boxes, val, self.items[k / 2]) <= 0 {
                break;
            }
            let parent = self.items[k / 2];
            self.set(k, parent);
            k >>= 1;
        }
        self.set(k, val);
    }

    fn remove(&mut self, boxes: &[BoxNode]) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        let top = self.items[1];
        let v = self.items[self.count];
        self.count -= 1;
        let mut k = 1;
        while k * 2 <= self.count {
            let mut l = k * 2;
            if l < self.count && box_cmp(boxes, self.items[l], self.items[l + 1]) < 0 {
                l += 1;
            }
            if box_cmp(boxes, v, self.items[l]) > 0 {
                break;
            }
            self.items[k] = self.items[l];
            k = l;
        }
        self.set(k, v);
        Some(top)
    }
}

impl Cutter {
    /// `median_cut`: split the fullest box (by pixel count) until there are
    /// `n_colors` boxes, skipping boxes that already hold a single colour.
    /// Returns the root; the leaves are the palette boxes.
    fn median_cut(&mut self, heads: [usize; 3], pixel_count: u32, n_colors: usize) -> usize {
        let mut tails = [NIL; 3];
        for i in 0..3 {
            let mut t = heads[i];
            while t != NIL && self.lists[t].next[i] != NIL {
                t = self.lists[t].next[i];
            }
            tails[i] = t;
        }
        let root = self.boxes.len();
        self.boxes.push(BoxNode {
            l: NIL,
            r: NIL,
            head: heads,
            tail: tails,
            volume: -1,
            pixel_count,
        });
        let mut heap = Heap::new();
        heap.add(&self.boxes, root);
        let mut remaining = n_colors;
        loop {
            remaining -= 1;
            if remaining == 0 {
                break;
            }
            let node = loop {
                let Some(node) = heap.remove(&self.boxes) else {
                    return root;
                };
                if self.volume(node) != 1 {
                    break node;
                }
            };
            self.split(node);
            let (l, r) = (self.boxes[node].l, self.boxes[node].r);
            heap.add(&self.boxes, l);
            heap.add(&self.boxes, r);
        }
        root
    }

    /// `annotate_hash_table`: number the leaf boxes left to right and record,
    /// for every colour in a leaf, which box it fell in.
    fn annotate(&self, node: usize, table: &mut HashTable, next_box: &mut u32) {
        let b = &self.boxes[node];
        if b.l != NIL && b.r != NIL {
            self.annotate(b.l, table, next_box);
            self.annotate(b.r, table, next_box);
            return;
        }
        let mut p = b.head[0];
        while p != NIL {
            let px = self.lists[p].px;
            let unscaled = [
                ((u32::from(px[0])) << table.scale) as u8,
                ((u32::from(px[1])) << table.scale) as u8,
                ((u32::from(px[2])) << table.scale) as u8,
            ];
            table.insert(unscaled, *next_box);
            p = self.lists[p].next[0];
        }
        if b.head[0] != NIL {
            *next_box += 1;
        }
    }
}

/// `quantize` in Quant.c with kmeans = 0: median cut, box-average palette,
/// then every pixel mapped to its nearest palette entry.
fn quantize_median_cut(pixels: &[Px], n_colors: usize) -> (Vec<Px>, Vec<u8>) {
    let mut table = create_pixel_hash(pixels);
    let mut cutter = Cutter {
        lists: Vec::new(),
        boxes: Vec::new(),
    };
    let mut heads = [NIL; 3];
    // hash_to_list: every entry is pushed on the front of all three lists.
    for (key, count) in table.entries().collect::<Vec<_>>() {
        let q = [
            key[0] >> table.scale,
            key[1] >> table.scale,
            key[2] >> table.scale,
        ];
        let id = cutter.lists.len();
        cutter.lists.push(PixelList {
            next: heads,
            prev: [NIL; 3],
            px: q,
            flag: false,
            count,
        });
        for (i, head) in heads.iter_mut().enumerate() {
            if *head != NIL {
                cutter.lists[*head].prev[i] = id;
            }
            *head = id;
        }
    }
    for (i, head) in heads.iter_mut().enumerate() {
        *head = cutter.mergesort(*head, i);
    }
    let root = cutter.median_cut(heads, pixels.len() as u32, n_colors);
    let mut n_palette = 0u32;
    cutter.annotate(root, &mut table, &mut n_palette);
    let n_palette = n_palette as usize;

    // compute_palette_from_median_cut: mean colour of each box's pixels.
    let mut sums = vec![[0u32; 3]; n_palette];
    let mut counts = vec![0u32; n_palette];
    let box_of: Vec<usize> = pixels
        .iter()
        .map(|&p| table.lookup(p).expect("pixel is in a box") as usize)
        .collect();
    for (&p, &entry) in pixels.iter().zip(&box_of) {
        for i in 0..3 {
            sums[entry][i] += u32::from(p[i]);
        }
        counts[entry] += 1;
    }
    let palette: Vec<Px> = (0..n_palette)
        .map(|e| {
            let mean = |i: usize| (0.5 + f64::from(sums[e][i]) / f64::from(counts[e])) as u8;
            [mean(0), mean(1), mean(2)]
        })
        .collect();

    // build_distance_tables: for each entry, every entry ordered by distance
    // then index, so the nearest-colour search can stop early.
    let dist: Vec<Vec<u32>> = palette
        .iter()
        .map(|&a| palette.iter().map(|&b| dist_sqr(a, b)).collect())
        .collect();
    let order: Vec<Vec<usize>> = dist
        .iter()
        .map(|row| {
            let mut idx: Vec<usize> = (0..n_palette).collect();
            idx.sort_by_key(|&j| (row[j], j));
            idx
        })
        .collect();

    // map_image_pixels_from_median_box, with the per-colour cache.
    let mut cache: std::collections::HashMap<Px, u8> = std::collections::HashMap::new();
    let indices = pixels
        .iter()
        .zip(&box_of)
        .map(|(&p, &start)| {
            *cache.entry(p).or_insert_with(|| {
                let mut best_dist = dist_sqr(palette[start], p);
                let mut best = start;
                let limit = best_dist << 2;
                for &j in &order[start] {
                    if dist[start][j] > limit {
                        break;
                    }
                    let d = dist_sqr(palette[j], p);
                    if d < best_dist {
                        best_dist = d;
                        best = j;
                    }
                }
                best as u8
            })
        })
        .collect();
    (palette, indices)
}

/// `rgb.convert("P", palette=Image.ADAPTIVE, colors=colors)` for packed RGB
/// (`width * height * 3` bytes): the palette (at most `colors` entries) and
/// one palette index per pixel.
pub fn quantize_adaptive_rgb(rgb: &[u8], colors: usize) -> Result<(Vec<[u8; 3]>, Vec<u8>)> {
    if !(1..=256).contains(&colors) {
        return Err(Error("bad number of colors".into()));
    }
    if rgb.is_empty() || !rgb.len().is_multiple_of(3) {
        return Err(Error("nothing to quantize".into()));
    }
    let pixels: Vec<Px> = rgb.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
    Ok(quantize_median_cut(&pixels, colors))
}

// Expected values below were taken from Pillow 12.0.0 (Image.resize,
// Image.getcolors, Image.convert) on the same synthetic inputs.
#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(len: usize, a: usize, b: usize, c: usize) -> Vec<u8> {
        (0..len).map(|i| ((i * a + b) % c) as u8).collect()
    }

    #[test]
    fn lanczos_rgb_shrinks_like_pillow() {
        let src = pattern(7 * 5 * 3, 37, 11, 251);
        let got = lanczos_resize_rgb(&src, 7, 5, 3, 2);
        assert_eq!(
            got,
            [
                86, 114, 152, 120, 106, 129, 150, 139, 115, 131, 130, 121, 96, 139, 138, 127, 97,
                135
            ]
        );
    }

    #[test]
    fn lanczos_rgb_enlarges_like_pillow() {
        let src = pattern(3 * 3 * 3, 37, 11, 251);
        let got = lanczos_resize_rgb(&src, 3, 3, 7, 5);
        #[rustfmt::skip]
        let want = [
            0, 21, 51, 10, 57, 98, 39, 113, 174, 108, 145, 217, 191, 95, 156, 243, 29, 70, 255, 0, 20,
            10, 40, 99, 43, 86, 109, 101, 160, 126, 166, 203, 132, 186, 142, 108, 173, 58, 81, 164,
            12, 65, 71, 108, 192, 109, 146, 150, 170, 207, 79, 204, 241, 27, 152, 189, 61, 81, 118, 122,
            39, 76, 160, 165, 203, 248, 151, 187, 210, 126, 160, 126, 99, 136, 65, 92, 142, 108, 99,
            159, 182, 104, 170, 228, 212, 248, 255, 160, 198, 239, 74, 112, 173, 14, 51, 123, 61, 94,
            155, 140, 170, 211, 189, 216, 247,
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn lanczos_single_channel_matches_pillow() {
        let src = pattern(16 * 12, 37, 11, 251);
        let got = lanczos_resize_l(&src, 16, 12, 4, 3);
        assert_eq!(
            got,
            [122, 127, 121, 125, 137, 121, 122, 127, 128, 120, 123, 127]
        );
    }

    #[test]
    fn same_size_is_left_alone() {
        let src = pattern(5 * 4 * 3, 7, 3, 256);
        assert_eq!(lanczos_resize_rgb(&src, 5, 4, 5, 4), src);
    }

    #[test]
    fn nearest_samples_the_footprint_centre() {
        let a = pattern(6 * 4 * 3, 29, 5, 256);
        assert_eq!(
            nearest_resize_rgb(&a, 6, 4, 2, 2),
            [102, 131, 160, 107, 136, 165, 122, 151, 180, 127, 156, 185]
        );
        let b = pattern(9 * 6 * 3, 29, 5, 256);
        assert_eq!(
            nearest_resize_rgb(&b, 9, 6, 3, 2),
            [
                107, 136, 165, 112, 141, 170, 117, 146, 175, 152, 181, 210, 157, 186, 215, 162,
                191, 220
            ]
        );
    }

    #[test]
    fn getcolors_reports_table_order_not_first_seen() {
        let cols: [[u8; 3]; 8] = [
            [0, 0, 0],
            [255, 255, 255],
            [10, 20, 30],
            [200, 100, 50],
            [1, 2, 3],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
        ];
        let rgb: Vec<u8> = (0..24).flat_map(|i| cols[(i * 5) % 8]).collect();
        let got = get_colors_rgb(&rgb, 256).unwrap();
        // First seen would start [0,0,0], [255,0,0]; Pillow's table does not.
        let want: Vec<[u8; 3]> = vec![
            [255, 255, 255],
            [1, 2, 3],
            [0, 255, 0],
            [255, 0, 0],
            [200, 100, 50],
            [10, 20, 30],
            [0, 0, 255],
            [0, 0, 0],
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn getcolors_gives_up_past_the_limit() {
        let rgb: Vec<u8> = (0..257u32)
            .flat_map(|i| [i as u8, (i >> 8) as u8, 7])
            .collect();
        assert!(get_colors_rgb(&rgb, 256).is_none());
        assert_eq!(get_colors_rgb(&rgb[..256 * 3], 256).unwrap().len(), 256);
    }

    fn quant_input() -> Vec<u8> {
        (0..64usize)
            .flat_map(|i| [(i * 37) % 256, (i * 91 + 13) % 256, (i * 53 + 101) % 256])
            .map(|v| v as u8)
            .collect()
    }

    #[test]
    fn median_cut_five_colours_matches_pillow() {
        let (palette, indices) = quantize_adaptive_rgb(&quant_input(), 5).unwrap();
        assert_eq!(
            palette,
            [
                [181, 216, 91],
                [49, 220, 166],
                [119, 150, 136],
                [187, 58, 145],
                [57, 58, 97]
            ]
        );
        #[rustfmt::skip]
        let want = [
            4, 4, 1, 4, 2, 0, 3, 1, 1, 4, 2, 0, 3, 0, 4, 4, 1, 3, 2, 0, 3, 1, 1, 4, 2, 0, 3, 3, 4, 4, 2, 3,
            0, 0, 3, 1, 1, 4, 2, 0, 3, 3, 1, 4, 2, 3, 3, 0, 3, 4, 1, 3, 2, 0, 3, 3, 1, 4, 2, 3, 3, 0, 3, 4,
        ];
        assert_eq!(indices, want);
    }

    #[test]
    fn median_cut_seven_colours_matches_pillow() {
        let (palette, indices) = quantize_adaptive_rgb(&quant_input(), 7).unwrap();
        let want_palette: [[u8; 3]; 7] = [
            [181, 216, 91],
            [49, 220, 166],
            [175, 151, 120],
            [46, 150, 156],
            [185, 84, 154],
            [191, 23, 134],
            [57, 58, 97],
        ];
        assert_eq!(palette, want_palette);
        assert_eq!(
            &indices[..16],
            [6, 3, 1, 6, 2, 0, 5, 3, 1, 6, 2, 0, 4, 0, 6, 6]
        );
        assert_eq!(
            &indices[48..],
            [5, 3, 1, 4, 2, 0, 4, 4, 1, 6, 3, 5, 4, 0, 5, 3]
        );
    }

    #[test]
    fn quantiser_rejects_bad_requests() {
        assert!(quantize_adaptive_rgb(&quant_input(), 0).is_err());
        assert!(quantize_adaptive_rgb(&quant_input(), 257).is_err());
        assert!(quantize_adaptive_rgb(&[], 8).is_err());
    }

    #[test]
    fn decodes_grey_and_alpha_pngs_like_convert_rgba() {
        let mut bytes = Vec::new();
        let mut enc = png::Encoder::new(&mut bytes, 2, 1);
        enc.set_color(png::ColorType::GrayscaleAlpha);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .unwrap()
            .write_image_data(&[10, 200, 30, 0])
            .unwrap();
        let (w, h, rgba) = decode_png_rgba(&bytes).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, [10, 10, 10, 200, 30, 30, 30, 0]);
        assert_eq!(rgba_to_rgb(&rgba), [10, 10, 10, 30, 30, 30]);
    }
}
