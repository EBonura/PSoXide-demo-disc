//! On-disc label for the demo disc: a 120 mm SVG with the 15 mm hole cut out.
//!
//!     disc-tools disc-label [--hl] [--link "LABEL=URL"]... > assets/label/disc-label.svg
//!
//! Up to two --link entries become QR codes left/right of the hub with the URL
//! printed under each (dark modules on a light panel: inverted QRs scan badly).
//!
//! Everything is in millimetres (viewBox = mm, width/height carry the unit), so
//! a print shop or a hub-printable CD-R template can drop it in at 1:1. Text
//! stays outside the 42 mm stacking ring; the background bleeds to the hole so a
//! hub-printable disc has no white collar.
//!
//! The SVG is byte-identical to what the Python original wrote, which is why
//! the stars come from a Mersenne Twister seeded the way CPython seeds
//! `random.Random(1)`, and why floats are printed the way Python's f-strings
//! and `repr` print them.

use std::f64::consts::{PI, TAU};
use std::io::Write;
use std::path::PathBuf;

use qrcode::bits::Bits;
use qrcode::canvas::{Canvas, MaskPattern};
use qrcode::ec;
use qrcode::types::Color;
use qrcode::{EcLevel, Version};
use regex::Regex;

use crate::args::Args;
use crate::util::{Error, Result};

const GAMES: [&str; 9] = [
    "CORTEX IGNITION",
    "QUAKE SHAREWARE",
    "VOXIDE",
    "NITROXIDE",
    "CELESTE COLLECTION",
    "PSXCEL",
    "BREAKOUT",
    "SPACE INVADERS",
    "MAGIKAAAAARP PONG",
];

const CX: f64 = 60.0;
const CY: f64 = 60.0;
/// Millimetres; the hub is the stacking ring's safe zone.
const R_DISC: f64 = 60.0;
const R_HOLE: f64 = 7.5;
const R_HUB: f64 = 21.0;
const CYAN: &str = "#00c4ec";
const INK: &str = "#ffffff";
const DIM: &str = "#8e9aa8";

/// The eight data masks in ISO 18004 order (mask 0 to 7).
const MASKS: [MaskPattern; 8] = [
    MaskPattern::Checkerboard,
    MaskPattern::HorizontalLines,
    MaskPattern::VerticalLines,
    MaskPattern::DiagonalLines,
    MaskPattern::LargeCheckerboard,
    MaskPattern::Fields,
    MaskPattern::Diamonds,
    MaskPattern::Meadow,
];

/// The brand wordmark the Python read from the editor's branding assets.
fn logo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../games/PSoXide-editor/assets/branding/psoxide-logo.svg")
}

// ---------------------------------------------------------------------------
// Python's random.Random, just enough of it for the star field.
// ---------------------------------------------------------------------------

/// MT19937 seeded exactly as CPython seeds `random.Random(int)`: the integer's
/// magnitude is split into 32-bit words and fed to `init_by_array`.
struct PyRandom {
    state: [u32; 624],
    index: usize,
}

impl PyRandom {
    fn new(seed: u64) -> PyRandom {
        let mut key = vec![seed as u32];
        if seed >> 32 != 0 {
            key.push((seed >> 32) as u32);
        }
        let mut rng = PyRandom {
            state: [0; 624],
            index: 624,
        };
        rng.init_genrand(19_650_218);
        rng.init_by_array(&key);
        rng
    }

    fn init_genrand(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..624 {
            let previous = self.state[i - 1];
            self.state[i] = 1_812_433_253u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(i as u32);
        }
        self.index = 624;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..624.max(key.len()) {
            let previous = self.state[i - 1];
            self.state[i] = (self.state[i] ^ (previous ^ (previous >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                self.state[0] = self.state[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            let previous = self.state[i - 1];
            self.state[i] = (self.state[i]
                ^ (previous ^ (previous >> 30)).wrapping_mul(1_566_083_941))
            .wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                self.state[0] = self.state[623];
                i = 1;
            }
        }
        self.state[0] = 0x8000_0000;
        self.index = 624;
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for k in 0..624 {
                let y = (self.state[k] & 0x8000_0000) | (self.state[(k + 1) % 624] & 0x7FFF_FFFF);
                let mut next = self.state[(k + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    next ^= 0x9908_B0DF;
                }
                self.state[k] = next;
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9D2C_5680;
        y ^= (y << 15) & 0xEFC6_0000;
        y ^= y >> 18;
        y
    }

    /// `random.random()`: 53 bits from two draws.
    fn random(&mut self) -> f64 {
        let a = (self.next_u32() >> 5) as f64;
        let b = (self.next_u32() >> 6) as f64;
        (a * 67_108_864.0 + b) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// `random.uniform(a, b)`.
    fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.random()
    }

    /// `random.getrandbits(k)` for k up to 32.
    fn getrandbits(&mut self, k: u32) -> u32 {
        self.next_u32() >> (32 - k)
    }

    /// `random.choice` over `len` items: `_randbelow` by rejection sampling.
    fn choice_index(&mut self, len: usize) -> usize {
        let bits = usize::BITS - len.leading_zeros();
        loop {
            let r = self.getrandbits(bits) as usize;
            if r < len {
                return r;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Python-style number printing.
// ---------------------------------------------------------------------------

/// Python's `repr(float)` for the magnitudes the label uses: the shortest text
/// that reads back to the same value, always with a point or exponent (so a
/// whole number prints as `60.0`, not `60`).
fn repr(x: f64) -> String {
    let text = format!("{x}");
    if text.contains('.') || text.contains("inf") || text.contains("NaN") {
        text
    } else {
        format!("{text}.0")
    }
}

/// Python's `sum()` of floats (Neumaier compensation since 3.12).
fn float_sum(values: &[f64]) -> f64 {
    let mut total = 0.0f64;
    let mut compensation = 0.0f64;
    for &x in values {
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

/// `math.degrees`, which CPython computes as `x * (180 / pi)`.
fn degrees(x: f64) -> f64 {
    x * (180.0 / PI)
}

/// `xml.sax.saxutils.escape`.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// The artwork.
// ---------------------------------------------------------------------------

/// The `<defs>` and the `<g>` from the brand SVG, minus the tagline text.
fn wordmark(svg: &str) -> Result<(String, String)> {
    let svg = svg.replace("\r\n", "\n").replace('\r', "\n");
    let defs = Regex::new(r"(?s)<defs>.*?</defs>")?;
    let group = Regex::new(r"(?s)<g transform=.*?</g>")?;
    let (Some(defs), Some(group)) = (defs.find(&svg), group.find(&svg)) else {
        bail!("the brand logo has no <defs> and <g transform=...> to reuse");
    };
    Ok((defs.as_str().to_string(), group.as_str().to_string()))
}

fn stars(n: usize, seed: u64) -> String {
    const SIZES: [f64; 5] = [0.08, 0.1, 0.12, 0.16, 0.22];
    let mut rnd = PyRandom::new(seed);
    let mut out = Vec::new();
    while out.len() < n {
        let x = rnd.uniform(0.0, 120.0);
        let y = rnd.uniform(0.0, 120.0);
        let d = ((x - CX) * (x - CX) + (y - CY) * (y - CY)).powf(0.5);
        if R_HUB + 1.0 < d && d < R_DISC - 1.0 {
            let r = SIZES[rnd.choice_index(SIZES.len())];
            let a = rnd.uniform(0.35, 1.0);
            out.push(format!(
                "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"{}\" fill=\"#fff\" opacity=\"{a:.2}\"/>",
                repr(r)
            ));
        }
    }
    out.join("\n")
}

fn rings() -> String {
    let mut out = Vec::new();
    for r in (24..60).step_by(4) {
        let a = 0.05 + 0.10 * (r - 24) as f64 / 36.0;
        out.push(format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{r}\" fill=\"none\" stroke=\"{CYAN}\" stroke-width=\"0.12\" opacity=\"{a:.2}\"/>",
            repr(CX),
            repr(CY)
        ));
    }
    for i in 0..24 {
        let t = TAU * i as f64 / 24.0;
        let (x1, y1) = (CX + R_HUB * t.cos(), CY + R_HUB * t.sin());
        let (x2, y2) = (CX + R_DISC * t.cos(), CY + R_DISC * t.sin());
        out.push(format!(
            "<line x1=\"{x1:.2}\" y1=\"{y1:.2}\" x2=\"{x2:.2}\" y2=\"{y2:.2}\" stroke=\"{CYAN}\" stroke-width=\"0.12\" opacity=\"0.10\"/>"
        ));
    }
    out.join("\n")
}

/// Glyph advances are eyeballed fractions of the font size, not real metrics.
/// Good enough for a title ring; librsvg has no <textPath>, so baking per-glyph
/// transforms is what makes this render in every tool.
fn advance_fraction(c: char) -> f64 {
    match c {
        ' ' => 0.30,
        '·' => 0.36,
        'I' => 0.30,
        'J' => 0.52,
        'L' => 0.60,
        '-' => 0.40,
        _ => 0.66,
    }
}

/// Text laid clockwise around the centre, starting at 12 o'clock.
fn ring_text(text: &str, radius: f64, size: f64, fill: &str, gap_deg: f64) -> String {
    let glyphs: Vec<char> = text.chars().collect();
    let adv: Vec<f64> = glyphs.iter().map(|&c| size * advance_fraction(c)).collect();
    let total = float_sum(&adv);
    let span = degrees(total / radius) - gap_deg;
    let mut a = -span / 2.0; // centre the run on 12 o'clock
    let mut out = Vec::new();
    for (&c, &w) in glyphs.iter().zip(&adv) {
        let step = degrees(w / radius);
        let mid = a + step / 2.0;
        if c != ' ' {
            out.push(format!(
                "<text x=\"0\" y=\"0\" text-anchor=\"middle\" fill=\"{fill}\" font-size=\"{}\" transform=\"translate({} {}) rotate({mid:.3}) translate(0 {})\">{}</text>",
                repr(size),
                repr(CX),
                repr(CY),
                repr(-radius),
                escape(&c.to_string())
            ));
        }
        a += step;
    }
    out.join("\n")
}

/// `bytes.find(pattern, from)` over a slice of 0/1 modules.
fn find_from(seq: &[u8], pattern: &[u8], from: usize) -> Option<usize> {
    if from > seq.len() || seq.len() - from < pattern.len() {
        return None;
    }
    (from..=seq.len() - pattern.len()).find(|&at| &seq[at..at + pattern.len()] == pattern)
}

/// Penalty for the dark-light-dark-dark-dark-light-dark finder lookalike in
/// one row or column: it counts when it touches the symbol edge or has four
/// light modules on either side.
fn finder_lookalikes(seq: &[u8]) -> u32 {
    const PATTERN: [u8; 7] = [1, 0, 1, 1, 1, 0, 1];
    let size = seq.len();
    let mut count = 0;
    let mut found = find_from(seq, &PATTERN, 0);
    while let Some(at) = found {
        let mut offset = at + 7;
        let before_clear = !seq[at.saturating_sub(4)..at.min(size)]
            .iter()
            .any(|&m| m != 0);
        let after_clear = !seq[offset.min(size)..(offset + 4).min(size)]
            .iter()
            .any(|&m| m != 0);
        if at == 0 || at == size - 7 || before_clear || after_clear {
            count += 40;
        } else {
            // Not enough light around it: resume at the next possible match.
            offset = at + 4;
        }
        found = find_from(seq, &PATTERN, offset);
    }
    count
}

/// The four ISO 18004 mask penalties summed, scored the way segno scores them.
/// `matrix` holds 0/1 modules with the format and version areas still light.
fn penalty(matrix: &[Vec<u8>]) -> u32 {
    let size = matrix.len();
    let (mut n1, mut n2, mut n3, mut dark) = (0u32, 0u32, 0u32, 0u32);
    let mut column = vec![0u8; size];
    for i in 0..size {
        let row = &matrix[i];
        let (mut row_prev, mut col_prev) = (-1i32, -1i32);
        let (mut row_run, mut col_run) = (0u32, 0u32);
        for j in 0..size {
            let (row_bit, col_bit) = (row[j] as i32, matrix[j][i] as i32);
            column[j] = matrix[j][i];
            dark += row[j] as u32;
            if row_bit == row_prev {
                row_run += 1;
            } else {
                if row_run >= 5 {
                    n1 += row_run - 2;
                }
                row_run = 1;
            }
            if col_bit == col_prev {
                col_run += 1;
            } else {
                if col_run >= 5 {
                    n1 += col_run - 2;
                }
                col_run = 1;
            }
            if i > 0 && j > 0 {
                let above = &matrix[i - 1];
                if row_bit == row_prev
                    && row_bit == above[j] as i32
                    && row_bit == above[j - 1] as i32
                {
                    n2 += 3;
                }
            }
            row_prev = row_bit;
            col_prev = col_bit;
        }
        n3 += finder_lookalikes(row) + finder_lookalikes(&column);
        if row_run >= 5 {
            n1 += row_run - 2;
        }
        if col_run >= 5 {
            n1 += col_run - 2;
        }
    }
    let percent = dark as f64 / (size * size) as f64;
    let n4 = 10 * (((percent * 100.0 - 50.0).abs() / 5.0) as u32);
    n1 + n2 + n3 + n4
}

/// Blank the format and version areas (and the dark module that sits in the
/// format column), which segno leaves light while it scores the masks.
fn clear_reserved(matrix: &mut [Vec<u8>], version: i16) {
    let size = matrix.len();
    // Index 6 of the format strip is the timing pattern, which stays.
    for i in (0..9).filter(|&i| i != 6) {
        matrix[i][8] = 0;
        matrix[8][i] = 0;
    }
    for i in 1..9 {
        matrix[size - i][8] = 0;
        matrix[8][size - i] = 0;
    }
    if version >= 7 {
        for k in 8..11 {
            for row in matrix.iter_mut().take(6) {
                row[size - k - 1] = 0;
            }
            matrix[size - k - 1][..6].fill(0);
        }
    }
}

/// The data codewords for one byte-mode segment, laid out the way segno lays
/// them out. That is the standard stream (mode, count, bytes, a terminator of up
/// to four zero bits, pad codewords 0xEC and 0x11 in turn) except for one quirk:
/// segno always pads to the byte boundary with `8 - len % 8` zero bits, so a
/// stream that already ends on a boundary gets a whole extra zero byte before
/// the pad codewords. It scans identically, but the bits, and therefore the
/// mask the penalty rules pick, are not the ones a strict encoder produces.
fn segno_codewords(data: &[u8], count_bits: usize, capacity_bits: usize) -> Vec<u8> {
    let mut stream: Vec<bool> = Vec::new();
    let mut push = |value: usize, width: usize| {
        for shift in (0..width).rev() {
            stream.push(value >> shift & 1 == 1);
        }
    };
    push(0b0100, 4);
    push(data.len(), count_bits);
    for &byte in data {
        push(byte as usize, 8);
    }
    let terminator = (capacity_bits - stream.len()).min(4);
    stream.extend(std::iter::repeat_n(false, terminator));
    stream.extend(std::iter::repeat_n(false, 8 - stream.len() % 8));
    let mut bytes: Vec<u8> = stream
        .chunks(8)
        .map(|chunk| chunk.iter().fold(0u8, |acc, &bit| acc << 1 | bit as u8))
        .collect();
    let mut pad = [0xEC, 0x11].into_iter().cycle();
    while bytes.len() < capacity_bits / 8 {
        bytes.push(pad.next().unwrap_or(0xEC));
    }
    bytes.truncate(capacity_bits / 8);
    bytes
}

/// The module matrix of `url` at error level H, as segno makes it: one byte
/// mode segment (a URL starting `http` is never numeric or alphanumeric), the
/// smallest version that fits, and the lowest-penalty mask.
///
/// Two things differ in the `qrcode` crate, and either one changes the picture.
/// It splits the text into the cheapest mix of modes, which can land on another
/// version; and it scores masks with its own finder-pattern rule, which picks a
/// different mask than segno on the label's links. So the crate only places the
/// bits here, and the mask is chosen with segno's scoring.
fn qr_matrix(url: &str) -> Result<Vec<Vec<bool>>> {
    // segno encodes Latin-1 text as plain bytes and anything else as UTF-8
    // behind an ECI marker; a link beyond Latin-1 has never been needed.
    let mut data = Vec::with_capacity(url.len());
    for c in url.chars() {
        ensure!(
            (c as u32) < 256,
            "QR text must be Latin-1, got {c:?} in {url:?}"
        );
        data.push(c as u8);
    }
    for number in 1..=40i16 {
        let version = Version::Normal(number);
        let capacity_bits = Bits::new(version)
            .max_len(EcLevel::H)
            .map_err(|e| Error(format!("QR capacity: {e:?}")))?;
        // Mode indicator, character count, then the bytes themselves.
        let count_bits = if number < 10 { 8 } else { 16 };
        if 4 + count_bits + 8 * data.len() > capacity_bits {
            continue;
        }
        let words = segno_codewords(&data, count_bits, capacity_bits);
        let fail = |e: qrcode::types::QrError| Error(format!("QR encode failed: {e:?}"));
        let (words, correction) =
            ec::construct_codewords(&words, version, EcLevel::H).map_err(fail)?;
        let mut canvas = Canvas::new(version, EcLevel::H);
        canvas.draw_all_functional_patterns();
        canvas.draw_data(&words, &correction);
        let width = version.width() as usize;
        let mut best: Option<(u32, Vec<bool>)> = None;
        for pattern in MASKS {
            let mut masked = canvas.clone();
            masked.apply_mask(pattern);
            let modules: Vec<bool> = masked
                .into_colors()
                .into_iter()
                .map(|c| c == Color::Dark)
                .collect();
            let mut scored: Vec<Vec<u8>> = modules
                .chunks(width)
                .map(|row| row.iter().map(|&m| m as u8).collect())
                .collect();
            clear_reserved(&mut scored, number);
            let score = penalty(&scored);
            if best.as_ref().is_none_or(|(lowest, _)| score < *lowest) {
                best = Some((score, modules));
            }
        }
        let Some((_, modules)) = best else {
            bail!("no QR mask candidates")
        };
        return Ok(modules.chunks(width).map(|row| row.to_vec()).collect());
    }
    bail!("{url:?} is too long for a QR code")
}

/// QR as SVG rects. Dark-on-light panel with a quiet zone, or it won't scan.
fn qr(url: &str, cx: f64, cy: f64, size: f64, caption: &str) -> Result<String> {
    let matrix = qr_matrix(url)?;
    let n = matrix.len();
    let quiet = 2.0;
    let step = size / (n as f64 + 2.0 * quiet);
    let pad = 0.8;
    let (x0, y0) = (cx - size / 2.0, cy - size / 2.0);
    let mut out = vec![format!(
        "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" rx=\"1\" fill=\"#ffffff\"/>",
        x0 - pad,
        y0 - pad,
        size + 2.0 * pad,
        size + 2.0 * pad
    )];
    for (r, row) in matrix.iter().enumerate() {
        for (c, &dark) in row.iter().enumerate() {
            if dark {
                out.push(format!(
                    "<rect x=\"{:.3}\" y=\"{:.3}\" width=\"{step:.3}\" height=\"{step:.3}\" fill=\"#04070d\"/>",
                    x0 + (c as f64 + quiet) * step,
                    y0 + (r as f64 + quiet) * step
                ));
            }
        }
    }
    out.push(format!(
        "<text x=\"{cx:.2}\" y=\"{:.2}\" text-anchor=\"middle\" fill=\"{DIM}\" font-size=\"2.0\" letter-spacing=\"0.15\">{caption}</text>",
        y0 + size + 3.4
    ));
    Ok(out.join("\n"))
}

fn build(hl: bool, links: &[(String, String)], logo: &str) -> Result<String> {
    let mut games: Vec<&str> = vec![GAMES[0]];
    if hl {
        games.push("HALF-LIFE");
    }
    games.extend(&GAMES[1..]);
    let rim = games.join("  ·  ");
    let (defs, mark) = wordmark(logo)?;
    // QRs flank the hub, clear of both the stacking ring and the title block.
    let slots = [(CX - 34.0, 56.0), (CX + 34.0, 56.0)];
    let mut blocks = Vec::new();
    for ((label, url), (x, y)) in links.iter().zip(slots) {
        blocks.push(qr(url, x, y, 17.0, &escape(label))?);
    }
    let qr_block = blocks.join("\n");
    let r_rim = 54.6; // clear of the printable edge
    let ring = ring_text(&rim, r_rim, 2.7, DIM, 0.0);
    let (cx, cy, r_disc, r_hole) = (repr(CX), repr(CY), repr(R_DISC), repr(R_HOLE));
    let (r_hub, r_hub_glow, r_hub_outer) = (repr(R_HUB), repr(R_HUB + 6.0), repr(R_HUB + 1.2));
    let (rings, stars) = (rings(), stars(220, 1));
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
     width="120mm" height="120mm" viewBox="0 0 120 120" role="img" aria-label="PSoXide Demo Disc label">
  {defs}
  <defs>
    <radialGradient id="bg" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="#0f1a2c"/>
      <stop offset="55%" stop-color="#0a1220"/>
      <stop offset="100%" stop-color="#04070d"/>
    </radialGradient>
    <radialGradient id="hubGlow" cx="50%" cy="50%" r="50%">
      <stop offset="60%" stop-color="{CYAN}" stop-opacity="0.18"/>
      <stop offset="100%" stop-color="{CYAN}" stop-opacity="0"/>
    </radialGradient>
    <mask id="disc">
      <circle cx="{cx}" cy="{cy}" r="{r_disc}" fill="#fff"/>
      <circle cx="{cx}" cy="{cy}" r="{r_hole}" fill="#000"/>
    </mask>
  </defs>

  <g mask="url(#disc)" font-family="Arial, Helvetica, sans-serif">
    <circle cx="{cx}" cy="{cy}" r="{r_disc}" fill="url(#bg)"/>
    <circle cx="{cx}" cy="{cy}" r="{r_hub_glow}" fill="url(#hubGlow)"/>
    {rings}
    {stars}

    <!-- hub accent + outer accent ring -->
    <circle cx="{cx}" cy="{cy}" r="{r_hub}" fill="none" stroke="{CYAN}" stroke-width="0.35" opacity="0.85"/>
    <circle cx="{cx}" cy="{cy}" r="{r_hub_outer}" fill="none" stroke="{CYAN}" stroke-width="0.12" opacity="0.35"/>
    <circle cx="{cx}" cy="{cy}" r="58.0" fill="none" stroke="{CYAN}" stroke-width="0.25" opacity="0.6"/>

    <!-- wordmark: brand group is 1000x260 with glyphs at x 96..900, y 48..172 -->
    <g transform="translate({cx} 26) scale(0.076) translate(-498 -110)">
      {mark}
    </g>
    <text x="{cx}" y="34.2" text-anchor="middle" fill="{DIM}" font-size="2.3" letter-spacing="0.5">PS1 SDK · ENGINE · EDITOR · EMULATOR</text>

    <text x="{cx}" y="93" text-anchor="middle" fill="{INK}" font-size="9" font-weight="bold" letter-spacing="1.5">DEMO DISC</text>
    <text x="{cx}" y="98.2" text-anchor="middle" fill="{CYAN}" font-size="2.6" letter-spacing="0.9">PLAYSTATION HOMEBREW · 2026</text>

    {qr_block}

{ring}
  </g>
</svg>
"##
    ))
}

pub fn run(args: &[String]) -> Result<i32> {
    let args = Args::parse(args, &["link"], &["hl"])?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    let mut links = Vec::new();
    for spec in args.all("link") {
        let Some((label, url)) = spec.split_once('=') else {
            bail!("--link needs LABEL=https://...");
        };
        ensure!(url.starts_with("http"), "--link needs LABEL=https://...");
        links.push((label.to_string(), url.to_string()));
    }
    ensure!(links.len() <= 2, "only two QR slots");
    let logo_file = logo_path();
    let logo = std::fs::read_to_string(&logo_file)
        .map_err(|e| Error(format!("{}: {e}", logo_file.display())))?;
    std::io::stdout().write_all(build(args.has("hl"), &links, &logo)?.as_bytes())?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::sha256_bytes;

    const ALL_LINKS: &str = "https://allmylinks.com/bonniestudiosdev";
    const PLAY: &str = "https://bonnie-studios.itch.io/psoxide";

    fn links() -> Vec<(String, String)> {
        vec![
            ("ALL LINKS".into(), ALL_LINKS.into()),
            ("PLAY IN BROWSER".into(), PLAY.into()),
        ]
    }

    /// The brand logo lives in a submodule; without it the artwork tests skip.
    fn logo() -> Option<String> {
        std::fs::read_to_string(logo_path()).ok()
    }

    #[test]
    fn mersenne_twister_matches_python_random_seed_1() {
        let mut rng = PyRandom::new(1);
        let got: Vec<u32> = (0..4).map(|_| rng.next_u32()).collect();
        assert_eq!(got, [577090037, 2444712010, 3639700191, 3445702192]);
        let mut rng = PyRandom::new(1);
        let got: Vec<f64> = (0..3).map(|_| rng.random()).collect();
        assert_eq!(
            got,
            [0.13436424411240122, 0.8474337369372327, 0.763774618976614]
        );
        // A seed wider than 32 bits goes through two key words.
        let mut rng = PyRandom::new((1 << 40) + 5);
        assert_eq!([rng.next_u32(), rng.next_u32()], [2166296868, 2220160828]);
    }

    #[test]
    fn uniform_and_choice_match_python() {
        let mut rng = PyRandom::new(1);
        assert_eq!(rng.uniform(0.0, 120.0), 16.123709293488147);
        assert_eq!(rng.uniform(0.0, 120.0), 101.69204843246791);
        assert_eq!(rng.uniform(0.35, 1.0), 0.8464535023347991);
        let mut rng = PyRandom::new(1);
        let picks: Vec<usize> = (0..12).map(|_| rng.choice_index(5)).collect();
        assert_eq!(picks, [1, 4, 0, 2, 0, 3, 3, 3, 3, 1, 0, 3]);
    }

    #[test]
    fn floats_print_like_python() {
        for (x, repr_text) in [
            (60.0, "60.0"),
            (0.08, "0.08"),
            (0.1, "0.1"),
            (0.12, "0.12"),
            (0.16, "0.16"),
            (0.22, "0.22"),
            (-54.6, "-54.6"),
            (7.5, "7.5"),
            (27.0, "27.0"),
            (22.2, "22.2"),
        ] {
            assert_eq!(repr(x), repr_text);
        }
        assert_eq!(repr(R_HUB + 1.2), "22.2");
        // Python's f"{x:.2f}" rounds the exact binary value, half to even on a true tie.
        assert_eq!(format!("{:.2}", 0.125), "0.12");
        assert_eq!(format!("{:.3}", 0.375), "0.375");
        assert_eq!(format!("{:.2}", 0.375), "0.38");
        assert_eq!(format!("{:.2}", 0.005), "0.01");
        assert_eq!(format!("{:.2}", 1.005), "1.00");
        assert_eq!(format!("{:.2}", 0.015), "0.01");
        assert_eq!(format!("{:.2}", 0.285), "0.28");
        assert_eq!(format!("{:.2}", 59.995), "59.99");
        assert_eq!(format!("{:.3}", -0.0004), "-0.000");
        assert_eq!(format!("{:.0}", 2.5), "2");
        assert_eq!(format!("{:.0}", 7.5), "8");
    }

    #[test]
    fn escape_matches_saxutils() {
        assert_eq!(escape("A & B <C> \"d\""), "A &amp; B &lt;C&gt; \"d\"");
    }

    #[test]
    fn production_qr_matrices_match_segno() {
        // Fingerprints taken from segno.make(url, error="h").matrix: version 5-H, 37 modules wide.
        for (url, digest, dark) in [
            (
                ALL_LINKS,
                "689818363c281c2d9836a3677faf475aa79a1559a4bf8c7ab929e772b954e59d",
                690,
            ),
            (
                PLAY,
                "e83c2d8f349012e896abdad4fae85a2e2f37494555b4a726ea1cc0e479f9bd69",
                698,
            ),
        ] {
            let matrix = qr_matrix(url).unwrap();
            assert_eq!(matrix.len(), 37);
            let text: Vec<String> = matrix
                .iter()
                .map(|row| row.iter().map(|&m| if m { '1' } else { '0' }).collect())
                .collect();
            assert_eq!(sha256_bytes(text.join("\n").as_bytes()), digest, "{url}");
            assert_eq!(matrix.iter().flatten().filter(|&&m| m).count(), dark);
        }
    }

    #[test]
    fn other_qr_sizes_match_segno() {
        // Fingerprints from segno for versions 1, 2, 4, 8 and 11. "http://" fills
        // version 1 exactly (the byte-boundary padding quirk); the others are
        // links whose mask the crate's own scoring would have chosen differently.
        for (url, width, digest) in [
            ("http://", 21, "ea5314537a285645c5645d8924bafd0b6fe5df068c3a58996c33856054e3bb7d"),
            ("http_o9a=~j", 25, "4fa0cc06e44c2e5975d2e3841ac938217dfa3b1c016243cf629df102327d28c1"),
            ("httpg9-&twbu5x/tki4o8iq5c1xs0/", 33, "c766d7e23dadb850141fc1cc5852a6cf47b05e49bb17ad9918bde37c72363e5c"),
            (
                "http?9q4r9/_gb?~0k0u1e19.183zcs.9o8_qgg_8lbgg?j8-?p?zxgp?qzfhd6wp5/_zakf",
                49,
                "f67283ca3fe0d0d842cff27f3236e610c7d9308ccca2b320b02b381e684a7ce7",
            ),
            (
                "httpvids?%klr-p4k=u8g_6~?kp7cmbqy2oz2=v.2k&l-taqw8-jjq78q.v1j/.ch%~mv28ye/a5ckmko335.346xxsmb_vte=59._~_u1_nbc4%ayq=d~ggkygcsvp=evuaz",
                61,
                "6d5ca2e1f02d8fac52167e7f185354ff62a46b446fb754e0526f0405f8d10ddd",
            ),
        ] {
            let matrix = qr_matrix(url).unwrap();
            assert_eq!(matrix.len(), width, "{url}");
            let text: Vec<String> =
                matrix.iter().map(|row| row.iter().map(|&m| if m { '1' } else { '0' }).collect()).collect();
            assert_eq!(sha256_bytes(text.join("\n").as_bytes()), digest, "{url}");
        }
    }

    #[test]
    fn penalty_rules_score_a_known_pattern() {
        // A row of eight dark modules is 6 points of N1; the finder lookalike
        // touching the edge is 40.
        let lookalike = [1u8, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0, 0];
        assert_eq!(finder_lookalikes(&lookalike), 40);
        assert_eq!(finder_lookalikes(&[0u8; 12]), 0);
        // Needs four light modules on a side unless it touches the edge.
        let boxed = [0u8, 1, 1, 0, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 0];
        assert_eq!(finder_lookalikes(&boxed), 0);
    }

    #[test]
    fn qr_with_a_quiet_zone_decodes() {
        // The check the old test made through OpenCV: rasterise the modules
        // dark on light with a quiet zone, and read them back.
        for url in [
            ALL_LINKS,
            PLAY,
            "https://example.com/a-much-longer-link/that/needs/a/bigger/symbol?x=1&y=2",
        ] {
            let matrix = qr_matrix(url).unwrap();
            let (quiet, scale) = (4, 6);
            let side = (matrix.len() + 2 * quiet) * scale;
            let mut image = rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| {
                let (c, r) = (x / scale, y / scale);
                let inside = c >= quiet
                    && r >= quiet
                    && c < quiet + matrix.len()
                    && r < quiet + matrix.len();
                if inside && matrix[r - quiet][c - quiet] {
                    0
                } else {
                    255
                }
            });
            let grids = image.detect_grids();
            assert_eq!(grids.len(), 1, "{url}");
            let (_, text) = grids[0].decode().unwrap();
            assert_eq!(text, url);
        }
    }

    #[test]
    fn whole_label_matches_the_python_output() {
        let Some(logo) = logo() else {
            eprintln!("skipping: the brand logo submodule is not checked out");
            return;
        };
        // sha256 of what tools/disc_label.py printed for the same arguments.
        let plain = build(false, &[], &logo).unwrap();
        assert_eq!(
            sha256_bytes(plain.as_bytes()),
            "6106c057976f118c86c3d1b22e7d6d93004fe5920c20ec07ffd3e43f503f5288"
        );
        let hl = build(true, &[], &logo).unwrap();
        assert_eq!(
            sha256_bytes(hl.as_bytes()),
            "bad5f1b5fd12d194ae061a2b7d5ba44b6acadeb3770254cb3d1416cd3ff00893"
        );
        let linked = build(false, &links(), &logo).unwrap();
        assert_eq!(
            sha256_bytes(linked.as_bytes()),
            "e5f1b7889fd8a665a914996c0fd0d436bf852949c35211def03b545e11f61cd8"
        );
        let one = build(true, &links()[..1], &logo).unwrap();
        assert_eq!(
            sha256_bytes(one.as_bytes()),
            "507935d63cc12dee1f2d393b8f76e5ab68a12bc5834947003f97982b3dcfa730"
        );
    }

    #[test]
    fn rendered_label_scans() {
        let rsvg = std::path::Path::new("/opt/homebrew/bin/rsvg-convert");
        let Some(logo) = logo().filter(|_| rsvg.exists()) else {
            eprintln!("skipping: rsvg-convert or the brand logo is missing");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (svg, png_path) = (dir.path().join("l.svg"), dir.path().join("l.png"));
        std::fs::write(&svg, build(false, &links(), &logo).unwrap()).unwrap();
        let status = std::process::Command::new(rsvg)
            .args(["-d", "600", "-p", "600"])
            .arg(&svg)
            .arg("-o")
            .arg(&png_path)
            .status()
            .unwrap();
        assert!(status.success());
        let mut reader = png::Decoder::new(std::fs::File::open(&png_path).unwrap())
            .read_info()
            .unwrap();
        let mut buffer = vec![0u8; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buffer).unwrap();
        let channels = frame.color_type.samples();
        let n = frame.width as usize;
        // The two QR slots, in the same millimetre coordinates build() places them at.
        for ((_, url), cx) in links().iter().zip([26.0, 94.0]) {
            let window = (24.0 / 120.0 * n as f64) as usize;
            let (x, y) = (
                (cx / 120.0 * n as f64) as usize,
                (56.0 / 120.0 * n as f64) as usize,
            );
            let (left, top) = (x - window / 2, y - window / 2);
            let mut crop = rqrr::PreparedImage::prepare_from_greyscale(window, window, |px, py| {
                let at = ((top + py) * n + left + px) * channels;
                let rgb = &buffer[at..at + 3.min(channels)];
                (rgb.iter().map(|&v| v as u32).sum::<u32>() / rgb.len() as u32) as u8
            });
            let grids = crop.detect_grids();
            assert_eq!(grids.len(), 1, "{url}");
            assert_eq!(grids[0].decode().unwrap().1, *url);
        }
    }

    #[test]
    fn links_are_validated_like_the_original() {
        let run_with =
            |items: &[&str]| run(&items.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(run_with(&["--link", "NOURL"])
            .unwrap_err()
            .0
            .contains("LABEL=https"));
        assert!(run_with(&["--link", "X=ftp://a"])
            .unwrap_err()
            .0
            .contains("LABEL=https"));
        assert!(run_with(&[
            "--link",
            "A=http://a",
            "--link",
            "B=http://b",
            "--link",
            "C=http://c"
        ])
        .unwrap_err()
        .0
        .contains("only two QR slots"));
        assert!(run_with(&["--bogus"]).is_err());
    }

    #[test]
    fn rim_text_for_the_half_life_pressing_includes_it() {
        let Some(logo) = logo() else { return };
        assert!(
            build(true, &[], &logo)
                .unwrap()
                .matches("<text x=\"0\" y=\"0\"")
                .count()
                > build(false, &[], &logo)
                    .unwrap()
                    .matches("<text x=\"0\" y=\"0\"")
                    .count()
        );
    }
}
