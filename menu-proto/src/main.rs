//! Demo disc menu prototype: the Shelf direction on one PS1 guest.
//!
//! Prototype only (TRIANGLE switches the language; the edition is a build-time
//! feature, `private`). Nothing here ships; it exists so the menu can be drawn on
//! the real GPU at 320x240 and measured.

#![no_std]
#![no_main]
#![allow(clippy::too_many_arguments)]

extern crate psx_rt;

mod attract;
mod common;
mod data;
mod ease;
mod gfx;
mod launch;
mod shelf;
mod text;
#[path = "../assets/ui_atlas.rs"]
#[allow(dead_code)]
mod ui_atlas;

use attract::{Attract, Variant};
use common::Ctx;
use gfx::Gfx;
use psx_pad::{button, ButtonState, PadReader};
use psx_rt::{tty, Peripherals};

static UI_TEX: &[u8] = include_bytes!("../assets/ui.tex");

/// Which attract mode this build shows (see the `only_tour` and `only_floor`
/// features); the shipping choice is both together.
#[cfg(feature = "only_tour")]
const VARIANT: Variant = Variant::Tour;
#[cfg(feature = "only_floor")]
const VARIANT: Variant = Variant::Floor;
#[cfg(not(any(feature = "only_tour", feature = "only_floor")))]
const VARIANT: Variant = Variant::Both;

/// Frames the prototype sits on black after a launch before resetting.
const AFTER_LAUNCH: i32 = 24;

#[no_mangle]
fn main() {
    tty::println("menu-proto: booted");
    let peripherals = Peripherals::take().expect("peripherals are taken once");
    let mut port = peripherals.controller_port;
    let mut g = Gfx::new(peripherals.gpu_dma);
    g.upload_ui(UI_TEX);
    for (i, art) in data::ARTS.iter().enumerate() {
        g.upload_art(i, art);
    }
    attract::upload_backs();

    let mut pad_reader = PadReader::port1();
    let mut prev = ButtonState::default();
    let mut ctx = Ctx {
        t: 0,
        it: false,
        entries: data::EDITION,
    };
    let mut sel = 0i32;
    let mut shelf = shelf::State { pos: 0 };
    // Frames since the start press, or -1.
    let mut launching = -1i32;
    // Frames with no button down, and the attract mode once they run out.
    let mut idle = 0u32;
    let mut attract: Option<Attract> = None;

    loop {
        ctx.t = ctx.t.wrapping_add(1);
        let pad = pad_reader.poll_on(&mut port);
        let now = ButtonState::from_bits(pad.buttons.bits());
        let pressed = |bt: u16| now.is_held(bt) && !prev.is_held(bt);
        let n = ctx.entries.len() as i32;
        // Any button ends the attract mode at once, on the case it was showing,
        // and is not also taken as a menu command.
        let mut woke = false;
        if let Some(a) = attract.as_ref() {
            if pad.buttons.bits() != 0 {
                sel = a.selection() as i32;
                shelf.pos = sel * 256;
                attract = None;
                woke = true;
            }
        }
        if pad.buttons.bits() != 0 {
            idle = 0;
        } else {
            idle = idle.saturating_add(1);
        }
        if launching < 0 && !woke {
            if pressed(button::RIGHT) {
                sel = (sel + 1).min(n - 1);
            }
            if pressed(button::LEFT) {
                sel = (sel - 1).max(0);
            }
            if pressed(button::TRIANGLE) {
                ctx.it = !ctx.it;
            }
            if pressed(button::CROSS) && ctx.entries[sel as usize].art.is_some() {
                launching = 0;
            }
            // SQUARE starts the attract mode at once (the prototype's shortcut
            // for looking at it); otherwise it starts after IDLE_FRAMES.
            if attract.is_none() && (pressed(button::SQUARE) || idle == attract::IDLE_FRAMES) {
                attract = Some(Attract::new(VARIANT, ctx.entries, sel as usize));
            }
        } else if launching > launch::FRAMES + AFTER_LAUNCH {
            launching = -1;
        }
        prev = now;

        g.begin();
        g.clear((4, 8, 12));
        if let Some(a) = attract.as_mut() {
            a.update(attract::recorded_bands(ctx.t));
            a.draw(&mut g, &ctx);
            g.end();
            continue;
        }
        shelf::update(&mut shelf, sel);
        // Past frame 30 the veil has taken the menu to black, so it is not drawn
        // at all: the hand-off is cheaper than the menu it replaces.
        if launching < 30 {
            shelf::draw(&mut g, &shelf, &ctx, sel, launching);
        }
        if (0..=launch::FRAMES).contains(&launching) {
            let from = launch::From {
                cx: 160,
                bottom: shelf::FLOOR - 3,
                sc: 256,
            };
            launch::draw(&mut g, launching, from, ctx.entries[sel as usize], &ctx);
        } else if launching > launch::FRAMES {
            g.rect(0, 0, 320, 240, (0, 0, 0));
        }
        g.end();
        if launching >= 0 {
            launching += 1;
        }
    }
}
