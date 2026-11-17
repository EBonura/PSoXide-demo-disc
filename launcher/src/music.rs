//! The menu's music: XA songs from the launcher's own ISO, played as a
//! shuffled playlist.
//!
//! `mkdisc` puts the songs in `MUSIC1.XA`, `MUSIC2.XA` (packed by
//! `psx-audio-cook xa-pack`, whose generated table is `include!`d below), and
//! this module finds them by name in the root directory at boot. The drive
//! decodes the audio itself; the CPU's whole share is one position read every
//! few frames.
//!
//! The drive is busy while a song plays, so nothing here reads the disc during
//! one: the table, the meter data and the backdrops are all read before the
//! first note, and [`Music::stop`] pauses the drive before a program is loaded.

use playlist::{OffSlot, Playlist, Slot};
use psx_io::cd::xa_clock::SongClock;
use psx_hw::cd::xa::END_GUARD_SECTORS;
use psx_io::iso;
use psx_io::cd::xa::{DriveSpeed, Event, File, Player, Song};
use psx_io::timers::{counter, Timer};
use psx_pack::cd::{SectorReader, SECTOR_WORDS};

/// The song table `xa-pack` wrote: which file and channel each song is on, and
/// how long it runs. Song `n` here is song `n` of the disc table.
#[allow(dead_code)]
mod pack {
    include!(env!("XA_SONGS_RS"));
}

/// Frames between position reads while a song plays: ten a second, which keeps
/// the song clock within a sector or two and the end of a song within a
/// tenth of a second.
const POLL_TICKS: u32 = 6;
/// A start that has not been seen streaming by then is redone from a stop.
/// Covers a spin-up and a long seek with room to spare.
const START_TIMEOUT_TICKS: u32 = 300;
/// Frames before a start the drive refused is tried again.
const RETRY_TICKS: u32 = 60;
/// How far the drive's head runs ahead of the sound, in milliseconds. The
/// head sits a sector or two past what is audible and the drive decodes in
/// four-sector groups, so the position read is early by a few tens of
/// milliseconds. This is an estimate from the SDK's notes, not a console
/// measurement; it only shifts when the visuals land against the audio.
const HEAD_LEAD_MS: i32 = 40;
/// Sectors a second a single-speed XA file is read at.
const SECTORS_PER_SECOND: u32 = 75;

/// What the music is doing right now.
#[derive(Copy, Clone, PartialEq, Eq)]
enum State {
    /// Nothing is playing, by choice (MUSIC OFF) or because there are no songs.
    Silent,
    /// A song was started at this frame and has not been seen streaming yet.
    Starting(u32),
    /// The drive refused a start; try again at this frame.
    Retry(u32),
    /// The head is inside the song.
    Playing,
}

pub struct Music {
    player: Player,
    /// `(first LBA, sectors)` per music file, as the directory lists them.
    files: [Option<(u32, u32)>; pack::XA_FILE_COUNT],
    list: Playlist,
    clock: SongClock,
    /// Song time at the last frame, `None` while nothing is streaming.
    ms: Option<u32>,
    state: State,
    next_poll: u32,
}

impl Music {
    /// Find the music files and set the playlist up for `songs` songs, dealt
    /// from `seed`. The disc table's song count is `songs`; a pressing whose
    /// files do not match the table this launcher was built with has no music
    /// rather than the wrong music.
    pub fn locate(mut reader: SectorReader, songs: usize) -> Music {
        let mut files = [None; pack::XA_FILE_COUNT];
        let found = locate_files(&mut reader, &mut files);
        let usable = if found && songs == pack::XA_SONGS.len() {
            songs
        } else {
            0
        };
        // The root counters keep running from power on and the CD reads before
        // this call take a different time every boot, so the low bits differ.
        let seed = (counter(Timer::Timer2) as u32) << 16
            ^ counter(Timer::Timer1) as u32
            ^ counter(Timer::Timer0) as u32 * 0x9E37;
        // The reader is done with the drive; the player takes its token.
        let mut player = Player::new(reader.release());
        // The drive's own mixer has to pass the song to the SPU; 0x80 is unity.
        player.set_volume(0x80, 0x80);
        Music {
            player,
            files,
            list: Playlist::new(usable, seed, OffSlot::After),
            clock: SongClock::new(crate::TICKS_HZ).with_head_lead_ms(HEAD_LEAD_MS),
            ms: None,
            state: State::Silent,
            next_poll: 0,
        }
    }

    /// Whether the disc carries songs this launcher can play.
    pub fn available(&self) -> bool {
        self.list.count() != 0
    }

    /// Where the playlist stands.
    pub fn slot(&self) -> Slot {
        self.list.slot()
    }

    /// Whether a song is chosen but not audible yet.
    pub fn loading(&self) -> bool {
        matches!(self.list.slot(), Slot::Song(_)) && self.state != State::Playing
    }

    /// Whether the clock has a reading to run from, so the beat visuals can
    /// follow the song.
    pub fn playing(&self) -> bool {
        self.ms.is_some()
    }

    /// Song time in milliseconds at frame `now`; 0 when nothing is playing.
    pub fn song_ms(&self) -> u32 {
        self.ms.unwrap_or(0)
    }

    /// Begin at the playlist's first slot.
    pub fn begin(&mut self, now: u32) {
        self.apply(now);
    }

    /// The shoulder button forward.
    pub fn skip_forward(&mut self, now: u32) {
        self.list.skip_forward();
        self.apply(now);
    }

    /// The shoulder button back.
    pub fn skip_back(&mut self, now: u32) {
        self.list.skip_back();
        self.apply(now);
    }

    /// Pause the drive for good before a program is loaded. The song restarts
    /// nowhere: the menu is gone.
    pub fn stop(&mut self) {
        self.ms = None;
        self.state = State::Silent;
        // The next thing to use the drive is the chain-load's seek: wait for the Pause.
        self.player.stop_and_wait();
    }

    /// Once a frame: read the head every few frames, notice the end of a song,
    /// retry a start that did not take.
    pub fn frame(&mut self, now: u32) {
        // A stop sends its Pause and returns; one look a frame completes it (also in `Silent`).
        self.player.poll_stop();
        match self.state {
            State::Silent => {}
            State::Retry(at) => {
                if now.wrapping_sub(at) < u32::MAX / 2 {
                    self.apply(now);
                }
            }
            State::Starting(_) | State::Playing => {
                if now.wrapping_sub(self.next_poll) < u32::MAX / 2 {
                    self.next_poll = now.wrapping_add(POLL_TICKS);
                    let event = self.player.poll();
                    if event == Event::Finished {
                        self.list.song_finished();
                        self.apply(now);
                    } else if self.player.is_streaming() {
                        self.state = State::Playing;
                    } else if let State::Starting(since) = self.state {
                        if now.wrapping_sub(since) > START_TIMEOUT_TICKS {
                            self.apply(now);
                        }
                    }
                }
            }
        }
        // The SDK's clock runs on the tick counter between the head readings the poll takes.
        self.ms = match self.state {
            State::Playing => self.clock.update(&self.player, now),
            _ => None,
        };
    }

    /// Make the drive match the playlist's slot.
    fn apply(&mut self, now: u32) {
        self.ms = None;
        self.player.stop();
        let Slot::Song(n) = self.list.slot() else {
            self.state = State::Silent;
            return;
        };
        let Some(song) = self.song(n) else {
            self.state = State::Silent;
            return;
        };
        self.next_poll = now.wrapping_add(POLL_TICKS);
        match self.player.play(song, false) {
            Ok(()) => self.state = State::Starting(now),
            Err(_) => {
                self.state = State::Retry(now.wrapping_add(RETRY_TICKS));
            }
        }
    }

    /// Song `n` as a view of its file that ends where the song does: every
    /// song of a file is padded to the longest, and the player stops at the
    /// end of the span it is given.
    fn song(&self, n: u8) -> Option<Song> {
        let (file_index, channel, ms) = *pack::XA_SONGS.get(n as usize)?;
        let (lba, sectors) = self.files.get(file_index as usize).copied().flatten()?;
        let own = playlist::sectors_for_millis(ms, SECTORS_PER_SECOND);
        let span = own.min(sectors.saturating_sub(END_GUARD_SECTORS));
        let speed = if pack::XA_SPEED_DOUBLE {
            DriveSpeed::Double
        } else {
            DriveSpeed::Single
        };
        Some(
            File::new(
                lba,
                span + END_GUARD_SECTORS,
                pack::XA_FILE_NUMBERS[file_index as usize],
                speed,
            )
            .song(channel),
        )
    }
}

/// Look every music file up in the root directory. True when all are there at
/// the size the song table says.
fn locate_files(
    reader: &mut SectorReader,
    files: &mut [Option<(u32, u32)>; pack::XA_FILE_COUNT],
) -> bool {
    let mut sector = [0u32; SECTOR_WORDS];
    if !iso::read_root_sector(&mut sector, |lba, into| read_one(reader, lba, into)) {
        return false;
    }
    let mut all = true;
    for (slot, name) in files.iter_mut().zip(pack::XA_FILE_NAMES) {
        match iso::find_in_directory(iso::as_bytes(&sector), name) {
            Some((lba, bytes)) if bytes / 2048 == sectors_of(name) => {
                *slot = Some((lba, bytes / 2048));
            }
            _ => all = false,
        }
    }
    all
}

/// The sector count the song table gives file `name`.
fn sectors_of(name: &str) -> u32 {
    pack::XA_FILE_NAMES
        .iter()
        .position(|n| *n == name)
        .map_or(0, |i| pack::XA_FILE_SECTORS[i])
}

fn read_one(reader: &mut SectorReader, lba: u32, into: &mut [u32; SECTOR_WORDS]) -> bool {
    let ok = reader.prepare() && reader.start_read(lba) && reader.read_sector(into);
    reader.stop();
    ok
}
