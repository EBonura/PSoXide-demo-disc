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

mod paint;

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

    // The screen comes up immediately: dark base plus a progress strip, so
    // a photo of a hang says which stage it died in even without a panel.
    paint::setup();
    paint::rect(0, 0, 320, 240, paint::RED_BASE);
    paint::show();

    let mut reader = SectorReader::new();
    let mut header = [0u32; SECTOR_WORDS];

    // Two attempts. The first silicon burn failed every chain-load, and the
    // second (instrumented) one showed prepare failing and an instant retry
    // reaching the header read: the drive is still winding down from the
    // menu's CD-DA when the first commands arrive, and the old settle delay
    // was an empty loop the optimizer deleted. The first failure paints its
    // panel (photograph it), waits a real two seconds, and retries from
    // scratch; the second failure paints a second panel below and halts.
    let mut attempt: u32 = 0;
    let exe = loop {
        match unsafe { try_load(&mut reader, exe_lba, &mut header) } {
            Ok(exe) => break exe,
            Err((stage, detail)) => {
                fail_panel(attempt, stage, detail, reader.diag());
                attempt += 1;
                if attempt >= 2 {
                    halt();
                }
                settle_delay();
            }
        }
    };

    unsafe { flush_cache() };
    // Seven blocks means the cache flush returned and the jump is the very
    // next instruction: anything wrong past this point is the game's own
    // first moments, not the load.
    progress(7);
    // Scratchpad probe, drawn as a second-row block under block 1: green
    // means the scratchpad answered a write/readback after the flush,
    // white means it did not -- the fingerprint of a cache-control
    // restore swallowed while the cache was isolated (see flush_cache's
    // ordering note). The game inherits whichever machine this saw.
    unsafe {
        let probe = 0x1F80_0000 as *mut u32;
        core::ptr::write_volatile(probe, 0xC0DE_5EED);
        let alive = core::ptr::read_volatile(probe) == 0xC0DE_5EED;
        paint::rect(
            8,
            22,
            14,
            10,
            if alive { paint::GREEN } else { paint::WHITE },
        );
    }
    unsafe { enter(exe.pc0, exe.gp0, exe.sp, lba_offset, cdda_track_base) }
}

/// Mark load stage `n` (1-based) as passed: a green block in the top strip.
fn progress(n: i16) {
    paint::rect(8 + (n - 1) * 18, 8, 14, 10, paint::GREEN);
}

struct LoadedExe {
    pc0: u32,
    gp0: u32,
    sp: u32,
}

/// Load stages, doubling as the fail panel's block count: 1 prepare,
/// 2 start_read, 3 header sector, 4 magic, 5 bounds, 6 payload sector
/// (detail = failing sector index), 7 panic.
unsafe fn try_load(
    reader: &mut SectorReader,
    exe_lba: u32,
    header: &mut [u32; SECTOR_WORDS],
) -> Result<LoadedExe, (u32, u32)> {
    unsafe {
        if !reader.prepare() {
            return Err((1, 0));
        }
        progress(1);
        if !reader.start_read(exe_lba) {
            reader.stop();
            return Err((2, exe_lba));
        }
        progress(2);
        if !reader.read_sector(header) {
            reader.stop();
            return Err((3, exe_lba));
        }
        progress(3);
    }
    if header[0] != EXE_MAGIC[0] || header[1] != EXE_MAGIC[1] {
        unsafe { reader.stop() };
        return Err((4, header[0]));
    }
    progress(4);

    let pc0 = header[HDR_PC0];
    let gp0 = header[HDR_GP0];
    let t_addr = header[HDR_T_ADDR];
    let t_size = header[HDR_T_SIZE];
    let sp = header[HDR_SP_BASE].wrapping_add(header[HDR_SP_OFFSET]);

    // The payload must not reach this blob; `mkdisc` rejects such a game at
    // build time, so a failure here means the disc and the blob disagree.
    if t_addr < 0x8001_0000 || t_addr.saturating_add(t_size) > loader_base() {
        return Err((5, t_addr));
    }
    progress(5);

    // Payload sectors follow the header sector contiguously, and `read_sector`
    // continues the same ReadN stream, so no reseek is needed. The strip's
    // right side is a coarse loading bar, one step per 16 sectors.
    let sectors = t_size.div_ceil(SECTOR_BYTES);
    let mut dst = t_addr as *mut [u32; SECTOR_WORDS];
    for sector in 0..sectors {
        if !unsafe { reader.read_sector(&mut *dst) } {
            unsafe { reader.stop() };
            return Err((6, sector));
        }
        dst = unsafe { dst.add(1) };
        if sector % 16 == 0 {
            let done = (sector * 200 / sectors.max(1)) as i16;
            paint::rect(112, 8, done.clamp(1, 200), 10, paint::WHITE);
        }
    }
    unsafe { reader.stop() };
    progress(6);
    Ok(LoadedExe { pc0, gp0, sp })
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
        // Disable every DMA channel but keep the BIOS's priority ladder.
        // The third debug burn proved silicon cares about the difference:
        // with DPCR fully zeroed, re-enabling channel 3 alone (enable bit,
        // priority nibble 0) left the CD DMA transferring nothing, and
        // every chain-loaded header arrived as all zeros with no drive
        // error. Standalone programs inherit 0x07654321 from the BIOS and
        // the identical reader code works there; hand the next program
        // the same baseline. (The emulator does not model priorities, so
        // only a burn could catch this.)
        psx_io::write32(0x1F80_10F0, 0x0765_4321); // DPCR
        // GP1(00h): reset the GPU (display off, FIFO cleared, defaults).
        psx_io::write32(0x1F80_1814, 0);
    }
    // Clear cop0 SR.IEc (bit 0) so no interrupt fires between here and the
    // game's own setup. Shift the bit out and back rather than masking with a
    // register: MIPS-I `andi` zero-extends, and letting the allocator pick a
    // mask register lands on $at, which the assembler reserves.
    //
    // The nop after mfc0 is load-bearing: MFC0 has a one-instruction
    // load-delay hazard on the R3000, so without it the srl reads the STALE
    // $8 (whatever the caller left there) and writes it into SR. That
    // exact failure shipped once: the launcher's cache flush leaves
    // 0xFFFE0000 in $8, which landed in SR with BEV set and sent every
    // interrupt to the ROM vector. See psx-rt's enable_cpu_interrupts for
    // the same hazard note.
    unsafe {
        core::arch::asm!(
            "mfc0 $8, $12",
            "nop",
            "srl  $8, $8, 1",
            "sll  $8, $8, 1",
            "mtc0 $8, $12",
            "nop",
            out("$8") _,
            options(nostack, preserves_flags),
        );
    }
}

// Direct instruction-cache invalidation, open-coded here so the blob does
// not link `psx-rt` (which would bring a second `_start` and panic handler
// with it).
//
// This replaces a BIOS A(44h) FlushCache tail call. The flush is the last
// thing standing between a loaded payload and a running game: the game's
// code lands at 0x80010000, which is exactly where the launcher was
// executing from moments earlier, so those cache lines hold launcher
// instructions. Miss the invalidation and `jr pc0` re-executes the
// launcher instead of the game, leaving the loader's own screen up --
// which is precisely what the console showed with five green stages and
// a full payload bar.
//
// The BIOS call was never proven on silicon in this position. This
// sequence is: it is the routine psx-rt uses, and the launcher boots and
// runs on the console with it. Steps, per the documented recipe: jump to
// this code's KSEG1 alias so fetches bypass the cache being cleared, put
// the cache-control port in tag-test mode with the i-cache enabled,
// isolate the cache (COP0 SR bit 16) so stores hit tags instead of
// memory, clear one tag per 16-byte line across the 4 KiB cache, then
// DROP ISOLATION FIRST (SR = 0, interrupts still off), restore the
// normal 0x1E988 cache-control value, then the caller's SR.
//
// The restore order is load-bearing and matches the BIOS routine (and
// psx-rt after its 2026-07-31 fix): with IsC still set, whether a store
// reaches the CPU-internal cache-control port or is swallowed by the
// isolated cache is undocumented. A swallowed restore leaves cache
// control at 0x804 -- tag-test latched, SCRATCHPAD UNMAPPED -- which is
// exactly the machine every chain-loaded game would then inherit: seven
// green blocks, full payload bar, dead game, and an emulator that
// forgives it. The scratchpad probe after the flush call makes the next
// burn answer this on screen either way.
core::arch::global_asm!(
    r#"
    .set noreorder
    .section .text.loader_flush_cache
    .globl __loader_flush_cache
__loader_flush_cache:
    la    $8, .Lloader_flush_body
    lui   $9, 0x2000
    or    $8, $8, $9
    jr    $8
    nop

.Lloader_flush_body:
    mfc0  $10, $12
    nop
    lui   $8, 0xfffe
    ori   $9, $zero, 0x0804
    sw    $9, 0x0130($8)
    lui   $9, 0x0001
    mtc0  $9, $12
    nop
    nop
    or    $9, $zero, $zero
    ori   $11, $zero, 0x1000
.Lloader_flush_line:
    sw    $zero, 0($9)
    addiu $9, $9, 0x0010
    bne   $9, $11, .Lloader_flush_line
    nop
    mtc0  $zero, $12
    nop
    nop
    lui   $9, 0x0001
    ori   $9, $9, 0xe988
    sw    $9, 0x0130($8)
    mtc0  $10, $12
    nop
    nop
    jr    $31
    nop
    .set reorder
    "#
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

/// The diagnostic panel: stage as a count of white blocks, an alignment
/// ruler, then the caller's detail word and the reader's diag word as bit
/// rows. Attempt 0 paints the upper half, attempt 1 the lower, so both
/// survive on screen together.
fn fail_panel(attempt: u32, stage: u32, detail: u32, diag: u32) {
    let y0 = 36 + (attempt as i16) * 104;
    for i in 0..stage.min(8) as i16 {
        paint::rect(8 + i * 30, y0, 22, 22, paint::WHITE);
    }
    paint::ruler(y0 + 24);
    paint::bits_rows(y0 + 32, detail);
    paint::bits_rows(y0 + 66, diag);
}

/// Give the drive time to settle before the retry: roughly two seconds of
/// GPUSTAT reads. Volatile MMIO reads, so unlike a plain spin loop the
/// optimizer cannot delete it (the first debug burn proved it will).
fn settle_delay() {
    for _ in 0..1_500_000u32 {
        unsafe { core::ptr::read_volatile(0x1F80_1814 as *const u32) };
    }
}

fn halt() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    paint::setup();
    paint::rect(0, 0, 320, 240, paint::RED_BASE);
    fail_panel(1, 7, 0, 0);
    paint::show();
    halt()
}
