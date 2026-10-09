//! `cook-shot`: cook a screenshot into the disc's menu-panel format.
//!
//! One shot is a 512-byte CLUT (256 RGB555 entries, little-endian) followed by
//! 120x90 pixel indices, which is what `mkdisc --shot` presses and the
//! launcher uploads straight into VRAM to fill the box beside the description.
//! Colours are quantized to 256: exactly, when the downscaled frame already
//! fits, and by median cut when it does not.
//!
//! Black maps to 0x8000 rather than 0x0000, because on the PlayStation a texel
//! of 0x0000 is transparent and a screenshot with holes in it reads as a bug.
//!
//! Usage: `disc-tools cook-shot <in.png> <out.shot>`
//!
//! This replaces the Python script of the same job. The resize and the
//! quantiser are ports of Pillow 12.0.0's (see `pillow.rs`), so a re-cooked
//! shot matches what the script made, byte for byte.

#[path = "pillow.rs"]
pub mod pillow;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::util::{Error, Result};

const W: usize = 120;
const H: usize = 90;

/// 15-bit colour for the PlayStation. 0x0000 is transparent on the console,
/// so true black is forced to the opaque-black encoding instead.
fn rgb555(r: u8, g: u8, b: u8) -> u16 {
    // round(c * 31 / 255), exact in integers: a tie would need c to be a
    // multiple of 255 over 62, which no byte is.
    let q = |c: u8| (u32::from(c) * 62 + 255) / 510;
    let v = q(r) | (q(g) << 5) | (q(b) << 10);
    if v == 0 {
        0x8000
    } else {
        v as u16
    }
}

/// Cook decoded RGB pixels of any size into the shot bytes, plus the number of
/// distinct palette indices the picture uses.
pub fn cook_rgb(rgb: &[u8], width: usize, height: usize) -> Result<(Vec<u8>, usize)> {
    let scaled;
    let mut pixels = rgb;
    if (width, height) != (W, H) {
        scaled = if width.is_multiple_of(W) && height.is_multiple_of(H) {
            pillow::nearest_resize_rgb(rgb, width, height, W, H)
        } else {
            pillow::lanczos_resize_rgb(rgb, width, height, W, H)
        };
        pixels = &scaled;
    }

    let (palette, indices) = match pillow::get_colors_rgb(pixels, 256) {
        Some(palette) => {
            // Fits as-is: an exact palette, no dithering to muddy clean pixels.
            let lookup: HashMap<[u8; 3], u8> = palette
                .iter()
                .enumerate()
                .map(|(i, &c)| (c, i as u8))
                .collect();
            let indices = pixels
                .chunks_exact(3)
                .map(|p| lookup[&[p[0], p[1], p[2]]])
                .collect();
            (palette, indices)
        }
        None => pillow::quantize_adaptive_rgb(pixels, 256)?,
    };
    if indices.len() != W * H {
        return Err(Error(format!(
            "expected {} pixels, got {}",
            W * H,
            indices.len()
        )));
    }

    let mut shot = vec![0u8; 512];
    for (i, c) in palette.iter().enumerate().take(256) {
        shot[i * 2..i * 2 + 2].copy_from_slice(&rgb555(c[0], c[1], c[2]).to_le_bytes());
    }
    let mut used = [false; 256];
    for &i in &indices {
        used[usize::from(i)] = true;
    }
    shot.extend_from_slice(&indices);
    Ok((shot, used.iter().filter(|&&u| u).count()))
}

/// Cook a PNG file's bytes.
pub fn cook_png(png: &[u8]) -> Result<(Vec<u8>, usize)> {
    let (width, height, rgba) = pillow::decode_png_rgba(png)?;
    cook_rgb(&pillow::rgba_to_rgb(&rgba), width, height)
}

pub fn run(args: &[String]) -> Result<i32> {
    let [src, out] = args else {
        return Err(Error(
            "usage: disc-tools cook-shot <in.png> <out.shot>".into(),
        ));
    };
    let png = fs::read(Path::new(src)).map_err(|e| Error(format!("{src}: {e}")))?;
    let (shot, used) = cook_png(&png)?;
    fs::write(Path::new(out), &shot).map_err(|e| Error(format!("{out}: {e}")))?;
    println!("{out}: {used} colours used");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    // sha256 of the cooked shot for every PNG in assets/shots, generated once
    // by the retired cook-shots.py running on Pillow 12.0.0.
    const GOLDEN: [(&str, &str); 26] = [
        (
            "breakout",
            "d51db0ca89a8fb6c388212e8a850126e7d833ae36a099aabc7e7be8d06619c3d",
        ),
        (
            "breakout2",
            "a3d18c6d63a4e81a1feca0f8c2b6a2aecac017bfdaebdb757da272ea45e874db",
        ),
        (
            "celeste",
            "8d6a2da33f142eab46700a1e1bd23a2fc480503f1167d54ca1fabecc99f579b7",
        ),
        (
            "celeste2",
            "ad4d7be9a16cc1522583708a54d1147ddd649d9fa8de5542de196ce2dcc7cd7b",
        ),
        (
            "cortex-current-gameplay",
            "2aa5bd046190350a79656cc60abf5f630a7ff9b6ec942e9913ccd01b610661f5",
        ),
        (
            "cortex-current-menu",
            "cb4a0d8573b4e57f871d72c9413bf02b66f0c5eb694071b358456c4747927720",
        ),
        (
            "counterstrike",
            "e9ebdd8029913f478b45c9c289bd8e8bfe4dbbf38dc2cc00fcfe45a821e05980",
        ),
        (
            "halflife",
            "b8414fe5f4e55cc922b3cd842f496b8289ceafdbadd28310ba15bf4ac1e25c90",
        ),
        (
            "hollowknight",
            "979d4cb3a0530aace4ed7df98a17aeb61f8c88c4dc017994b484bfb461c70192",
        ),
        (
            "hwtests",
            "0d47b6584313b1d2906cfa146fe0cc8a4bc20960b77f6cb63f63896f33541869",
        ),
        (
            "hwtests2",
            "193c98661f9f79185cb607703a60e10b842d8a91101eca7c7efc95251344e82c",
        ),
        (
            "invaders",
            "93f271c5ad9ee5f78dd9878a8b2030a18b64b048516ed29ea127321e85ef4d08",
        ),
        (
            "invaders2",
            "f253289de25f76cb2c8ef3cfe4b03389a90650235e90a279fe4d96fabc72499f",
        ),
        (
            "nitroxide-aerial",
            "f1f590ce91a8c1d80a21b6acca584652900927a9b4899696369ff8fb05039122",
        ),
        (
            "nitroxide-boost",
            "c77a07ea942e70dc0e106c53611c6ff90fad76d3b6e8fbd102a11a3d96be97b9",
        ),
        (
            "nitroxide-goal",
            "ba1c4709e424682a3c377e32e4821d247851356ca0e87422dc249c6ab3bb4c36",
        ),
        (
            "pong",
            "eaf0b3e6450c06e8a7ebb0831b19745be207fc860eb8a4ea96da1831af2e17d2",
        ),
        (
            "pong2",
            "ad2a8cee6b6e0ab9ed5169fd27947e269a61606bf1564b4c879b7b846b398a54",
        ),
        (
            "psxcel-chart",
            "1e29dbd83922db645080ff12c368f6ac542730bd86825dab5200edf55bc860ab",
        ),
        (
            "psxcel-editing",
            "151b8167a66fec5ffed0fdf524b7aaebf3c7f63f17c1130e4867bcce31d7925c",
        ),
        (
            "quake-gameplay",
            "4dac2da4637bae4b5bf898e03c45f896d22abbde61fdf3c2b9fe1cdf8c4c5f3d",
        ),
        (
            "quake-menu",
            "a400d8256ce9b902da3ace48ecc2dd5e136d46ec878ec701407be1e301322001",
        ),
        (
            "voxide-day",
            "50cfa3758f18a7a988a2544313881401c90ecc999e7617e544dda9d06fbdfa33",
        ),
        (
            "voxide-night",
            "245de69bf45d8c9aaacf14499b44bf071a4eb75d5298f4227c376a6fe6f285a1",
        ),
        (
            "wipeout1",
            "4dccb926801ce9d00e0b019d5331413c418ef0706258abee8f565445ef1708dd",
        ),
        (
            "wipeout2",
            "8da8e2e2dea214dde35b2650243cb7fdf93896d343481b4f0f31043a4aab0b7f",
        ),
    ];

    fn sha256(data: &[u8]) -> String {
        Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn cooks_every_checked_in_shot_to_the_pillow_bytes() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/shots");
        for (name, want) in GOLDEN {
            let png = fs::read(dir.join(format!("{name}.png"))).unwrap();
            let (shot, _) = cook_png(&png).unwrap();
            assert_eq!(shot.len(), 512 + W * H, "{name}");
            assert_eq!(
                sha256(&shot),
                want,
                "{name} differs from the Pillow-cooked shot"
            );
        }
        let pngs = fs::read_dir(&dir).unwrap().filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "png")
        });
        assert_eq!(
            pngs.count(),
            GOLDEN.len(),
            "a shot was added without a golden hash"
        );
    }

    #[test]
    fn rgb555_rounds_and_keeps_black_opaque() {
        assert_eq!(rgb555(0, 0, 0), 0x8000, "0x0000 would be transparent");
        assert_eq!(rgb555(255, 255, 255), 0x7FFF);
        assert_eq!(rgb555(255, 0, 0), 0x001F);
        assert_eq!(rgb555(0, 255, 0), 0x03E0);
        assert_eq!(rgb555(0, 0, 255), 0x7C00);
        // Near-black that rounds to zero in all three channels is still
        // black, so it gets the opaque encoding too.
        assert_eq!(rgb555(4, 4, 4), 0x8000);
        assert_eq!(rgb555(5, 0, 0), 0x0001);
        // Round to nearest, not truncate: 128 / 255 * 31 = 15.56.
        assert_eq!(rgb555(128, 0, 0), 16);
    }

    fn flat(colours: &[[u8; 3]]) -> Vec<u8> {
        (0..W * H)
            .flat_map(|i| colours[i % colours.len()])
            .collect()
    }

    #[test]
    fn exact_palette_is_not_quantised_and_keeps_pillows_order() {
        // 256 colours that all differ, many only by one step: a quantiser
        // asked for 256 would still keep them, but only the exact path keeps
        // the colours in getcolors order with unused entries left zero.
        let colours: Vec<[u8; 3]> = (0..256u32)
            .map(|i| [i as u8, (i * 7) as u8, (255 - i) as u8])
            .collect();
        let rgb = flat(&colours);
        let (shot, used) = cook_rgb(&rgb, W, H).unwrap();
        assert_eq!(used, 256);
        let table = pillow::get_colors_rgb(&rgb, 256).unwrap();
        for (i, c) in table.iter().enumerate() {
            let entry = u16::from_le_bytes([shot[i * 2], shot[i * 2 + 1]]);
            assert_eq!(entry, rgb555(c[0], c[1], c[2]), "palette entry {i}");
        }
        for (i, px) in rgb.chunks_exact(3).enumerate() {
            let c = table[usize::from(shot[512 + i])];
            assert_eq!(
                c,
                [px[0], px[1], px[2]],
                "pixel {i} must keep its exact colour"
            );
        }
    }

    #[test]
    fn few_colours_leave_the_rest_of_the_clut_zero() {
        let (shot, used) = cook_rgb(&flat(&[[0, 0, 0], [255, 255, 255]]), W, H).unwrap();
        assert_eq!(used, 2);
        let table = pillow::get_colors_rgb(&flat(&[[0, 0, 0], [255, 255, 255]]), 256).unwrap();
        let black = table.iter().position(|c| *c == [0, 0, 0]).unwrap();
        assert_eq!(
            u16::from_le_bytes([shot[black * 2], shot[black * 2 + 1]]),
            0x8000
        );
        assert!(shot[4..512]
            .chunks_exact(2)
            .all(|e| e == [0, 0] || e == [0x00, 0x80] || e == [0xFF, 0x7F]));
        assert_eq!(shot.len(), 512 + W * H);
    }

    #[test]
    fn more_than_256_colours_go_through_the_quantiser() {
        let colours: Vec<[u8; 3]> = (0..400u32)
            .map(|i| [(i % 256) as u8, (i / 256 * 60) as u8, 9])
            .collect();
        let rgb = flat(&colours);
        let (shot, used) = cook_rgb(&rgb, W, H).unwrap();
        assert!(used <= 256);
        let (palette, indices) = pillow::quantize_adaptive_rgb(&rgb, 256).unwrap();
        assert_eq!(&shot[512..], &indices[..]);
        assert_eq!(
            u16::from_le_bytes([shot[0], shot[1]]),
            rgb555(palette[0][0], palette[0][1], palette[0][2])
        );
    }

    #[test]
    fn exact_multiples_use_nearest_and_others_lanczos() {
        // Stripes one pixel wide. Nearest reads the odd column and row of
        // every 2x2 footprint, so the cooked picture is one flat colour;
        // a filtered shrink would blend the stripes to grey.
        let stripes = |w: usize, h: usize| -> Vec<u8> {
            (0..w * h)
                .flat_map(|i| [(i % w % 2 * 200) as u8, (i / w % 2 * 200) as u8, 40])
                .collect()
        };
        let (shot, used) = cook_rgb(&stripes(240, 180), 240, 180).unwrap();
        assert_eq!(used, 1);
        assert_eq!(u16::from_le_bytes([shot[0], shot[1]]), rgb555(200, 200, 40));

        // 241x181 is not a multiple, so it is filtered and the stripes blur.
        let (_, blurred) = cook_rgb(&stripes(241, 181), 241, 181).unwrap();
        assert!(blurred > 1);
    }

    #[test]
    fn run_writes_the_shot_and_reports_colours() {
        let dir = tempfile::tempdir().unwrap();
        let (src, out) = (dir.path().join("in.png"), dir.path().join("out.shot"));
        let mut file = fs::File::create(&src).unwrap();
        let mut enc = png::Encoder::new(&mut file, W as u32, H as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .unwrap()
            .write_image_data(&flat(&[[0, 0, 0], [255, 0, 0]]))
            .unwrap();
        drop(file);
        let args = [src.display().to_string(), out.display().to_string()];
        assert_eq!(run(&args).unwrap(), 0);
        assert_eq!(fs::read(&out).unwrap().len(), 512 + W * H);
        assert!(run(&args[..1]).is_err());
    }
}
