//! Chain-load blob: read a PSX-EXE off the disc and jump to it.
//!
//! Every PSoXide program links to the same load address (`psoxide.ld`:
//! `LOAD_ADDR = 0x80010000`), so the launcher cannot stream a game into RAM
//! itself: it would overwrite its own `.text` mid-copy. This blob is linked
//! separately at `LOADER_BASE` (see `loader.ld`) and embedded in the launcher
//! as raw bytes. The launcher copies it high, flushes the instruction cache,
//! and jumps to offset 0 -- from there nothing below `LOADER_BASE` is live,
//! so the target payload can land wherever it wants.
//!
//! Only the header LBA is passed in. Load address, payload size, entry point
//! and stack all come out of the target's own PSX-EXE header, so the launcher
//! needs no per-game build-time knowledge.

#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

use psx_pack::cd::{SectorReader, SECTOR_WORDS};

const SECTOR_BYTES: u32 = (SECTOR_WORDS * 4) as u32;

// PSX-EXE header word offsets (see `psoxide.ld`).
const HDR_PC0: usize = 0x10 / 4;
const HDR_GP0: usize = 0x14 / 4;
const HDR_T_ADDR: usize = 0x18 / 4;
const HDR_T_SIZE: usize = 0x1C / 4;
const HDR_SP_BASE: usize = 0x30 / 4;
const HDR_SP_OFFSET: usize = 0x34 / 4;

const EXE_MAGIC: [u32; 2] = [0x582D_5350, 0x4558_4520]; // "PS-X EXE"

/// Read the PSX-EXE at `exe_lba` into its load address and run it.
///
/// `lba_offset` and `cdda_track_base` describe where the target's own disc
/// image landed on this one; they are handed to the target's `_start` in the
/// argument registers, where `psx_io::disc_base` picks them up. See
/// `psx-io/src/disc_base.rs`.
///
/// # Safety
/// Overwrites RAM from the target's load address onwards, including the
/// caller. Never returns.
#[link_section = ".text.loader_entry"]
#[no_mangle]
pub unsafe extern "C" fn loader_entry(exe_lba: u32, lba_offset: u32, cdda_track_base: u32) -> ! {
    unsafe { quiesce() };

    let mut reader = SectorReader::new();
    let mut header = [0u32; SECTOR_WORDS];

    let ok = unsafe {
        reader.prepare() && reader.start_read(exe_lba) && reader.read_sector(&mut header)
    };
    if !ok || header[0] != EXE_MAGIC[0] || header[1] != EXE_MAGIC[1] {
        fail();
    }

    let pc0 = header[HDR_PC0];
    let gp0 = header[HDR_GP0];
    let t_addr = header[HDR_T_ADDR];
    let t_size = header[HDR_T_SIZE];
    let sp = header[HDR_SP_BASE].wrapping_add(header[HDR_SP_OFFSET]);

    // The payload must not reach this blob; `mkdisc` rejects such a game at
    // build time, so a failure here means the disc and the blob disagree.
    if t_addr < 0x8001_0000 || t_addr.saturating_add(t_size) > loader_base() {
        fail();
    }

    // Payload sectors follow the header sector contiguously, and `read_sector`
    // continues the same ReadN stream, so no reseek is needed.
    let sectors = t_size.div_ceil(SECTOR_BYTES);
    let mut dst = t_addr as *mut [u32; SECTOR_WORDS];
    for _ in 0..sectors {
        if !unsafe { reader.read_sector(&mut *dst) } {
            fail();
        }
        dst = unsafe { dst.add(1) };
    }
    unsafe { reader.stop() };

    unsafe { flush_cache() };
    unsafe { enter(pc0, gp0, sp, lba_offset, cdda_track_base) }
}

/// This blob's link base, read from the linker script rather than repeated
/// here so the two cannot drift.
#[inline(always)]
fn loader_base() -> u32 {
    loader_entry as *const () as u32
}

/// Put the hardware back roughly where the BIOS leaves it before a game
/// boots: interrupts masked, DMA channels off, GPU reset. The launcher has
/// been driving the GPU and vblank IRQs, and a game's `_start` does not
/// expect to inherit that.
unsafe fn quiesce() {
    unsafe {
        // Mask + acknowledge every interrupt source.
        psx_io::write32(0x1F80_1074, 0); // I_MASK
        psx_io::write32(0x1F80_1070, 0); // I_STAT
        // Disable every DMA channel (the CD reader re-enables ch3 itself).
        psx_io::write32(0x1F80_10F0, 0); // DPCR
        // GP1(00h): reset the GPU (display off, FIFO cleared, defaults).
        psx_io::write32(0x1F80_1814, 0);
    }
    // Clear cop0 SR.IEc (bit 0) so no interrupt fires between here and the
    // game's own setup. Shift the bit out and back rather than masking with a
    // register: MIPS-I `andi` zero-extends, and letting the allocator pick a
    // mask register lands on $at, which the assembler reserves.
    unsafe {
        core::arch::asm!(
            "mfc0 $8, $12",
            "srl  $8, $8, 1",
            "sll  $8, $8, 1",
            "mtc0 $8, $12",
            "nop",
            out("$8") _,
            options(nostack, preserves_flags),
        );
    }
}

// BIOS A(44h) `FlushCache`, as a tail-call trampoline. Same 3-instruction
// shape as `psx-rt`'s `bios_calls!`, open-coded here so the blob does not link
// `psx-rt` (which would bring a second `_start` and panic handler with it).
core::arch::global_asm!(
    ".set noreorder",
    ".section .text.loader_flush_cache",
    ".globl __loader_flush_cache",
    "__loader_flush_cache:",
    "  la $8, 0xA0",
    "  jr $8",
    "  li $9, 0x44",
);

extern "C" {
    #[link_name = "__loader_flush_cache"]
    fn flush_cache();
}

/// Seed GP / SP / the disc-base handover and jump to the game's entry point.
/// Nothing after this touches the stack, which is about to belong to the
/// target.
///
/// `$a0..$a2` are the MIPS ABI's first three arguments, which is exactly how
/// `psx-rt`'s `_start` declares them, so the handover needs no assembly on the
/// receiving side.
unsafe fn enter(pc0: u32, gp0: u32, sp: u32, lba_offset: u32, cdda_track_base: u32) -> ! {
    unsafe {
        core::arch::asm!(
            "move $28, {gp}",
            "move $29, {sp}",
            "move $30, $0",
            "jr   {pc}",
            "nop",
            gp = in(reg) gp0,
            sp = in(reg) sp,
            pc = in(reg) pc0,
            in("$4") psx_io::disc_base::HANDOFF_MAGIC,
            in("$5") lba_offset,
            in("$6") cdda_track_base,
            options(noreturn),
        );
    }
}

/// Nothing sane is left to do: the disc is unreadable or the EXE is not one.
/// Paint the screen red through a freshly reset GPU so the failure is visible
/// on a TV, then stop.
fn fail() -> ! {
    unsafe {
        psx_io::write32(0x1F80_1814, 0x0300_0001); // GP1(03h): display off
        psx_io::write32(0x1F80_1810, 0x0200_0040); // GP0(02h): fill rect, dark red
        psx_io::write32(0x1F80_1810, 0x0000_0000); //   at (0,0)
        psx_io::write32(0x1F80_1810, 0x0100_0280); //   640x256
        psx_io::write32(0x1F80_1814, 0x0300_0000); // GP1(03h): display on
    }
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fail()
}
