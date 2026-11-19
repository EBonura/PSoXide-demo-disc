//! Art for the menu's attract mode: the back-cover screenshots.
//!
//! `backs`: for every program, one or two screenshots cut to 64x48 and sharing
//! one 256-colour palette, as `back_<id>.art` (512 bytes of CLUT, then the 8bpp
//! pixels of each shot in turn).

use super::{index_with, median_cut, palette_to_clut, Rgb};
use image::imageops::FilterType;
use std::fs;
use std::path::Path;

/// Size of one back-cover screenshot, in texels.
const SHOT_W: u32 = 64;
const SHOT_H: u32 = 48;

/// Program id used by the guest, and its screenshots in the shots directory.
const BACKS: &[(&str, &[&str])] = &[
    ("citadel", &["cortex-citadel1", "cortex-citadel2"]),
];

pub fn bake_backs(shots: &Path, out: &Path) {
    fs::create_dir_all(out).expect("create the output directory");
    for (id, stems) in BACKS {
        let files: Vec<String> = stems.iter().map(|s| format!("{s}.png")).collect();
        bake_back(shots, out, id, &files);
    }
    println!("baked {} back covers", BACKS.len());
}

/// One program's `back_<id>.art` from the pictures `files` (names in `shots`).
pub fn bake_back(shots: &Path, out: &Path, id: &str, files: &[String]) {
    let pixels: Vec<Vec<Rgb>> = files
        .iter()
        .map(|file| {
            let path = shots.join(file);
            image::open(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .resize_exact(SHOT_W, SHOT_H, FilterType::Lanczos3)
                .to_rgb8()
                .pixels()
                .map(|p| p.0)
                .collect()
        })
        .collect();
    let all: Vec<Rgb> = pixels.iter().flatten().copied().collect();
    let palette = median_cut(&all);
    let mut blob = palette_to_clut(&palette);
    for shot in &pixels {
        blob.extend(index_with(&palette, shot));
    }
    fs::write(out.join(format!("back_{id}.art")), blob).expect("write a back-cover blob");
}
