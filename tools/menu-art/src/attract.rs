//! Art for the menu's attract mode: the back-cover screenshots and the
//! recorded level curve that stands in for the music in the prototype.
//!
//! * `backs`: for every program, one or two screenshots cut to 64x48 and
//!   sharing one 256-colour palette, as `back_<id>.bin` (512 bytes of CLUT,
//!   then the 8bpp pixels of each shot in turn).
//! * `curve`: a slice of `menu.spectrum` (16 bands, 30 frames a second) for one
//!   song, as raw bytes the guest replays as if the music were playing.

use super::{index_with, median_cut, palette_to_clut, Rgb};
use image::imageops::FilterType;
use std::fs;
use std::path::Path;

/// Size of one back-cover screenshot, in texels.
const SHOT_W: u32 = 64;
const SHOT_H: u32 = 48;

/// Program id used by the guest, and its screenshots in the shots directory.
const BACKS: &[(&str, &[&str])] = &[
    ("arcade", &["breakout", "invaders"]),
    ("celeste", &["celeste", "celeste2"]),
    ("nitro", &["nitroxide-aerial", "nitroxide-goal"]),
    ("voxide", &["voxide-day", "voxide-night"]),
    ("quake", &["quake-gameplay", "quake-menu"]),
    (
        "cortex",
        &["cortex-current-gameplay", "cortex-current-menu"],
    ),
    ("psxcel", &["psxcel-chart", "psxcel-editing"]),
    ("wipeout", &["wipeout1", "wipeout2"]),
    ("hollow", &["hollowknight"]),
    ("cstrike", &["counterstrike"]),
    ("halflife", &["halflife"]),
];

pub fn bake_backs(shots: &Path, out: &Path) {
    fs::create_dir_all(out).expect("create the output directory");
    for (id, stems) in BACKS {
        let pixels: Vec<Vec<Rgb>> = stems
            .iter()
            .map(|stem| {
                let path = shots.join(format!("{stem}.png"));
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
        fs::write(out.join(format!("back_{id}.bin")), blob).expect("write a back-cover blob");
    }
    println!("baked {} back covers", BACKS.len());
}

/// Bands per spectrum frame and frames per second, as `mkdisc analyse` writes.
const BANDS: usize = 16;
const RATE: usize = 30;

/// `curve <menu.spectrum> <song> <start-s> <secs> <out.bin>`; with `secs` of 0
/// it only prints the bass level of each second of the song.
pub fn cut_curve(args: &[String]) {
    let bytes = fs::read(&args[0]).expect("read the spectrum file");
    let song: usize = args[1].parse().expect("song index");
    let start: usize = args[2].parse().expect("start second");
    let secs: usize = args[3].parse().expect("seconds");
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[..8], b"PSXSPEC1");
    let songs = word(8);
    let frames: Vec<usize> = (0..songs).map(|i| word(12 + 4 * i)).collect();
    let data = &bytes[12 + 4 * songs..];
    let first: usize = frames[..song].iter().sum();
    let song_data = &data[first * BANDS..(first + frames[song]) * BANDS];
    if secs == 0 {
        for s in 0..frames[song] / RATE {
            let mut bass = 0u32;
            let mut all = 0u32;
            let mut peak = 0u32;
            for f in s * RATE..(s + 1) * RATE {
                let row = &song_data[f * BANDS..(f + 1) * BANDS];
                let b = row[..4].iter().map(|&v| v as u32).sum::<u32>() / 4;
                bass += b;
                peak = peak.max(b);
                all += row.iter().map(|&v| v as u32).sum::<u32>() / BANDS as u32;
            }
            println!(
                "{s:4}s bass {:3} peak {:3} all {:3}",
                bass / RATE as u32,
                peak,
                all / RATE as u32
            );
        }
        return;
    }
    let from = start * RATE * BANDS;
    let to = (start + secs) * RATE * BANDS;
    fs::write(&args[4], &song_data[from..to]).expect("write the curve");
    println!("wrote {} frames", secs * RATE);
}
