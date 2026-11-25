//! The hand-off to the chosen program, through the scene core.
//!
//! Every program on the disc links on the same scene core (`psx-core`, CORE-ABI): the launcher
//! starts one with `psx_core_switch`, which loads the program's payload below the core's window,
//! checks it and jumps to it with the disc base of the program's member image. The launcher's
//! own picture stays on screen through the load (the core does not reset the GPU).
//!
//! What the launcher does itself is get the drive ready: on frame 0 of the launch animation it
//! stops the song and sends the head to the first sector the core will read, so the long seek
//! out of the music file happens while the case opens instead of after the press.
//!
//! The seek is also the launcher's last look at the drive before a switch it cannot come back
//! from: the core's fatal screen has no way back, the launcher still has one. A seek that does not
//! land, or a drive still busy after it, is not handed to the core; the launcher brings the head
//! home, says the game could not start and goes back to the shelf.
//!
//! Console 2026-11-25 (SCPH-9002, v0.59 Private burn): Half-Life stopped on the core's 0x12 right
//! after it was chosen. The emulator reproduces it with the launch seek's target unreadable
//! (emu-cd-1125, `cd-marginal`): the SeekL never completes, the old launcher handed the seeking
//! drive over after its wait, and the core's Pauses into it were refused (hwtest v2.7 on this
//! console: a Pause during a seek is refused with INT5, six of six, and the Pauses after it fail
//! too) until 0x12. Nothing here sends a Pause.

use crate::gfx::Gfx;
use crate::music::Music;
use disc_toc::Entry;
use psx_hw::cd::{CMD_SEEKL, MODE_DOUBLE_SPEED, STAT_PLAYING, STAT_READING, STAT_SEEKING};
use psx_io::periph::Cd;

/// Polls a single command may wait for its first answer. The commands answer
/// in about a millisecond; this only bounds a dead drive.
const COMMAND_SPINS: u32 = 200_000;

/// The status bits of a drive that is doing something.
const ACTIVE: u8 = STAT_PLAYING | STAT_SEEKING | STAT_READING;

/// Where the head goes when a launch seek failed: the disc's tables (`SCENES.TBL`, LBA 22), which
/// the launcher read at boot, so the drive is known to read there.
const HOME_LBA: u32 = 22;

/// What the seek is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing was started (no drive, or the drive refused).
    Idle,
    /// SeekL is running; `saved` is the IRQ enable to give back.
    Seeking { saved: u8 },
    /// The seek answered its completion.
    Landed,
    /// The drive answered the seek with an error.
    Failed,
}

/// The seek to the first sector the core will read, run in the background of the launch
/// animation.
pub struct Seek {
    cd: Cd,
    state: State,
}

impl Seek {
    /// Stop the menu song and start the seek to `entry`'s first payload sector.
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
        // play in XA mode at single speed. Sent here, the speed change happens during this seek,
        // not inside the core's first load, and the core's own SetMode changes nothing. The drive
        // is paused (the song's Pause waited out above). A refusal is harmless: the core sends
        // the mode again.
        let _ = cd.try_set_mode(MODE_DOUBLE_SPEED, COMMAND_SPINS);
        // The sector after the program's header: the core reads the payload from there and never
        // reads the header itself, so the seek proves the sector the load starts on.
        let state = seek(&mut cd, entry.exe_lba + 1);
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
            self.state = if flag == 2 { State::Landed } else { State::Failed };
        }
    }

    /// Wait the seek out, at most [`SEEK_WAIT_VBLANKS`], then for a drive with no activity bit,
    /// at most [`IDLE_WAIT_VBLANKS`]. True when the head landed and the drive is at rest: the
    /// drive can go to the core. False otherwise, with the head sent home (see [`Seek::recover`]).
    /// A launcher that never reached the drive (`Idle`) hands over as before: the core proves
    /// the drive itself.
    pub fn finish(&mut self) -> bool {
        if self.state == State::Idle {
            return true;
        }
        self.wait_out(SEEK_WAIT_VBLANKS);
        let ok = self.state == State::Landed && self.wait_idle();
        if !ok {
            self.recover();
        }
        ok
    }

    /// Run [`Seek::frame`] once a VBlank until the seek has answered, at most `vblanks`; a seek
    /// still running then is given up as failed (its IRQ enable given back).
    fn wait_out(&mut self, vblanks: u32) {
        let mut n = 0;
        while matches!(self.state, State::Seeking { .. }) && n < vblanks {
            self.frame();
            psx_rt::interrupts::wait_vblank();
            n += 1;
        }
        if let State::Seeking { saved } = self.state {
            self.cd.restore_irq_output(saved);
            self.state = State::Failed;
        }
    }

    /// Ask GetStat once a VBlank until no activity bit is set, at most [`IDLE_WAIT_VBLANKS`].
    /// A GetStat the drive refuses counts as busy. Only after the seek's own answers are in, so
    /// no GetStat lands between the SeekL's two responses.
    fn wait_idle(&mut self) -> bool {
        let mut n = 0;
        loop {
            let status = self.cd.try_status(COMMAND_SPINS);
            if let Some(&s) = status.as_ref().and_then(|r| r.bytes().first()) {
                if s & ACTIVE == 0 {
                    return true;
                }
            }
            if n == IDLE_WAIT_VBLANKS {
                return false;
            }
            psx_rt::interrupts::wait_vblank();
            n += 1;
        }
    }

    /// After a failed launch seek: send the head home with a new SetLoc and SeekL, wait that seek
    /// out and the drive to rest, each with the same bounds. A new seek is the one command here
    /// that takes the drive off a target it cannot find without a Pause; whether silicon lets a
    /// SeekL replace a running one is not measured (the emulator's model does). Whatever comes of
    /// it, the menu's player starts its songs from a stop and retries a refused start.
    fn recover(&mut self) {
        self.state = seek(&mut self.cd, HOME_LBA);
        self.wait_out(SEEK_WAIT_VBLANKS);
        if self.state == State::Landed {
            let _ = self.wait_idle();
        }
        self.state = State::Idle;
    }
}

/// SetLoc `lba` and dispatch SeekL; `Idle` when the drive did not take either.
fn seek(cd: &mut Cd, lba: u32) -> State {
    if cd.try_set_target_lba(lba, COMMAND_SPINS).is_none() {
        return State::Idle;
    }
    match cd.dispatch_command(CMD_SEEKL, &[], COMMAND_SPINS) {
        Some(saved) => State::Seeking { saved },
        None => State::Idle,
    }
}

/// The longest the switch waits for the launch seek's completion after the launch animation
/// (which gave it 72 frames already): a full-stroke seek on the console takes under a second;
/// the core gives its own seeks 10 s. A seek past this is treated as failed: the player sees
/// "could not start" and can try again, instead of the core's fatal screen.
const SEEK_WAIT_VBLANKS: u32 = 180;

/// The longest [`Seek::wait_idle`] asks: a second at 60 Hz.
const IDLE_WAIT_VBLANKS: u32 = 60;

/// Start carousel entry `index` with a core switch to its scene (`scene_layout`). Never returns
/// when the drive is handed over and the switch accepted; a refusal (a disc whose tables
/// disagree, a program on another core) is a broken pressing, shown on the core's fatal screen
/// with its code. Returns only when the launch seek failed: the "could not start" frame has been
/// shown and the drive sent home, and the caller goes back to the shelf.
pub fn boot(index: usize, entry: &Entry, scenes: &mut [u32], g: &mut Gfx, seek: &mut Seek) {
    // The one static frame the player sees until the game draws (the core keeps it).
    crate::loading::show(g, entry);
    g.wait_idle();
    if !seek.finish() {
        crate::loading::show_failed(g, entry);
        for _ in 0..FAILED_FRAMES {
            psx_rt::interrupts::wait_vblank();
        }
        return;
    }
    psx_core::link::release();
    let code = psx_core::link::switch(scene_layout::scene_of_entry(index), &[0; 4], scenes);
    psx_core::link::fatal(code)
}

/// How long "could not start" stays up before the shelf returns: two seconds at 60 Hz.
const FAILED_FRAMES: u32 = 120;
