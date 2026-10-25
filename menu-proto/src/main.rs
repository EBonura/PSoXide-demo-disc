//! Demo disc menu prototype: the Shelf direction on one PS1 guest.
//!
//! Prototype only (SQUARE swaps the Public and Private entry sets, TRIANGLE
//! the language). Nothing here ships; it exists so the menu can be drawn on
//! the real GPU at 320x240 and measured.

#![no_std]
#![no_main]
#![allow(clippy::too_many_arguments)]

extern crate psx_rt;

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

use common::Ctx;
use gfx::Gfx;
use psx_pad::{button, ButtonState, PadReader};
use psx_rt::tty;

static UI_TEX: &[u8] = include_bytes!("../assets/ui.tex");

/// Frames the prototype sits on black after a launch before resetting.
const AFTER_LAUNCH: i32 = 24;

#[no_mangle]
fn main() {
    tty::println("menu-proto: booted");
    let mut g = Gfx::new();
    g.upload_ui(UI_TEX);
    for (i, art) in data::ARTS.iter().enumerate() {
        g.upload_art(i, art);
    }

    let mut pad_reader = PadReader::port1();
    let mut prev = ButtonState::default();
    let mut ctx = Ctx {
        t: 0,
        it: false,
        entries: &data::PUBLIC,
        private: false,
    };
    let mut sel = 0i32;
    let mut shelf = shelf::State { pos: 0 };
    // Frames since the start press, or -1.
    let mut launching = -1i32;

    loop {
        ctx.t = ctx.t.wrapping_add(1);
        let pad = pad_reader.poll();
        let now = ButtonState::from_bits(pad.buttons.bits());
        let pressed = |bt: u16| now.is_held(bt) && !prev.is_held(bt);
        let n = ctx.entries.len() as i32;
        if launching < 0 {
            if pressed(button::RIGHT) {
                sel = (sel + 1).min(n - 1);
            }
            if pressed(button::LEFT) {
                sel = (sel - 1).max(0);
            }
            if pressed(button::TRIANGLE) {
                ctx.it = !ctx.it;
            }
            if pressed(button::SQUARE) || pressed(button::SELECT) {
                ctx.private = !ctx.private;
                ctx.entries = if ctx.private {
                    &data::PRIVATE
                } else {
                    &data::PUBLIC
                };
                sel = sel.min(ctx.entries.len() as i32 - 1);
            }
            if pressed(button::CROSS) && ctx.entries[sel as usize].art.is_some() {
                launching = 0;
            }
        } else if launching > launch::FRAMES + AFTER_LAUNCH {
            launching = -1;
        }
        prev = now;

        g.begin();
        g.clear((4, 8, 12));
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
