//! Demo disc menu prototype: three directions on one PS1 guest.
//!
//! Prototype only (L2/R2 switch direction, SQUARE swaps the Public and Private
//! entry sets). Nothing here ships; it exists so each direction can be drawn
//! on the real GPU at 320x240 and measured.

#![no_std]
#![no_main]
#![allow(clippy::too_many_arguments)]

extern crate psx_rt;

mod common;
mod data;
mod dir_a;
mod dir_b;
mod dir_c;
mod ease;
mod gfx;
mod launch;
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
    let mut dir = 0u8;
    let mut sel = 0i32;
    let mut a = dir_a::State { pos: 0 };
    let mut b = dir_b::State {
        focus: [0; 16],
        scroll: 0,
    };
    let mut c = dir_c::State {
        bar: 0,
        scroll: 0,
        swap: 256,
        last: 0,
    };
    let mut launching = -1i32;

    loop {
        ctx.t = ctx.t.wrapping_add(1);
        let pad = pad_reader.poll();
        let now = ButtonState::from_bits(pad.buttons.bits());
        let pressed = |bt: u16| now.is_held(bt) && !prev.is_held(bt);
        let n = ctx.entries.len() as i32;
        if launching < 0 {
            let step_v = match dir {
                1 => dir_b::COLS,
                2 => 1,
                _ => 0,
            };
            if pressed(button::RIGHT) {
                sel = (sel + 1).min(n - 1);
            }
            if pressed(button::LEFT) {
                sel = (sel - 1).max(0);
            }
            if pressed(button::DOWN) && step_v != 0 {
                sel = (sel + step_v).min(n - 1);
            }
            if pressed(button::UP) && step_v != 0 {
                sel = (sel - step_v).max(0);
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
            if pressed(button::R2) {
                dir = (dir + 1) % 3;
            }
            if pressed(button::L2) {
                dir = (dir + 2) % 3;
            }
            if pressed(button::CROSS) && ctx.entries[sel as usize].art.is_some() {
                launching = 0;
            }
        } else {
            launching += 1;
            if launching > launch::FRAMES + AFTER_LAUNCH {
                launching = -1;
            }
        }
        prev = now;

        g.begin();
        g.clear((4, 8, 12));
        let hide = launching >= 0;
        let n = ctx.entries.len() as i32;
        let from = match dir {
            0 => {
                dir_a::update(&mut a, sel);
                launch::From {
                    cx: 160,
                    bottom: dir_a::FLOOR - 3,
                    sc: 256,
                }
            }
            1 => {
                dir_b::update(&mut b, sel, n);
                let (x, y) = dir_b::tile_origin(sel, b.scroll);
                launch::From {
                    cx: x + dir_b::TILE_W / 2,
                    bottom: y + dir_b::TILE_H,
                    sc: 160,
                }
            }
            _ => {
                dir_c::update(&mut c, sel, n);
                launch::From {
                    cx: 162 + (150 - 104) / 2 - 3 + 52,
                    bottom: 42 + 98,
                    sc: 256,
                }
            }
        };
        // Past frame 30 the veil has taken the menu to black, so it is not drawn
        // at all: the hand-off is cheaper than the menu it replaces.
        if launching < 30 {
            match dir {
                0 => dir_a::draw(&mut g, &a, &ctx, sel, hide),
                1 => dir_b::draw(&mut g, &b, &ctx, sel, hide),
                _ => dir_c::draw(&mut g, &c, &ctx, sel, hide),
            }
        }
        if launching > 0 && launching <= launch::FRAMES {
            launch::draw(&mut g, launching, from, ctx.entries[sel as usize], &ctx);
        } else if launching > launch::FRAMES {
            g.rect(0, 0, 320, 240, (0, 0, 0));
        }
        g.end();
    }
}
