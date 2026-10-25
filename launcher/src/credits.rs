//! The credits screen: who made the music, where to find everything else, and
//! a QR code for the link hub. Opened from the footer's SELECT hint; the
//! Shelf's floor and header stay, the card stands on them.
//!
//! The music is here by licence (CC BY 4.0 wants the author, the work, the
//! licence and a link), so the attribution is always on this screen in full;
//! it does not page.

use crate::common::*;
use crate::gfx::*;
use crate::shelf;
use crate::text;
use crate::ui_atlas::*;

/// Panel the card sits on.
const PANEL: (i32, i32, i32, i32) = (20, 52, 280, 150);
/// Left edge of the text column and the top of its first row.
const TEXT_X: i32 = 36;
const LINKS_TOP: i32 = 62;
const LINK_PITCH: i32 = 13;
const CREDIT_PITCH: i32 = 11;

/// Where each platform mark is in the strip, and the handle shown beside it.
const LINKS: [(i32, &str); 6] = [
    (0, "EBonura/PSoXide"),
    (1, "bonnie-studios"),
    (2, "@_bonniestudios"),
    (3, "@izzy88izzy"),
    (4, "bonniestudios"),
    (5, "@magikAAAAArp"),
];

/// Marks in the strip (for the upload).
pub const ICON_COUNT: i32 = 6;

/// The music attribution, one line each. Not translated: a credit names its
/// author and its licence as written.
const MUSIC_CREDIT: [&str; 4] = [
    "Music: Mulula VGM Loops, CC BY 4.0",
    "Ambient Jungle Vol 1, Vol 2 and",
    "Ambient Jungle D&B Vol 3",
    "mululavgmloops.itch.io",
];

const QR_MODULES: i32 = 37;
const QR_QUIET: i32 = 2;
const QR_SCALE: i32 = 2;
const QR_SIZE: i32 = (QR_MODULES + QR_QUIET * 2) * QR_SCALE;
/// Exact 37x37 module grid from the supplied QR, one row per low 37 bits.
const QR_ROWS: [u64; QR_MODULES as usize] = [
    136752566399,
    70202754881,
    100365685853,
    100053188189,
    99925316701,
    70064700225,
    136723125631,
    398509312,
    31607192039,
    58141150508,
    19069816641,
    97668523363,
    95946071262,
    99100424620,
    53167849275,
    48038650609,
    129944075734,
    4917952804,
    91750251395,
    84261004338,
    104221637876,
    73689287716,
    59362993863,
    110133073121,
    61732027871,
    97477476516,
    72002531959,
    78020666754,
    100145545206,
    399991568,
    136541063515,
    69995382033,
    100345143286,
    100210234204,
    100241999265,
    70032520081,
    136491338767,
];

/// Draw the screen.
pub fn draw(g: &mut Gfx, ctx: &Ctx) {
    shelf::backdrop(g);
    header(g, ctx);
    footer(g, ctx);

    let title = if ctx.it { "CREDITI E LINK" } else { "CREDITS AND LINKS" };
    text::draw_centered_mid(g, &MICH12, 160, 38, title, TEXT, 0);

    // The card: a dark glass panel with the case brackets.
    let (px, py, pw, ph) = PANEL;
    let (top, bot) = ((9, 20, 27), (4, 11, 15));
    g.quad_g([(px, py), (px + pw, py), (px, py + ph), (px + pw, py + ph)], [top, top, bot, bot]);
    g.frame(px, py, pw, ph, RING);
    g.quad_g_add(
        [(px, py), (px + pw * 3 / 5, py), (px, py + ph / 2), (px + pw / 4, py + ph / 2)],
        [(14, 24, 28), (0, 0, 0), (0, 0, 0), (0, 0, 0)],
    );

    // Links, each behind the mark of its platform.
    for (row, (slot, handle)) in LINKS.iter().enumerate() {
        let y = LINKS_TOP + row as i32 * LINK_PITCH;
        g.link_icon(*slot, TEXT_X, y - 1, MUTED);
        text::draw_mid(g, &SMALL8, TEXT_X + ICON_CELL + 6, y + 5, handle, TEXT, 0);
    }
    // The music credit under them.
    let credit_top = LINKS_TOP + LINKS.len() as i32 * LINK_PITCH + 10;
    g.rect(TEXT_X, credit_top - 5, 150, 1, (22, 38, 46));
    for (row, line) in MUSIC_CREDIT.iter().enumerate() {
        let tint = if row == 0 { ACCENT } else { MUTED };
        text::draw_mid(
            g,
            &SMALL8,
            TEXT_X,
            credit_top + 4 + row as i32 * CREDIT_PITCH,
            line,
            tint,
            0,
        );
    }

    // The QR, on a white square with its quiet zone.
    let qx = px + pw - 16 - QR_SIZE;
    let qy = py + (ph - QR_SIZE) / 2 - 6;
    g.rect(qx, qy, QR_SIZE, QR_SIZE, (255, 255, 255));
    for (row, bits) in QR_ROWS.iter().enumerate() {
        let mut col = 0;
        while col < QR_MODULES {
            if (bits >> (QR_MODULES - 1 - col)) & 1 == 0 {
                col += 1;
                continue;
            }
            let start = col;
            while col < QR_MODULES && (bits >> (QR_MODULES - 1 - col)) & 1 != 0 {
                col += 1;
            }
            g.rect(
                qx + (QR_QUIET + start) * QR_SCALE,
                qy + (QR_QUIET + row as i32) * QR_SCALE,
                (col - start) * QR_SCALE,
                QR_SCALE,
                (0, 0, 0),
            );
        }
    }
    let caption = if ctx.it { "Inquadra per tutti i link" } else { "Scan for every link" };
    text::draw_centered_mid(g, &SMALL8, qx + QR_SIZE / 2, qy + QR_SIZE + 10, caption, MUTED, 0);
}
