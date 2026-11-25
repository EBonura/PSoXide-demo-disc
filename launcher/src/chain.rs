//! The hand-off to the chosen program, through the scene core.
//!
//! Every program on the disc links on the same scene core (`psx-core`, CORE-ABI): the launcher
//! starts one with `psx_core_switch`, which loads the program's payload below the core's window,
//! checks it and jumps to it with the disc base of the program's member image. The launcher's
//! own picture stays on screen through the load (the core does not reset the GPU).
//!
//! What the launcher does itself is get the drive ready: on frame 0 of the launch animation it
//! stops the song and sends the head to the program's first sector, so the long seek out of the
//! music file happens while the case opens instead of after the press.

use crate::gfx::Gfx;
use crate::music::Music;
use disc_toc::Entry;
use psx_hw::cd::{CMD_SEEKL, MODE_DOUBLE_SPEED, STAT_PLAYING, STAT_READING, STAT_SEEKING};
use psx_io::periph::Cd;

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
/// The core proves the drive stopped before its first command of the switch, whatever this
/// left it doing, and then seeks to the same sector itself, so a seek that fails or never
/// finishes costs only the time it was meant to save.
pub struct Seek {
    cd: Cd,
    state: State,
    /// The drive answered the SetLoc: there is a drive to wait for.
    aimed: bool,
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
        // The core's first command of the switch is SetMode 0x80 (data, double speed); the songs
        // play in XA mode at single speed. Sent here, the speed change happens during this seek
        // instead of inside the core's first load, so the core's own SetMode changes nothing.
        // Console 2026-11-25 (SCPH-9002): Half-Life, the smallest switch load on the disc (36
        // sectors), stopped on 0x12 with its load and Pause within about 0.25 s of that change.
        // The drive is paused here (the song's Pause waited out above), so this is not a command
        // to a seeking drive. A refusal is harmless: the core sends the mode again.
        let _ = cd.try_set_mode(MODE_DOUBLE_SPEED, COMMAND_SPINS);
        let aimed = cd.try_set_target_lba(entry.exe_lba, COMMAND_SPINS).is_some();
        let state = if aimed {
            match cd.dispatch_command(CMD_SEEKL, &[], COMMAND_SPINS) {
                Some(saved) => State::Seeking { saved },
                None => State::Idle,
            }
        } else {
            State::Idle
        };
        Seek { cd, state, aimed }
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

    /// Wait the seek out (its completion answered, or refused), at most [`SEEK_WAIT_VBLANKS`], then
    /// give the drive up with nothing pending. Review 1.3: the switch must not start while a seek
    /// the launcher started is still running.
    pub fn finish(&mut self) {
        let mut vblanks = 0;
        while matches!(self.state, State::Seeking { .. }) && vblanks < SEEK_WAIT_VBLANKS {
            self.frame();
            psx_rt::interrupts::wait_vblank();
            vblanks += 1;
        }
        if let State::Seeking { saved } = self.state {
            self.cd.restore_irq_output(saved);
            self.state = State::Idle;
        }
        if self.aimed {
            self.wait_idle();
        }
    }

    /// Ask the drive its status, once a VBlank, until no activity bit is set, at most
    /// [`IDLE_WAIT_VBLANKS`]; when the bound runs out the drive is handed over as it is.
    ///
    /// The seek's completion is not proof the drive is still: the core's first command of the
    /// switch is a probe, and a drive it finds active gets a Pause. hwtest v2.7 on this console:
    /// a Pause while the drive seeks is refused (INT5, six of six) and the Pauses after it fail
    /// too, which the core's three attempts turn into 0x12. Only GetStat is sent here, never a
    /// Pause; a GetStat the drive refuses mid-seek just counts as busy. Polled only after the
    /// seek's own completion was taken (or its wait ran out), so no GetStat lands between the
    /// SeekL's two answers.
    fn wait_idle(&mut self) {
        const ACTIVE: u8 = STAT_PLAYING | STAT_SEEKING | STAT_READING;
        let mut vblanks = 0;
        while vblanks < IDLE_WAIT_VBLANKS {
            let status = self.cd.try_status(COMMAND_SPINS);
            if let Some(&s) = status.as_ref().and_then(|r| r.bytes().first()) {
                if s & ACTIVE == 0 {
                    return;
                }
            }
            psx_rt::interrupts::wait_vblank();
            vblanks += 1;
        }
    }
}

/// The longest [`Seek::wait_idle`] asks: a second at 60 Hz. The kernel's own Pause proof allows
/// about three seconds before 0x12; a drive still busy after this one is the kernel's to handle,
/// as it was before the wait existed.
const IDLE_WAIT_VBLANKS: u32 = 60;

/// The longest the switch waits for the launch seek: a full-stroke seek on the console takes
/// under a second, and the seek has had the 72 frames of the launch animation already.
const SEEK_WAIT_VBLANKS: u32 = 120;

/// Start carousel entry `index` with a core switch to its scene (`scene_layout`). Never
/// returns when the switch is accepted; a refusal (a disc whose tables disagree, a program on
/// another core) is a broken pressing, shown on the core's fatal screen with its code. There is
/// no way back: no program on the disc has a scene that starts the launcher.
pub fn boot(index: usize, entry: &Entry, scenes: &mut [u32], g: &mut Gfx, seek: &mut Seek) -> ! {
    // The one static frame the player sees until the game draws (the core keeps it).
    crate::loading::show(g, entry);
    g.wait_idle();
    // The drive: the launch seek waited out, then told to the core (the launcher drove it with its
    // own commands), which proves it stopped before its first command of the switch.
    seek.finish();
    psx_core::link::release();
    let code = psx_core::link::switch(scene_layout::scene_of_entry(index), &[0; 4], scenes);
    psx_core::link::fatal(code)
}
