//! Find the beat grid of a raw CD-DA track.
//!
//! The menu pulses its visuals on the beat, driven by `psx_io::cdda::CddaClock`,
//! which reports milliseconds into the playing track. To turn that into beats
//! the launcher needs a tempo and a phase, and both have to be accurate: a
//! tempo off by 1 BPM drifts several beats out of step over a four-minute
//! track, which reads as broken rather than as an effect.
//!
//! So this measures rather than trusts the tempo the artist quoted. It builds
//! an onset-strength envelope, scores every (tempo, phase) pair on a fine grid
//! by how much onset energy lands on the beats, and prints the winner in the
//! form `mkdisc --menu-beat` wants.
//!
//!     disc-tools beatgrid audio/*.cdda
//!
//! Input is raw 44.1 kHz 16-bit stereo PCM, which is what the disc carries.
//!
//! The arithmetic follows the Python original operation for operation, so the
//! tempo it picks cannot differ on a tie or a rounding: the winner is the
//! first of the highest scores, and the scores are summed in the same order.

use std::fs;

use crate::util::{Error, Result};

const SAMPLE_RATE: i64 = 44100;
/// About 5.8 ms, fine enough to place a beat inside one frame at 60 Hz.
const HOP: usize = 256;

/// Python 3.12+ sums floats with Neumaier compensation, and the original
/// leaned on `sum()`. Doing the same keeps the mean bit-identical.
fn float_sum(values: &[f64]) -> f64 {
    let mut total = 0.0f64;
    let mut compensation = 0.0f64;
    for &x in values {
        let t = total + x;
        if total.abs() >= x.abs() {
            compensation += (total - t) + x;
        } else {
            compensation += (x - t) + total;
        }
        total = t;
    }
    if compensation != 0.0 && compensation.is_finite() {
        total += compensation;
    }
    total
}

/// Half-wave rectified energy difference, one value per HOP samples.
fn onset_envelope(raw: &[u8]) -> Vec<f64> {
    let frames = raw.len() / 4;
    let sample = |n: usize| i16::from_le_bytes([raw[2 * n], raw[2 * n + 1]]) as i64;
    let mut energy = Vec::new();
    let mut start = 0;
    while start + HOP < frames {
        let mut total: i64 = 0;
        for i in start..start + HOP {
            total += sample(2 * i).abs() + sample(2 * i + 1).abs();
        }
        energy.push(total as f64 / (2 * HOP) as f64);
        start += HOP;
    }
    (1..energy.len())
        .map(|i| (energy[i] - energy[i - 1]).max(0.0))
        .collect()
}

/// Total onset strength landing on a grid at `bpm` starting `phase_frames` in.
fn score(envelope: &[f64], rate: f64, bpm: f64, phase_frames: usize) -> f64 {
    let spacing = rate * 60.0 / bpm;
    let mut total = 0.0;
    let mut beat = 0u64;
    loop {
        let at = phase_frames as f64 + beat as f64 * spacing;
        let index = (at + 0.5) as usize;
        if index >= envelope.len() {
            return total;
        }
        // A beat rarely lands exactly on a frame boundary, so take the best of
        // the frame and its neighbours.
        let lo = index.saturating_sub(1);
        let hi = (index + 2).min(envelope.len());
        let mut best = envelope[lo];
        for &value in &envelope[lo + 1..hi] {
            if value > best {
                best = value;
            }
        }
        total += best;
        beat += 1;
    }
}

/// `(milli_bpm, phase_ms)` for one track's PCM.
fn beat_grid(raw: &[u8]) -> Result<(i64, i64)> {
    let mut envelope = onset_envelope(raw);
    ensure!(!envelope.is_empty(), "track is too short to measure");
    let rate = SAMPLE_RATE as f64 / HOP as f64;
    let mean = float_sum(&envelope) / envelope.len() as f64;
    for e in envelope.iter_mut() {
        *e = (*e - mean).max(0.0);
    }

    // Coarse pass over plausible drum and bass tempos, then refine around the
    // winner. A single fine sweep would be minutes of work for the same answer.
    let mut best: Option<(f64, f64, usize)> = None;
    for tenths in 1600..1900 {
        let bpm = tenths as f64 / 10.0;
        let spacing = rate * 60.0 / bpm;
        let mut phase = 0;
        while phase <= spacing as usize {
            let value = score(&envelope, rate, bpm, phase);
            if best.is_none_or(|b| value > b.0) {
                best = Some((value, bpm, phase));
            }
            phase += 2;
        }
    }
    let Some((_, coarse_bpm, coarse_phase)) = best else {
        bail!("no tempo candidates")
    };

    let centre = (coarse_bpm * 1000.0) as i64;
    let mut best: Option<(f64, i64, usize)> = None;
    let mut milli = centre - 400;
    while milli < centre + 401 {
        let bpm = milli as f64 / 1000.0;
        for phase in coarse_phase.saturating_sub(4)..coarse_phase + 5 {
            let value = score(&envelope, rate, bpm, phase);
            if best.is_none_or(|b| value > b.0) {
                best = Some((value, milli, phase));
            }
        }
        milli += 10;
    }
    let Some((_, milli_bpm, phase_frames)) = best else {
        bail!("no tempo candidates")
    };
    let phase_ms = (phase_frames as i64 * HOP as i64 * 1000) as f64 / SAMPLE_RATE as f64;
    Ok((milli_bpm, phase_ms as i64))
}

/// Python's `"%-28s %7.3f BPM, first beat %4d ms   --menu-beat %d:%d"`.
fn format_line(path: &str, milli_bpm: i64, phase_ms: i64) -> String {
    format!(
        "{path:<28} {:7.3} BPM, first beat {phase_ms:4} ms   --menu-beat {milli_bpm}:{phase_ms}",
        milli_bpm as f64 / 1000.0
    )
}

pub fn run(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        println!("usage: disc-tools beatgrid <file.cdda>...");
        return Ok(1);
    }
    for path in args {
        let raw = fs::read(path).map_err(|e| Error(format!("{path}: {e}")))?;
        let (milli_bpm, phase_ms) = beat_grid(&raw)?;
        println!("{}", format_line(path, milli_bpm, phase_ms));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click track: one loud frame block every `period` hops over silence.
    fn click_track(bpm: f64, seconds: usize) -> Vec<u8> {
        let frames = SAMPLE_RATE as usize * seconds;
        let mut raw = vec![0u8; frames * 4];
        let spacing = SAMPLE_RATE as f64 * 60.0 / bpm;
        let mut at = 0.0;
        while (at as usize) + 600 < frames {
            for i in at as usize..at as usize + 600 {
                raw[4 * i..4 * i + 2].copy_from_slice(&20000i16.to_le_bytes());
                raw[4 * i + 2..4 * i + 4].copy_from_slice(&20000i16.to_le_bytes());
            }
            at += spacing;
        }
        raw
    }

    #[test]
    fn finds_the_tempo_of_a_click_track() {
        let (milli, phase) = beat_grid(&click_track(172.0, 40)).unwrap();
        assert!((milli - 172_000).abs() <= 300, "got {milli}");
        assert!(phase < 40, "first beat at {phase} ms");
    }

    #[test]
    fn output_line_matches_the_python_format() {
        assert_eq!(
            format_line("audio/knuckle-dust.cdda", 176010, 359),
            "audio/knuckle-dust.cdda      176.010 BPM, first beat  359 ms   --menu-beat 176010:359"
        );
        assert_eq!(
            format_line("a", 1000, 5),
            format!(
                "{:<28}   1.000 BPM, first beat    5 ms   --menu-beat 1000:5",
                "a"
            )
        );
    }

    #[test]
    fn float_sum_is_compensated_like_python() {
        // Plain left-to-right addition of these gives 0.0; Python 3.12+ gives 1.0.
        assert_eq!(float_sum(&[1.0, 1e100, 1.0, -1e100]), 2.0);
        assert_eq!(float_sum(&[0.1; 10]), 1.0);
    }

    #[test]
    fn short_input_is_an_error_not_a_panic() {
        assert!(beat_grid(&[0u8; 1000]).is_err());
    }
}
