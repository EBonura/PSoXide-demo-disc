//! What the launcher is doing, published for the test harness.
//!
//! The disc gate (psoxide-test, `harness-core/DISC.md` "LAUNCHER_STATE") reads this
//! block from RAM by its symbol name to walk the shelf and launch each program
//! closed-loop: it waits for a phase, presses until `selected` names the program,
//! then waits for the launch. It is in every build, shipping included, because the
//! gate judges the image that ships. The launcher only ever writes it.

use crate::common::Screen;

/// Before the first presented frame (and while the disc table is unreadable).
pub const BOOT: u32 = 0;
/// The "needs an analog controller" notice: port 1 holds no DualShock in analog mode.
pub const NOTICE: u32 = 1;
/// The shelf, accepting input.
pub const SHELF: u32 = 2;
/// The attract tour.
pub const ATTRACT: u32 = 3;
/// The credits screen.
pub const CREDITS: u32 = 4;
/// The launch animation has started for `launching`.
pub const LAUNCH: u32 = 5;
/// The jump to the loader blob is next; the last value the launcher writes.
pub const HAND_OFF: u32 = 6;

/// No program is being launched.
pub const NOT_LAUNCHING: u32 = 0xFFFF_FFFF;

/// The published words, little-endian `u32` at fixed offsets.
#[repr(C)]
pub struct LauncherState {
    /// +0: one of the phase constants above.
    pub phase: u32,
    /// +4: index of the focused program, 0-based, in the disc's table order.
    pub selected: u32,
    /// +8: number of programs on the shelf (the table's entry count).
    pub entries: u32,
    /// +12: port 1 as last polled: 0 none, 1 digital, 2 analog.
    pub pad: u32,
    /// +16: table index of the program being launched (from [`LAUNCH`] on), else
    /// [`NOT_LAUNCHING`].
    pub launching: u32,
    /// +20: launcher frames since boot (diagnostic only).
    pub frames: u32,
}

/// The block the harness reads by name. Written only by [`publish`] and [`hand_off`].
#[no_mangle]
#[used]
pub static mut LAUNCHER_STATE: LauncherState = LauncherState {
    phase: BOOT,
    selected: 0,
    entries: 0,
    pad: 0,
    launching: NOT_LAUNCHING,
    frames: 0,
};

/// One frame's view of the menu, as the main loop knows it.
pub struct Frame {
    /// Programs in the disc table (0 when it was unreadable).
    pub entries: usize,
    /// The menu takes input (port 1 is a DualShock in analog mode).
    pub analog_ok: bool,
    /// Port 1 at the last poll.
    pub pad: psx_pad::AnalogRequirement,
    /// The screen the menu shows.
    pub screen: Screen,
    /// The attract tour is running.
    pub attract: bool,
    /// The focused program's table index.
    pub selected: i32,
    /// The launch animation has started for `selected`.
    pub launching: bool,
    /// Frames since boot.
    pub frames: u32,
}

/// Write this frame's state. Called once a frame by the main loop, after input.
pub fn publish(f: &Frame) {
    let phase = if f.entries == 0 {
        BOOT
    } else if f.launching {
        LAUNCH
    } else if !f.analog_ok {
        NOTICE
    } else if f.screen == Screen::Credits {
        CREDITS
    } else if f.attract {
        ATTRACT
    } else {
        SHELF
    };
    // `AnalogRequirement` is Analog, DigitalOnly, Absent in that order: 2, 1, 0 here.
    let pad = 2 - f.pad as u32;
    let selected = f.selected as u32;
    let launching = if f.launching { selected } else { NOT_LAUNCHING };
    // SAFETY: single-threaded and polled; nothing in the launcher reads the block, and
    // the harness reads it from outside the machine. Volatile so every word is stored.
    unsafe {
        let s = core::ptr::addr_of_mut!(LAUNCHER_STATE);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*s).phase), phase);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*s).selected), selected);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*s).entries), f.entries as u32);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*s).pad), pad);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*s).launching), launching);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*s).frames), f.frames);
    }
}

/// Mark the hand-off, just before the jump to the loader blob.
pub fn hand_off() {
    // SAFETY: as `publish`.
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(LAUNCHER_STATE.phase), HAND_OFF);
    }
}
