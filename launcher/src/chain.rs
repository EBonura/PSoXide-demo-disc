//! The hand-off to the chosen program.
//!
//! The chain-load itself cannot happen in the launcher: every PSoXide program
//! links to `0x80010000`, which is where the launcher is running, so streaming
//! a game in would overwrite the code doing the streaming. Instead the `loader`
//! crate is built separately at its own base address (`chainload-layout`),
//! embedded here as raw bytes, copied there and jumped to.
//!
//! What the launcher does itself is get the drive ready: on frame 0 of the
//! launch animation it stops the song and sends the head to the program's first
//! sector, so the long seek out of the music file happens while the case opens
//! instead of after the screen has gone to the loader.

use crate::gfx::Gfx;
use crate::music::Music;
use disc_toc::Entry;
use psx_hw::cd::CMD_SEEKL;
use psx_io::periph::Cd;
use psx_rt::tty;

/// The chain-load blob, linked at `LOADER_BASE` from `loader/loader.ld`.
const LOADER_BLOB: &[u8] = include_bytes!(env!("LOADER_BLOB"));

/// Where the blob runs, from the one place the loader link and mkdisc read too.
const LOADER_BASE: u32 = chainload_layout::LOADER_BASE;

/// Blob budget. Overrunning it would walk into whatever loads above the blob,
/// so check rather than trust.
const LOADER_LIMIT: usize = chainload_layout::LOADER_BUDGET as usize;

/// Polls a single command may wait for its first answer. The commands answer
/// in about a millisecond; this only bounds a dead drive.
const COMMAND_SPINS: u32 = 200_000;

/// What the seek is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing was started (no drive, or the drive refused).
    Idle,
    /// SeekL is running; `saved` is the IRQ enable to give back.
    Seeking { saved: u8 },
}

/// The seek to the program's first sector, run in the background of the
/// launch animation.
///
/// It leaves the drive where [`psx_chainloader`]'s first read expects it: spun
/// up, idle and with no answer pending. The loader aims at the same sector
/// itself, so a seek that fails or never finishes costs only the time it was
/// meant to save.
pub struct Seek {
    cd: Cd,
    state: State,
}

impl Seek {
    /// Stop the menu song and start the seek to `entry`'s first sector.
    ///
    /// Takes the drive over: the song must be the only thing that was using it
    /// and nothing may use it until [`Seek::finish`].
    pub fn start(entry: &Entry, music: &mut Music) -> Self {
        // Pause the drive for good before a program is loaded. The song
        // restarts nowhere: the menu is going away. This waits for the pause
        // to land, which takes a few frames on the console.
        music.stop();
        // SAFETY: the music player was the only owner of the drive and has
        // just paused it; it issues no further commands.
        let mut cd = unsafe { Cd::steal() };
        let aimed = cd.try_set_target_lba(entry.exe_lba, COMMAND_SPINS).is_some();
        let state = if aimed {
            match cd.dispatch_command(CMD_SEEKL, &[], COMMAND_SPINS) {
                Some(saved) => State::Seeking { saved },
                None => State::Idle,
            }
        } else {
            State::Idle
        };
        Seek { cd, state }
    }

    /// Once a frame: take in whatever the drive has said so far. Answers are
    /// read and acknowledged here so nothing is left pending; the first is the
    /// command's acknowledge, the second says the head has arrived.
    pub fn frame(&mut self) {
        let State::Seeking { saved } = self.state else {
            return;
        };
        // 3 = acknowledge, 2 = complete, 5 = refused or failed.
        let flag = self.cd.irq_flag_value() & 7;
        if flag == 0 {
            return;
        }
        self.cd.discard_response();
        self.cd.acknowledge_irq(flag);
        if flag == 2 || flag == 5 {
            self.cd.restore_irq_output(saved);
            self.state = State::Idle;
        }
    }

    /// Give the drive up: acknowledge anything still pending. The loader's
    /// first command cancels a seek that is still under way.
    pub fn finish(&mut self) {
        if let State::Seeking { saved } = self.state {
            self.cd.restore_irq_output(saved);
            self.state = State::Idle;
        }
    }
}

/// Copy the chain-load blob high and jump to it. Never returns while the disc
/// is readable; the blob paints the screen red and stops if it is not.
pub fn boot(entry: &Entry, g: &mut Gfx, seek: &mut Seek) -> ! {
    assert!(LOADER_BLOB.len() <= LOADER_LIMIT, "loader blob too large");
    tty::println("launcher: chain-loading");

    // Let the GPU finish before the blob resets it out from under whatever is
    // still on screen, and hand the drive over with nothing pending.
    g.wait_idle();
    seek.finish();

    // SAFETY: `LOADER_BASE` is clear of every program's payload (mkdisc enforces
    // that) and of the stack, so nothing live is being overwritten. The
    // blob is position-dependent and linked for exactly this address.
    unsafe {
        core::ptr::copy_nonoverlapping(
            LOADER_BLOB.as_ptr(),
            LOADER_BASE as *mut u8,
            LOADER_BLOB.len(),
        );
        psx_rt::cache::flush_instruction_cache();
        let blob: unsafe extern "C" fn(u32, u32, u32, u32) -> ! =
            core::mem::transmute(LOADER_BASE as usize);
        blob(
            entry.exe_lba,
            entry.lba_offset,
            entry.cdda_track_base,
            entry.payload_fnv,
        )
    }
}
