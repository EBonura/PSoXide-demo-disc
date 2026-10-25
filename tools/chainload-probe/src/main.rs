// SPDX-License-Identifier: GPL-2.0-or-later
//! `chainload-probe`: the payload that tells whether the chain loader handed
//! over a quiet CD drive.
//!
//! A Pause answers twice and on the console the second answer (INT2, "the drive
//! has stopped") comes 1.2 to 3.9 million system clocks after the first, so a
//! loader that stops its last read with a plain Pause and jumps leaves that INT2
//! to land inside the next program. The emulator's silicon Pause model does the
//! same. This program runs as the chain-loaded payload and checks, in order:
//!
//! 1. at entry: no CD interrupt flag latched in the controller, no CD bit in
//!    `I_STAT`, and the status register shows no response FIFO or busy bit;
//! 2. over a commandless window of `WINDOW_CLOCKS` system clocks: no response
//!    ever arrives (a late INT2 would show up here, 1.2 to 3.9 million clocks
//!    after the loader's Pause);
//! 3. an immediate read: `SectorReader::prepare`, a read of the image's first
//!    volume descriptor, a proved stop, with the reader's diagnostics clean.
//!
//! Results go to the TTY; the last line is `chainload-probe: PASS` or `FAIL`.
//! The program is image-relative: it reads sector 16 of its own image through
//! `psx_io::disc_base`, so it must be pressed as an image, not a bare EXE.

#![no_std]
#![no_main]

extern crate psx_rt;

use psx_io::cd::reader::{SectorReader, SECTOR_WORDS};
use psx_io::periph::Cd;
use psx_io::timers::{self, Timer};
use psx_io::irq;

/// System clocks the commandless watch lasts: over twice the slowest Pause
/// completion measured on the console (3.9 million), so a late INT2 cannot
/// slip past it.
const WINDOW_CLOCKS: u32 = 8_000_000;
/// Root counter 2 in system-clock/8 mode: one count per 8 system clocks.
const TIMER_SYSCLK_DIV8: u16 = 0x0200;
/// The ISO 9660 primary volume descriptor.
const PVD_LBA: u32 = 16;

static mut BAD: u32 = 0;

fn check(label: &str, ok: bool, detail: u32) {
    psx_rt::tty::print("chainload-probe: ");
    psx_rt::tty::print(label);
    psx_rt::tty::print(if ok { ": ok " } else { ": BAD " });
    hex(detail);
    psx_rt::tty::print("\n");
    if !ok {
        // SAFETY: single threaded.
        unsafe { BAD += 1 };
    }
}

fn hex(value: u32) {
    let mut digits = *b"0x00000000";
    for i in 0..8 {
        let nibble = (value >> (28 - 4 * i)) & 0xF;
        digits[2 + i] = b"0123456789abcdef"[nibble as usize];
    }
    psx_rt::tty::print(core::str::from_utf8(&digits).unwrap_or("?"));
}

/// Watch the controller, sending nothing, for `WINDOW_CLOCKS` system clocks.
/// Returns the first response flag seen (0 for none) and the system clocks
/// elapsed when the watch ended.
fn watch(cd: &mut Cd) -> (u8, u32) {
    timers::set_mode(Timer::Timer2, TIMER_SYSCLK_DIV8);
    let mut last = timers::counter(Timer::Timer2);
    let mut clocks: u32 = 0;
    while clocks < WINDOW_CLOCKS {
        let now = timers::counter(Timer::Timer2);
        clocks += u32::from(now.wrapping_sub(last)) * 8;
        last = now;
        let flag = cd.irq_flag_value();
        if flag != 0 {
            return (flag, clocks);
        }
    }
    (0, clocks)
}

#[no_mangle]
fn main() {
    let Some(peripherals) = psx_rt::Peripherals::take() else {
        return;
    };
    let mut cd: Cd = peripherals.cd;

    // 1. Entry state, before this program touches the controller.
    let entry_flag = cd.irq_flag_value();
    let entry_status = cd.status_register();
    let entry_istat = irq::pending();
    check("entry: no CD interrupt flag latched", entry_flag == 0, entry_flag as u32);
    check(
        "entry: no CD bit in I_STAT",
        entry_istat & (1 << psx_hw::irq::source::CDROM) == 0,
        entry_istat,
    );
    // Informational: the status register at entry (index, busy, FIFO bits).
    check("entry: status register (info)", true, entry_status as u32);

    // 2. A commandless watch: nothing may arrive.
    let (stray_flag, stray_at) = watch(&mut cd);
    check("window: no response arrives with no command sent", stray_flag == 0, stray_flag as u32);
    if stray_flag != 0 {
        check("window: the stray response came after (clocks)", false, stray_at);
    }

    // 3. An immediate read, as any program would start.
    let mut reader = SectorReader::with_cd(cd);
    let mut sector = [0u32; SECTOR_WORDS];
    let prepared = reader.prepare();
    check("read: prepare", prepared, reader.diagnostics());
    let started = prepared && reader.start_read(PVD_LBA);
    check("read: start_read", started, reader.diagnostics());
    let got = started && reader.read_sector(&mut sector);
    let bytes = sector[0].to_le_bytes();
    let volume = sector[1].to_le_bytes();
    check(
        "read: the volume descriptor arrives (01 'CD001')",
        got && bytes[0] == 1
            && bytes[1] == b'C'
            && bytes[2] == b'D'
            && bytes[3] == b'0'
            && volume[0] == b'0',
        sector[0],
    );
    let stopped = reader.stop_proved();
    check("read: stop_proved", stopped.is_ok(), reader.diagnostics());

    // 4. Positive control: the same watch after a PLAIN stop() must see the late
    // INT2, or this program could not tell a quiet drive from a deaf one.
    let mut ok = reader.prepare() && reader.start_read(PVD_LBA + 40);
    let mut n = 0;
    while ok && n < 12 {
        ok = reader.read_sector(&mut sector);
        n += 1;
    }
    reader.stop();
    let (late_flag, late_at) = watch(reader.cd_mut());
    check("control: 12 sectors read, plain stop()", ok, n);
    check("control: a plain stop() leaves INT2 pending (flag)", late_flag == 2, late_flag as u32);
    check("control: ... arriving this many clocks after the watch began (info)", true, late_at);

    // 4b. The same with a bare Pause command and no SectorReader cleanup.
    let mut ok = reader.prepare() && reader.start_read(PVD_LBA + 60);
    let mut n = 0;
    while ok && n < 12 {
        ok = reader.read_sector(&mut sector);
        n += 1;
    }
    let ack = reader.cd_mut().try_command(0x09, &[], 131_072);
    let acked = ack.is_some();
    let (raw_flag, raw_at) = watch(reader.cd_mut());
    check("control: bare Pause acknowledged", ok && acked, n);
    check("control: bare Pause, flag seen in the watch (info)", true, raw_flag as u32);
    check("control: bare Pause, clocks until it (info)", true, raw_at);

    // 5. The status a GetStat reports right after a plain stop(). On the console
    // it reads 0x22 ("still reading") at the Pause's acknowledge and 0x02 some
    // 230,000 clocks later; the cd-command-log timestamps of the Pause and this
    // GetStat give the time stop() spent waiting.
    let mut ok = reader.prepare() && reader.start_read(PVD_LBA + 80);
    let mut n = 0;
    while ok && n < 12 {
        ok = reader.read_sector(&mut sector);
        n += 1;
    }
    reader.stop();
    let status = reader
        .cd_mut()
        .try_status(131_072)
        .and_then(|r| r.bytes().first().copied());
    check("control: GetStat after a plain stop() (info)", ok, status.map_or(0xFFFF, u32::from));

    // SAFETY: single threaded.
    let bad = unsafe { BAD };
    psx_rt::tty::println(if bad == 0 {
        "chainload-probe: PASS"
    } else {
        "chainload-probe: FAIL"
    });
    loop {}
}
