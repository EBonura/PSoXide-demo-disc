//! `icons`: cook the link icons the credits card draws beside each URL.
//!
//! The links panel writes only the part after the slash (`EBonura/PSoXide`,
//! not `github.com/EBonura/PSoXide`), because the description column is 23
//! characters wide and every full URL overflows it. The icon carries the
//! domain, so the text only has to carry the handle.
//!
//! Sources are Simple Icons (simpleicons.org), whose SVG paths are CC0. Each
//! is rasterised large and reduced with a proper filter: these are 24x24
//! viewBox glyphs and nearest-neighbour at this size loses the GitHub cat
//! entirely.
//!
//! Coverage becomes brightness. The SVG paths are solid black on transparent,
//! so the alpha channel IS the shape, and it maps onto a 16-step grey ramp
//! that the GPU then tints with the primitive colour, the same way the font
//! draws.
//!
//! CELL is deliberately wider than ICON and a multiple of four. A 4bpp texel
//! is half a byte, so a cell whose width is not a multiple of four starts a
//! later icon mid-nibble; that is the exact fault that turned SPLEEN's 'f'
//! into a bare crossbar on silicon and never showed in an emulator. Four
//! texels a byte, and the row stride lands on a whole halfword too.
//!
//! Usage: `disc-tools icons <svg-dir> <out-dir>`
//!
//! Writes `icons.tex` (packed 4bpp indices) and `icons.clut` (16 little-endian
//! 15-bit colours) for `include_bytes!`. Needs `rsvg-convert` on the PATH.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::shots::pillow;
use crate::util::{Error, Result};

/// The order here is the order the launcher indexes them by, so it is API.
const NAMES: [&str; 6] = [
    "github",
    "itchdotio",
    "x",
    "instagram",
    "buymeacoffee",
    "youtube",
];

const ICON: usize = 12;
/// Padded to a multiple of four texels; see the module docs.
const CELL: usize = 12;
/// Rasterise well above target so the reduction has something to filter.
const SUPER: usize = 256;
const LEVELS: usize = 16;

/// SVG to an ICON x ICON coverage map (one byte per pixel).
fn rasterise(svg: &Path) -> Result<Vec<u8>> {
    let size = SUPER.to_string();
    let output = Command::new("rsvg-convert")
        .args(["-w", &size, "-h", &size])
        .arg(svg)
        .output()
        .map_err(|e| Error(format!("cannot run rsvg-convert: {e}")))?;
    if !output.status.success() {
        return Err(Error(format!(
            "rsvg-convert failed on {}: {}",
            svg.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let (w, h, rgba) = pillow::decode_png_rgba(&output.stdout)?;
    let alpha: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
    Ok(pillow::lanczos_resize_l(&alpha, w, h, ICON, ICON))
}

/// Pack the coverage maps into the 4bpp texture (one row of cells, index 0
/// transparent) and build the grey-ramp CLUT.
fn build(coverage: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
    let width = CELL * coverage.len();
    let pad = (CELL - ICON) / 2;
    let mut rows = vec![vec![0u8; width]; ICON];
    for (slot, cov) in coverage.iter().enumerate() {
        for y in 0..ICON {
            for x in 0..ICON {
                // 0 has to stay empty: on this hardware a CLUT entry of
                // 0x0000 is see-through, and that is what lets the panel show
                // through around the mark.
                let level = usize::from(cov[y * ICON + x]) * (LEVELS - 1) / 255;
                rows[y][slot * CELL + pad + x] = level as u8;
            }
        }
    }
    let mut tex = Vec::with_capacity(width / 2 * ICON);
    for row in &rows {
        for pair in row.chunks_exact(2) {
            // Low nibble is the left texel.
            tex.push(pair[0] | (pair[1] << 4));
        }
    }

    // Grey ramp. Entry 0 transparent, the rest a straight 15-bit grey the
    // primitive colour then tints.
    let mut clut = Vec::with_capacity(LEVELS * 2);
    for i in 0..LEVELS {
        let entry: u16 = if i == 0 {
            0
        } else {
            let five = (i * 31 / (LEVELS - 1)) as u16;
            0x8000 | five | five << 5 | five << 10
        };
        clut.extend_from_slice(&entry.to_le_bytes());
    }
    (tex, clut)
}

pub fn run(args: &[String]) -> Result<i32> {
    let [svg_dir, out_dir] = args else {
        return Err(Error("usage: disc-tools icons <svg-dir> <out-dir>".into()));
    };
    let (svg_dir, out_dir) = (Path::new(svg_dir), Path::new(out_dir));
    fs::create_dir_all(out_dir)?;
    let width = CELL * NAMES.len();
    if !width.is_multiple_of(4) {
        return Err(Error(format!(
            "row of {width} texels does not pack to whole bytes"
        )));
    }
    let coverage = NAMES
        .iter()
        .map(|name| rasterise(&svg_dir.join(format!("{name}.svg"))))
        .collect::<Result<Vec<_>>>()?;
    let (tex, clut) = build(&coverage);
    fs::write(out_dir.join("icons.tex"), &tex)?;
    fs::write(out_dir.join("icons.clut"), &clut)?;
    println!(
        "{} icons, {ICON}x{ICON} in {CELL}-texel cells, {width}x{ICON} texels -> icons.tex {}B, icons.clut {}B",
        NAMES.len(),
        tex.len(),
        clut.len()
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn clut_is_a_transparent_zero_then_a_grey_ramp() {
        let (_, clut) = build(&vec![vec![0; ICON * ICON]; NAMES.len()]);
        // Expected bytes taken from the retired icons.py.
        let want = "000042888490c69808a14aa98cb1ceb910c252ca94d2d6da18e35aeb9cf3ffff";
        let got: String = clut.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn coverage_becomes_low_nibble_first_levels() {
        let mut cells = vec![vec![0u8; ICON * ICON]; NAMES.len()];
        // Slot 1, first row: full (15), just under one step (0), one step (1).
        cells[1][0] = 255;
        cells[1][1] = 16;
        cells[1][2] = 17;
        let (tex, _) = build(&cells);
        assert_eq!(tex.len(), CELL * NAMES.len() / 2 * ICON);
        // Cell 1 starts at texel 12, which is byte 6 of the row.
        assert_eq!(&tex[6..9], [0x0F, 0x01, 0x00]);
        assert!(tex[..6].iter().all(|&b| b == 0));
    }

    #[test]
    fn cooks_svgs_to_the_pillow_bytes() {
        // Needs rsvg-convert, like the tool itself; on a machine without it
        // there is nothing to compare, so say so and pass.
        if Command::new("rsvg-convert")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("rsvg-convert not installed, skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (svgs, out) = (dir.path().join("svg"), dir.path().join("out"));
        fs::create_dir(&svgs).unwrap();
        for name in NAMES {
            let body = r#"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="9"/><rect x="2" y="2" width="5" height="3" opacity="0.5"/></svg>"#;
            fs::write(svgs.join(format!("{name}.svg")), body).unwrap();
        }
        let args = [svgs.display().to_string(), out.display().to_string()];
        assert_eq!(run(&args).unwrap(), 0);
        let tex = fs::read(out.join("icons.tex")).unwrap();
        let hash: String = Sha256::digest(&tex)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        // From the retired icons.py (Pillow 12.0.0) on the same SVGs.
        assert_eq!(
            hash,
            "8d8eb3be68f3c8870def3884070b5a0f887ca63a28dc6144f99a1fb66e5f494d"
        );
    }

    #[test]
    fn a_missing_svg_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let args = [
            dir.path().display().to_string(),
            dir.path().join("out").display().to_string(),
        ];
        assert!(run(&args).is_err());
    }
}
