//! The menu music's brain, with no hardware in it: which song plays next, and
//! where in the song the sound is.
//!
//! # Order
//!
//! The songs play in a shuffled order and none repeats until every song has
//! been through: a [`Playlist`] deals the songs out of one permutation and
//! reshuffles when it is spent. The slot after the last song is
//! [`Slot::Off`] (MUSIC OFF): the shoulder buttons walk through every song
//! and then silence, so muting needs no button of its own. A song that ends
//! by itself goes straight on to the next song and never lands on `Off`.
//!
//! # Time
//!
//! The XA player reports the drive head's position a sector at a time (13 ms),
//! only when it is polled, and the head runs a little ahead of the sound.
//! [`SongClock`] turns that into a time that moves every frame: it counts
//! frames between polls and pulls itself toward each new reading instead of
//! jumping to it, so the beat visuals do not stutter on a late poll.

#![no_std]

/// Songs a playlist can hold; the disc table's limit.
pub const MAX_SONGS: usize = 8;

/// What the music is doing at a playlist position.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Song `n`, as numbered by the disc table.
    Song(u8),
    /// MUSIC OFF.
    Off,
}

/// A shuffled order over `count` songs, plus the off slot after it.
#[derive(Copy, Clone, Debug)]
pub struct Playlist {
    order: [u8; MAX_SONGS],
    count: u8,
    /// `0..count` is a song, `count` is the off slot.
    pos: u8,
    rng: u32,
}

impl Playlist {
    /// A playlist over `count` songs (at most [`MAX_SONGS`]) dealt from
    /// `seed`, positioned on its first song.
    pub fn new(count: usize, seed: u32) -> Self {
        let count = count.min(MAX_SONGS) as u8;
        let mut list = Playlist {
            order: [0; MAX_SONGS],
            count,
            pos: 0,
            // xorshift has one fixed point; never start on it.
            rng: if seed == 0 { 0x9E37_79B9 } else { seed },
        };
        list.shuffle(None);
        list
    }

    /// Number of songs.
    pub fn count(&self) -> u8 {
        self.count
    }

    /// Where the playlist stands.
    pub fn slot(&self) -> Slot {
        if self.pos >= self.count {
            Slot::Off
        } else {
            Slot::Song(self.order[self.pos as usize])
        }
    }

    /// The shoulder button forward: the next song, then MUSIC OFF, then a
    /// fresh shuffle from its first song.
    pub fn skip_forward(&mut self) -> Slot {
        if self.count == 0 {
            return Slot::Off;
        }
        if self.pos < self.count {
            self.pos += 1;
        } else {
            let last = self.order[self.count as usize - 1];
            self.shuffle(Some(last));
            self.pos = 0;
        }
        self.slot()
    }

    /// The shoulder button back: the previous song of this shuffle; before the
    /// first, MUSIC OFF; from MUSIC OFF, the shuffle's last song.
    pub fn skip_back(&mut self) -> Slot {
        if self.count == 0 {
            return Slot::Off;
        }
        self.pos = if self.pos == 0 {
            self.count
        } else {
            self.pos - 1
        };
        self.slot()
    }

    /// The song ended by itself: on to the next, reshuffling (without
    /// repeating the song just heard) when this shuffle is spent.
    pub fn song_finished(&mut self) -> Slot {
        if self.count == 0 {
            return Slot::Off;
        }
        if self.pos + 1 < self.count {
            self.pos += 1;
        } else {
            let last = self.order[self.count as usize - 1];
            self.shuffle(Some(last));
            self.pos = 0;
        }
        self.slot()
    }

    fn next_random(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// Deal a new permutation (Fisher-Yates). With `avoid_first`, the song
    /// that just finished never opens the new one.
    fn shuffle(&mut self, avoid_first: Option<u8>) {
        let n = self.count as usize;
        for (i, slot) in self.order.iter_mut().enumerate().take(n) {
            *slot = i as u8;
        }
        for i in (1..n).rev() {
            let j = (self.next_random() % (i as u32 + 1)) as usize;
            self.order.swap(i, j);
        }
        if n > 1 && Some(self.order[0]) == avoid_first {
            self.order.swap(0, n - 1);
        }
    }
}

/// `frames * 1000 / ticks_per_second`, wrapped to `u32`, in 32-bit operations:
/// whole seconds of frames and a remainder below `ticks_per_second`, so the
/// remainder's product is at most `ticks_per_second * 1000` (a frame rate, not
/// above four million). Equal to the `u64` expression for every input.
pub const fn millis_of_frames(frames: u32, ticks_per_second: u32) -> u32 {
    let whole = frames / ticks_per_second;
    let part = frames % ticks_per_second;
    whole
        .wrapping_mul(1000)
        .wrapping_add(part * 1000 / ticks_per_second)
}

/// `ceil(millis * sectors_per_second / 1000)`, wrapped to `u32`, in 32-bit
/// operations: the sectors of a song `millis` long on a drive reading
/// `sectors_per_second` (75 or 150). Whole seconds of the length and a
/// remainder below a thousand milliseconds keep every product in a word.
/// Equal to the `u64` expression for every input.
pub const fn sectors_for_millis(millis: u32, sectors_per_second: u32) -> u32 {
    let whole = millis / 1000;
    let part = millis % 1000;
    whole
        .wrapping_mul(sectors_per_second)
        .wrapping_add((part * sectors_per_second).div_ceil(1000))
}

/// Newest reading further than this from the running time is a seek or a
/// restart, not jitter: take it whole.
const SNAP_MS: u32 = 80;

/// A song time that advances every frame between drive readings.
#[derive(Copy, Clone, Debug)]
pub struct SongClock {
    ticks_per_second: u32,
    anchor_ms: u32,
    anchor_tick: u32,
    running: bool,
}

impl SongClock {
    /// A stopped clock counting frames at `ticks_per_second`.
    pub const fn new(ticks_per_second: u32) -> Self {
        SongClock {
            ticks_per_second,
            anchor_ms: 0,
            anchor_tick: 0,
            running: false,
        }
    }

    /// Back to a stopped clock: a new song, or none.
    pub fn stop(&mut self) {
        self.running = false;
        self.anchor_ms = 0;
    }

    /// Whether the clock has a reading to run from.
    pub fn running(&self) -> bool {
        self.running
    }

    /// Song time at frame `tick`; 0 while stopped.
    pub fn now(&self, tick: u32) -> u32 {
        if !self.running {
            return 0;
        }
        let frames = tick.wrapping_sub(self.anchor_tick);
        self.anchor_ms
            .saturating_add(millis_of_frames(frames, self.ticks_per_second))
    }

    /// The drive says the song is `song_ms` in, at frame `tick`.
    pub fn observe(&mut self, tick: u32, song_ms: u32) {
        if self.running {
            let predicted = self.now(tick);
            let error = song_ms.abs_diff(predicted);
            self.anchor_ms = if error > SNAP_MS {
                song_ms
            } else {
                // A quarter of the way, so one late poll moves the beat a
                // few milliseconds rather than a sector.
                if song_ms >= predicted {
                    predicted + error / 4
                } else {
                    predicted - error / 4
                }
            };
        } else {
            self.anchor_ms = song_ms;
            self.running = true;
        }
        self.anchor_tick = tick;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_32_bit_time_conversions_match_the_wide_forms() {
        let mut state = 0x2545_f491u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let edges = [0, 1, 2, 59, 60, 61, 74, 75, 76, 149, 150, 151, 999, 1000, 1001];
        for rate in [30u32, 50, 60, 75, 150] {
            for frames in edges
                .into_iter()
                .chain([u32::MAX, u32::MAX - 1, 1 << 31, (1 << 31) - 1])
                .chain((0..100_000).map(|_| next()))
            {
                let wide = (u64::from(frames) * 1000 / u64::from(rate)) as u32;
                assert_eq!(millis_of_frames(frames, rate), wide, "{frames} at {rate}");
            }
            for frames in 0..200_000 {
                let wide = (u64::from(frames) * 1000 / u64::from(rate)) as u32;
                assert_eq!(millis_of_frames(frames, rate), wide, "{frames} at {rate}");
            }
        }
        for rate in [75u32, 150] {
            for millis in edges
                .into_iter()
                .chain([u32::MAX, u32::MAX - 1, 1 << 31, (1 << 31) - 1])
                .chain((0..100_000).map(|_| next()))
                .chain(0..300_000)
            {
                let wide = (u64::from(millis) * u64::from(rate)).div_ceil(1000) as u32;
                assert_eq!(sectors_for_millis(millis, rate), wide, "{millis} at {rate}");
            }
        }
    }

    fn play_through(list: &mut Playlist, songs: usize) -> [u8; MAX_SONGS] {
        let mut heard = [0u8; MAX_SONGS];
        let Slot::Song(first) = list.slot() else {
            panic!("starts on a song")
        };
        heard[0] = first;
        for slot in heard.iter_mut().take(songs).skip(1) {
            let Slot::Song(n) = list.song_finished() else {
                panic!("a finished song never lands on off")
            };
            *slot = n;
        }
        heard
    }

    #[test]
    fn every_song_plays_once_before_any_repeats() {
        for seed in 1..200u32 {
            let mut list = Playlist::new(6, seed.wrapping_mul(0x9E37_79B1));
            let heard = play_through(&mut list, 6);
            let mut seen = [false; MAX_SONGS];
            for &n in &heard[..6] {
                assert!(!seen[n as usize], "seed {seed} repeated song {n}: {heard:?}");
                seen[n as usize] = true;
            }
        }
    }

    #[test]
    fn a_new_shuffle_never_opens_with_the_song_just_heard() {
        for seed in 1..500u32 {
            let mut list = Playlist::new(6, seed.wrapping_mul(2_654_435_761));
            let mut previous = 255u8;
            for step in 0..60 {
                let Slot::Song(n) = (if step == 0 {
                    list.slot()
                } else {
                    list.song_finished()
                }) else {
                    panic!("song_finished never lands on off")
                };
                assert_ne!(n, previous, "seed {seed} step {step} repeated back to back");
                previous = n;
            }
        }
    }

    #[test]
    fn different_seeds_deal_different_orders() {
        let a = play_through(&mut Playlist::new(6, 1), 6);
        let b = play_through(&mut Playlist::new(6, 2), 6);
        let c = play_through(&mut Playlist::new(6, 3), 6);
        assert!(a != b || b != c, "three seeds, one order: {a:?}");
    }

    #[test]
    fn the_shoulder_buttons_walk_every_song_then_off() {
        let mut list = Playlist::new(4, 7);
        let mut songs = [false; 4];
        let Slot::Song(first) = list.slot() else {
            panic!()
        };
        songs[first as usize] = true;
        for _ in 0..3 {
            let Slot::Song(n) = list.skip_forward() else {
                panic!("three more songs before off")
            };
            assert!(!songs[n as usize]);
            songs[n as usize] = true;
        }
        assert_eq!(list.skip_forward(), Slot::Off);
        assert!(matches!(list.skip_forward(), Slot::Song(_)), "off then a fresh shuffle");
    }

    #[test]
    fn back_from_the_first_song_is_off_and_back_from_off_is_the_last_song() {
        let mut list = Playlist::new(3, 11);
        let first = list.slot();
        assert_eq!(list.skip_back(), Slot::Off);
        let last = list.skip_back();
        assert!(matches!(last, Slot::Song(_)));
        assert_ne!(last, first);
        list.skip_back();
        list.skip_back();
        assert_eq!(list.slot(), first);
    }

    #[test]
    fn a_single_song_just_repeats_and_no_songs_is_off() {
        let mut one = Playlist::new(1, 5);
        assert_eq!(one.slot(), Slot::Song(0));
        assert_eq!(one.song_finished(), Slot::Song(0));
        let mut none = Playlist::new(0, 5);
        assert_eq!(none.slot(), Slot::Off);
        assert_eq!(none.song_finished(), Slot::Off);
        assert_eq!(none.skip_forward(), Slot::Off);
        assert_eq!(Playlist::new(40, 5).count() as usize, MAX_SONGS);
    }

    #[test]
    fn the_clock_runs_between_readings_and_snaps_on_a_seek() {
        let mut clock = SongClock::new(60);
        assert_eq!(clock.now(100), 0, "stopped reads zero");
        clock.observe(100, 5_000);
        assert_eq!(clock.now(100), 5_000);
        assert_eq!(clock.now(160), 6_000, "sixty frames is a second");
        clock.observe(160, 6_040);
        let t = clock.now(160);
        assert!((6_001..6_040).contains(&t), "pulled part way: {t}");
        clock.observe(220, 40_000);
        assert_eq!(clock.now(220), 40_000, "a seek is taken whole");
        clock.stop();
        assert!(!clock.running());
        assert_eq!(clock.now(300), 0);
    }

    #[test]
    fn the_clock_survives_the_frame_counter_wrapping() {
        let mut clock = SongClock::new(60);
        clock.observe(u32::MAX - 29, 1_000);
        assert_eq!(clock.now(30), 2_000, "sixty frames across the wrap");
    }
}
