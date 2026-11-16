//! The menu music's brain, with no hardware in it: which song plays next, and
//! where in the song the sound is.
//!
//! The shuffled order, with the MUSIC OFF slot after the last song, is the SDK's
//! `psx_fx::playlist`; what stays here is the song clock's sector arithmetic.

#![no_std]

pub use psx_fx::playlist::{OffSlot, Playlist, Slot, MAX_SONGS};

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
}
