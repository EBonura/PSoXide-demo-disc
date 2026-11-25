//! The one static frame a launch leaves on screen (Manny, 11-24): the game's cover and its name.
//!
//! The scene core keeps the loading-frame rectangle of VRAM (`psx_io::gpu::LOADING_FRAME`, 320x240 at
//! (0,0)) and does not touch the display registers when it starts a program, so this frame stays
//! up through the load and the game's start, until the game's first display set-up clears it. It
//! must therefore be drawn into the buffer at (0,0) and be the frame on screen when the switch is
//! called: [`show`] draws it and waits until that buffer is displayed.

use crate::cover::{self, Cover};
use crate::gfx::{self, Gfx, Size};
use crate::text;
use crate::ui_atlas;
use disc_toc::Entry;

/// How much larger than its shelf size the cover is drawn.
const SCALE: i32 = 2;

/// Draw the loading frame for `entry` and return once it is on screen from VRAM (0,0).
pub fn show(g: &mut Gfx, entry: &Entry) {
    show_with(g, entry, &["LOADING"]);
}

/// The same frame when the drive did not get to the program (`chain::boot`): the launch goes
/// back to the shelf instead of to the core's fatal screen.
pub fn show_failed(g: &mut Gfx, entry: &Entry) {
    show_with(g, entry, &["COULD NOT START", "TRY AGAIN"]);
}

fn show_with(g: &mut Gfx, entry: &Entry, lines: &[&str]) {
    let (x0, y0, ..) = psx_io::gpu::LOADING_FRAME;
    // The launcher double-buffers at y 0 and 240: draw until the frame lands in the buffer at
    // the loading frame's place (at most two frames), each one shown before the next is drawn.
    for _ in 0..2 {
        g.begin();
        let at_frame = g.fb.draw_origin() == (x0, y0);
        draw(g, entry, lines);
        g.end_blocking();
        if at_frame {
            break;
        }
    }
}

fn draw(g: &mut Gfx, entry: &Entry, lines: &[&str]) {
    g.clear((0, 0, 0));
    let (w, h) = Size::L.dims();
    let (w, h) = (w * SCALE, h * SCALE);
    let (x, y) = (160 - w / 2, 100 - h / 2);
    match cover::of(entry.name_str()) {
        Cover::Art(art) => {
            g.art_ribbon(
                Size::L,
                art,
                &[(x, y), (x + w, y)],
                &[(x, y + h), (x + w, y + h)],
                (128, 128, 128),
            );
            g.frame(x - 1, y - 1, w + 2, h + 2, gfx::MUTED);
        }
        Cover::Plain => g.frame(x, y, w, h, gfx::MUTED),
    }
    text::draw_centered_mid(
        g,
        &ui_atlas::MICH12,
        160,
        y + h + 22,
        entry.name_str(),
        gfx::TEXT,
        0,
    );
    for (i, line) in lines.iter().enumerate() {
        let at = y + h + 42 + 14 * i as i32;
        text::draw_centered_mid(g, &ui_atlas::SMALL8, 160, at, line, gfx::MUTED, 1);
    }
}
